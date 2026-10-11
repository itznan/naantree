use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

fn run_cmd(dir: &Path, bin: &str, args: &[&str]) -> (i32, String, String) {
    let out = Command::new(bin)
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("Failed to run {} {:?}: {}", bin, args, e));
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    let code = out.status.code().unwrap_or(-1);
    (code, stdout, stderr)
}

#[test]
fn test_concurrency_10_agents() {
    let binary_path = PathBuf::from(env!("CARGO_BIN_EXE_naantree"));
    let temp_dir = std::env::temp_dir().join(format!(
        "naantree_concurrency_{}_{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis()
    ));

    fs::create_dir_all(&temp_dir).unwrap();

    // 1. Initialize git repo
    run_cmd(&temp_dir, "git", &["init", "-b", "main"]);
    run_cmd(&temp_dir, "git", &["config", "user.name", "Tester"]);
    run_cmd(
        &temp_dir,
        "git",
        &["config", "user.email", "tester@example.com"],
    );

    // Initial commit
    fs::write(temp_dir.join("init.txt"), "init\n").unwrap();
    run_cmd(&temp_dir, "git", &["add", "init.txt"]);
    run_cmd(&temp_dir, "git", &["commit", "-m", "initial commit"]);

    let naantree_bin = binary_path.to_str().unwrap();

    // Coordinator joins and sets up plan & 10 tasks
    let (code, out, err) = run_cmd(&temp_dir, naantree_bin, &["join", "--tags", "coord"]);
    assert_eq!(code, 0, "Coordinator join failed: {}\n{}", out, err);
    assert!(
        out.contains("role=coordinator"),
        "First agent must be coordinator"
    );

    let (code, out, err) = run_cmd(
        &temp_dir,
        naantree_bin,
        &[
            "--as",
            "agent-1",
            "plan",
            "init",
            "--goal",
            "Test Concurrency",
        ],
    );
    assert_eq!(code, 0, "Plan init failed: {}\n{}", out, err);

    // Create 10 tasks with distinct paths
    fs::create_dir_all(temp_dir.join("work")).unwrap();
    for i in 1..=10 {
        let title = format!("Task {}", i);
        let path = format!("work/file_{}.txt", i);
        let (code, out, err) = run_cmd(
            &temp_dir,
            naantree_bin,
            &[
                "--as", "agent-1", "task", "add", "--title", &title, "--paths", &path, "--size",
                "S",
            ],
        );
        assert_eq!(code, 0, "Task add {} failed: {}\n{}", i, out, err);
    }

    // Reset agents to test 10 workers joining concurrently
    // Let's spawn 10 threads running join, claim, file edit, and commit simultaneously!
    let repo_arc = Arc::new(temp_dir.clone());
    let bin_arc = Arc::new(binary_path.clone());
    let mut handles = Vec::new();

    for worker_idx in 1..=10 {
        let repo = Arc::clone(&repo_arc);
        let bin = Arc::clone(&bin_arc);

        let h = thread::spawn(move || {
            let bin_str = bin.to_str().unwrap();

            // 1. Join
            let (code, join_out, join_err) =
                run_cmd(&repo, bin_str, &["join", "--tags", "worker", "--json"]);
            assert_eq!(
                code, 0,
                "Worker {} join failed: {}\n{}",
                worker_idx, join_out, join_err
            );

            let json: serde_json::Value = serde_json::from_str(&join_out).expect("Parse join json");
            let my_id = json["agent"].as_str().expect("agent id").to_string();

            // 2. Claim task (auto-picks best ready task and claims its paths)
            let mut claimed_task_id = String::new();
            let mut claimed_path = String::new();

            for _retry in 0..30 {
                let (claim_code, claim_out, _) =
                    run_cmd(&repo, bin_str, &["--as", &my_id, "task", "claim", "--json"]);
                if claim_code == 0 {
                    let task_json: serde_json::Value =
                        serde_json::from_str(&claim_out).expect("Parse claim json");
                    claimed_task_id = task_json["id"].as_str().unwrap_or("").to_string();
                    if let Some(paths) = task_json["claimed_paths"].as_array() {
                        if let Some(p) = paths.first().and_then(|v| v.as_str()) {
                            claimed_path = p.to_string();
                        }
                    }
                    break;
                }
                thread::sleep(Duration::from_millis(50));
            }

            assert!(
                !claimed_task_id.is_empty(),
                "Agent {} failed to claim any task",
                my_id
            );

            // 3. Do work on claimed file
            let file_to_edit = repo.join(&claimed_path);
            fs::write(
                &file_to_edit,
                format!("Content by {} for {}\n", my_id, claimed_task_id),
            )
            .unwrap();

            // 4. Commit
            let msg = format!("Work on {} by {}", claimed_task_id, my_id);
            let (commit_code, commit_out, commit_err) = run_cmd(
                &repo,
                bin_str,
                &[
                    "--as",
                    &my_id,
                    "commit",
                    "-m",
                    &msg,
                    "--files",
                    &claimed_path,
                ],
            );
            assert_eq!(
                commit_code, 0,
                "Agent {} commit failed: {}\n{}",
                my_id, commit_out, commit_err
            );

            (my_id, claimed_task_id, claimed_path)
        });
        handles.push(h);
    }

    let mut results = Vec::new();
    for h in handles {
        results.push(h.join().unwrap());
    }

    // Assertions:
    // 1. All 10 worker agent IDs are unique
    let mut agent_ids: Vec<String> = results.iter().map(|(id, _, _)| id.clone()).collect();
    agent_ids.sort();
    let original_len = agent_ids.len();
    agent_ids.dedup();
    assert_eq!(
        agent_ids.len(),
        original_len,
        "All agent IDs must be unique: {:?}",
        agent_ids
    );

    // 2. No double-claimed task: all 10 claimed task IDs are distinct
    let mut task_ids: Vec<String> = results.iter().map(|(_, t, _)| t.clone()).collect();
    task_ids.sort();
    task_ids.dedup();
    assert_eq!(
        task_ids.len(),
        10,
        "All 10 tasks must be claimed uniquely without duplicates: {:?}",
        task_ids
    );

    // 3. No overlapping paths
    let mut paths: Vec<String> = results.iter().map(|(_, _, p)| p.clone()).collect();
    paths.sort();
    paths.dedup();
    assert_eq!(
        paths.len(),
        10,
        "All 10 claimed paths must be unique: {:?}",
        paths
    );

    // 4. Check git linear history:
    // Initial commit (1) + 10 worker commits = 11 commits total
    let (code, git_log_out, _) = run_cmd(&temp_dir, "git", &["rev-list", "--count", "HEAD"]);
    assert_eq!(code, 0);
    assert_eq!(
        git_log_out.trim(),
        "11",
        "Git must have exactly 11 commits (1 init + 10 workers)"
    );

    // Cleanup
    let _ = fs::remove_dir_all(&temp_dir);
}
