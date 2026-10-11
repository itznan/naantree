use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::thread;
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

fn run_cmd_env(
    dir: &Path,
    bin: &str,
    args: &[&str],
    envs: &[(&str, &str)],
) -> (i32, String, String) {
    let mut cmd = Command::new(bin);
    cmd.current_dir(dir).args(args);
    for (k, v) in envs {
        cmd.env(k, v);
    }
    let out = cmd
        .output()
        .unwrap_or_else(|e| panic!("Failed to run {} {:?}: {}", bin, args, e));
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    let code = out.status.code().unwrap_or(-1);
    (code, stdout, stderr)
}

fn create_temp_repo(prefix: &str) -> (PathBuf, String) {
    let temp_dir = std::env::temp_dir().join(format!(
        "naantree_{}_{}_{}",
        prefix,
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&temp_dir).unwrap();
    run_cmd(&temp_dir, "git", &["init", "-b", "main"]);
    run_cmd(&temp_dir, "git", &["config", "user.name", "Tester"]);
    run_cmd(
        &temp_dir,
        "git",
        &["config", "user.email", "tester@example.com"],
    );
    fs::write(temp_dir.join("init.txt"), "init\n").unwrap();
    run_cmd(&temp_dir, "git", &["add", "init.txt"]);
    run_cmd(&temp_dir, "git", &["commit", "-m", "init"]);

    let bin = env!("CARGO_BIN_EXE_naantree").to_string();
    (temp_dir, bin)
}

#[test]
fn test_parallel_joins_single_coordinator() {
    let (temp_dir, bin) = create_temp_repo("parallel_join");
    let temp_dir = Arc::new(temp_dir);
    let bin = Arc::new(bin);

    let n = 10;
    let mut handles = Vec::new();

    for _ in 0..n {
        let dir = Arc::clone(&temp_dir);
        let b = Arc::clone(&bin);
        handles.push(thread::spawn(move || run_cmd(&dir, &b, &["join"])));
    }

    let mut outputs = Vec::new();
    for h in handles {
        let (code, stdout, stderr) = h.join().unwrap();
        assert_eq!(code, 0, "Join failed: stdout={}, stderr={}", stdout, stderr);
        outputs.push(stdout);
    }

    // Read agents.json
    let agents_path = temp_dir.join(".naantree").join("state").join("agents.json");
    let content = fs::read_to_string(&agents_path).expect("agents.json must exist");
    let data: serde_json::Value = serde_json::from_str(&content).unwrap();
    let agents = data["agents"].as_array().expect("agents array");

    assert_eq!(agents.len(), 10, "Must have exactly 10 agents");

    let mut coordinator_count = 0;
    let mut worker_count = 0;
    let mut ids = std::collections::HashSet::new();

    for a in agents {
        let id = a["id"].as_str().unwrap();
        ids.insert(id.to_string());
        match a["role"].as_str().unwrap() {
            "coordinator" => coordinator_count += 1,
            "worker" => worker_count += 1,
            other => panic!("Unexpected role: {}", other),
        }
    }

    assert_eq!(ids.len(), 10, "All 10 agent IDs must be unique");
    assert_eq!(coordinator_count, 1, "Exactly one coordinator must exist");
    assert_eq!(worker_count, 9, "Remaining 9 agents must be workers");
}

#[test]
fn test_preseeded_stale_coordinator_new_join_takes_over() {
    let (temp_dir, bin) = create_temp_repo("stale_coord_takeover");
    let state_dir = temp_dir.join(".naantree").join("state");
    fs::create_dir_all(&state_dir).unwrap();

    // Pre-seed a stale agent-1 coordinator with heartbeat in past
    let stale_agents = serde_json::json!({
        "agents": [
            {
                "id": "agent-1",
                "role": "coordinator",
                "tags": [],
                "role_hint": null,
                "pid": 12345,
                "joined_at": 1000,
                "last_heartbeat": 1000,
                "left_at": null,
                "is_stale": false
            }
        ]
    });
    fs::write(
        state_dir.join("agents.json"),
        serde_json::to_string_pretty(&stale_agents).unwrap(),
    )
    .unwrap();

    // New join runs
    let (code, stdout, stderr) = run_cmd(&temp_dir, &bin, &["join"]);
    assert_eq!(code, 0, "Join failed: stdout={}, stderr={}", stdout, stderr);

    assert!(
        stdout.contains("role=coordinator"),
        "New joiner must become coordinator: {}",
        stdout
    );
    assert!(
        stdout.contains("coordinator_alive=true"),
        "Coordinator alive must be true: {}",
        stdout
    );

    // Verify agents.json
    let content = fs::read_to_string(state_dir.join("agents.json")).unwrap();
    let data: serde_json::Value = serde_json::from_str(&content).unwrap();
    let agents = data["agents"].as_array().unwrap();

    let a1 = agents.iter().find(|a| a["id"] == "agent-1").unwrap();
    assert_eq!(a1["is_stale"], true, "Old agent-1 must be marked stale");

    let a2 = agents.iter().find(|a| a["id"] == "agent-2").unwrap();
    assert_eq!(a2["role"], "coordinator", "New agent-2 must be coordinator");
    assert_eq!(a2["is_stale"], false, "New agent-2 must be alive");
}

#[test]
fn test_resume_validation() {
    let (temp_dir, bin) = create_temp_repo("resume_val");

    // Agent-1 joins as coordinator
    let (code, out, _) = run_cmd(&temp_dir, &bin, &["join"]);
    assert_eq!(code, 0);
    assert!(out.contains("agent=agent-1 role=coordinator"));

    // Agent-2 joins as worker
    let (code, out, _) = run_cmd(&temp_dir, &bin, &["join"]);
    assert_eq!(code, 0);
    assert!(out.contains("agent=agent-2 role=worker"));

    // 1. Resuming alive agent-1 without --force must fail
    let (code, _, err) = run_cmd(&temp_dir, &bin, &["join", "--resume", "agent-1"]);
    assert_ne!(code, 0, "Resuming alive agent without --force must fail");
    assert!(err.contains("currently alive") && err.contains("--force"));

    // 2. Resuming alive agent-1 with --force succeeds
    let (code, out, _) = run_cmd(&temp_dir, &bin, &["join", "--resume", "agent-1", "--force"]);
    assert_eq!(
        code, 0,
        "Resuming alive agent with --force must succeed: {}",
        out
    );
    assert!(out.contains("status=resumed"));

    // 3. Demote superseded coordinator on resume:
    // Make agent-1 stale, promote agent-2 to coordinator
    let state_dir = temp_dir.join(".naantree").join("state");
    let mut data: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(state_dir.join("agents.json")).unwrap()).unwrap();
    data["agents"][0]["is_stale"] = serde_json::json!(true);
    data["agents"][0]["role"] = serde_json::json!("coordinator");
    data["agents"][1]["is_stale"] = serde_json::json!(false);
    data["agents"][1]["role"] = serde_json::json!("coordinator");
    fs::write(
        state_dir.join("agents.json"),
        serde_json::to_string_pretty(&data).unwrap(),
    )
    .unwrap();

    // Now resume agent-1
    let (code, out, _) = run_cmd(&temp_dir, &bin, &["join", "--resume", "agent-1"]);
    assert_eq!(code, 0, "Resume must succeed: {}", out);
    assert!(
        out.contains("role=worker"),
        "Superseded coordinator must be demoted to worker: {}",
        out
    );

    // Verify agents.json: exactly one coordinator
    let updated: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(state_dir.join("agents.json")).unwrap()).unwrap();
    let agents = updated["agents"].as_array().unwrap();
    let coordinators: Vec<&serde_json::Value> = agents
        .iter()
        .filter(|a| a["role"] == "coordinator" && a["is_stale"] == false)
        .collect();
    assert_eq!(
        coordinators.len(),
        1,
        "Must never have two live coordinators"
    );
    assert_eq!(coordinators[0]["id"], "agent-2");
}

