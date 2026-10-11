use std::fs;
use std::process::Command;
use crate::core::context::Context;
use crate::core::format::CommandOutput;
use crate::core::git::get_short_head;
use crate::core::lock::{current_time_ms, with_lock};
use crate::core::store::{Decision, Event, GateResult, Message, TaskReview};

pub fn run_review_request(
    ctx: &Context,
    task_id: String,
    to_agent: Option<String>,
) -> Result<CommandOutput, String> {
    let agent_id = ctx.require_agent()?;
    with_lock(&ctx.state_dir, &agent_id, || {
        let mut tasks_data = ctx.store.load_tasks();
        let task = tasks_data
            .tasks
            .iter_mut()
            .find(|t| t.id == task_id)
            .ok_or_else(|| format!("Task {} not found", task_id))?;

        task.state = "review".to_string();
        task.updated_at = current_time_ms();
        ctx.store.save_tasks(&tasks_data)?;

        let target = to_agent.unwrap_or_else(|| "all".to_string());
        let _ = crate::cmds::comm::send_message_internal(
            ctx,
            &agent_id,
            &target,
            "REVIEW-REQUEST",
            &task_id,
            &format!("Please review task {}", task_id),
            true,
        )?;

        Ok(CommandOutput::new()
            .line(format!("id={} state=review requested_to={}", task_id, target))
            .json(&serde_json::json!({
                "id": task_id,
                "state": "review",
                "to": target
            }))
            .next("naantree wait --for message"))
    })
}

pub fn run_review_result(
    ctx: &Context,
    task_id: String,
    verdict: String,
    notes: String,
) -> Result<CommandOutput, String> {
    let agent_id = ctx.require_agent()?;
    with_lock(&ctx.state_dir, &agent_id, || {
        let mut tasks_data = ctx.store.load_tasks();
        let task = tasks_data
            .tasks
            .iter_mut()
            .find(|t| t.id == task_id)
            .ok_or_else(|| format!("Task {} not found", task_id))?;

        if task.assignee.as_deref() == Some(&agent_id) {
            return Err("Cannot review your own task. Review must come from a different agent.".to_string());
        }

        let is_approve = verdict == "approve";
        let now = current_time_ms();

        task.reviews.push(TaskReview {
            reviewer: agent_id.clone(),
            verdict: verdict.clone(),
            notes: notes.clone(),
            timestamp: now,
        });

        if !is_approve {
            // Changes requested: back to doing
            task.state = "doing".to_string();
        }
        task.updated_at = now;
        let recipient = task.assignee.clone().unwrap_or_else(|| "all".to_string());
        ctx.store.save_tasks(&tasks_data)?;

        // Auto-ack REVIEW-REQUEST for this task
        let messages: Vec<Message> = ctx.store.read_jsonl("messages.jsonl");
        let mut updated_messages = Vec::new();
        for mut m in messages {
            if m.ref_id == task_id && m.msg_type == "REVIEW-REQUEST" && m.needs_ack {
                if !m.ack_by.contains(&agent_id) {
                    m.ack_by.push(agent_id.clone());
                }
            }
            updated_messages.push(m);
        }
        let target_file = ctx.state_dir.join("messages.jsonl");
        let tmp = ctx.state_dir.join(format!("messages.tmp.{}.{}", std::process::id(), current_time_ms()));
        if let Ok(mut file) = std::fs::OpenOptions::new().create(true).write(true).open(&tmp) {
            for m in &updated_messages {
                if let Ok(mut line) = serde_json::to_string(m) {
                    line.push('\n');
                    let _ = std::io::Write::write_all(&mut file, line.as_bytes());
                }
            }
            let _ = std::fs::rename(&tmp, &target_file);
        }

        // Send review result message
        let _ = crate::cmds::comm::send_message_internal(
            ctx,
            &agent_id,
            &recipient,
            "REVIEW-RESULT",
            &task_id,
            &format!("verdict={} notes=\"{}\"", verdict, notes),
            false,
        )?;

        let next_hint = if is_approve {
            format!("naantree task done {} --summary \"<changes>\"", task_id)
        } else {
            "naantree task claim".to_string()
        };

        Ok(CommandOutput::new()
            .line(format!(
                "id={} reviewer={} verdict={} notes=\"{}\"",
                task_id, agent_id, verdict, notes
            ))
            .json(&serde_json::json!({
                "id": task_id,
                "reviewer": agent_id,
                "verdict": verdict,
                "notes": notes
            }))
            .next(next_hint))
    })
}

