use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use super::lock::current_time_ms;
use super::store::Store;

pub struct Context {
    pub repo_root: PathBuf,
    pub state_dir: PathBuf,
    pub store: Store,
    pub agent_id: Option<String>,
}

impl Context {
    pub fn init(
        as_agent: Option<String>,
        session_arg: Option<String>,
        auto_reap: bool,
    ) -> Result<Self, String> {
        let repo_root = find_repo_root()?;
        let state_dir = repo_root.join(".naantree").join("state");
        let store = Store::new(state_dir.clone());

        let agent_id = resolve_agent_id(&state_dir, as_agent, session_arg);

        // Reap dead agents and refresh heartbeat under lock if auto_reap is enabled
        if auto_reap {
            let caller_lock_id = agent_id.as_deref().unwrap_or("anon");
            let _ = crate::core::lock::with_lock(&state_dir, caller_lock_id, || {
                let _ = store.reap_dead_agents();
                if let Some(id) = &agent_id {
                    let mut agents_data = store.load_agents();
                    let mut updated = false;
                    let now = current_time_ms();
                    for a in &mut agents_data.agents {
                        if a.id == *id && !a.is_stale && a.left_at.is_none() {
                            a.last_heartbeat = now;
                            a.pid = get_parent_pid();
                            updated = true;
                            break;
                        }
                    }
                    if updated {
                        let _ = store.save_agents(&agents_data);
                    }
                }
                Ok(())
            });
        }

        Ok(Self {
            repo_root,
            state_dir,
            store,
            agent_id,
        })
    }

    pub fn require_agent(&self) -> Result<String, String> {
        match &self.agent_id {
            Some(id) => Ok(id.clone()),
            None => Err(
                "No agent identity found. Pass --as <agent-id> or set NAANTREE_AGENT=<agent-id>, or run 'naantree join'."
                    .to_string(),
            ),
        }
    }
}

pub fn find_repo_root() -> Result<PathBuf, String> {
    let mut current = env::current_dir().map_err(|e| e.to_string())?;
    loop {
        if current.join(".git").exists() || current.join(".naantree").exists() {
            return Ok(current);
        }
        if let Some(parent) = current.parent() {
            current = parent.to_path_buf();
        } else {
            break;
        }
    }
    // Fallback to current working directory
    env::current_dir().map_err(|e| e.to_string())
}

pub fn get_parent_pid() -> u32 {
    #[cfg(unix)]
    {
        std::os::unix::process::parent_id()
    }
    #[cfg(not(unix))]
    {
        std::process::id()
    }
}

pub fn get_grandparent_pid(parent_pid: u32) -> Option<u32> {
    #[cfg(target_os = "linux")]
    {
        let stat_path = format!("/proc/{}/stat", parent_pid);
        if let Ok(content) = fs::read_to_string(stat_path) {
            if let Some(idx) = content.rfind(')') {
                let rest = content[idx + 1..].trim();
                let fields: Vec<&str> = rest.split_whitespace().collect();
                if fields.len() >= 2 {
                    if let Ok(ppid) = fields[1].parse::<u32>() {
                        if ppid > 1 {
                            return Some(ppid);
                        }
                    }
                }
            }
        }
        None
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = parent_pid;
        None
    }
}

pub fn get_tty_name() -> Option<String> {
    #[cfg(unix)]
    {
        let link = fs::read_link("/proc/self/fd/0")
            .or_else(|_| fs::read_link("/dev/fd/0"))
            .ok()?;
        let path_str = link.to_string_lossy();
        if path_str.starts_with("/dev/pts/") || path_str.starts_with("/dev/tty") {
            let sanitized = path_str
                .trim_start_matches("/dev/")
                .replace('/', "_");
            return Some(sanitized);
        }
    }
    None
}

pub fn is_pid_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    #[cfg(target_os = "linux")]
    {
        Path::new(&format!("/proc/{}", pid)).exists()
    }
    #[cfg(not(target_os = "linux"))]
    {
        std::process::Command::new("kill")
            .arg("-0")
            .arg(pid.to_string())
            .output()
            .map(|o| o.status.success())
            .unwrap_or(true)
    }
}

pub fn get_session_key() -> Option<String> {
    if let Ok(v) = env::var("NAANTREE_SESSION") {
        if !v.trim().is_empty() {
            return Some(v.trim().to_string());
        }
    }
    if let Ok(v) = env::var("TERM_SESSION_ID") {
        if !v.trim().is_empty() {
            return Some(v.trim().to_string());
        }
    }
    if let Ok(v) = env::var("WINDOWID") {
        if !v.trim().is_empty() {
            return Some(v.trim().to_string());
        }
    }
    None
}

fn read_id_from_file(path: &Path) -> Option<String> {
    if path.exists() {
        if let Ok(content) = fs::read_to_string(path) {
            let trimmed = content.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

pub fn resolve_agent_id(
    state_dir: &Path,
    as_arg: Option<String>,
    session_arg: Option<String>,
) -> Option<String> {
    // 1. --as argument
    if let Some(arg) = as_arg {
        let trimmed = arg.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }

    // 2. NAANTREE_AGENT env var
    if let Ok(env_id) = env::var("NAANTREE_AGENT") {
        let trimmed = env_id.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }

    // 3. Session resolution
    // 3a. Explicit --session argument
    if let Some(key) = session_arg {
        let trimmed = key.trim();
        if !trimmed.is_empty() {
            if let Some(id) = read_id_from_file(&state_dir.join(format!("me.{}", trimmed))) {
                return Some(id);
            }
        }
    }

    // 3b. NAANTREE_SESSION env var
    if let Ok(env_sess) = env::var("NAANTREE_SESSION") {
        let trimmed = env_sess.trim();
        if !trimmed.is_empty() {
            if let Some(id) = read_id_from_file(&state_dir.join(format!("me.{}", trimmed))) {
                return Some(id);
            }
        }
    }

    // 3c. Session file keyed by parent PID or grandparent PID
    let ppid = get_parent_pid();
    if let Some(id) = read_id_from_file(&state_dir.join(format!("me.pid_{}", ppid))) {
        return Some(id);
    }
    if let Some(id) = read_id_from_file(&state_dir.join(format!("me.{}", ppid))) {
        return Some(id);
    }
    if let Some(gppid) = get_grandparent_pid(ppid) {
        if let Some(id) = read_id_from_file(&state_dir.join(format!("me.pid_{}", gppid))) {
            return Some(id);
        }
        if let Some(id) = read_id_from_file(&state_dir.join(format!("me.{}", gppid))) {
            return Some(id);
        }
    }

    // 3d. Session file keyed by tty
    if let Some(tty) = get_tty_name() {
        if let Some(id) = read_id_from_file(&state_dir.join(format!("me.tty_{}", tty))) {
            return Some(id);
        }
        if let Some(id) = read_id_from_file(&state_dir.join(format!("me.{}", tty))) {
            return Some(id);
        }
    }

    // 3e. Existing keys: TERM_SESSION_ID, WINDOWID
    if let Some(key) = get_session_key() {
        if let Some(id) = read_id_from_file(&state_dir.join(format!("me.{}", key))) {
            return Some(id);
        }
    }

    None
}
