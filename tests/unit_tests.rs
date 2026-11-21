use naantree::cmds::git_cmds::run_diff;
use naantree::core::context::Context;
use naantree::core::ids::{extract_id_num, next_agent_id, next_claim_id, next_task_id};
use naantree::core::lock::{acquire_lock, current_time_ms};
use naantree::core::paths::{file_matches_pattern, normalize_path, paths_overlap};
use naantree::core::store::{Agent, Claim, ClaimsData, Store, Task};
use std::fs;
use std::process::Command;

#[test]
fn test_ids() {
    assert_eq!(extract_id_num("agent-1"), Some(1));
    assert_eq!(extract_id_num("task-42"), Some(42));
    assert_eq!(
        next_agent_id(&["agent-1".into(), "agent-2".into()]),
        "agent-3"
    );
    assert_eq!(next_task_id(&[]), "task-1");
    assert_eq!(next_claim_id(&["claim-10".into()]), "claim-11");
}

#[test]
fn test_paths_matching_and_overlap() {
    assert_eq!(normalize_path("./src/main.rs/"), "src/main.rs");
    assert_eq!(normalize_path("\\src\\auth\\login.rs"), "src/auth/login.rs");

    // Exact
    assert!(paths_overlap("src/auth.rs", "src/auth.rs"));
    assert!(!paths_overlap("src/auth.rs", "src/user.rs"));

    // Directory recursion
    assert!(paths_overlap("src/auth", "src/auth/login.rs"));
    assert!(paths_overlap("src/auth/login.rs", "src/auth"));

    // Globs
    assert!(paths_overlap("src/**/*.rs", "src/auth/login.rs"));
    assert!(paths_overlap("*.json", "package.json"));
    assert!(!paths_overlap("src/*.rs", "tests/*.rs"));

    // File match
    assert!(file_matches_pattern("src/**", "src/auth/login.rs"));
    assert!(file_matches_pattern("src/auth", "src/auth/login.rs"));
    assert!(file_matches_pattern("*.json", "package.json"));
    assert!(!file_matches_pattern("src/*.rs", "src/auth/login.rs"));
}