#[test]
fn test_coordinator_stale_failover_on_worker_command() {
    let (temp_dir, bin) = create_temp_repo("stale_failover");

    // Agent-1 joins (coordinator)
    let (code, _, _) = run_cmd(&temp_dir, &bin, &["join"]);
    assert_eq!(code, 0);

    // Agent-2 joins (worker)
    let (code, _, _) = run_cmd(&temp_dir, &bin, &["join"]);
    assert_eq!(code, 0);

    // Age agent-1 heartbeat by 200 seconds
    let state_dir = temp_dir.join(".naantree").join("state");
    let mut data: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(state_dir.join("agents.json")).unwrap()).unwrap();
    let old_time = data["agents"][0]["last_heartbeat"]
        .as_u64()
        .unwrap()
        .saturating_sub(200_000);
    data["agents"][0]["last_heartbeat"] = serde_json::json!(old_time);
    fs::write(
        state_dir.join("agents.json"),
        serde_json::to_string_pretty(&data).unwrap(),
    )
    .unwrap();

    // Worker agent-2 runs status
    let (code, out, err) = run_cmd(&temp_dir, &bin, &["--as", "agent-2", "status"]);
    assert_eq!(code, 0, "Status failed: {}\n{}", out, err);
    assert!(
        out.contains("coordinator=agent-2"),
        "Agent-2 must be promoted to coordinator: {}",
        out
    );
    assert!(
        out.contains("coordinator_state=alive"),
        "Coordinator state must be alive: {}",
        out
    );

    // Check log events: coordinator_promoted
    let (code, log_out, _) = run_cmd(&temp_dir, &bin, &["--as", "agent-2", "log"]);
    assert_eq!(code, 0);
    assert!(log_out.contains("coordinator_promoted"));
}