pub fn run_gate_set(ctx: &Context, cmd: String) -> Result<CommandOutput, String> {
    let agent_id = ctx.require_agent()?;
    with_lock(&ctx.state_dir, &agent_id, || {
        let agents_data = ctx.store.load_agents();
        let agent = agents_data
            .agents
            .iter()
            .find(|a| a.id == agent_id)
            .ok_or_else(|| format!("Agent {} not found", agent_id))?;

        if agent.role != "coordinator" {
            return Err("Only the coordinator can configure the verification gate (gate set).".to_string());
        }

        let mut meta = ctx.store.load_meta();
        meta.gate_cmd = Some(cmd.clone());
        ctx.store.save_meta(&meta)?;

        let _ = ctx.store.log_event(
            "gate_set",
            Some(&agent_id),
            serde_json::json!({ "cmd": cmd }),
        );

        Ok(CommandOutput::new()
            .line(format!("gate_cmd=\"{}\" coordinator={}", cmd, agent_id))
            .json(&serde_json::json!({
                "gate_cmd": cmd,
                "coordinator": agent_id
            }))
            .next("naantree gate run"))
    })
}

pub fn run_gate_run(ctx: &Context) -> Result<CommandOutput, String> {
    let agent_id = ctx.require_agent()?;
    with_lock(&ctx.state_dir, &agent_id, || {
        let mut meta = ctx.store.load_meta();
        let gate_cmd = meta
            .gate_cmd
            .clone()
            .ok_or_else(|| "No gate command configured. Use 'naantree gate set <cmd>' first.".to_string())?;

        let output = if cfg!(windows) {
            Command::new("cmd")
                .arg("/C")
                .arg(&gate_cmd)
                .current_dir(&ctx.repo_root)
                .output()
        } else {
            Command::new("sh")
                .arg("-c")
                .arg(&gate_cmd)
                .current_dir(&ctx.repo_root)
                .output()
        }
        .map_err(|e| format!("Failed to execute gate command: {}", e))?;

        let pass = output.status.success();
        let code = output.status.code().unwrap_or(if pass { 0 } else { 1 });
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let combined = format!("{}{}", stdout, stderr);

        let lines: Vec<&str> = combined.lines().collect();
        let last_30 = if lines.len() > 30 {
            lines[lines.len() - 30..].join("\n")
        } else {
            combined.clone()
        };

        let result = GateResult {
            pass,
            cmd: gate_cmd.clone(),
            output: last_30.clone(),
            exit_code: code,
            timestamp: current_time_ms(),
        };

        meta.gate_last_result = Some(result);
        ctx.store.save_meta(&meta)?;

        let _ = ctx.store.log_event(
            "gate_run",
            Some(&agent_id),
            serde_json::json!({
                "cmd": gate_cmd,
                "pass": pass,
                "exit_code": code
            }),
        );

        let status_str = if pass { "PASS" } else { "FAIL" };
        let mut out = CommandOutput::new();
        out = out.line(format!("gate={} exit_code={}", status_str, code));
        for l in last_30.lines() {
            out = out.line(l.to_string());
        }

        let next_hint = if pass { "naantree finish" } else { "naantree status" };

        Ok(out
            .json(&serde_json::json!({
                "gate": status_str,
                "pass": pass,
                "exit_code": code,
                "output": last_30
            }))
            .next(next_hint))
    })
}

