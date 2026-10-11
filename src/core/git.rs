use std::path::Path;
use std::process::Command;
use std::thread;
use std::time::Duration;

pub fn run_git_cmd(repo_root: &Path, args: &[&str], envs: &[(&str, &str)]) -> Result<String, String> {
    let mut cmd = Command::new("git");
    cmd.current_dir(repo_root);
    cmd.args(args);
    for (k, v) in envs {
        cmd.env(k, v);
    }

    let output = cmd
        .output()
        .map_err(|e| format!("Failed to execute git {}: {}", args.join(" "), e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        return Err(format!(
            "git {} failed (exit code {:?}):\n{}{}",
            args.join(" "),
            output.status.code(),
            stdout,
            stderr
        ));
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

pub fn has_remote(repo_root: &Path) -> bool {
    run_git_cmd(repo_root, &["remote"], &[])
        .map(|out| !out.is_empty())
        .unwrap_or(false)
}

pub fn get_short_head(repo_root: &Path) -> Result<String, String> {
    run_git_cmd(repo_root, &["rev-parse", "--short", "HEAD"], &[])
}

pub fn get_staged_files(repo_root: &Path) -> Result<Vec<String>, String> {
    let out = run_git_cmd(repo_root, &["diff", "--cached", "--name-only"], &[])?;
    Ok(out
        .lines()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect())
}

pub fn wait_and_clear_index_lock(repo_root: &Path) -> Result<(), String> {
    let git_dir = repo_root.join(".git");
    let lock_file = git_dir.join("index.lock");

    for attempt in 1..=10 {
        if !lock_file.exists() {
            return Ok(());
        }
        if let Ok(meta) = std::fs::metadata(&lock_file) {
            if let Ok(modified) = meta.modified() {
                if let Ok(elapsed) = modified.elapsed() {
                    if elapsed.as_secs() > 10 {
                        let _ = std::fs::remove_file(&lock_file);
                        return Ok(());
                    }
                }
            }
        }
        let backoff = Duration::from_millis((attempt * 100).min(1000));
        thread::sleep(backoff);
    }

    if lock_file.exists() {
        let _ = std::fs::remove_file(&lock_file);
    }
    Ok(())
}

pub fn git_add_files(repo_root: &Path, files: &[String]) -> Result<(), String> {
    wait_and_clear_index_lock(repo_root)?;
    let mut args = vec!["add", "--"];
    for f in files {
        args.push(f);
    }
    run_git_cmd(repo_root, &args, &[])?;
    Ok(())
}

pub fn git_commit(repo_root: &Path, message: &str) -> Result<String, String> {
    wait_and_clear_index_lock(repo_root)?;
    run_git_cmd(
        repo_root,
        &["commit", "-m", message],
        &[("NAANTREE_INSIDE_COMMIT", "1")],
    )?;

    if has_remote(repo_root) {
        let _ = run_git_cmd(
            repo_root,
            &["pull", "--rebase", "--autostash"],
            &[],
        );
    }

    get_short_head(repo_root)
}

pub fn git_sync(repo_root: &Path) -> Result<String, String> {
    wait_and_clear_index_lock(repo_root)?;
    if has_remote(repo_root) {
        run_git_cmd(repo_root, &["pull", "--rebase", "--autostash"], &[])
    } else {
        Ok("No remote configured; sync skipped".to_string())
    }
}

pub fn git_push(repo_root: &Path) -> Result<String, String> {
    wait_and_clear_index_lock(repo_root)?;
    if has_remote(repo_root) {
        run_git_cmd(repo_root, &["push", "origin", "HEAD"], &[])
    } else {
        Err("Cannot push: no remote configured".to_string())
    }
}
