use crate::core::context::Context;
use crate::core::format::CommandOutput;
use crate::core::ids::{extract_id_num, next_claim_id, next_task_id};
use crate::core::lock::{current_time_ms, with_lock};
use crate::core::paths::{normalize_path, paths_overlap};
use crate::core::store::{Claim, Task};

pub fn run_task_add(
    ctx: &Context,
    title: String,
    desc: Option<String>,
    tags: Option<String>,
    paths: Option<String>,
    deps: Option<String>,
    size: Option<String>,
    for_agent: Option<String>,
    reviewers: Option<usize>,
) -> Result<CommandOutput, String> {
    let agent_id = ctx.require_agent()?;
    with_lock(&ctx.state_dir, &agent_id, || {
        let agents_data = ctx.store.load_agents();
        let agent = agents_data
            .agents
            .iter()
            .find(|a| a.id == agent_id)
            .ok_or_else(|| format!("Agent {} not found", agent_id))?;

        let mut tasks_data = ctx.store.load_tasks();
        let existing_ids: Vec<String> = tasks_data.tasks.iter().map(|t| t.id.clone()).collect();
        let task_id = next_task_id(&existing_ids);

        let is_coordinator = agent.role == "coordinator";
        let coordinator_stale = agents_data
            .agents
            .iter()
            .find(|a| a.role == "coordinator")
            .map(|a| a.is_stale || a.left_at.is_some())
            .unwrap_or(true);

        // Workers' tasks are 'proposed' unless coordinator is stale
        let state = if is_coordinator || coordinator_stale {
            "todo".to_string()
        } else {
            "proposed".to_string()
        };

        let tag_list: Vec<String> = tags
            .unwrap_or_default()
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        let path_list: Vec<String> = paths
            .unwrap_or_default()
            .split(',')
            .map(|s| normalize_path(s))
            .filter(|s| !s.is_empty())
            .collect();

        let dep_list: Vec<String> = deps
            .unwrap_or_default()
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        let task_size = size.unwrap_or_else(|| "M".to_string());
        let now = current_time_ms();

        let task = Task {
            id: task_id.clone(),
            title: title.clone(),
            desc: desc.unwrap_or_default(),
            tags: tag_list.clone(),
            paths: path_list.clone(),
            deps: dep_list.clone(),
            size: task_size.clone(),
            state: state.clone(),
            assignee: for_agent.clone(),
            proposed_by: if is_coordinator {
                None
            } else {
                Some(agent_id.clone())
            },
            commit_hash: None,
            reviewers: reviewers.unwrap_or(1),
            reviews: Vec::new(),
            note: None,
            blocked_reason: None,
            summary: None,
            created_at: now,
            updated_at: now,
        };

        tasks_data.tasks.push(task);
        ctx.store.save_tasks(&tasks_data)?;

        let _ = ctx.store.log_event(
            "task_added",
            Some(&agent_id),
            serde_json::json!({
                "id": task_id,
                "title": title,
                "state": state,
                "paths": path_list
            }),
        );

        let next_hint = if state == "proposed" {
            "naantree wait --for task"
        } else {
            "naantree task claim"
        };

        Ok(CommandOutput::new()
            .line(format!(
                "id={} title=\"{}\" state={} size={} paths=\"{}\" deps=\"{}\"",
                task_id,
                title,
                state,
                task_size,
                path_list.join(","),
                dep_list.join(",")
            ))
            .json(&serde_json::json!({
                "id": task_id,
                "title": title,
                "state": state,
                "size": task_size,
                "paths": path_list,
                "deps": dep_list
            }))
            .next(next_hint))
    })
}

pub fn run_task_accept(ctx: &Context, task_id: String) -> Result<CommandOutput, String> {
    let agent_id = ctx.require_agent()?;
    with_lock(&ctx.state_dir, &agent_id, || {
        let agents_data = ctx.store.load_agents();
        let agent = agents_data
            .agents
            .iter()
            .find(|a| a.id == agent_id)
            .ok_or_else(|| format!("Agent {} not found", agent_id))?;

        if agent.role != "coordinator" {
            return Err("Only coordinator can accept proposed tasks".to_string());
        }

        let mut tasks_data = ctx.store.load_tasks();
        let task = tasks_data
            .tasks
            .iter_mut()
            .find(|t| t.id == task_id)
            .ok_or_else(|| format!("Task {} not found", task_id))?;

        if task.state != "proposed" {
            return Err(format!("Task {} is in state '{}', not proposed", task_id, task.state));
        }

        task.state = "todo".to_string();
        task.updated_at = current_time_ms();
        ctx.store.save_tasks(&tasks_data)?;

        let _ = ctx.store.log_event(
            "task_accepted",
            Some(&agent_id),
            serde_json::json!({ "id": task_id }),
        );

        Ok(CommandOutput::new()
            .line(format!("id={} state=todo", task_id))
            .json(&serde_json::json!({ "id": task_id, "state": "todo" }))
            .next("naantree task claim"))
    })
}