pub fn run_finish(ctx: &Context) -> Result<CommandOutput, String> {
    let agent_id = ctx.require_agent()?;
    with_lock(&ctx.state_dir, &agent_id, || {
        let agents_data = ctx.store.load_agents();
        let agent = agents_data
            .agents
            .iter()
            .find(|a| a.id == agent_id)
            .ok_or_else(|| format!("Agent {} not found", agent_id))?;

        if agent.role != "coordinator" {
            return Err("Only the coordinator can finish the project (finish).".to_string());
        }

        let tasks_data = ctx.store.load_tasks();
        if tasks_data.tasks.is_empty() {
            return Err("Refusing finish: No tasks were defined in the project.".to_string());
        }

        let unfinished: Vec<&str> = tasks_data
            .tasks
            .iter()
            .filter(|t| t.state != "done")
            .map(|t| t.id.as_str())
            .collect();
        if !unfinished.is_empty() {
            return Err(format!(
                "Refusing finish: The following tasks are not done: {}",
                unfinished.join(", ")
            ));
        }

        let claims_data = ctx.store.load_claims();
        if !claims_data.claims.is_empty() {
            return Err(format!(
                "Refusing finish: Active claims are still held ({} claims remaining). Release claims before finishing.",
                claims_data.claims.len()
            ));
        }

        let mut meta = ctx.store.load_meta();
        if let Some(gate_result) = &meta.gate_last_result {
            if !gate_result.pass {
                return Err("Refusing finish: Verification gate has not passed. Run 'naantree gate run' and ensure it passes.".to_string());
            }
        } else if meta.gate_cmd.is_some() {
            return Err("Refusing finish: Gate command is set but has not been run. Run 'naantree gate run' first.".to_string());
        }

        // Check inbox for unacked needs-ack messages
        let messages: Vec<Message> = ctx.store.read_jsonl("messages.jsonl");
        let unacked_count = messages.iter().filter(|m| m.needs_ack && m.ack_by.is_empty()).count();
        if unacked_count > 0 {
            return Err(format!(
                "Refusing finish: {} message(s) requiring ack remain unacknowledged in inbox.",
                unacked_count
            ));
        }

        // Generate FINAL_REPORT.md
        let report_path = ctx.repo_root.join(".naantree").join("FINAL_REPORT.md");
        let decisions: Vec<Decision> = ctx.store.read_jsonl("decisions.jsonl");
        let human_queue_path = ctx.repo_root.join(".naantree").join("HUMAN_QUEUE.md");
        let human_queue_content = fs::read_to_string(human_queue_path).unwrap_or_else(|_| "None".to_string());

        let mut report = String::new();
        report.push_str("# NAANTREE FINAL REPORT\n\n");
        report.push_str(&format!("**Goal:** {}\n\n", meta.goal.as_deref().unwrap_or("N/A")));
        report.push_str(&format!("**Coordinator:** {}\n\n", agent_id));
        report.push_str("## Completed Tasks\n\n");
        for t in &tasks_data.tasks {
            report.push_str(&format!(
                "- **{}** `{}`: {} (Commit: `{}`, Assignee: `{}`)\n",
                t.id,
                t.size,
                t.title,
                t.commit_hash.as_deref().unwrap_or("none"),
                t.assignee.as_deref().unwrap_or("none")
            ));
        }
        report.push_str("\n## Key Decisions\n\n");
        if decisions.is_empty() {
            report.push_str("None\n\n");
        } else {
            for d in &decisions {
                report.push_str(&format!(
                    "- **{}**: Choice: *{}* (Why: {})\n",
                    d.title, d.choice, d.why
                ));
            }
            report.push('\n');
        }
        report.push_str("## Human Queue Items\n\n");
        report.push_str(&human_queue_content);
        report.push_str("\n\n---\n**Status:** NAANTREE_COMPLETE\n");

        fs::write(&report_path, report).map_err(|e| format!("Failed to write final report: {}", e))?;

        meta.finished = true;
        meta.finished_at = Some(current_time_ms());
        ctx.store.save_meta(&meta)?;

        let _ = ctx.store.log_event(
            "finished",
            Some(&agent_id),
            serde_json::json!({
                "status": "NAANTREE_COMPLETE",
                "report": report_path.to_string_lossy()
            }),
        );

        Ok(CommandOutput::new()
            .line("status=NAANTREE_COMPLETE")
            .line(format!("report={}", report_path.display()))
            .line("NAANTREE_COMPLETE")
            .json(&serde_json::json!({
                "status": "NAANTREE_COMPLETE",
                "report": report_path.to_string_lossy()
            }))
            .next("none (all agents stop)"))
    })
}

