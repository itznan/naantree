use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn run_nt(dir: &Path, bin: &str, args: &[&str]) -> (i32, String, String) {
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
fn test_simulated_4_agent_dry_run() {
    let binary_path = PathBuf::from(env!("CARGO_BIN_EXE_naantree"));
    let temp_dir = std::env::temp_dir().join(format!(
        "naantree_dryrun_{}_{}",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis()
    ));

    fs::create_dir_all(&temp_dir).unwrap();
    let nt = binary_path.to_str().unwrap();

    // 1. Initialize git repo
    let (code, _, _) = run_nt(&temp_dir, "git", &["init", "-b", "main"]);
    assert_eq!(code, 0);
    run_nt(&temp_dir, "git", &["config", "user.name", "Tester"]);
    run_nt(&temp_dir, "git", &["config", "user.email", "tester@example.com"]);
    fs::write(temp_dir.join("init.txt"), "init").unwrap();
    run_nt(&temp_dir, "git", &["add", "init.txt"]);
    run_nt(&temp_dir, "git", &["commit", "-m", "init"]);

    // 2. Doctor check
    let (code, out, _) = run_nt(&temp_dir, nt, &["doctor"]);
    assert_eq!(code, 0);
    assert!(out.contains("doctor=PASS"));

    // 3. Agent 1 (Coordinator) joins
    let (code, out, _) = run_nt(&temp_dir, nt, &["join", "--tags", "coord"]);
    assert_eq!(code, 0);
    assert!(out.contains("agent=agent-1"));
    assert!(out.contains("role=coordinator"));

    // Coordinator initializes plan
    let (code, _, _) = run_nt(
        &temp_dir,
        nt,
        &["--as", "agent-1", "plan", "init", "--goal", "Build Greetings Suite"],
    );
    assert_eq!(code, 0);

    // Coordinator creates 4 tasks
    let tasks_spec = [
        ("Add hello.txt", "hello.txt"),
        ("Add welcome.txt", "welcome.txt"),
        ("Add bye.txt", "bye.txt"),
        ("Add docs.txt", "docs.txt"),
    ];

    for (title, path) in &tasks_spec {
        let (code, _, _) = run_nt(
            &temp_dir,
            nt,
            &[
                "--as",
                "agent-1",
                "task",
                "add",
                "--title",
                title,
                "--paths",
                path,
                "--size",
                "S",
            ],
        );
        assert_eq!(code, 0);
    }

    // Set gate command
    let gate_cmd = if cfg!(windows) {
        "type hello.txt welcome.txt bye.txt docs.txt"
    } else {
        "cat hello.txt welcome.txt bye.txt docs.txt"
    };
    let (code, _, _) = run_nt(&temp_dir, nt, &["--as", "agent-1", "gate", "set", gate_cmd]);
    assert_eq!(code, 0);

    // Announce plan to all
    let (code, _, _) = run_nt(
        &temp_dir,
        nt,
        &[
            "--as",
            "agent-1",
            "send",
            "--to",
            "all",
            "--type",
            "INFO",
            "--ref",
            "plan",
            "-m",
            "Plan initialized with 4 tasks",
        ],
    );
    assert_eq!(code, 0);

    // 4. Agents 2, 3, 4 join as workers
    for i in 2..=4 {
        let (code, out, _) = run_nt(&temp_dir, nt, &["join", "--tags", "worker"]);
        assert_eq!(code, 0);
        assert!(out.contains(&format!("agent=agent-{}", i)));
        assert!(out.contains("role=worker"));

        // Check inbox
        let (code, out, _) = run_nt(&temp_dir, nt, &["--as", &format!("agent-{}", i), "inbox"]);
        assert_eq!(code, 0);
        assert!(out.contains("Plan initialized with 4 tasks"));
    }

    // 5. Work loop for each agent (agent-1 through agent-4)
    // Agent 1 claims task-1, Agent 2 claims task-2, Agent 3 claims task-3, Agent 4 claims task-4
    let agent_tasks = [
        ("agent-1", "task-1", "hello.txt", "Hello World!\n"),
        ("agent-2", "task-2", "welcome.txt", "Welcome World!\n"),
        ("agent-3", "task-3", "bye.txt", "Goodbye World!\n"),
        ("agent-4", "task-4", "docs.txt", "Documentation!\n"),
    ];

    for (agent_id, task_id, file_path, content) in &agent_tasks {
        // Claim task
        let (code, out, err) = run_nt(&temp_dir, nt, &["--as", agent_id, "task", "claim", task_id]);
        assert_eq!(code, 0, "Claim {} by {} failed: {}\n{}", task_id, agent_id, out, err);

        // Edit file
        fs::write(temp_dir.join(file_path), content).unwrap();

        // Commit
        let msg = format!("Implement {}", task_id);
        let (code, out, err) = run_nt(
            &temp_dir,
            nt,
            &["--as", agent_id, "commit", "-m", &msg, "--files", file_path],
        );
        assert_eq!(code, 0, "Commit by {} failed: {}\n{}", agent_id, out, err);

        // Request review
        let (code, _, _) = run_nt(&temp_dir, nt, &["--as", agent_id, "review", "request", task_id]);
        assert_eq!(code, 0);
    }

    // 6. Cross-peer reviews:
    // Agent 2 reviews task-1
    let (code, _, _) = run_nt(
        &temp_dir,
        nt,
        &[
            "--as", "agent-2", "review", "result", "task-1", "--verdict", "approve", "--notes", "LGTM",
        ],
    );
    assert_eq!(code, 0);

    // Agent 1 reviews task-2
    let (code, _, _) = run_nt(
        &temp_dir,
        nt,
        &[
            "--as", "agent-1", "review", "result", "task-2", "--verdict", "approve", "--notes", "LGTM",
        ],
    );
    assert_eq!(code, 0);

    // Agent 4 reviews task-3
    let (code, _, _) = run_nt(
        &temp_dir,
        nt,
        &[
            "--as", "agent-4", "review", "result", "task-3", "--verdict", "approve", "--notes", "LGTM",
        ],
    );
    assert_eq!(code, 0);

    // Agent 3 reviews task-4
    let (code, _, _) = run_nt(
        &temp_dir,
        nt,
        &[
            "--as", "agent-3", "review", "result", "task-4", "--verdict", "approve", "--notes", "LGTM",
        ],
    );
    assert_eq!(code, 0);

    // 7. Mark tasks done & release claims
    for (agent_id, task_id, _, _) in &agent_tasks {
        let (code, out, err) = run_nt(
            &temp_dir,
            nt,
            &[
                "--as",
                agent_id,
                "task",
                "done",
                task_id,
                "--summary",
                "Feature implemented and reviewed",
            ],
        );
        assert_eq!(code, 0, "Task done {} failed: {}\n{}", task_id, out, err);

        let (code, _, _) = run_nt(&temp_dir, nt, &["--as", agent_id, "release", "--all"]);
        assert_eq!(code, 0);
    }

    // 8. Coordinator runs gate & finish
    let (code, out, _) = run_nt(&temp_dir, nt, &["--as", "agent-1", "gate", "run"]);
    assert_eq!(code, 0);
    assert!(out.contains("gate=PASS"));

    let (code, out, err) = run_nt(&temp_dir, nt, &["--as", "agent-1", "finish"]);
    assert_eq!(code, 0, "Finish failed: {}\n{}", out, err);
    assert!(out.contains("NAANTREE_COMPLETE"));

    // Check FINAL_REPORT.md
    let report_file = temp_dir.join(".naantree").join("FINAL_REPORT.md");
    assert!(report_file.exists(), "FINAL_REPORT.md must exist");
    let report_text = fs::read_to_string(report_file).unwrap();
    assert!(report_text.contains("Build Greetings Suite"));
    assert!(report_text.contains("NAANTREE_COMPLETE"));

    // Workers see completion in status
    let (code, out, _) = run_nt(&temp_dir, nt, &["--as", "agent-2", "status"]);
    assert_eq!(code, 0);
    assert!(out.contains("status=NAANTREE_COMPLETE"));

    // Clean up
    let _ = fs::remove_dir_all(temp_dir);
}