#[test]
fn test_identity_resolution_session_file() {
    let (temp_dir, bin) = create_temp_repo("identity_session");

    // Join with session key
    let (code, out, _) = run_cmd(&temp_dir, &bin, &["join", "--session", "sess-alpha"]);
    assert_eq!(code, 0);
    assert!(out.contains("agent=agent-1"));

    // Subshell with NAANTREE_SESSION
    let (code, who_out, _) = run_cmd_env(
        &temp_dir,
        &bin,
        &["whoami"],
        &[("NAANTREE_SESSION", "sess-alpha")],
    );
    assert_eq!(code, 0);
    assert!(
        who_out.contains("id=agent-1"),
        "Resolved via NAANTREE_SESSION: {}",
        who_out
    );

    // Command with --session
    let (code, who_out2, _) = run_cmd(&temp_dir, &bin, &["--session", "sess-alpha", "whoami"]);
    assert_eq!(code, 0);
    assert!(
        who_out2.contains("id=agent-1"),
        "Resolved via --session: {}",
        who_out2
    );

    // Subshell with NAANTREE_AGENT
    let (code, who_out3, _) = run_cmd_env(
        &temp_dir,
        &bin,
        &["whoami"],
        &[("NAANTREE_AGENT", "agent-1")],
    );
    assert_eq!(code, 0);
    assert!(
        who_out3.contains("id=agent-1"),
        "Resolved via NAANTREE_AGENT: {}",
        who_out3
    );

    // Subshell with --as
    let (code, who_out4, _) = run_cmd(&temp_dir, &bin, &["--as", "agent-1", "whoami"]);
    assert_eq!(code, 0);
    assert!(
        who_out4.contains("id=agent-1"),
        "Resolved via --as: {}",
        who_out4
    );
}