pub fn run_task_list(
    ctx: &Context,
    state_filter: Option<String>,
    mine: bool,
    ready: bool,
) -> Result<CommandOutput, String> {
    let tasks_data = ctx.store.load_tasks();
    let caller_id = ctx.agent_id.as_deref();

    // Map of completed tasks for dependency check
    let done_set: std::collections::HashSet<String> = tasks_data
        .tasks
        .iter()
        .filter(|t| t.state == "done")
        .map(|t| t.id.clone())
        .collect();

    let mut filtered = Vec::new();
    for t in &tasks_data.tasks {
        if let Some(st) = &state_filter {
            if &t.state != st {
                continue;
            }
        }
        if mine {
            if t.assignee.as_deref() != caller_id {
                continue;
            }
        }
        if ready {
            if t.state != "todo" {
                continue;
            }
            let all_deps_done = t.deps.iter().all(|d| done_set.contains(d));
            if !all_deps_done {
                continue;
            }
        }
        filtered.push(t);
    }

    let mut lines = Vec::new();
    for t in &filtered {
        lines.push(format!(
            "id={} state={} size={} title=\"{}\" paths=\"{}\" deps=\"{}\" assignee={}",
            t.id,
            t.state,
            t.size,
            t.title,
            t.paths.join(","),
            t.deps.join(","),
            t.assignee.as_deref().unwrap_or("none")
        ));
    }

    if lines.is_empty() {
        lines.push("tasks=0 (no matching tasks)".to_string());
    }

    Ok(CommandOutput::new()
        .lines(lines)
        .json(&serde_json::json!({
            "tasks": filtered
        }))
        .next("naantree task claim"))
}

