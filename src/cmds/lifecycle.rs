use std::fs;
use crate::core::context::{get_session_key, Context};
use crate::core::format::CommandOutput;
use crate::core::ids::{extract_id_num, next_agent_id};
use crate::core::lock::{current_time_ms, with_lock};
use crate::core::store::Agent;

pub fn run_join(
    ctx: &Context,
    role_hint: Option<String>,
    tags: Option<String>,
    session_opt: Option<String>,
    resume: Option<String>,
) -> Result<CommandOutput, String> {
    with_lock(&ctx.state_dir, "join", || {
        let _ = ctx.store.reap_dead_agents();
        let mut agents_data = ctx.store.load_agents();
        let now = current_time_ms();

        // Check resume
        if let Some(res_id) = resume {
            if let Some(agent) = agents_data.agents.iter_mut().find(|a| a.id == res_id) {
                agent.is_stale = false;
                agent.left_at = None;
                agent.last_heartbeat = now;
                agent.pid = std::process::id();
                let role = agent.role.clone();
                let id = agent.id.clone();
                ctx.store.save_agents(&agents_data)?;
                let _ = ctx.store.log_event(
                    "agent_resumed",
                    Some(&id),
                    serde_json::json!({ "id": id, "role": role }),
                );

                return Ok(CommandOutput::new()
                    .line(format!("agent={} role={} status=resumed pid={}", id, role, std::process::id()))
                    .json(&serde_json::json!({
                        "agent": id,
                        "role": role,
                        "status": "resumed",
                        "pid": std::process::id()
                    }))
                    .next("naantree status"));
            }
        }

        let existing_ids: Vec<String> = agents_data.agents.iter().map(|a| a.id.clone()).collect();
        let agent_id = next_agent_id(&existing_ids);

        // Check if there is currently an active coordinator
        let has_alive_coordinator = agents_data
            .agents
            .iter()
            .any(|a| a.role == "coordinator" && !a.is_stale && a.left_at.is_none());

        let role = if has_alive_coordinator {
            "worker".to_string()
        } else {
            "coordinator".to_string()
        };

        let tag_list: Vec<String> = tags
            .unwrap_or_default()
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        let new_agent = Agent {
            id: agent_id.clone(),
            role: role.clone(),
            tags: tag_list.clone(),
            role_hint: role_hint.clone(),
            pid: std::process::id(),
            joined_at: now,
            last_heartbeat: now,
            left_at: None,
            is_stale: false,
        };

        agents_data.agents.push(new_agent);
        ctx.store.save_agents(&agents_data)?;

        // Write session file if session key is detected or provided
        let session_key = session_opt.or_else(get_session_key);
        if let Some(key) = &session_key {
            let session_file = ctx.state_dir.join(format!("me.{}", key));
            let _ = fs::write(session_file, &agent_id);
        }

        let _ = ctx.store.log_event(
            "agent_joined",
            Some(&agent_id),
            serde_json::json!({
                "agent": agent_id,
                "role": role,
                "tags": tag_list,
                "pid": std::process::id()
            }),
        );

        let mut out = CommandOutput::new();
        out = out.line(format!(
            "agent={} role={} pid={} tags={}",
            agent_id,
            role,
            std::process::id(),
            tag_list.join(",")
        ));

        if session_key.is_none() {
            out = out.line(format!(
                "(Pass --as {} or set NAANTREE_AGENT={} on future commands)",
                agent_id, agent_id
            ));
        }

        Ok(out
            .json(&serde_json::json!({
                "agent": agent_id,
                "role": role,
                "tags": tag_list,
                "pid": std::process::id()
            }))
            .next("naantree status"))
    })
}

