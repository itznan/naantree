use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use uuid::Uuid;

use super::ids::{extract_id_num, next_evt_id};
use super::lock::current_time_ms;

pub const DEFAULT_AGENT_STALE_MS: u64 = 90_000; // 90 seconds (1.5 minutes)

pub fn get_stale_timeout_ms() -> u64 {
    std::env::var("NAANTREE_STALE_TIMEOUT_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_AGENT_STALE_MS)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Agent {
    pub id: String,
    pub role: String, // "coordinator" | "worker"
    pub tags: Vec<String>,
    pub role_hint: Option<String>,
    pub pid: u32,
    #[serde(default)]
    pub pid_start_time: Option<u64>,
    pub joined_at: u64,
    pub last_heartbeat: u64,
    pub left_at: Option<u64>,
    pub is_stale: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AgentsData {
    pub agents: Vec<Agent>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskReview {
    pub reviewer: String,
    pub verdict: String, // "approve" | "changes"
    pub notes: String,
    pub timestamp: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub title: String,
    pub desc: String,
    pub tags: Vec<String>,
    pub paths: Vec<String>,
    pub deps: Vec<String>,
    pub size: String,  // "S" | "M" | "L"
    pub state: String, // "proposed" | "todo" | "doing" | "review" | "done" | "blocked"
    pub assignee: Option<String>,
    pub proposed_by: Option<String>,
    pub commit_hash: Option<String>,
    pub reviewers: usize,
    pub reviews: Vec<TaskReview>,
    pub note: Option<String>,
    pub blocked_reason: Option<String>,
    pub summary: Option<String>,
    pub created_at: u64,
    pub updated_at: u64,
    #[serde(default)]
    pub kind: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TasksData {
    pub tasks: Vec<Task>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claim {
    pub id: String,
    pub agent_id: String,
    pub paths: Vec<String>,
    pub task_id: Option<String>,
    pub shared: bool,
    pub ttl_ms: u64,
    pub created_at: u64,
    pub expires_at: u64,
    pub queue: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ClaimsData {
    pub claims: Vec<Claim>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub id: String,
    pub from: String,
    pub to: String,
    pub msg_type: String,
    pub ref_id: String,
    pub text: String,
    pub needs_ack: bool,
    pub ack_by: Vec<String>,
    pub timestamp: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Decision {
    pub id: String,
    pub title: String,
    pub choice: String,
    pub why: String,
    pub alternatives: Vec<String>,
    pub agent_id: String,
    pub timestamp: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub id: String,
    pub event_type: String,
    pub agent_id: Option<String>,
    pub data: serde_json::Value,
    pub timestamp: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GateResult {
    pub pass: bool,
    pub cmd: String,
    pub output: String,
    pub exit_code: i32,
    pub timestamp: u64,
    #[serde(default)]
    pub git_head: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MetaData {
    pub goal: Option<String>,
    pub allow_push: bool,
    pub gate_cmd: Option<String>,
    pub gate_last_result: Option<GateResult>,
    pub finished: bool,
    pub finished_at: Option<u64>,
}

pub struct Store {
    pub state_dir: PathBuf,
}

impl Store {
    pub fn new(state_dir: PathBuf) -> Self {
        let _ = fs::create_dir_all(&state_dir);
        Self { state_dir }
    }

    pub fn write_json_atomic<T: Serialize>(&self, filename: &str, data: &T) -> Result<(), String> {
        let target = self.state_dir.join(filename);
        let tmp = self.state_dir.join(format!(
            "{}.tmp.{}.{}.{}",
            filename,
            std::process::id(),
            current_time_ms(),
            Uuid::new_v4().simple()
        ));
        let content = serde_json::to_string_pretty(data).map_err(|e| e.to_string())?;
        fs::write(&tmp, content).map_err(|e| format!("Failed to write tmp file: {}", e))?;
        fs::rename(&tmp, &target).map_err(|e| format!("Failed to rename file: {}", e))?;
        Ok(())
    }

    pub fn read_json<T: for<'de> Deserialize<'de> + Default>(&self, filename: &str) -> T {
        let path = self.state_dir.join(filename);
        if !path.exists() {
            return T::default();
        }
        match fs::read_to_string(&path) {
            Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
            Err(_) => T::default(),
        }
    }

    pub fn append_jsonl<T: Serialize>(&self, filename: &str, record: &T) -> Result<(), String> {
        let path = self.state_dir.join(filename);
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|e| format!("Failed to open {}: {}", filename, e))?;
        let mut line = serde_json::to_string(record).map_err(|e| e.to_string())?;
        line.push('\n');
        file.write_all(line.as_bytes())
            .map_err(|e| format!("Failed to append to {}: {}", filename, e))?;
        Ok(())
    }

    pub fn read_jsonl<T: for<'de> Deserialize<'de>>(&self, filename: &str) -> Vec<T> {
        let path = self.state_dir.join(filename);
        if !path.exists() {
            return Vec::new();
        }
        let file = match fs::File::open(&path) {
            Ok(f) => f,
            Err(_) => return Vec::new(),
        };
        let reader = BufReader::new(file);
        let mut items = Vec::new();
        for line in reader.lines().map_while(Result::ok) {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            if let Ok(item) = serde_json::from_str::<T>(trimmed) {
                items.push(item);
            }
        }
        items
    }

    pub fn load_agents(&self) -> AgentsData {
        self.read_json("agents.json")
    }

    pub fn save_agents(&self, data: &AgentsData) -> Result<(), String> {
        self.write_json_atomic("agents.json", data)
    }

    pub fn load_tasks(&self) -> TasksData {
        self.read_json("tasks.json")
    }

    pub fn save_tasks(&self, data: &TasksData) -> Result<(), String> {
        self.write_json_atomic("tasks.json", data)
    }

    pub fn load_claims(&self) -> ClaimsData {
        self.read_json("claims.json")
    }

    pub fn save_claims(&self, data: &ClaimsData) -> Result<(), String> {
        self.write_json_atomic("claims.json", data)
    }

    pub fn load_meta(&self) -> MetaData {
        self.read_json("meta.json")
    }

    pub fn save_meta(&self, data: &MetaData) -> Result<(), String> {
        self.write_json_atomic("meta.json", data)
    }

    pub fn load_inbox_reads(&self) -> HashMap<String, HashSet<String>> {
        self.read_json("inbox_reads.json")
    }

    pub fn save_inbox_reads(&self, data: &HashMap<String, HashSet<String>>) -> Result<(), String> {
        self.write_json_atomic("inbox_reads.json", data)
    }

    pub fn log_event(
        &self,
        event_type: &str,
        agent_id: Option<&str>,
        data: serde_json::Value,
    ) -> Result<String, String> {
        let existing: Vec<Event> = self.read_jsonl("events.jsonl");
        let ids: Vec<String> = existing.into_iter().map(|e| e.id).collect();
        let evt_id = next_evt_id(&ids);
        let evt = Event {
            id: evt_id.clone(),
            event_type: event_type.to_string(),
            agent_id: agent_id.map(|s| s.to_string()),
            data,
            timestamp: current_time_ms(),
        };
        self.append_jsonl("events.jsonl", &evt)?;
        Ok(evt_id)
    }

    pub fn reap_dead_agents(&self) -> Result<Vec<String>, String> {
        let stale_timeout_ms = get_stale_timeout_ms();

        let mut agents_data = self.load_agents();
        let now = current_time_ms();
        let mut reaped = Vec::new();
        let mut coordinator_was_reaped = false;

        for agent in &mut agents_data.agents {
            if !agent.is_stale && agent.left_at.is_none() {
                let heartbeat_expired = now.saturating_sub(agent.last_heartbeat) > stale_timeout_ms;
                let process_dead = if agent.pid_start_time.is_some() {
                    !crate::core::context::is_pid_alive_matching(agent.pid, agent.pid_start_time)
                } else {
                    false
                };
                if heartbeat_expired || process_dead {
                    agent.is_stale = true;
                    reaped.push(agent.id.clone());
                    if agent.role == "coordinator" {
                        coordinator_was_reaped = true;
                    }
                }
            }
        }

        if reaped.is_empty() {
            return Ok(Vec::new());
        }

        // 1. Release claims owned by reaped agents
        let mut claims_data = self.load_claims();
        let reaped_set: HashSet<String> = reaped.iter().cloned().collect();
        for claim in &mut claims_data.claims {
            if reaped_set.contains(&claim.agent_id) {
                // If shared and queue exists, transfer to next
                if claim.shared && !claim.queue.is_empty() {
                    let next_agent = claim.queue.remove(0);
                    claim.agent_id = next_agent;
                    claim.created_at = now;
                    claim.expires_at = now + claim.ttl_ms;
                }
            }
        }
        claims_data
            .claims
            .retain(|c| !reaped_set.contains(&c.agent_id));
        self.save_claims(&claims_data)?;

        // 2. Return tasks doing/review by reaped agents to todo
        let mut tasks_data = self.load_tasks();
        for task in &mut tasks_data.tasks {
            if let Some(assignee) = &task.assignee {
                if reaped_set.contains(assignee)
                    && (task.state == "doing" || task.state == "review")
                {
                    task.state = "todo".to_string();
                    task.assignee = None;
                    task.updated_at = now;
                }
            }
            // If coordinator died, auto-accept proposed tasks
            if coordinator_was_reaped && task.state == "proposed" {
                task.state = "todo".to_string();
                task.updated_at = now;
            }
        }
        self.save_tasks(&tasks_data)?;

        // 3. Promote lowest-id alive agent if coordinator was reaped
        if coordinator_was_reaped {
            let already_has_live_coordinator = agents_data
                .agents
                .iter()
                .any(|a| a.role == "coordinator" && !a.is_stale && a.left_at.is_none());

            if !already_has_live_coordinator {
                let mut alive_candidates: Vec<&mut Agent> = agents_data
                    .agents
                    .iter_mut()
                    .filter(|a| !a.is_stale && a.left_at.is_none())
                    .collect();

                alive_candidates.sort_by_key(|a| extract_id_num(&a.id).unwrap_or(u64::MAX));

                if let Some(promoted) = alive_candidates.first_mut() {
                    promoted.role = "coordinator".to_string();
                    let _ = self.log_event(
                        "coordinator_promoted",
                        Some(&promoted.id),
                        serde_json::json!({
                            "promoted_agent": promoted.id,
                            "reason": "dead_coordinator_reaped"
                        }),
                    );
                }
            }
        }

        // Invariant: at most one live coordinator
        let mut live_coordinators: Vec<usize> = agents_data
            .agents
            .iter()
            .enumerate()
            .filter(|(_, a)| a.role == "coordinator" && !a.is_stale && a.left_at.is_none())
            .map(|(idx, _)| idx)
            .collect();
        if live_coordinators.len() > 1 {
            live_coordinators.sort_by_key(|&idx| {
                extract_id_num(&agents_data.agents[idx].id).unwrap_or(u64::MAX)
            });
            for &idx in &live_coordinators[1..] {
                agents_data.agents[idx].role = "worker".to_string();
                let demoted_id = agents_data.agents[idx].id.clone();
                let _ = self.log_event(
                    "coordinator_demoted",
                    Some(&demoted_id),
                    serde_json::json!({
                        "id": demoted_id,
                        "reason": "enforce_single_coordinator_invariant",
                        "new_role": "worker"
                    }),
                );
            }
        }

        for agent_id in &reaped {
            let _ = self.log_event(
                "agent_reaped",
                Some(agent_id),
                serde_json::json!({ "agent_id": agent_id }),
            );
        }

        self.save_agents(&agents_data)?;
        Ok(reaped)
    }
}
