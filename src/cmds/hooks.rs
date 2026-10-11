use std::fs;
use crate::core::context::Context;
use crate::core::format::CommandOutput;
use crate::core::git::get_staged_files;
use crate::core::paths::file_matches_pattern;

pub fn run_hooks_install(ctx: &Context) -> Result<CommandOutput, String> {
    let hooks_dir = ctx.repo_root.join(".git").join("hooks");
    fs::create_dir_all(&hooks_dir).map_err(|e| e.to_string())?;

    let hook_path = hooks_dir.join("pre-commit");
    let hook_script = r#"#!/bin/sh
# Naantree pre-commit collision guard
if command -v naantree >/dev/null 2>&1; then
    naantree hooks pre-commit
elif [ -f "./target/release/naantree" ]; then
    ./target/release/naantree hooks pre-commit
elif [ -f "./target/debug/naantree" ]; then
    ./target/debug/naantree hooks pre-commit
else
    cargo run --quiet -- hooks pre-commit
fi
"#;

    fs::write(&hook_path, hook_script).map_err(|e| e.to_string())?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&hook_path)
            .map_err(|e| e.to_string())?
            .permissions();
        perms.set_mode(0o755);
        let _ = fs::set_permissions(&hook_path, perms);
    }

    Ok(CommandOutput::new()
        .line(format!("hook_installed={}", hook_path.display()))
        .json(&serde_json::json!({
            "hook": hook_path.to_string_lossy(),
            "installed": true
        }))
        .next("naantree status"))
}

pub fn run_hooks_pre_commit(ctx: &Context) -> Result<CommandOutput, String> {
    if std::env::var("NAANTREE_STRICT").as_deref() == Ok("1")
        && std::env::var("NAANTREE_INSIDE_COMMIT").as_deref() != Ok("1")
    {
        return Err("Commits outside 'naantree commit' are forbidden when NAANTREE_STRICT=1.".to_string());
    }

    let staged = get_staged_files(&ctx.repo_root)?;
    let claims_data = ctx.store.load_claims();
    let caller_id = ctx.agent_id.as_deref().unwrap_or("unknown");

    for file in &staged {
        for claim in &claims_data.claims {
            if claim.agent_id != caller_id {
                for pattern in &claim.paths {
                    if file_matches_pattern(pattern, file) {
                        return Err(format!(
                            "Pre-commit guard rejected: file '{}' is claimed by another agent ({})",
                            file, claim.agent_id
                        ));
                    }
                }
            }
        }
    }

    Ok(CommandOutput::new()
        .line("pre-commit=passed")
        .json(&serde_json::json!({
            "pre-commit": "passed",
            "staged_files": staged
        }))
        .next("none"))
}
