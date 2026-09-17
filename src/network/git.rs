use anyhow::{Context, Result, anyhow};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tracing::info;

/// Clones or updates a git repository at `git_url` to a cached directory,
/// checking out `committish` (branch, tag, or commit hash),
/// and returns the local PathBuf to the cloned repository.
pub fn fetch_git_repo(
    cache_dir: &Path,
    git_url: &str,
    committish: Option<&str>,
) -> Result<(PathBuf, String)> {
    // Generate a stable directory name for this git repo
    let mut hasher = sha2::Sha256::new();
    use sha2::Digest;
    hasher.update(git_url.as_bytes());
    let hash = hex::encode(hasher.finalize());
    let repo_cache_dir = cache_dir.join("git").join(&hash[..16]);

    if !repo_cache_dir.exists() {
        if let Some(parent) = repo_cache_dir.parent() {
            let _ = fs::create_dir_all(parent);
        }

        info!("Cloning git repository {}...", git_url);
        let status = Command::new("git")
            .arg("clone")
            .arg(git_url)
            .arg(&repo_cache_dir)
            .status()
            .with_context(|| format!("Failed to execute git clone for {}", git_url))?;

        if !status.success() {
            let _ = fs::remove_dir_all(&repo_cache_dir);
            return Err(anyhow!("git clone failed for {}", git_url));
        }
    } else {
        // Fetch updates
        let _ = Command::new("git")
            .current_dir(&repo_cache_dir)
            .arg("fetch")
            .arg("--all")
            .arg("--tags")
            .status();
    }

    // Checkout committish if specified
    if let Some(ref_name) = committish {
        let status = Command::new("git")
            .current_dir(&repo_cache_dir)
            .arg("checkout")
            .arg(ref_name)
            .status()
            .with_context(|| {
                format!(
                    "Failed to checkout ref {} in {:?}",
                    ref_name, repo_cache_dir
                )
            })?;

        if !status.success() {
            return Err(anyhow!(
                "git checkout {} failed in {:?}",
                ref_name,
                repo_cache_dir
            ));
        }
    }

    // Get current HEAD commit hash
    let rev_output = Command::new("git")
        .current_dir(&repo_cache_dir)
        .arg("rev-parse")
        .arg("HEAD")
        .output()
        .context("Failed to get git rev-parse HEAD")?;

    let commit_hash = String::from_utf8_lossy(&rev_output.stdout)
        .trim()
        .to_string();

    Ok((repo_cache_dir, commit_hash))
}
