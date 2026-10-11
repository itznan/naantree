use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

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
fn test_crash_recovery_abandoned_lock() {
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
fn test_crash_recovery_leftover_git_index_lock() {
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
fn test_crash_recovery_dead_agent_in_doing_state() {
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

    // Simulate Agent 2 dying: artificially age Agent 2's heartbeat
    let mut agents_data: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(state_dir.join("agents.json")).unwrap()).unwrap();
    for a in agents_data["agents"].as_array_mut().unwrap() {
        if a["id"] == "agent-2" {
            a["last_heartbeat"] = serde_json::json!(1000); // Very old heartbeat
        }
    }
    fs::write(
        state_dir.join("agents.json"),
        serde_json::to_string_pretty(&agents_data).unwrap(),
    )
    .unwrap();

    // Now Coordinator runs status, which triggers auto_reap
    let (code, _out, _) = run_cmd(&temp_dir, &bin, &["--as", "agent-1", "status"]);
    assert_eq!(code, 0);

    // Task-1 must have returned to todo!
    let (code, out, _) = run_cmd(&temp_dir, &bin, &["--as", "agent-1", "task", "list"]);
    assert_eq!(code, 0);
    assert!(out.contains("id=task-1") && out.contains("todo"));

    // Claims must be empty / released
    let (code, out, _) = run_cmd(&temp_dir, &bin, &["--as", "agent-1", "claims"]);
    assert_eq!(code, 0);
    assert!(out.contains("claims=0") || out.contains("no claims"));

    let _ = fs::remove_dir_all(&temp_dir);
}