#[test]
fn test_doctor_invariant_violations() {
    let (temp_dir, bin) = create_temp_repo("doctor_invariants");
    let state_dir = temp_dir.join(".naantree").join("state");
    fs::create_dir_all(&state_dir).unwrap();

    // 0. Clean repo without agents: doctor PASS
    let (code, out, _) = run_cmd(&temp_dir, &bin, &["doctor"]);
    assert_eq!(code, 0);
    assert!(out.contains("doctor=PASS"));

    // Case A: Multiple live coordinators
    let double_coord = serde_json::json!({
        "agents": [
            {
                "id": "agent-1",
                "role": "coordinator",
                "tags": [],
                "role_hint": null,
                "pid": std::process::id(),
                "joined_at": 1000,
                "last_heartbeat": 1000,
                "left_at": null,
                "is_stale": false
            },
            {
                "id": "agent-2",
                "role": "coordinator",
                "tags": [],
                "role_hint": null,
                "pid": std::process::id(),
                "joined_at": 1000,
                "last_heartbeat": 1000,
                "left_at": null,
                "is_stale": false
            }
        ]
    });
    fs::write(
        state_dir.join("agents.json"),
        serde_json::to_string_pretty(&double_coord).unwrap(),
    )
    .unwrap();

    let (code, out, err) = run_cmd(&temp_dir, &bin, &["doctor"]);
    assert_ne!(code, 0, "Multiple coordinators must fail doctor");
    let combined = format!("{}{}", out, err);
    assert!(combined.contains("Multiple live coordinators") && combined.contains("doctor=FAIL"));

    // Case B: No live coordinator
    let no_coord = serde_json::json!({
        "agents": [
            {
                "id": "agent-1",
                "role": "worker",
                "tags": [],
                "role_hint": null,
                "pid": std::process::id(),
                "joined_at": 1000,
                "last_heartbeat": 1000,
                "left_at": null,
                "is_stale": false
            }
        ]
    });
    fs::write(
        state_dir.join("agents.json"),
        serde_json::to_string_pretty(&no_coord).unwrap(),
    )
    .unwrap();

    let (code, out, err) = run_cmd(&temp_dir, &bin, &["doctor"]);
    assert_ne!(code, 0, "No live coordinator must fail doctor");
    let combined = format!("{}{}", out, err);
    assert!(combined.contains("No live coordinator") && combined.contains("doctor=FAIL"));

    // Case C: Ghost agent (never resumed)
    let ghost = serde_json::json!({
        "agents": [
            {
                "id": "agent-1",
                "role": "coordinator",
                "tags": [],
                "role_hint": null,
                "pid": std::process::id(),
                "joined_at": 1000,
                "last_heartbeat": 1000,
                "left_at": null,
                "is_stale": false
            },
            {
                "id": "agent-2",
                "role": "worker",
                "tags": [],
                "role_hint": null,
                "pid": std::process::id(),
                "joined_at": 1000,
                "last_heartbeat": 1000,
                "left_at": null,
                "is_stale": true
            }
        ]
    });
    fs::write(
        state_dir.join("agents.json"),
        serde_json::to_string_pretty(&ghost).unwrap(),
    )
    .unwrap();

    let (code, out, err) = run_cmd(&temp_dir, &bin, &["doctor"]);
    assert_ne!(code, 0, "Ghost agent must fail doctor");
    let combined = format!("{}{}", out, err);
    assert!(combined.contains("Ghost agent detected") && combined.contains("doctor=FAIL"));

    // Case D: Dead recorded PID
    let dead_pid = serde_json::json!({
        "agents": [
            {
                "id": "agent-1",
                "role": "coordinator",
                "tags": [],
                "role_hint": null,
                "pid": 999999999,
                "joined_at": 1000,
                "last_heartbeat": 1000,
                "left_at": null,
                "is_stale": false
            }
        ]
    });
    fs::write(
        state_dir.join("agents.json"),
        serde_json::to_string_pretty(&dead_pid).unwrap(),
    )
    .unwrap();

    let (code, out, err) = run_cmd(&temp_dir, &bin, &["doctor"]);
    assert_ne!(code, 0, "Dead recorded PID must fail doctor");
    let combined = format!("{}{}", out, err);
    assert!(
        combined.contains("recorded PID 999999999 is dead") && combined.contains("doctor=FAIL")
    );

    // Case E: Claims owned by stale agents
    let alive_agent_and_stale_claim = serde_json::json!({
        "agents": [
            {
                "id": "agent-1",
                "role": "coordinator",
                "tags": [],
                "role_hint": null,
                "pid": std::process::id(),
                "joined_at": 1000,
                "last_heartbeat": 1000,
                "left_at": null,
                "is_stale": false
            },
            {
                "id": "agent-2",
                "role": "worker",
                "tags": [],
                "role_hint": null,
                "pid": std::process::id(),
                "joined_at": 1000,
                "last_heartbeat": 1000,
                "left_at": null,
                "is_stale": true
            }
        ]
    });
    let claims = serde_json::json!({
        "claims": [
            {
                "id": "claim-1",
                "agent_id": "agent-2",
                "paths": ["src/**"],
                "task_id": null,
                "shared": false,
                "ttl_ms": 60000,
                "created_at": 1000,
                "expires_at": 61000,
                "queue": []
            }
        ]
    });
    fs::write(
        state_dir.join("agents.json"),
        serde_json::to_string_pretty(&alive_agent_and_stale_claim).unwrap(),
    )
    .unwrap();
    fs::write(
        state_dir.join("claims.json"),
        serde_json::to_string_pretty(&claims).unwrap(),
    )
    .unwrap();

    let (code, out, err) = run_cmd(&temp_dir, &bin, &["doctor"]);
    assert_ne!(code, 0, "Claims owned by stale agent must fail doctor");
    let combined = format!("{}{}", out, err);
    assert!(combined.contains("held by stale agent agent-2") && combined.contains("doctor=FAIL"));
}

