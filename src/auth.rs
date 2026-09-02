//! GitHub authentication via delegation to the `gh` CLI.
//!
//! Rather than implementing our own OAuth/token storage, we shell out to
//! `gh auth token`, which returns the currently active token managed by the
//! GitHub CLI. This means our app "inherits" whatever account the user is
//! logged in as, including access to private repositories.

use anyhow::{Context, Result};
use std::process::Command;

/// Retrieves a GitHub access token from the `gh` CLI.
///
/// Returns a descriptive error if `gh` is not installed or the user is not
/// logged in, so callers can surface actionable guidance.
pub fn token_from_gh_cli() -> Result<String> {
    let output = Command::new("gh")
        .args(["auth", "token"])
        .output()
        .context(
            "failed to run `gh`. Is the GitHub CLI installed and available on your PATH?\n\
             Install instructions: https://cli.github.com/",
        )?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!(
            "`gh auth token` failed (exit code {:?}):\n{}\n\
             Try running `gh auth login` first.",
            output.status.code(),
            stderr.trim()
        );
    }

    let token = String::from_utf8(output.stdout)
        .context("`gh auth token` produced invalid UTF-8 output")?
        .trim()
        .to_string();

    if token.is_empty() {
        anyhow::bail!(
            "`gh auth token` returned an empty token. Try running `gh auth login` first."
        );
    }

    Ok(token)
}
