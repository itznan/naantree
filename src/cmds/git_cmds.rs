use crate::core::context::Context;
use crate::core::format::CommandOutput;
use crate::core::git::{git_add_files, git_commit, git_push, git_sync};
use crate::core::lock::with_lock;
use crate::core::paths::{file_matches_pattern, normalize_path};

pub fn run_commit(
    ctx: &Context,
    message: String,
    files: Vec<String>,
) -> Result<CommandOutput, String> {
    let agent_id = ctx.require_agent()?;
    let normalized_files: Vec<String> = files.iter().map(|f| normalize_path(f)).collect();

    if normalized_files.is_empty() {
        return Err("No files specified to commit. Pass --files <file1,file2...>".to_string());
    }

    with_lock(&ctx.state_dir, &agent_id, || {
        let claims_data = ctx.store.load_claims();
        let agent_claims: Vec<String> = claims_data
            .claims
            .iter()
            .filter(|c| c.agent_id == agent_id)
            .flat_map(|c| c.paths.clone())
            .collect();

        // Verify that every single file is claimed by the caller
        for file in &normalized_files {
            let is_claimed = agent_claims
                .iter()
                .any(|pattern| file_matches_pattern(pattern, file));
            if !is_claimed {
                return Err(format!(
                    "Cannot commit: file '{}' is NOT claimed by agent {}. Claim it first with 'naantree claim {}'.",
                    file, agent_id, file
                ));
            }
        }

        // Add ONLY those files
        git_add_files(&ctx.repo_root, &normalized_files)?;

        // Commit with index.lock retries and autostash pull
        let commit_hash = git_commit(&ctx.repo_root, &message)?;

        // Associate commit hash with caller's currently doing task
        let mut tasks_data = ctx.store.load_tasks();
        let mut doing_task_id = None;
        for t in &mut tasks_data.tasks {
            if t.assignee.as_deref() == Some(&agent_id) && t.state == "doing" {
                t.commit_hash = Some(commit_hash.clone());
                doing_task_id = Some(t.id.clone());
                break;
            }
        }
        if doing_task_id.is_some() {
            ctx.store.save_tasks(&tasks_data)?;
        }

        let _ = ctx.store.log_event(
            "commit",
            Some(&agent_id),
            serde_json::json!({
                "hash": commit_hash,
                "files": normalized_files,
                "msg": message,
                "task": doing_task_id
            }),
        );

        let next_hint = if let Some(t_id) = &doing_task_id {
            format!("naantree review request {}", t_id)
        } else {
            "naantree status".to_string()
        };

        Ok(CommandOutput::new()
            .line(format!(
                "commit={} files=\"{}\" msg=\"{}\"",
                commit_hash,
                normalized_files.join(","),
                message
            ))
            .json(&serde_json::json!({
                "commit": commit_hash,
                "files": normalized_files,
                "message": message,
                "task": doing_task_id
            }))
            .next(next_hint))
    })
}

pub fn run_sync(ctx: &Context) -> Result<CommandOutput, String> {
    let agent_id = ctx.require_agent()?;
    with_lock(&ctx.state_dir, &agent_id, || {
        let res = git_sync(&ctx.repo_root)?;
        Ok(CommandOutput::new()
            .line(format!("sync=ok status=\"{}\"", res))
            .json(&serde_json::json!({
                "sync": "ok",
                "output": res
            }))
            .next("naantree task claim"))
    })
}

pub fn run_push(ctx: &Context) -> Result<CommandOutput, String> {
    let agent_id = ctx.require_agent()?;
    with_lock(&ctx.state_dir, &agent_id, || {
        let agents_data = ctx.store.load_agents();
        let agent = agents_data
            .agents
            .iter()
            .find(|a| a.id == agent_id)
            .ok_or_else(|| format!("Agent {} not found", agent_id))?;

        if agent.role != "coordinator" {
            return Err("Only the coordinator can push to remote".to_string());
        }

        let meta = ctx.store.load_meta();
        if !meta.allow_push {
            return Err(
                "Pushing to remote is not allowed by current plan (allow_push is false)."
                    .to_string(),
            );
        }

        let res = git_push(&ctx.repo_root)?;
        Ok(CommandOutput::new()
            .line(format!("push=ok result=\"{}\"", res))
            .json(&serde_json::json!({
                "push": "ok",
                "output": res
            }))
            .next("naantree finish"))
    })
}