#[test]
fn test_coordinator_claim_and_wait() {
    let (temp_dir, bin) = create_temp_repo("coord_claim_wait");

    // Agent-1 joins as coordinator
    let (code, _, _) = run_cmd(&temp_dir, &bin, &["join"]);
    assert_eq!(code, 0);

    // Agent-2 joins as worker
    let (code, _, _) = run_cmd(&temp_dir, &bin, &["join"]);
    assert_eq!(code, 0);

    // Agent-2 tries to claim coordinator role while Agent-1 is alive -> fails
    let (code, _, err) = run_cmd(
        &temp_dir,
        &bin,
        &["--as", "agent-2", "coordinator", "claim"],
    );
    assert_ne!(code, 0);
    assert!(err.contains("active coordinator 'agent-1' is still alive"));

    // Make Agent-1 stale
    let state_dir = temp_dir.join(".naantree").join("state");
    let mut data: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(state_dir.join("agents.json")).unwrap()).unwrap();
    let old_time = data["agents"][0]["last_heartbeat"]
        .as_u64()
        .unwrap()
        .saturating_sub(200_000);
    data["agents"][0]["last_heartbeat"] = serde_json::json!(old_time);
    fs::write(
        state_dir.join("agents.json"),
        serde_json::to_string_pretty(&data).unwrap(),
    )
    .unwrap();

    // Now Agent-2 claims coordinator role -> succeeds
    let (code, out, _) = run_cmd(
        &temp_dir,
        &bin,
        &["--as", "agent-2", "coordinator", "claim"],
    );
    assert_eq!(code, 0);
    assert!(out.contains("role=coordinator coordinator_alive=true status=claimed"));

    // Test wait --for coordinator returns immediately since coordinator is alive
    let (code, out, _) = run_cmd(
        &temp_dir,
        &bin,
        &[
            "--as",
            "agent-2",
            "wait",
            "--for",
            "coordinator",
            "--timeout",
            "5",
        ],
    );
    assert_eq!(code, 0);
    assert!(out.contains("event=coordinator id=agent-2"));
}

#[test]
fn test_gate_set_baseline_and_env_task_gating() {
    let (temp_dir, bin) = create_temp_repo("gate_and_env");

    // Agent-1 joins (coordinator)
    let (code, _, _) = run_cmd(&temp_dir, &bin, &["join"]);
    assert_eq!(code, 0);

    // Gate set with passing command
    let (code, out, _) = run_cmd(
        &temp_dir,
        &bin,
        &["--as", "agent-1", "gate", "set", "echo 'all good'"],
    );
    assert_eq!(code, 0);
    assert!(out.contains("gate_baseline=PASS"));

    // Add env task
    let (code, out, _) = run_cmd(
        &temp_dir,
        &bin,
        &[
            "--as",
            "agent-1",
            "task",
            "add",
            "--title",
            "Setup Environment",
            "--kind",
            "env",
            "--paths",
            "env/**",
            "--size",
            "S",
        ],
    );
    assert_eq!(code, 0);
    assert!(out.contains("kind=env"));

    // Add code task with overlapping path -> should warn
    let (code, out, _) = run_cmd(
        &temp_dir,
        &bin,
        &[
            "--as",
            "agent-1",
            "task",
            "add",
            "--title",
            "Install Tooling",
            "--paths",
            "env/tool.sh",
            "--size",
            "S",
        ],
    );
    assert_eq!(code, 0);
    assert!(out.contains("[WARN]") && out.contains("overlaps with existing task task-1"));

    // Check ready tasks: only task-1 (env) should be ready, task-2 is gated by unfinished env task!
    let (code, out, _) = run_cmd(
        &temp_dir,
        &bin,
        &["--as", "agent-1", "task", "list", "--ready"],
    );
    assert_eq!(code, 0);
    assert!(out.contains("id=task-1") && !out.contains("id=task-2"));

    // Complete task-1
    run_cmd(
        &temp_dir,
        &bin,
        &["--as", "agent-1", "task", "claim", "task-1"],
    );
    run_cmd(
        &temp_dir,
        &bin,
        &[
            "--as",
            "agent-1",
            "task",
            "done",
            "task-1",
            "--summary",
            "Env ready",
        ],
    );

    // Now task-2 should be ready
    let (code, out, _) = run_cmd(
        &temp_dir,
        &bin,
        &["--as", "agent-1", "task", "list", "--ready"],
    );
    assert_eq!(code, 0);
    assert!(out.contains("id=task-2"));
}
