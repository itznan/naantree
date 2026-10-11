use std::collections::HashSet;
use std::fs::OpenOptions;
use std::io::Write;
use std::thread;
use std::time::Duration;
use crate::core::context::Context;
use crate::core::format::CommandOutput;
use crate::core::ids::{next_dec_id, next_msg_id};
use crate::core::lock::{current_time_ms, with_lock};
use crate::core::store::{Decision, Message};

pub fn send_message_internal(
    ctx: &Context,
    agent_id: &str,
    to: &str,
    msg_type: &str,
    ref_id: &str,
    text: &str,
    needs_ack: bool,
) -> Result<String, String> {
    let messages: Vec<Message> = ctx.store.read_jsonl("messages.jsonl");
    let existing_ids: Vec<String> = messages.into_iter().map(|m| m.id).collect();
    let msg_id = next_msg_id(&existing_ids);

    let msg = Message {
        id: msg_id.clone(),
        from: agent_id.to_string(),
        to: to.to_string(),
        msg_type: msg_type.to_string(),
        ref_id: ref_id.to_string(),
        text: text.to_string(),
        needs_ack,
        ack_by: Vec::new(),
        timestamp: current_time_ms(),
    };

    ctx.store.append_jsonl("messages.jsonl", &msg)?;

    let _ = ctx.store.log_event(
        "message_sent",
        Some(agent_id),
        serde_json::json!({
            "id": msg_id,
            "to": to,
            "type": msg_type,
            "ref": ref_id,
            "needs_ack": needs_ack
        }),
    );

    Ok(msg_id)
}

pub fn run_send(
    ctx: &Context,
    to: String,
    msg_type: String,
    ref_id: String,
    text: String,
    needs_ack: bool,
) -> Result<CommandOutput, String> {
    let agent_id = ctx.require_agent()?;
    with_lock(&ctx.state_dir, &agent_id, || {
        let msg_id = send_message_internal(ctx, &agent_id, &to, &msg_type, &ref_id, &text, needs_ack)?;

        Ok(CommandOutput::new()
            .line(format!(
                "id={} from={} to={} type={} ref={} text=\"{}\" needs_ack={}",
                msg_id, agent_id, to, msg_type, ref_id, text, needs_ack
            ))
            .json(&serde_json::json!({
                "id": msg_id,
                "from": agent_id,
                "to": to,
                "type": msg_type,
                "ref": ref_id,
                "text": text,
                "needs_ack": needs_ack
            }))
            .next("naantree wait --for message"))
    })
}

pub fn run_inbox(
    ctx: &Context,
    unread_only: bool,
    mark_read: bool,
) -> Result<CommandOutput, String> {
    let agent_id = ctx.require_agent()?;
    with_lock(&ctx.state_dir, &agent_id, || {
        let agents_data = ctx.store.load_agents();
        let caller_agent = agents_data.agents.iter().find(|a| a.id == agent_id);
        let caller_tags: HashSet<String> = caller_agent
            .map(|a| a.tags.iter().cloned().collect())
            .unwrap_or_default();
        let is_coordinator = caller_agent
            .map(|a| a.role == "coordinator")
            .unwrap_or(false);

        let messages: Vec<Message> = ctx.store.read_jsonl("messages.jsonl");
        let mut read_map = ctx.store.load_inbox_reads();
        let mut caller_read = read_map.entry(agent_id.clone()).or_default().clone();

        let mut matched = Vec::new();
        for m in &messages {
            let target = m.to.as_str();
            let is_recipient = target == agent_id
                || target == "all"
                || (target == "coordinator" && is_coordinator)
                || (target.starts_with("role:") && caller_tags.contains(&target[5..]));

            if !is_recipient {
                continue;
            }

            let is_read = caller_read.contains(&m.id);
            if unread_only && is_read {
                continue;
            }

            matched.push(m.clone());
            if mark_read {
                caller_read.insert(m.id.clone());
            }
        }

        if mark_read {
            read_map.insert(agent_id.clone(), caller_read);
            ctx.store.save_inbox_reads(&read_map)?;
        }

        let mut lines = Vec::new();
        for m in &matched {
            let acked = if m.needs_ack {
                format!(" [ack_by:{}]", m.ack_by.join(","))
            } else {
                "".to_string()
            };
            lines.push(format!(
                "id={} from={} type={} ref={} text=\"{}\"{}",
                m.id, m.from, m.msg_type, m.ref_id, m.text, acked
            ));
        }

        if lines.is_empty() {
            lines.push("inbox=empty (no messages)".to_string());
        }

        let has_unacked = matched.iter().any(|m| m.needs_ack && !m.ack_by.contains(&agent_id));
        let next_hint = if has_unacked {
            let unacked_id = matched
                .iter()
                .find(|m| m.needs_ack && !m.ack_by.contains(&agent_id))
                .map(|m| m.id.as_str())
                .unwrap_or("msg-1");
            format!("naantree ack {}", unacked_id)
        } else {
            "naantree status".to_string()
        };

        Ok(CommandOutput::new()
            .lines(lines)
            .json(&serde_json::json!({
                "messages": matched
            }))
            .next(next_hint))
    })
}

