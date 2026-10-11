use std::fs;
use naantree::core::ids::{extract_id_num, next_agent_id, next_claim_id, next_task_id};
use naantree::core::lock::{acquire_lock, current_time_ms};
use naantree::core::paths::{file_matches_pattern, normalize_path, paths_overlap};
use naantree::core::store::{Agent, Store, Task};

#[test]
fn test_ids() {
    assert_eq!(extract_id_num("agent-1"), Some(1));
    assert_eq!(extract_id_num("task-42"), Some(42));
    assert_eq!(next_agent_id(&["agent-1".into(), "agent-2".into()]), "agent-3");
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