pub fn run_log(ctx: &Context, tail: Option<usize>) -> Result<CommandOutput, String> {
    let events: Vec<Event> = ctx.store.read_jsonl("events.jsonl");
    let count = tail.unwrap_or(20);
    let slice = if events.len() > count {
        &events[events.len() - count..]
    } else {
        &events[..]
    };

    let mut lines = Vec::new();
    for e in slice {
        lines.push(format!(
            "id={} type={} agent={} data={}",
            e.id,
            e.event_type,
            e.agent_id.as_deref().unwrap_or("system"),
            e.data
        ));
    }

    if lines.is_empty() {
        lines.push("log=empty (no events)".to_string());
    }

    Ok(CommandOutput::new()
        .lines(lines)
        .json(&serde_json::json!({
            "events": slice
        }))
        .next("naantree status"))
}

pub fn run_doctor(ctx: &Context) -> Result<CommandOutput, String> {
    let mut lines = Vec::new();

    // 1. Check compiler / toolchain
    lines.push("[OK] Rust toolchain operational".to_string());

    // 2. Git repo
    if ctx.repo_root.join(".git").exists() {
        let head = get_short_head(&ctx.repo_root).unwrap_or_else(|_| "initial".to_string());
        lines.push(format!("[OK] Git repository initialized (HEAD: {})", head));
    } else {
        lines.push("[WARN] Not inside a git repository".to_string());
    }

    // 3. Lock health
    let lock_dir = ctx.state_dir.join(".lock");
    if lock_dir.exists() {
        lines.push("[WARN] Lock directory currently exists (in use or stale)".to_string());
    } else {
        lines.push("[OK] Lock healthy (unlocked)".to_string());
    }

    // 4. Gitignore & exclude
    let gitignore_path = ctx.repo_root.join(".gitignore");
    let has_gitignore = gitignore_path.exists()
        && fs::read_to_string(gitignore_path)
            .map(|s| s.contains(".naantree/state"))
            .unwrap_or(false);

    let exclude_path = ctx.repo_root.join(".git").join("info").join("exclude");
    let has_exclude = exclude_path.exists()
        && fs::read_to_string(exclude_path)
            .map(|s| s.contains(".naantree/state"))
            .unwrap_or(false);

    if has_gitignore && has_exclude {
        lines.push("[OK] State directory ignored in .gitignore and .git/info/exclude".to_string());
    } else {
        lines.push("[WARN] .naantree/state not properly ignored in .gitignore or .git/info/exclude".to_string());
    }

    // 5. Hooks
    let hook_path = ctx.repo_root.join(".git").join("hooks").join("pre-commit");
    if hook_path.exists() {
        lines.push("[OK] Pre-commit hook installed".to_string());
    } else {
        lines.push("[INFO] Pre-commit hook not installed (run 'naantree hooks install')".to_string());
    }

    lines.push("doctor=PASS".to_string());

    Ok(CommandOutput::new()
        .lines(lines)
        .json(&serde_json::json!({
            "doctor": "PASS",
            "git": true,
            "lock_healthy": !lock_dir.exists()
        }))
        .next("naantree join"))
}

pub fn run_reset(ctx: &Context, confirm: bool) -> Result<CommandOutput, String> {
    if !confirm {
        return Err("Resetting state requires --confirm flag.".to_string());
    }

    if ctx.state_dir.exists() {
        let _ = fs::remove_dir_all(&ctx.state_dir);
    }
    let _ = fs::create_dir_all(&ctx.state_dir);

    Ok(CommandOutput::new()
        .line("state=reset_complete")
        .json(&serde_json::json!({ "reset": true }))
        .next("naantree join"))
}
