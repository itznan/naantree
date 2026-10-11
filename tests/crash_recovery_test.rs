use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn run_cmd(dir: &Path, bin: &str, args: &[&str]) -> (i32, String, String) {
    let out = Command::new(bin)
        .current_dir(dir)
        .env("NAANTREE_STALE_LOCK_MS", "300")
        .env("NAANTREE_LOCK_TIMEOUT_MS", "2000")
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("Failed to run {} {:?}: {}", bin, args, e));
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    let code = out.status.code().unwrap_or(-1);
    (code, stdout, stderr)
}

fn create_temp_repo(prefix: &str) -> (PathBuf, String) {
    let temp_dir = std::env::temp_dir().join(format!(
        "naantree_crash_{}_{}_{}",
        prefix,
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&temp_dir).unwrap();
    run_cmd(&temp_dir, "git", &["init", "-b", "main"]);
    run_cmd(&temp_dir, "git", &["config", "user.name", "CrashTester"]);
    run_cmd(
        &temp_dir,
        "git",
        &["config", "user.email", "crashtester@example.com"],
    );
    fs::write(temp_dir.join("init.txt"), "init\n").unwrap();
    run_cmd(&temp_dir, "git", &["add", "init.txt"]);
    run_cmd(&temp_dir, "git", &["commit", "-m", "init"]);

    let bin = env!("CARGO_BIN_EXE_naantree").to_string();
    (temp_dir, bin)
}

#[test]
fn test_simulated_crash_recovery_abandoned_lock() {
    let (temp_dir, bin) = create_temp_repo("abandoned_lock");

    // Initialize state directory
    let state_dir = temp_dir.join(".naantree").join("state");
    fs::create_dir_all(&state_dir).unwrap();

    // Create a stale lock directory simulating a process that was killed while holding the lock
    let lock_dir = state_dir.join(".lock");
    fs::create_dir_all(&lock_dir).unwrap();
    let stale_meta = serde_json::json!({
        "pid": 999999,
        "agent_id": "crashed-agent",
        "created_at": 1000, // Very old timestamp
        "token": "crashed-token"
    });
    fs::write(
        lock_dir.join("lock.json"),
        serde_json::to_string(&stale_meta).unwrap(),
    )
    .unwrap();

    // Joining agent must break stale lock and succeed
    let (code, out, err) = run_cmd(&temp_dir, &bin, &["join"]);
    assert_eq!(code, 0, "join must succeed despite stale lock: {}", err);
    assert!(out.contains("role=coordinator"));

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_simulated_crash_recovery_leftover_git_index_lock() {
    let (temp_dir, bin) = create_temp_repo("git_index_lock");

    // Agent joins and claims a file
    let (code, _, _) = run_cmd(&temp_dir, &bin, &["join"]);
    assert_eq!(code, 0);

    let (code, _, _) = run_cmd(
        &temp_dir,
        &bin,
        &["--as", "agent-1", "claim", "recovered.txt"],
    );
    assert_eq!(code, 0);

    fs::write(temp_dir.join("recovered.txt"), "recovered content\n").unwrap();

    // Simulate mid-git crash: create a fake stale .git/index.lock
    let git_dir = temp_dir.join(".git");
    let index_lock = git_dir.join("index.lock");
    fs::write(&index_lock, "stale git index lock from killed git process").unwrap();

    // naantree commit should clear index.lock and commit successfully
    let (code, out, err) = run_cmd(
        &temp_dir,
        &bin,
        &[
            "--as",
            "agent-1",
            "commit",
            "-m",
            "recovered commit",
            "--files",
            "recovered.txt",
        ],
    );
    assert_eq!(
        code, 0,
        "commit must clear stale index.lock and succeed: {}",
        err
    );
    assert!(out.contains("commit="));

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_simulated_crash_recovery_dead_agent_in_doing_state() {
    let (temp_dir, bin) = create_temp_repo("dead_agent_doing");

    // Coordinator joins
    let (code, _, _) = run_cmd(&temp_dir, &bin, &["join"]);
    assert_eq!(code, 0);

    // Plan and task added
    run_cmd(
        &temp_dir,
        &bin,
        &[
            "--as",
            "agent-1",
            "plan",
            "init",
            "--goal",
            "Crash recovery test",
        ],
    );
    run_cmd(
        &temp_dir,
        &bin,
        &[
            "--as",
            "agent-1",
            "task",
            "add",
            "--title",
            "Feature X",
            "--paths",
            "src/x.rs",
            "--size",
            "S",
        ],
    );

    // Worker joins
    let (code, _, _) = run_cmd(&temp_dir, &bin, &["join"]);
    assert_eq!(code, 0);

    // Worker claims task-1
    let (code, out, _) = run_cmd(
        &temp_dir,
        &bin,
        &["--as", "agent-2", "task", "claim", "task-1"],
    );
    assert_eq!(code, 0);
    assert!(out.contains("state=doing"));

    // Verify claims.json has src/x.rs claimed by agent-2
    let state_dir = temp_dir.join(".naantree").join("state");
    let claims_content = fs::read_to_string(state_dir.join("claims.json")).unwrap();
    assert!(claims_content.contains("agent-2"));

    // Simulate Agent 2 dying: artificially age Agent 2's heartbeat and set dead PID
    let mut agents_data: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(state_dir.join("agents.json")).unwrap()).unwrap();
    for a in agents_data["agents"].as_array_mut().unwrap() {
        if a["id"] == "agent-2" {
            let old_time = a["last_heartbeat"]
                .as_u64()
                .unwrap_or(0)
                .saturating_sub(200_000);
            a["last_heartbeat"] = serde_json::json!(old_time);
            a["pid"] = serde_json::json!(999999);
            a["pid_start_time"] = serde_json::Value::Null;
        }
    }
    fs::write(
        state_dir.join("agents.json"),
        serde_json::to_string_pretty(&agents_data).unwrap(),
    )
    .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(50));

    // Now Coordinator runs status, which triggers auto_reap
    let (code, _out, err) = run_cmd(&temp_dir, &bin, &["--as", "agent-1", "status"]);
    assert_eq!(code, 0, "status failed: {}\n{}", _out, err);

    // Task-1 must have returned to todo!
    let (code, out, err) = run_cmd(&temp_dir, &bin, &["--as", "agent-1", "task", "list"]);
    assert_eq!(code, 0, "task list failed: {}\n{}", out, err);
    assert!(
        out.contains("id=task-1") && out.contains("todo"),
        "Expected task-1 to return to todo, got output:\n{}",
        out
    );

    // Claims must be empty / released
    let (code, out, err) = run_cmd(&temp_dir, &bin, &["--as", "agent-1", "claims"]);
    assert_eq!(code, 0, "claims failed: {}\n{}", out, err);
    assert!(
        out.contains("claims=0") || out.contains("no claims"),
        "Expected claims to be released, got output:\n{}",
        out
    );

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_real_sigkill_mid_commit_and_write_recovery() {
    let (temp_dir, bin) = create_temp_repo("real_sigkill");

    // Agent joins as coordinator
    let (code, _, _) = run_cmd(&temp_dir, &bin, &["join"]);
    assert_eq!(code, 0);

    // Run real process executions interrupted with SIGKILL / TerminateProcess
    let iterations = if cfg!(windows) { 5 } else { 15 };
    for i in 0..iterations {
        let filename = format!("churn_{}.txt", i);
        let filepath = temp_dir.join(&filename);
        let _ = fs::write(&filepath, format!("content {}\n", i));

        // Claim file
        let _ = run_cmd(&temp_dir, &bin, &["--as", "agent-1", "claim", &filename]);

        // Spawn naantree commit child process
        let mut child = Command::new(&bin)
            .current_dir(&temp_dir)
            .env("NAANTREE_STALE_LOCK_MS", "300")
            .env("NAANTREE_LOCK_TIMEOUT_MS", "2000")
            .args([
                "--as",
                "agent-1",
                "commit",
                "-m",
                &format!("churn commit {}", i),
                "--files",
                &filename,
            ])
            .spawn()
            .unwrap();

        // Kill at varying intervals: 1ms to 15ms
        let delay_ms = (i % 15) + 1;
        std::thread::sleep(std::time::Duration::from_millis(delay_ms));

        // Abruptly terminate process (SIGKILL on Unix, TerminateProcess on Windows)
        let _ = child.kill();
        let _ = child.wait();
    }

    // After 15 abrupt process kills, verify state files are valid uncorrupted JSON/JSONL
    let state_dir = temp_dir.join(".naantree").join("state");
    if state_dir.join("agents.json").exists() {
        let content = fs::read_to_string(state_dir.join("agents.json")).unwrap();
        let _: serde_json::Value =
            serde_json::from_str(&content).expect("agents.json must be valid JSON");
    }
    if state_dir.join("claims.json").exists() {
        let content = fs::read_to_string(state_dir.join("claims.json")).unwrap();
        let _: serde_json::Value =
            serde_json::from_str(&content).expect("claims.json must be valid JSON");
    }
    if state_dir.join("tasks.json").exists() {
        let content = fs::read_to_string(state_dir.join("tasks.json")).unwrap();
        let _: serde_json::Value =
            serde_json::from_str(&content).expect("tasks.json must be valid JSON");
    }
    if state_dir.join("events.jsonl").exists() {
        let content = fs::read_to_string(state_dir.join("events.jsonl")).unwrap();
        for line in content.lines() {
            if !line.trim().is_empty() {
                let _: serde_json::Value =
                    serde_json::from_str(line).expect("events.jsonl lines must be valid JSON");
            }
        }
    }

    // Clean commit recovery on the repository
    let final_file = "final_recovered.txt";
    fs::write(temp_dir.join(final_file), "final recovered data\n").unwrap();
    let (code, _, _) = run_cmd(&temp_dir, &bin, &["--as", "agent-1", "claim", final_file]);
    assert_eq!(code, 0);

    let (code, out, err) = run_cmd(
        &temp_dir,
        &bin,
        &[
            "--as",
            "agent-1",
            "commit",
            "-m",
            "clean recovery commit after process churn",
            "--files",
            final_file,
        ],
    );
    assert_eq!(
        code, 0,
        "Clean commit must succeed after SIGKILL churn: {}",
        err
    );
    assert!(out.contains("commit="));

    let _ = fs::remove_dir_all(&temp_dir);
}