pub fn run_ack(ctx: &Context, msg_id: String) -> Result<CommandOutput, String> {
    let agent_id = ctx.require_agent()?;
    with_lock(&ctx.state_dir, &agent_id, || {
        let messages: Vec<Message> = ctx.store.read_jsonl("messages.jsonl");
        let mut found = false;
        let mut updated_messages = Vec::new();

        for mut m in messages {
            if m.id == msg_id {
                if !m.ack_by.contains(&agent_id) {
                    m.ack_by.push(agent_id.clone());
                }
                found = true;
            }
            updated_messages.push(m);
        }

        if !found {
            return Err(format!("Message {} not found", msg_id));
        }

        // Rewrite messages.jsonl
        let target = ctx.state_dir.join("messages.jsonl");
        let tmp = ctx.state_dir.join(format!("messages.tmp.{}.{}", std::process::id(), current_time_ms()));
        let mut file = OpenOptions::new().create(true).write(true).truncate(true).open(&tmp).map_err(|e| e.to_string())?;
        for m in &updated_messages {
            let mut line = serde_json::to_string(m).map_err(|e| e.to_string())?;
            line.push('\n');
            file.write_all(line.as_bytes()).map_err(|e| e.to_string())?;
        }
        std::fs::rename(&tmp, &target).map_err(|e| e.to_string())?;

        // Mark as read
        let mut read_map = ctx.store.load_inbox_reads();
        let caller_read = read_map.entry(agent_id.clone()).or_default();
        caller_read.insert(msg_id.clone());
        ctx.store.save_inbox_reads(&read_map)?;

        Ok(CommandOutput::new()
            .line(format!("ack={} agent={}", msg_id, agent_id))
            .json(&serde_json::json!({
                "ack": msg_id,
                "agent": agent_id
            }))
            .next("naantree status"))
    })
}

pub fn run_thread(ctx: &Context, ref_id: String) -> Result<CommandOutput, String> {
    let messages: Vec<Message> = ctx.store.read_jsonl("messages.jsonl");
    let thread: Vec<Message> = messages
        .into_iter()
        .filter(|m| m.ref_id == ref_id || m.id == ref_id)
        .collect();

    let mut lines = Vec::new();
    for m in &thread {
        lines.push(format!(
            "id={} from={} to={} type={} text=\"{}\"",
            m.id, m.from, m.to, m.msg_type, m.text
        ));
    }

    if lines.is_empty() {
        lines.push(format!("thread={} (no messages)", ref_id));
    }

    Ok(CommandOutput::new()
        .lines(lines)
        .json(&serde_json::json!({
            "thread": thread
        }))
        .next("naantree status"))
}