pub fn run_task_claim(ctx: &Context, task_id_opt: Option<String>) -> Result<CommandOutput, String> {
    let agent_id = ctx.require_agent()?;
    with_lock(&ctx.state_dir, &agent_id, || {
        let _ = ctx.store.reap_dead_agents();
        let agents_data = ctx.store.load_agents();
        let agent = agents_data
            .agents
            .iter()
            .find(|a| a.id == agent_id)
            .ok_or_else(|| format!("Agent {} not found", agent_id))?;

        let mut tasks_data = ctx.store.load_tasks();
        let done_set: std::collections::HashSet<String> = tasks_data
            .tasks
            .iter()
            .filter(|t| t.state == "done")
            .map(|t| t.id.clone())
            .collect();

        let chosen_id = match task_id_opt {
            Some(id) => {
                let t = tasks_data
                    .tasks
                    .iter()
                    .find(|t| t.id == id)
                    .ok_or_else(|| format!("Task {} not found", id))?;
                if t.state != "todo" {
                    return Err(format!("Task {} is not in todo state (currently '{}')", id, t.state));
                }
                let all_deps_done = t.deps.iter().all(|d| done_set.contains(d));
                if !all_deps_done {
                    return Err(format!("Task {} has unfinished dependencies", id));
                }
                id
            }
            None => {
                // Auto-pick: ready tasks, tag match first, then smallest id
                let mut candidates: Vec<&Task> = tasks_data
                    .tasks
                    .iter()
                    .filter(|t| t.state == "todo" && t.deps.iter().all(|d| done_set.contains(d)))
                    .collect();

                if candidates.is_empty() {
                    return Err("No ready tasks available to claim".to_string());
                }

                candidates.sort_by(|a, b| {
                    let a_tag_match = a.tags.iter().any(|tag| agent.tags.contains(tag));
                    let b_tag_match = b.tags.iter().any(|tag| agent.tags.contains(tag));
                    match (a_tag_match, b_tag_match) {
                        (true, false) => std::cmp::Ordering::Less,
                        (false, true) => std::cmp::Ordering::Greater,
                        _ => {
                            let a_num = extract_id_num(&a.id).unwrap_or(u64::MAX);
                            let b_num = extract_id_num(&b.id).unwrap_or(u64::MAX);
                            a_num.cmp(&b_num)
                        }
                    }
                });

                candidates[0].id.clone()
            }
        };

        // Auto-claim its paths
        let task_paths = {
            let t = tasks_data.tasks.iter().find(|t| t.id == chosen_id).unwrap();
            t.paths.clone()
        };

        let mut claims_data = ctx.store.load_claims();
        let now = current_time_ms();

        // Check path collisions
        for path in &task_paths {
            for existing in &claims_data.claims {
                if existing.agent_id != agent_id {
                    for ext_path in &existing.paths {
                        if paths_overlap(path, ext_path) {
                            return Err(format!(
                                "Cannot claim task {}: path '{}' is already claimed by agent {}",
                                chosen_id, path, existing.agent_id
                            ));
                        }
                    }
                }
            }
        }

        // Add claim for paths
        if !task_paths.is_empty() {
            let existing_claim_ids: Vec<String> = claims_data.claims.iter().map(|c| c.id.clone()).collect();
            let claim_id = next_claim_id(&existing_claim_ids);
            let ttl_ms = 20 * 60 * 1000; // 20m default TTL
            claims_data.claims.push(Claim {
                id: claim_id,
                agent_id: agent_id.clone(),
                paths: task_paths.clone(),
                task_id: Some(chosen_id.clone()),
                shared: false,
                ttl_ms,
                created_at: now,
                expires_at: now + ttl_ms,
                queue: Vec::new(),
            });
            ctx.store.save_claims(&claims_data)?;
        }

        // Update task state
        let task = tasks_data.tasks.iter_mut().find(|t| t.id == chosen_id).unwrap();
        task.state = "doing".to_string();
        task.assignee = Some(agent_id.clone());
        task.updated_at = now;
        ctx.store.save_tasks(&tasks_data)?;

        let _ = ctx.store.log_event(
            "task_claimed",
            Some(&agent_id),
            serde_json::json!({
                "id": chosen_id,
                "agent": agent_id,
                "paths": task_paths
            }),
        );

        Ok(CommandOutput::new()
            .line(format!(
                "id={} state=doing assignee={} claimed_paths=\"{}\"",
                chosen_id,
                agent_id,
                task_paths.join(",")
            ))
            .json(&serde_json::json!({
                "id": chosen_id,
                "state": "doing",
                "assignee": agent_id,
                "claimed_paths": task_paths
            }))
            .next("naantree commit -m \"<summary>\" --files <paths>"))
    })
}

pub fn run_task_update(
    ctx: &Context,
    task_id: String,
    state: Option<String>,
    note: Option<String>,
) -> Result<CommandOutput, String> {
    let agent_id = ctx.require_agent()?;
    with_lock(&ctx.state_dir, &agent_id, || {
        let mut tasks_data = ctx.store.load_tasks();
        let task = tasks_data
            .tasks
            .iter_mut()
            .find(|t| t.id == task_id)
            .ok_or_else(|| format!("Task {} not found", task_id))?;

        if let Some(st) = state {
            task.state = st;
        }
        if let Some(nt) = note {
            task.note = Some(nt);
        }
        task.updated_at = current_time_ms();
        let final_state = task.state.clone();
        ctx.store.save_tasks(&tasks_data)?;

        Ok(CommandOutput::new()
            .line(format!("id={} state={} updated=true", task_id, final_state))
            .json(&serde_json::json!({
                "id": task_id,
                "state": final_state,
                "updated": true
            }))
            .next("naantree status"))
    })
}

