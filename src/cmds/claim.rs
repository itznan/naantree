use std::thread;
use std::time::Duration;
use crate::core::context::Context;
use crate::core::format::CommandOutput;
use crate::core::ids::next_claim_id;
use crate::core::lock::{current_time_ms, with_lock};
use crate::core::paths::{normalize_path, paths_overlap};
use crate::core::store::Claim;

pub fn parse_ttl_ms(ttl: Option<String>) -> u64 {
    if let Some(s) = ttl {
        let trimmed = s.trim();
        if let Some(stripped) = trimmed.strip_suffix('m') {
            if let Ok(mins) = stripped.parse::<u64>() {
                return mins * 60 * 1000;
            }
        }
        if let Some(stripped) = trimmed.strip_suffix('s') {
            if let Ok(secs) = stripped.parse::<u64>() {
                return secs * 1000;
            }
        }
        if let Ok(val) = trimmed.parse::<u64>() {
            return val * 1000;
        }
    }
    20 * 60 * 1000 // default 20m
}

pub fn run_claim(
    ctx: &Context,
    paths: Vec<String>,
    task_id: Option<String>,
    ttl_opt: Option<String>,
    shared: bool,
    wait: bool,
    timeout_secs: Option<u64>,
) -> Result<CommandOutput, String> {
    let agent_id = ctx.require_agent()?;
    let normalized_paths: Vec<String> = paths.iter().map(|p| normalize_path(p)).collect();

    if normalized_paths.is_empty() {
        return Err("No paths specified to claim".to_string());
    }

    let ttl_ms = if shared {
        5 * 60 * 1000 // shared hot files: 5m
    } else {
        parse_ttl_ms(ttl_opt)
    };

    let max_wait_ms = timeout_secs.unwrap_or(30) * 1000;
    let start_wait = current_time_ms();

    loop {
        let outcome = with_lock(&ctx.state_dir, &agent_id, || {
            let mut claims_data = ctx.store.load_claims();
            let now = current_time_ms();

            // Expire claims past expires_at
            claims_data.claims.retain(|c| c.expires_at > now);

            if shared {
                let target_path = &normalized_paths[0];
                let existing_idx = claims_data
                    .claims
                    .iter()
                    .position(|c| c.paths.contains(target_path));

                match existing_idx {
                    Some(idx) => {
                        let is_owner = claims_data.claims[idx].agent_id == agent_id;
                        if is_owner {
                            claims_data.claims[idx].expires_at = now + ttl_ms;
                            let claim_id = claims_data.claims[idx].id.clone();
                            ctx.store.save_claims(&claims_data)?;
                            return Ok((
                                true,
                                format!("claim={} agent={} path={} status=renewed", claim_id, agent_id, target_path),
                                claim_id,
                            ));
                        }
                        // Not owner: check queue
                        if !claims_data.claims[idx].queue.contains(&agent_id) {
                            claims_data.claims[idx].queue.push(agent_id.clone());
                            ctx.store.save_claims(&claims_data)?;
                        }
                        let owner = claims_data.claims[idx].agent_id.clone();
                        let expires_in = (claims_data.claims[idx].expires_at.saturating_sub(now)) / 1000;
                        if wait {
                            Ok((false, format!("waiting for owner {}", owner), "".to_string()))
                        } else {
                            Err(format!(
                                "Path '{}' is currently held by {} (expires in {}s). Added to waiting queue.",
                                target_path, owner, expires_in
                            ))
                        }
                    }
                    None => {
                        // Create shared claim
                        let existing_ids: Vec<String> = claims_data.claims.iter().map(|c| c.id.clone()).collect();
                        let claim_id = next_claim_id(&existing_ids);
                        claims_data.claims.push(Claim {
                            id: claim_id.clone(),
                            agent_id: agent_id.clone(),
                            paths: vec![target_path.clone()],
                            task_id: task_id.clone(),
                            shared: true,
                            ttl_ms,
                            created_at: now,
                            expires_at: now + ttl_ms,
                            queue: Vec::new(),
                        });
                        ctx.store.save_claims(&claims_data)?;
                        Ok((
                            true,
                            format!("claim={} agent={} path={} shared=true", claim_id, agent_id, target_path),
                            claim_id,
                        ))
                    }
                }
            } else {
                // Non-shared claim: check overlap with all existing active claims by other agents
                for path in &normalized_paths {
                    for ext in &claims_data.claims {
                        if ext.agent_id != agent_id {
                            for ext_path in &ext.paths {
                                if paths_overlap(path, ext_path) {
                                    let expires_in = (ext.expires_at.saturating_sub(now)) / 1000;
                                    return Err(format!(
                                        "Conflict: path '{}' overlaps '{}' claimed by {} (expires in {}s)",
                                        path, ext_path, ext.agent_id, expires_in
                                    ));
                                }
                            }
                        }
                    }
                }

                // Granted!
                let existing_ids: Vec<String> = claims_data.claims.iter().map(|c| c.id.clone()).collect();
                let claim_id = next_claim_id(&existing_ids);
                claims_data.claims.push(Claim {
                    id: claim_id.clone(),
                    agent_id: agent_id.clone(),
                    paths: normalized_paths.clone(),
                    task_id: task_id.clone(),
                    shared: false,
                    ttl_ms,
                    created_at: now,
                    expires_at: now + ttl_ms,
                    queue: Vec::new(),
                });
                ctx.store.save_claims(&claims_data)?;

                Ok((
                    true,
                    format!(
                        "claim={} agent={} paths=\"{}\" expires_in={}s",
                        claim_id,
                        agent_id,
                        normalized_paths.join(","),
                        ttl_ms / 1000
                    ),
                    claim_id,
                ))
            }
        });

        match outcome {
            Ok((true, line, id)) => {
                return Ok(CommandOutput::new()
                    .line(line)
                    .json(&serde_json::json!({
                        "claim": id,
                        "agent": agent_id,
                        "paths": normalized_paths,
                        "granted": true
                    }))
                    .next("naantree commit -m \"<summary>\" --files <paths>"));
            }
            Ok((false, _, _)) => {
                if current_time_ms() - start_wait > max_wait_ms {
                    return Err(format!("Timed out after {}s waiting for claim", max_wait_ms / 1000));
                }
                thread::sleep(Duration::from_millis(500));
            }
            Err(e) => return Err(e),
        }
    }
}