pub fn run_wait(
    ctx: &Context,
    for_condition: Option<String>,
    timeout_secs: Option<u64>,
) -> Result<CommandOutput, String> {
    let agent_id = ctx.require_agent()?;
    let max_timeout = timeout_secs.unwrap_or(120);
    let start_time = current_time_ms();
    let cond = for_condition.unwrap_or_else(|| "any".to_string());

    loop {
        // Heartbeat & check condition under lock
        let check_res = with_lock(&ctx.state_dir, &agent_id, || {
            let _ = ctx.store.reap_dead_agents();
            let mut agents_data = ctx.store.load_agents();
            let now = current_time_ms();
            let mut is_coordinator = false;
            let mut caller_tags = HashSet::new();

            for a in &mut agents_data.agents {
                if a.id == agent_id {
                    a.last_heartbeat = now;
                    is_coordinator = a.role == "coordinator";
                    caller_tags = a.tags.iter().cloned().collect();
                }
            }
            let _ = ctx.store.save_agents(&agents_data);

            let meta = ctx.store.load_meta();
            if meta.finished {
                return Ok(Some("event=complete reason=\"NAANTREE_COMPLETE\"".to_string()));
            }

            // 1. Check messages
            if cond == "any" || cond == "message" {
                let messages: Vec<Message> = ctx.store.read_jsonl("messages.jsonl");
                let read_map = ctx.store.load_inbox_reads();
                let caller_read = read_map.get(&agent_id).cloned().unwrap_or_default();

                for m in &messages {
                    if caller_read.contains(&m.id) {
                        continue;
                    }
                    let target = m.to.as_str();
                    let matches = target == agent_id
                        || target == "all"
                        || (target == "coordinator" && is_coordinator)
                        || (target.starts_with("role:") && caller_tags.contains(&target[5..]));
                    if matches {
                        return Ok(Some(format!(
                            "event=message id={} from={} type={} ref={}",
                            m.id, m.from, m.msg_type, m.ref_id
                        )));
                    }
                }
            }

            // 2. Check task
            if cond == "any" || cond == "task" {
                let tasks_data = ctx.store.load_tasks();
                let done_set: HashSet<String> = tasks_data
                    .tasks
                    .iter()
                    .filter(|t| t.state == "done")
                    .map(|t| t.id.clone())
                    .collect();

                let has_unfinished_env = tasks_data
                    .tasks
                    .iter()
                    .any(|task| task.kind.as_deref() == Some("env") && task.state != "done");

                let ready_task = tasks_data
                    .tasks
                    .iter()
                    .find(|t| {
                        t.state == "todo"
                            && t.deps.iter().all(|d| done_set.contains(d))
                            && (t.kind.as_deref() == Some("env") || !has_unfinished_env)
                    });

                if let Some(t) = ready_task {
                    return Ok(Some(format!(
                        "event=task id={} ready=true title=\"{}\"",
                        t.id, t.title
                    )));
                }
            }

            // 3. Check claim
            if cond == "any" || cond == "claim" {
                let claims_data = ctx.store.load_claims();
                for c in &claims_data.claims {
                    if c.agent_id == agent_id && c.shared {
                        return Ok(Some(format!("event=claim id={} granted=true", c.id)));
                    }
                }
            }

            // 4. Check unblock
            if cond == "any" || cond == "unblock" {
                let tasks_data = ctx.store.load_tasks();
                if let Some(t) = tasks_data.tasks.iter().find(|t| t.state == "todo" && t.note.as_deref() == Some("unblocked")) {
                    return Ok(Some(format!("event=unblocked id={}", t.id)));
                }
            }

            // 5. Check coordinator
            if cond == "coordinator" {
                if let Some(coord) = agents_data
                    .agents
                    .iter()
                    .find(|a| a.role == "coordinator" && !a.is_stale && a.left_at.is_none())
                {
                    return Ok(Some(format!("event=coordinator id={} alive=true", coord.id)));
                }
            }

            Ok(None)
        })?;

        if let Some(event_str) = check_res {
            let next_hint = if event_str.contains("event=coordinator") {
                if event_str.contains(&format!("id={}", agent_id)) {
                    "naantree plan init --goal \"<description>\""
                } else {
                    "naantree wait --for task"
                }
            } else if event_str.contains("event=task") {
                "naantree task claim"
            } else if event_str.contains("event=message") {
                "naantree inbox --unread"
            } else if event_str.contains("event=complete") {
                "none (NAANTREE_COMPLETE)"
            } else {
                "naantree status"
            };

            return Ok(CommandOutput::new()
                .line(event_str.clone())
                .json(&serde_json::json!({
                    "event": event_str,
                    "agent": agent_id
                }))
                .next(next_hint));
        }

        if (current_time_ms() - start_time) >= max_timeout * 1000 {
            return Ok(CommandOutput::new()
                .line(format!("event=timeout seconds={}", max_timeout))
                .json(&serde_json::json!({
                    "event": "timeout",
                    "seconds": max_timeout
                }))
                .next("naantree status"));
        }

        thread::sleep(Duration::from_secs(2));
    }
}