pub fn run_whoami(ctx: &Context) -> Result<CommandOutput, String> {
    let agent_id = match &ctx.agent_id {
        Some(id) => id,
        None => {
            return Ok(CommandOutput::new()
                .line("agent=none status=unassigned")
                .next("naantree join"));
        }
    };

    let agents_data = ctx.store.load_agents();
    if let Some(agent) = agents_data.agents.iter().find(|a| a.id == *agent_id) {
        let status = if agent.left_at.is_some() {
            "left"
        } else if agent.is_stale {
            "stale"
        } else {
            "alive"
        };

        Ok(CommandOutput::new()
            .line(format!(
                "id={} role={} status={} tags={} pid={}",
                agent.id,
                agent.role,
                status,
                agent.tags.join(","),
                agent.pid
            ))
            .json(&serde_json::json!({
                "id": agent.id,
                "role": agent.role,
                "status": status,
                "tags": agent.tags,
                "pid": agent.pid
            }))
            .next("naantree status"))
    } else {
        Ok(CommandOutput::new()
            .line(format!("id={} status=not_registered", agent_id))
            .next("naantree join"))
    }
}

pub fn run_heartbeat(ctx: &Context) -> Result<CommandOutput, String> {
    let agent_id = ctx.require_agent()?;
    with_lock(&ctx.state_dir, &agent_id, || {
        let mut agents_data = ctx.store.load_agents();
        let now = current_time_ms();
        let mut found = false;
        for a in &mut agents_data.agents {
            if a.id == agent_id {
                a.last_heartbeat = now;
                a.is_stale = false;
                found = true;
                break;
            }
        }
        if found {
            ctx.store.save_agents(&agents_data)?;
        }
        Ok(CommandOutput::new()
            .line(format!("agent={} heartbeat=ok timestamp={}", agent_id, now))
            .json(&serde_json::json!({
                "agent": agent_id,
                "heartbeat": "ok",
                "timestamp": now
            }))
            .next("naantree status"))
    })
}

pub fn run_leave(ctx: &Context) -> Result<CommandOutput, String> {
    let agent_id = ctx.require_agent()?;
    with_lock(&ctx.state_dir, &agent_id, || {
        let mut agents_data = ctx.store.load_agents();
        let now = current_time_ms();
        let mut was_coordinator = false;

        for a in &mut agents_data.agents {
            if a.id == agent_id {
                a.left_at = Some(now);
                if a.role == "coordinator" {
                    was_coordinator = true;
                }
                break;
            }
        }

        // Release caller's claims
        let mut claims_data = ctx.store.load_claims();
        claims_data.claims.retain(|c| c.agent_id != agent_id);
        ctx.store.save_claims(&claims_data)?;

        // Return doing/review tasks to todo
        let mut tasks_data = ctx.store.load_tasks();
        for t in &mut tasks_data.tasks {
            if t.assignee.as_deref() == Some(&agent_id) && (t.state == "doing" || t.state == "review") {
                t.state = "todo".to_string();
                t.assignee = None;
                t.updated_at = now;
            }
        }
        ctx.store.save_tasks(&tasks_data)?;

        // If coordinator left, promote next lowest-id alive agent
        if was_coordinator {
            let mut candidates: Vec<&mut Agent> = agents_data
                .agents
                .iter_mut()
                .filter(|a| !a.is_stale && a.left_at.is_none() && a.id != agent_id)
                .collect();
            candidates.sort_by_key(|a| extract_id_num(&a.id).unwrap_or(u64::MAX));
            if let Some(promoted) = candidates.first_mut() {
                promoted.role = "coordinator".to_string();
                let _ = ctx.store.log_event(
                    "coordinator_promoted",
                    Some(&promoted.id),
                    serde_json::json!({
                        "promoted_agent": promoted.id,
                        "reason": "coordinator_left"
                    }),
                );
            }
        }

        ctx.store.save_agents(&agents_data)?;
        let _ = ctx.store.log_event(
            "agent_left",
            Some(&agent_id),
            serde_json::json!({ "agent": agent_id }),
        );

        Ok(CommandOutput::new()
            .line(format!("agent={} left=true", agent_id))
            .json(&serde_json::json!({ "agent": agent_id, "left": true }))
            .next("none"))
    })
}
