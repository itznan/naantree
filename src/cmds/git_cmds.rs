use crate::core::context::Context;
use crate::core::format::CommandOutput;
use crate::core::git::{git_add_files, git_commit, git_push, git_sync, run_git_cmd};
use crate::core::lock::with_lock;
use crate::core::paths::{file_matches_pattern, normalize_path};

pub fn run_diff(ctx: &Context, staged: bool) -> Result<CommandOutput, String> {
    let agent_id = ctx.require_agent()?;

    // 1. Run git status --porcelain -u to get modified/staged/untracked files
    let status_output = run_git_cmd(&ctx.repo_root, &["status", "--porcelain", "-u"], &[])?;

    let mut seen = std::collections::HashSet::new();
    let mut modified_files = Vec::new();

    for line in status_output.lines() {
        if line.len() < 3 {
            continue;
        }
        let bytes = line.as_bytes();
        let index_status = bytes[0];
        let worktree_status = bytes[1];

        if staged {
            // Staged changes only (index_status is not ' ' and not '?')
            if index_status == b' ' || index_status == b'?' {
                continue;
            }
        } else {
            // All modified / dirty files
            if index_status == b' ' && worktree_status == b' ' {
                continue;
            }
        }

        let remainder = if bytes.len() >= 3 && bytes[2] == b' ' {
            &line[3..]
        } else {
            &line[2..]
        };
        let raw_path = remainder.trim();
        let path_str = if let Some((_, dest)) = raw_path.split_once(" -> ") {
            dest.trim()
        } else {
            raw_path
        };
        let clean_path = path_str.trim_matches('"');
        if clean_path.is_empty() {
            continue;
        }

        let normalized = normalize_path(clean_path);
        if normalized.starts_with(".naantree/state") || normalized == ".naantree" {
            continue;
        }
        if seen.insert(normalized.clone()) {
            modified_files.push(normalized);
        }
    }

    // 2. Load active claims
    let claims_data = ctx.store.load_claims();
    let now = crate::core::lock::current_time_ms();

    let caller_claims: Vec<String> = claims_data
        .claims
        .iter()
        .filter(|c| c.agent_id == agent_id && (c.expires_at == 0 || c.expires_at > now))
        .flat_map(|c| c.paths.clone())
        .collect();

    let mut within_claims = Vec::new();
    let mut outside_claims = Vec::new();

    for file in &modified_files {
        let is_claimed = caller_claims
            .iter()
            .any(|pat| file_matches_pattern(pat, file));
        if is_claimed {
            within_claims.push(file.clone());
        } else {
            outside_claims.push(file.clone());
        }
    }

    // 3. Format clean human and JSON output
    let mut lines = Vec::new();
    lines.push(format!(
        "diff=ok agent={} staged={} total={} within_claims={} outside_claims={}",
        agent_id,
        staged,
        modified_files.len(),
        within_claims.len(),
        outside_claims.len()
    ));

    if modified_files.is_empty() {
        lines.push(if staged {
            "No staged files detected.".to_string()
        } else {
            "No modified files detected.".to_string()
        });
    } else {
        for file in &within_claims {
            lines.push(format!("  [within]  {}", file));
        }

        for file in &outside_claims {
            let other_owner = claims_data
                .claims
                .iter()
                .filter(|c| c.agent_id != agent_id && (c.expires_at == 0 || c.expires_at > now))
                .find(|c| c.paths.iter().any(|pat| file_matches_pattern(pat, file)))
                .map(|c| c.agent_id.clone());

            match other_owner {
                Some(owner) => lines.push(format!("  [outside] {} (claimed by {})", file, owner)),
                None => lines.push(format!("  [outside] {} (unclaimed)", file)),
            }
        }
    }

    let warning = if !outside_claims.is_empty() {
        let msg = format!(
            "WARNING: {} modified file(s) are outside your claims! Claim them before committing: naantree claim {}",
            outside_claims.len(),
            outside_claims.join(" ")
        );
        lines.push(msg.clone());
        Some(msg)
    } else {
        None
    };

    let next_hint = if !outside_claims.is_empty() {
        format!("naantree claim {}", outside_claims.join(" "))
    } else if !within_claims.is_empty() {
        format!(
            "naantree commit -m \"<summary>\" --files {}",
            within_claims.join(",")
        )
    } else {
        "naantree status".to_string()
    };

    let mut json_val = serde_json::json!({
        "diff": "ok",
        "agent": agent_id,
        "staged": staged,
        "total": modified_files.len(),
        "within_claims": within_claims,
        "outside_claims": outside_claims,
    });

    if let Some(warn) = warning {
        json_val["warning"] = serde_json::Value::String(warn);
    }

    Ok(CommandOutput::new()
        .lines(lines)
        .json(&json_val)
        .next(next_hint))
}

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
