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
    pub fn init(as_agent: Option<String>) -> Result<Self, String> {
        let repo_root = find_repo_root()?;
        let state_dir = repo_root.join(".naantree").join("state");
        let store = Store::new(state_dir.clone());

        let agent_id = resolve_agent_id(&state_dir, as_agent);

        // Implicit heartbeat if agent is identified
        if let Some(id) = &agent_id {
            let _ = crate::core::lock::with_lock(&state_dir, id, || {
                let mut agents_data = store.load_agents();
                let mut updated = false;
                let now = current_time_ms();
                for a in &mut agents_data.agents {
                    if a.id == *id && !a.is_stale && a.left_at.is_none() {
                        a.last_heartbeat = now;
                        updated = true;
                        break;
                    }
                }
                if updated {
                    let _ = store.save_agents(&agents_data);
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

pub fn resolve_agent_id(state_dir: &Path, as_arg: Option<String>) -> Option<String> {
    if let Some(arg) = as_arg {
        let trimmed = arg.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }

    if let Ok(env_id) = env::var("NAANTREE_AGENT") {
        let trimmed = env_id.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }

    if let Some(key) = get_session_key() {
        let file_path = state_dir.join(format!("me.{}", key));
        if file_path.exists() {
            if let Ok(content) = fs::read_to_string(file_path) {
                let trimmed = content.trim();
                if !trimmed.is_empty() {
                    return Some(trimmed.to_string());
                }
            }
        }
    }

    None
}
