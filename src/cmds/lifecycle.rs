use std::fs;
use crate::core::context::{
    get_grandparent_pid, get_parent_pid, get_session_key, get_tty_name, Context,
};
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
    force: bool,
) -> Result<CommandOutput, String> {
    with_lock(&ctx.state_dir, "join", || {
        let _ = ctx.store.reap_dead_agents();
        let mut agents_data = ctx.store.load_agents();
        let now = current_time_ms();
        let ppid = get_parent_pid();

        // Check resume
        if let Some(res_id) = resume {
            // Check if another live coordinator exists
            let other_live_coordinator = agents_data
                .agents
                .iter()
                .any(|a| a.role == "coordinator" && !a.is_stale && a.left_at.is_none() && a.id != res_id);

            let agent = agents_data
                .agents
                .iter_mut()
                .find(|a| a.id == res_id)
                .ok_or_else(|| format!("Agent {} not found to resume", res_id))?;

            if !agent.is_stale && agent.left_at.is_none() && !force {
                return Err(format!(
                    "Agent {} is currently alive. Use --force to resume anyway.",
                    res_id
                ));
            }

            let mut was_demoted = false;
            if agent.role == "coordinator" && other_live_coordinator {
                agent.role = "worker".to_string();
                was_demoted = true;
                let _ = ctx.store.log_event(
                    "coordinator_demoted",
                    Some(&res_id),
                    serde_json::json!({
                        "id": res_id,
                        "reason": "superseded_by_existing_coordinator",
                        "new_role": "worker"
                    }),
                );
            } else if !other_live_coordinator && agent.role != "coordinator" {
                agent.role = "coordinator".to_string();
                let _ = ctx.store.log_event(
                    "coordinator_promoted",
                    Some(&res_id),
                    serde_json::json!({
                        "promoted_agent": res_id,
                        "reason": "resume_no_coordinator_alive"
                    }),
                );
            }

            agent.is_stale = false;
            agent.left_at = None;
            agent.last_heartbeat = now;
            agent.pid = ppid;
            let role = agent.role.clone();
            let id = agent.id.clone();
            ctx.store.save_agents(&agents_data)?;

            // Write session files
            let _ = fs::write(ctx.state_dir.join(format!("me.pid_{}", ppid)), &id);
            let _ = fs::write(ctx.state_dir.join(format!("me.{}", ppid)), &id);
            if let Some(gppid) = get_grandparent_pid(ppid) {
                let _ = fs::write(ctx.state_dir.join(format!("me.pid_{}", gppid)), &id);
                let _ = fs::write(ctx.state_dir.join(format!("me.{}", gppid)), &id);
            }
            if let Some(tty) = get_tty_name() {
                let _ = fs::write(ctx.state_dir.join(format!("me.tty_{}", tty)), &id);
                let _ = fs::write(ctx.state_dir.join(format!("me.{}", tty)), &id);
            }
            let session_key = session_opt.or_else(get_session_key);
            if let Some(key) = &session_key {
                let _ = fs::write(ctx.state_dir.join(format!("me.{}", key)), &id);
            }

            let _ = ctx.store.log_event(
                "agent_resumed",
                Some(&id),
                serde_json::json!({ "id": id, "role": role, "pid": ppid }),
            );

            let coordinator_alive = agents_data
                .agents
                .iter()
                .any(|a| a.role == "coordinator" && !a.is_stale && a.left_at.is_none());

            let mut out = CommandOutput::new();
            out = out.line(format!(
                "agent={} role={} coordinator_alive={} status=resumed pid={}",
                id, role, coordinator_alive, ppid
            ));
            if was_demoted {
                out = out.line(format!(
                    "(Resumed coordinator {} demoted to worker because another live coordinator exists)",
                    id
                ));
            }
            out = out.line(format!("prefix=--as {}", id));
            out = out.line(format!("Pass --as {} on every command", id));

            return Ok(out
                .json(&serde_json::json!({
                    "agent": id,
                    "role": role,
                    "coordinator_alive": coordinator_alive,
                    "status": "resumed",
                    "prefix": format!("--as {}", id),
                    "pid": ppid
                }))
                .next("naantree status"));
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
            pid: ppid,
            joined_at: now,
            last_heartbeat: now,
            left_at: None,
            is_stale: false,
        };

        agents_data.agents.push(new_agent);
        ctx.store.save_agents(&agents_data)?;

        // Write session files
        let _ = fs::write(ctx.state_dir.join(format!("me.pid_{}", ppid)), &agent_id);
        let _ = fs::write(ctx.state_dir.join(format!("me.{}", ppid)), &agent_id);
        if let Some(gppid) = get_grandparent_pid(ppid) {
            let _ = fs::write(ctx.state_dir.join(format!("me.pid_{}", gppid)), &agent_id);
            let _ = fs::write(ctx.state_dir.join(format!("me.{}", gppid)), &agent_id);
        }
        if let Some(tty) = get_tty_name() {
            let _ = fs::write(ctx.state_dir.join(format!("me.tty_{}", tty)), &agent_id);
            let _ = fs::write(ctx.state_dir.join(format!("me.{}", tty)), &agent_id);
        }
        let session_key = session_opt.or_else(get_session_key);
        if let Some(key) = &session_key {
            let _ = fs::write(ctx.state_dir.join(format!("me.{}", key)), &agent_id);
        }

        let _ = ctx.store.log_event(
            "agent_joined",
            Some(&agent_id),
            serde_json::json!({
                "agent": agent_id,
                "role": role,
                "tags": tag_list,
                "pid": ppid
            }),
        );

        let mut out = CommandOutput::new();
        out = out.line(format!(
            "agent={} role={} coordinator_alive=true pid={} tags={}",
            agent_id,
            role,
            ppid,
            tag_list.join(",")
        ));
        out = out.line(format!("prefix=--as {}", agent_id));
        out = out.line(format!("Pass --as {} on every command", agent_id));

        Ok(out
            .json(&serde_json::json!({
                "agent": agent_id,
                "role": role,
                "coordinator_alive": true,
                "prefix": format!("--as {}", agent_id),
                "tags": tag_list,
                "pid": ppid
            }))
            .next("naantree status"))
    })
}