pub fn run_release(
    ctx: &Context,
    paths: Vec<String>,
    all: bool,
) -> Result<CommandOutput, String> {
    let agent_id = ctx.require_agent()?;
    with_lock(&ctx.state_dir, &agent_id, || {
        let mut claims_data = ctx.store.load_claims();
        let now = current_time_ms();
        let normalized: Vec<String> = paths.iter().map(|p| normalize_path(p)).collect();

        let mut released_count = 0;
        let mut i = 0;
        while i < claims_data.claims.len() {
            let claim = &mut claims_data.claims[i];
            let should_release = if claim.agent_id == agent_id {
                if all {
                    true
                } else {
                    claim.paths.iter().any(|p| normalized.contains(p))
                }
            } else {
                false
            };

            if should_release {
                released_count += 1;
                if claim.shared && !claim.queue.is_empty() {
                    // Transfer ownership to next in queue
                    let next_agent = claim.queue.remove(0);
                    claim.agent_id = next_agent;
                    claim.created_at = now;
                    claim.expires_at = now + claim.ttl_ms;
                    i += 1;
                } else {
                    claims_data.claims.remove(i);
                }
            } else {
                i += 1;
            }
        }

        ctx.store.save_claims(&claims_data)?;

        Ok(CommandOutput::new()
            .line(format!(
                "agent={} released={} remaining_active={}",
                agent_id,
                released_count,
                claims_data.claims.len()
            ))
            .json(&serde_json::json!({
                "agent": agent_id,
                "released": released_count,
                "remaining": claims_data.claims.len()
            }))
            .next("naantree task claim"))
    })
}

pub fn run_claims(ctx: &Context) -> Result<CommandOutput, String> {
    let claims_data = ctx.store.load_claims();
    let now = current_time_ms();
    let mut lines = Vec::new();

    for c in &claims_data.claims {
        let remaining_secs = (c.expires_at.saturating_sub(now)) / 1000;
        lines.push(format!(
            "id={} agent={} paths=\"{}\" shared={} expires_in={}s queue=[{}]",
            c.id,
            c.agent_id,
            c.paths.join(","),
            c.shared,
            remaining_secs,
            c.queue.join(",")
        ));
    }

    if lines.is_empty() {
        lines.push("claims=0 (no active claims)".to_string());
    }

    Ok(CommandOutput::new()
        .lines(lines)
        .json(&serde_json::json!({
            "claims": claims_data.claims
        }))
        .next("naantree task claim"))
}
