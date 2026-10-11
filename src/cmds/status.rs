use crate::core::context::Context;
use crate::core::format::CommandOutput;
use crate::core::lock::with_lock;
use crate::core::store::Message;
use std::collections::HashSet;

pub fn run_status(ctx: &Context) -> Result<CommandOutput, String> {
    with_lock(&ctx.state_dir, "status", || {
        let _ = ctx.store.reap_dead_agents();
        let agents_data = ctx.store.load_agents();
        let tasks_data = ctx.store.load_tasks();
        let claims_data = ctx.store.load_claims();
        let meta = ctx.store.load_meta();

        let now = crate::core::lock::current_time_ms();
        let coordinator_agent = agents_data
            .agents
            .iter()
            .find(|a| a.role == "coordinator" && !a.is_stale && a.left_at.is_none());

        let (coordinator, coordinator_state) = if let Some(coord) = coordinator_agent {
            (coord.id.clone(), "alive")
        } else if agents_data
            .agents
            .iter()
            .any(|a| a.role == "coordinator" && a.is_stale)
        {
            ("none".to_string(), "stale")
        } else {
            ("none".to_string(), "none")
        };

        let mut alive_agents = Vec::new();
        let mut stale_agents = Vec::new();
        let mut agents_detail = Vec::new();
        for a in &agents_data.agents {
            if a.left_at.is_some() {
                continue;
            }
            let age_secs = now.saturating_sub(a.last_heartbeat) / 1000;
            if a.is_stale {
                stale_agents.push(format!("{}(stale, {}s)", a.id, age_secs));
                agents_detail.push(serde_json::json!({
                    "id": a.id,
                    "role": a.role,
                    "status": "stale",
                    "heartbeat_age_secs": age_secs,
                    "pid": a.pid
                }));
            } else {
                alive_agents.push(format!("{}({}, {}s)", a.id, a.role, age_secs));
                agents_detail.push(serde_json::json!({
                    "id": a.id,
                    "role": a.role,
                    "status": "alive",
                    "heartbeat_age_secs": age_secs,
                    "pid": a.pid
                }));
            }
        }

        let mut todo = 0;
        let mut doing = 0;
        let mut review = 0;
        let mut done = 0;
        let mut blocked = 0;
        let mut proposed = 0;

        for t in &tasks_data.tasks {
            match t.state.as_str() {
                "todo" => todo += 1,
                "doing" => doing += 1,
                "review" => review += 1,
                "done" => done += 1,
                "blocked" => blocked += 1,
                "proposed" => proposed += 1,
                _ => {}
            }
        }

        // Unread messages count for caller
        let mut unread_count = 0;
        if let Some(agent_id) = &ctx.agent_id {
            let messages: Vec<Message> = ctx.store.read_jsonl("messages.jsonl");
            let read_map = ctx.store.load_inbox_reads();
            let read_set = read_map.get(agent_id).cloned().unwrap_or_default();
            let caller_tags: HashSet<String> = agents_data
                .agents
                .iter()
                .find(|a| a.id == *agent_id)
                .map(|a| a.tags.iter().cloned().collect())
                .unwrap_or_default();
            let is_coord = coordinator == *agent_id;

            for m in &messages {
                if read_set.contains(&m.id) {
                    continue;
                }
                let target = m.to.as_str();
                let matches = target == agent_id
                    || target == "all"
                    || (target == "coordinator" && is_coord)
                    || (target.starts_with("role:") && caller_tags.contains(&target[5..]));
                if matches {
                    unread_count += 1;
                }
            }
        }

        let mut lines = Vec::new();
        lines.push(format!("coordinator={}", coordinator));
        lines.push(format!("coordinator_state={}", coordinator_state));
        lines.push(format!(
            "agents=alive:[{}] stale:[{}]",
            alive_agents.join(", "),
            stale_agents.join(", ")
        ));
        lines.push(format!(
            "tasks=total:{} todo:{} doing:{} review:{} done:{} blocked:{} proposed:{}",
            tasks_data.tasks.len(),
            todo,
            doing,
            review,
            done,
            blocked,
            proposed
        ));
        lines.push(format!(
            "claims=active:{} [{}]",
            claims_data.claims.len(),
            claims_data
                .claims
                .iter()
                .map(|c| format!("{}:{}", c.agent_id, c.paths.join(",")))
                .collect::<Vec<_>>()
                .join("; ")
        ));
        lines.push(format!("unread={}", unread_count));

        if let Some(goal) = &meta.goal {
            lines.push(format!("goal=\"{}\"", goal));
        }

        if let Some(gate) = &meta.gate_last_result {
            lines.push(format!("gate={}", if gate.pass { "PASS" } else { "FAIL" }));
        }

        if meta.finished {
            lines.push("status=NAANTREE_COMPLETE".to_string());
        }

        // Determine next hint
        let is_caller_coordinator = ctx.agent_id.as_deref() == Some(&coordinator);
        let caller_doing_task = ctx.agent_id.as_ref().and_then(|id| {
            tasks_data
                .tasks
                .iter()
                .find(|t| t.assignee.as_deref() == Some(id) && t.state == "doing")
        });

        let next_hint = if meta.finished {
            "none (NAANTREE_COMPLETE)"
        } else if coordinator == "none" {
            "naantree coordinator claim"
        } else if meta.goal.is_none() {
            if is_caller_coordinator {
                "naantree plan init --goal \"<description>\""
            } else {
                "naantree wait --for task"
            }
        } else if let Some(_task) = caller_doing_task {
            "naantree commit -m \"<summary>\" --files <paths>"
        } else if todo > 0 {
            "naantree task claim"
        } else if review > 0 {
            "naantree inbox"
        } else if is_caller_coordinator
            && doing == 0
            && review == 0
            && todo == 0
            && !tasks_data.tasks.is_empty()
        {
            "naantree finish"
        } else {
            "naantree wait --timeout 120"
        };

        Ok(CommandOutput::new()
            .lines(lines)
            .json(&serde_json::json!({
                "coordinator": coordinator,
                "coordinator_state": coordinator_state,
                "alive_agents": alive_agents,
                "stale_agents": stale_agents,
                "agents_detail": agents_detail,
                "tasks": {
                    "total": tasks_data.tasks.len(),
                    "todo": todo,
                    "doing": doing,
                    "review": review,
                    "done": done,
                    "blocked": blocked,
                    "proposed": proposed
                },
                "claims_count": claims_data.claims.len(),
                "unread": unread_count,
                "goal": meta.goal,
                "gate_passed": meta.gate_last_result.as_ref().map(|g| g.pass),
                "finished": meta.finished
            }))
            .next(next_hint))
    })
}