#[test]
fn test_atomic_lock() {
    let temp_dir = std::env::temp_dir().join(format!("naantree_test_lock_{}", current_time_ms()));
    let _ = fs::create_dir_all(&temp_dir);

    let guard1 = acquire_lock(&temp_dir, "agent-1", Some(1000), Some(2000)).unwrap();
    assert!(temp_dir.join(".lock").exists());

    // Second lock with very short timeout should fail while guard1 held
    let res = acquire_lock(&temp_dir, "agent-2", Some(1000), Some(100));
    assert!(res.is_err());

    drop(guard1);

    // After drop, lock can be acquired
    let guard2 = acquire_lock(&temp_dir, "agent-2", Some(1000), Some(1000)).unwrap();
    drop(guard2);

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_dead_agent_reaping() {
    let temp_dir = std::env::temp_dir().join(format!("naantree_test_reap_{}", current_time_ms()));
    let store = Store::new(temp_dir.clone());
    let now = current_time_ms();

    // 1 coordinator (stale), 1 worker (alive)
    let agents = naantree::core::store::AgentsData {
        agents: vec![
            Agent {
                id: "agent-1".into(),
                role: "coordinator".into(),
                tags: vec![],
                role_hint: None,
                pid: 1234,
                pid_start_time: None,
                joined_at: now - 500_000,
                last_heartbeat: now - 400_000, // > 300s -> stale
                left_at: None,
                is_stale: false,
            },
            Agent {
                id: "agent-2".into(),
                role: "worker".into(),
                tags: vec![],
                role_hint: None,
                pid: 5678,
                pid_start_time: None,
                joined_at: now,
                last_heartbeat: now,
                left_at: None,
                is_stale: false,
            },
        ],
    };
    store.save_agents(&agents).unwrap();

    let tasks = naantree::core::store::TasksData {
        tasks: vec![Task {
            id: "task-1".into(),
            title: "Test Task".into(),
            desc: "".into(),
            tags: vec![],
            paths: vec!["src/**".into()],
            deps: vec![],
            size: "S".into(),
            state: "doing".into(),
            assignee: Some("agent-1".into()),
            proposed_by: None,
            commit_hash: None,
            reviewers: 1,
            reviews: vec![],
            note: None,
            blocked_reason: None,
            summary: None,
            created_at: now,
            updated_at: now,
            kind: None,
        }],
    };
    store.save_tasks(&tasks).unwrap();

    // Reap
    let reaped = store.reap_dead_agents().unwrap();
    assert_eq!(reaped, vec!["agent-1"]);

    // Verify task reverted to todo
    let updated_tasks = store.load_tasks();
    assert_eq!(updated_tasks.tasks[0].state, "todo");
    assert_eq!(updated_tasks.tasks[0].assignee, None);

    // Verify agent-2 promoted to coordinator
    let updated_agents = store.load_agents();
    assert!(updated_agents.agents[0].is_stale);
    assert_eq!(updated_agents.agents[1].role, "coordinator");

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_skills_in_sync() {
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let naantree_skill =
        fs::read_to_string(manifest_dir.join(".naantree").join("SKILL.md")).unwrap();
    let agent_skill = fs::read_to_string(
        manifest_dir
            .join(".agent")
            .join("skills")
            .join("naantree")
            .join("SKILL.md"),
    )
    .unwrap();
    let agents_skill = fs::read_to_string(
        manifest_dir
            .join(".agents")
            .join("skills")
            .join("naantree")
            .join("SKILL.md"),
    )
    .unwrap();

    assert_eq!(
        naantree_skill, agent_skill,
        ".naantree/SKILL.md and .agent/skills/naantree/SKILL.md must be identical"
    );
    assert_eq!(
        naantree_skill, agents_skill,
        ".naantree/SKILL.md and .agents/skills/naantree/SKILL.md must be identical"
    );
}

#[test]
fn test_diff_within_and_outside_claims() {
    let temp_dir = std::env::temp_dir().join(format!("naantree_test_diff_{}", current_time_ms()));
    let _ = fs::create_dir_all(&temp_dir);

    // Initialize git repository with main branch
    let git_init = Command::new("git")
        .args(["init", "-b", "main"])
        .current_dir(&temp_dir)
        .output()
        .expect("git init failed");
    assert!(git_init.status.success());

    let _ = Command::new("git")
        .args(["config", "user.name", "Tester"])
        .current_dir(&temp_dir)
        .output();
    let _ = Command::new("git")
        .args(["config", "user.email", "tester@example.com"])
        .current_dir(&temp_dir)
        .output();

    // Initial commit
    fs::write(temp_dir.join("init.txt"), "init").unwrap();
    fs::write(temp_dir.join(".gitignore"), ".naantree/\n").unwrap();
    let _ = Command::new("git")
        .args(["add", "init.txt", ".gitignore"])
        .current_dir(&temp_dir)
        .output();
    let _ = Command::new("git")
        .args(["commit", "-m", "init"])
        .current_dir(&temp_dir)
        .output();

    let state_dir = temp_dir.join(".naantree").join("state");
    let store = Store::new(state_dir.clone());
    let now = current_time_ms();

    // Setup active claims:
    // agent-1 owns "src/auth/**"
    // agent-2 owns "src/payment/**"
    let claims = ClaimsData {
        claims: vec![
            Claim {
                id: "claim-1".into(),
                agent_id: "agent-1".into(),
                paths: vec!["src/auth/**".into()],
                task_id: Some("task-1".into()),
                shared: false,
                ttl_ms: 600_000,
                created_at: now,
                expires_at: now + 600_000,
                queue: vec![],
            },
            Claim {
                id: "claim-2".into(),
                agent_id: "agent-2".into(),
                paths: vec!["src/payment/**".into()],
                task_id: Some("task-2".into()),
                shared: false,
                ttl_ms: 600_000,
                created_at: now,
                expires_at: now + 600_000,
                queue: vec![],
            },
        ],
    };
    store.save_claims(&claims).unwrap();

    let ctx = Context {
        repo_root: temp_dir.clone(),
        state_dir,
        store,
        agent_id: Some("agent-1".into()),
    };

    // Before making changes: run_diff reports 0 modified files
    let diff_clean = run_diff(&ctx, false).unwrap();
    assert_eq!(diff_clean.json_val.as_ref().unwrap()["total"], 0);
    assert_eq!(
        diff_clean.json_val.as_ref().unwrap()["within_claims"],
        serde_json::json!([])
    );
    assert_eq!(
        diff_clean.json_val.as_ref().unwrap()["outside_claims"],
        serde_json::json!([])
    );

    // Create 3 files:
    // 1. src/auth/login.rs (within agent-1 claim)
    // 2. src/payment/checkout.rs (outside agent-1 claim, claimed by agent-2)
    // 3. docs/readme.txt (outside agent-1 claim, unclaimed)
    fs::create_dir_all(temp_dir.join("src").join("auth")).unwrap();
    fs::create_dir_all(temp_dir.join("src").join("payment")).unwrap();
    fs::create_dir_all(temp_dir.join("docs")).unwrap();

    fs::write(temp_dir.join("src/auth/login.rs"), "fn login() {}").unwrap();
    fs::write(temp_dir.join("src/payment/checkout.rs"), "fn pay() {}").unwrap();
    fs::write(temp_dir.join("docs/readme.txt"), "readme").unwrap();

    // Unstaged diff (default)
    let diff_all = run_diff(&ctx, false).unwrap();
    let json_all = diff_all.json_val.unwrap();
    assert_eq!(json_all["total"], 3);

    let within: Vec<String> = serde_json::from_value(json_all["within_claims"].clone()).unwrap();
    let outside: Vec<String> = serde_json::from_value(json_all["outside_claims"].clone()).unwrap();

    assert_eq!(within, vec!["src/auth/login.rs"]);
    assert!(outside.contains(&"src/payment/checkout.rs".to_string()));
    assert!(outside.contains(&"docs/readme.txt".to_string()));
    assert!(json_all["warning"].is_string());

    // Check human output formatting
    let output_lines = diff_all.lines.join("\n");
    assert!(output_lines.contains("[within]  src/auth/login.rs"));
    assert!(output_lines.contains("[outside] src/payment/checkout.rs (claimed by agent-2)"));
    assert!(output_lines.contains("[outside] docs/readme.txt (unclaimed)"));
    assert!(output_lines.contains("WARNING:"));

    // Now stage only src/auth/login.rs and src/payment/checkout.rs
    let _ = Command::new("git")
        .args(["add", "src/auth/login.rs", "src/payment/checkout.rs"])
        .current_dir(&temp_dir)
        .output();

    // Staged-only diff
    let diff_staged = run_diff(&ctx, true).unwrap();
    let json_staged = diff_staged.json_val.unwrap();
    assert_eq!(json_staged["total"], 2);

    let staged_within: Vec<String> =
        serde_json::from_value(json_staged["within_claims"].clone()).unwrap();
    let staged_outside: Vec<String> =
        serde_json::from_value(json_staged["outside_claims"].clone()).unwrap();

    assert_eq!(staged_within, vec!["src/auth/login.rs"]);
    assert_eq!(staged_outside, vec!["src/payment/checkout.rs"]);
    assert!(!staged_outside.contains(&"docs/readme.txt".to_string()));

    // Also verify CLI invocation via binary
    let binary_path = std::path::PathBuf::from(env!("CARGO_BIN_EXE_naantree"));
    let cli_out = Command::new(&binary_path)
        .current_dir(&temp_dir)
        .args(["--as", "agent-1", "--json", "diff", "--staged"])
        .output()
        .expect("CLI execution failed");
    assert!(cli_out.status.success());
    let cli_json: serde_json::Value =
        serde_json::from_slice(&cli_out.stdout).expect("Valid JSON output from CLI");
    assert_eq!(cli_json["total"], 2);
    assert_eq!(cli_json["staged"], true);

    let _ = fs::remove_dir_all(temp_dir);
}
