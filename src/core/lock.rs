use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use uuid::Uuid;

pub const DEFAULT_STALE_LOCK_MS: u64 = 30_000;
pub const DEFAULT_LOCK_TIMEOUT_MS: u64 = 35_000;

#[derive(Debug, Serialize, Deserialize)]
pub struct LockMetadata {
    pub pid: u32,
    pub agent_id: String,
    pub created_at: u64,
    pub token: String,
}

pub struct LockGuard {
    lock_dir: PathBuf,
    metadata_path: PathBuf,
    token: String,
    released: bool,
}

impl LockGuard {
    pub fn release(&mut self) {
        if self.released {
            return;
        }
        self.released = true;
        if self.metadata_path.exists() {
            if let Ok(content) = fs::read_to_string(&self.metadata_path) {
                if let Ok(meta) = serde_json::from_str::<LockMetadata>(&content) {
                    if meta.token != self.token {
                        return; // Lock was overtaken or broken
                    }
                }
            }
        }
        let release_dir = self.lock_dir.with_extension(format!(
            "rel.{}.{}.{}",
            std::process::id(),
            current_time_ms(),
            Uuid::new_v4().simple()
        ));
        if fs::rename(&self.lock_dir, &release_dir).is_ok() {
            let _ = fs::remove_dir_all(&release_dir);
        } else {
            let _ = fs::remove_dir_all(&self.lock_dir);
        }
    }
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        self.release();
    }
}

pub fn current_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::from_secs(0))
        .as_millis() as u64
}

fn try_break_stale_lock(lock_dir: &Path) -> bool {
    let stale_dir = lock_dir.with_extension(format!(
        "stale.{}.{}.{}",
        std::process::id(),
        current_time_ms(),
        Uuid::new_v4().simple()
    ));
    if fs::rename(lock_dir, &stale_dir).is_ok() {
        let _ = fs::remove_dir_all(&stale_dir);
        return true;
    }
    let _ = fs::remove_dir_all(lock_dir);
    true
}

pub fn acquire_lock(
    state_dir: &Path,
    agent_id: &str,
    stale_timeout_ms: Option<u64>,
    timeout_ms: Option<u64>,
) -> Result<LockGuard, String> {
    let stale_ms = stale_timeout_ms
        .or_else(|| {
            std::env::var("NAANTREE_STALE_LOCK_MS")
                .ok()
                .and_then(|v| v.parse().ok())
        })
        .unwrap_or(DEFAULT_STALE_LOCK_MS);

    let max_timeout_ms = timeout_ms.unwrap_or(DEFAULT_LOCK_TIMEOUT_MS);
    let lock_dir = state_dir.join(".lock");
    let metadata_path = lock_dir.join("lock.json");

    let _ = fs::create_dir_all(state_dir);
    let start_time = current_time_ms();

    while current_time_ms() - start_time < max_timeout_ms {
        match fs::create_dir(&lock_dir) {
            Ok(()) => {
                let token = Uuid::new_v4().to_string();
                let meta = LockMetadata {
                    pid: std::process::id(),
                    agent_id: agent_id.to_string(),
                    created_at: current_time_ms(),
                    token: token.clone(),
                };
                let content = serde_json::to_string(&meta).unwrap_or_default();
                let _ = fs::write(&metadata_path, content);

                return Ok(LockGuard {
                    lock_dir,
                    metadata_path,
                    token,
                    released: false,
                });
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                let mut is_stale = false;
                if metadata_path.exists() {
                    if let Ok(content) = fs::read_to_string(&metadata_path) {
                        if let Ok(meta) = serde_json::from_str::<LockMetadata>(&content) {
                            if current_time_ms().saturating_sub(meta.created_at) > stale_ms {
                                is_stale = true;
                            }
                        }
                    }
                } else if let Ok(meta) = fs::metadata(&lock_dir) {
                    if let Ok(modified) = meta.modified() {
                        if let Ok(dur) = modified.elapsed() {
                            if dur.as_millis() as u64 > stale_ms {
                                is_stale = true;
                            }
                        }
                    }
                }

                if is_stale {
                    try_break_stale_lock(&lock_dir);
                    continue;
                }

                // Random backoff jitter: 10ms - 35ms
                let jitter = 10 + (Uuid::new_v4().as_u128() % 25) as u64;
                thread::sleep(Duration::from_millis(jitter));
            }
            Err(e) => return Err(format!("Failed to create lock dir: {}", e)),
        }
    }

    Err(format!(
        "Timed out waiting for lock on {} after {}ms",
        state_dir.display(),
        max_timeout_ms
    ))
}

pub fn with_lock<F, R>(state_dir: &Path, agent_id: &str, f: F) -> Result<R, String>
where
    F: FnOnce() -> Result<R, String>,
{
    let mut guard = acquire_lock(state_dir, agent_id, None, None)?;
    let res = f();
    guard.release();
    res
}