pub fn run_coordinator_claim(ctx: &Context) -> Result<CommandOutput, String> {
    let agent_id = ctx.require_agent()?;
    with_lock(&ctx.state_dir, "coordinator_claim", || {
        let _ = ctx.store.reap_dead_agents()?;
        let mut agents_data = ctx.store.load_agents();
        let now = current_time_ms();

        // Check if another live coordinator exists
        if let Some(live_coord) = agents_data
            .agents
            .iter()
            .find(|a| a.role == "coordinator" && !a.is_stale && a.left_at.is_none() && a.id != agent_id)
        {
            return Err(format!(
                "Cannot claim coordinator role: active coordinator '{}' is still alive",
                live_coord.id
            ));
        }

        let caller = agents_data
            .agents
            .iter_mut()
            .find(|a| a.id == agent_id)
            .ok_or_else(|| format!("Agent {} not found", agent_id))?;

        let was_already_coord = caller.role == "coordinator";
        caller.role = "coordinator".to_string();
        caller.last_heartbeat = now;
        caller.is_stale = false;
        caller.left_at = None;
        ctx.store.save_agents(&agents_data)?;

        if !was_already_coord {
            let _ = ctx.store.log_event(
                "coordinator_promoted",
                Some(&agent_id),
                serde_json::json!({
                    "promoted_agent": agent_id,
                    "reason": "coordinator_claim"
                }),
            );
        }

        Ok(CommandOutput::new()
            .line(format!(
                "agent={} role=coordinator coordinator_alive=true status=claimed",
                agent_id
            ))
            .json(&serde_json::json!({
                "agent": agent_id,
                "role": "coordinator",
                "coordinator_alive": true,
                "status": "claimed"
            }))
            .next("naantree plan init --goal \"<description>\""))
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