pub fn run_decide(
    ctx: &Context,
    title: String,
    choice: String,
    why: String,
    alternatives: Option<String>,
) -> Result<CommandOutput, String> {
    let agent_id = ctx.require_agent()?;
    with_lock(&ctx.state_dir, &agent_id, || {
        let decisions: Vec<Decision> = ctx.store.read_jsonl("decisions.jsonl");
        let existing_ids: Vec<String> = decisions.into_iter().map(|d| d.id).collect();
        let dec_id = next_dec_id(&existing_ids);

        let alt_list: Vec<String> = alternatives
            .unwrap_or_default()
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        let decision = Decision {
            id: dec_id.clone(),
            title: title.clone(),
            choice: choice.clone(),
            why: why.clone(),
            alternatives: alt_list.clone(),
            agent_id: agent_id.clone(),
            timestamp: current_time_ms(),
        };

        ctx.store.append_jsonl("decisions.jsonl", &decision)?;

        let _ = ctx.store.log_event(
            "decision_made",
            Some(&agent_id),
            serde_json::json!({
                "id": dec_id,
                "title": title,
                "choice": choice
            }),
        );

        Ok(CommandOutput::new()
            .line(format!(
                "id={} title=\"{}\" choice=\"{}\" decided_by={}",
                dec_id, title, choice, agent_id
            ))
            .json(&serde_json::json!({
                "id": dec_id,
                "title": title,
                "choice": choice,
                "why": why,
                "alternatives": alt_list,
                "decided_by": agent_id
            }))
            .next("naantree status"))
    })
}

pub fn run_decisions(ctx: &Context) -> Result<CommandOutput, String> {
    let decisions: Vec<Decision> = ctx.store.read_jsonl("decisions.jsonl");
    let mut lines = Vec::new();

    for d in &decisions {
        lines.push(format!(
            "id={} agent={} title=\"{}\" choice=\"{}\" why=\"{}\"",
            d.id, d.agent_id, d.title, d.choice, d.why
        ));
    }

    if lines.is_empty() {
        lines.push("decisions=0 (none recorded)".to_string());
    }

    Ok(CommandOutput::new()
        .lines(lines)
        .json(&serde_json::json!({
            "decisions": decisions
        }))
        .next("naantree status"))
}

pub fn run_escalate(ctx: &Context, message: String) -> Result<CommandOutput, String> {
    let agent_id = ctx.require_agent()?;
    let queue_path = ctx.repo_root.join(".naantree").join("HUMAN_QUEUE.md");

    let entry = format!(
        "- **[{}] Agent {}:** {}\n",
        current_time_ms(),
        agent_id,
        message
    );

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&queue_path)
        .map_err(|e| format!("Failed to open HUMAN_QUEUE.md: {}", e))?;

    file.write_all(entry.as_bytes())
        .map_err(|e| format!("Failed to write to HUMAN_QUEUE.md: {}", e))?;

    let _ = ctx.store.log_event(
        "escalated_to_human",
        Some(&agent_id),
        serde_json::json!({ "message": message }),
    );

    Ok(CommandOutput::new()
        .line(format!(
            "escalated=true file=.naantree/HUMAN_QUEUE.md msg=\"{}\"",
            message
        ))
        .json(&serde_json::json!({
            "escalated": true,
            "file": ".naantree/HUMAN_QUEUE.md",
            "message": message
        }))
        .next("naantree status"))
}