pub fn run_task_done(
    ctx: &Context,
    task_id: String,
    summary: String,
    commit_opt: Option<String>,
) -> Result<CommandOutput, String> {
    let agent_id = ctx.require_agent()?;
    with_lock(&ctx.state_dir, &agent_id, || {
        let mut tasks_data = ctx.store.load_tasks();
        let task = tasks_data
            .tasks
            .iter_mut()
            .find(|t| t.id == task_id)
            .ok_or_else(|| format!("Task {} not found", task_id))?;

        let commit_hash = commit_opt.or_else(|| task.commit_hash.clone());
        let hash = commit_hash.ok_or_else(|| {
            format!(
                "Task {} has no recorded commit hash. Commit your changes via 'naantree commit' first or pass --commit <hash>.",
                task_id
            )
        })?;

        // Verify review requirement
        if task.reviewers > 0 {
            let approvals = task
                .reviews
                .iter()
                .filter(|r| r.verdict == "approve" && r.reviewer != agent_id)
                .count();

            if approvals < task.reviewers {
                return Err(format!(
                    "Task {} requires {} approval(s) from a different agent before completion (currently has {}). Request review via 'naantree review request {}'.",
                    task_id, task.reviewers, approvals, task_id
                ));
            }
        }

        task.state = "done".to_string();
        task.summary = Some(summary.clone());
        task.commit_hash = Some(hash.clone());
        task.updated_at = current_time_ms();
        ctx.store.save_tasks(&tasks_data)?;

        // Auto-release claims associated with this task
        let mut claims_data = ctx.store.load_claims();
        claims_data.claims.retain(|c| c.task_id.as_deref() != Some(&task_id));
        ctx.store.save_claims(&claims_data)?;

        let _ = ctx.store.log_event(
            "task_done",
            Some(&agent_id),
            serde_json::json!({
                "id": task_id,
                "summary": summary,
                "commit": hash
            }),
        );

        Ok(CommandOutput::new()
            .line(format!(
                "id={} state=done commit={} summary=\"{}\"",
                task_id, hash, summary
            ))
            .json(&serde_json::json!({
                "id": task_id,
                "state": "done",
                "commit": hash,
                "summary": summary
            }))
            .next("naantree release --all"))
    })
}

pub fn run_task_block(
    ctx: &Context,
    task_id: String,
    reason: String,
) -> Result<CommandOutput, String> {
    let agent_id = ctx.require_agent()?;
    with_lock(&ctx.state_dir, &agent_id, || {
        let mut tasks_data = ctx.store.load_tasks();
        let task = tasks_data
            .tasks
            .iter_mut()
            .find(|t| t.id == task_id)
            .ok_or_else(|| format!("Task {} not found", task_id))?;

        task.state = "blocked".to_string();
        task.blocked_reason = Some(reason.clone());
        task.updated_at = current_time_ms();
        ctx.store.save_tasks(&tasks_data)?;

        // Release task's claims so paths are not locked
        let mut claims_data = ctx.store.load_claims();
        claims_data.claims.retain(|c| c.task_id.as_deref() != Some(&task_id));
        ctx.store.save_claims(&claims_data)?;

        let _ = ctx.store.log_event(
            "task_blocked",
            Some(&agent_id),
            serde_json::json!({
                "id": task_id,
                "reason": reason
            }),
        );

        Ok(CommandOutput::new()
            .line(format!("id={} state=blocked reason=\"{}\"", task_id, reason))
            .json(&serde_json::json!({
                "id": task_id,
                "state": "blocked",
                "reason": reason
            }))
            .next(format!(
                "naantree send --to coordinator --type BLOCKED --ref {} -m \"{}\"",
                task_id, reason
            )))
    })
}

pub fn run_task_split(
    ctx: &Context,
    task_id: String,
    into: Option<String>,
) -> Result<CommandOutput, String> {
    let agent_id = ctx.require_agent()?;
    with_lock(&ctx.state_dir, &agent_id, || {
        let mut tasks_data = ctx.store.load_tasks();
        let parent = tasks_data
            .tasks
            .iter()
            .find(|t| t.id == task_id)
            .ok_or_else(|| format!("Task {} not found", task_id))?
            .clone();

        let titles: Vec<String> = into
            .unwrap_or_else(|| format!("{}-part1|{}-part2", parent.title, parent.title))
            .split('|')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        let mut created = Vec::new();
        let now = current_time_ms();

        for title in titles {
            let existing_ids: Vec<String> = tasks_data.tasks.iter().map(|t| t.id.clone()).collect();
            let new_id = next_task_id(&existing_ids);
            let subtask = Task {
                id: new_id.clone(),
                title: title.clone(),
                desc: format!("Subtask of {}", parent.id),
                tags: parent.tags.clone(),
                paths: parent.paths.clone(),
                deps: parent.deps.clone(),
                size: "S".to_string(),
                state: "todo".to_string(),
                assignee: None,
                proposed_by: Some(agent_id.clone()),
                commit_hash: None,
                reviewers: parent.reviewers,
                reviews: Vec::new(),
                note: None,
                blocked_reason: None,
                summary: None,
                created_at: now,
                updated_at: now,
            };
            tasks_data.tasks.push(subtask);
            created.push(new_id);
        }

        ctx.store.save_tasks(&tasks_data)?;

        Ok(CommandOutput::new()
            .line(format!(
                "parent={} split_into=\"{}\"",
                task_id,
                created.join(",")
            ))
            .json(&serde_json::json!({
                "parent": task_id,
                "subtasks": created
            }))
            .next("naantree task claim"))
    })
}
