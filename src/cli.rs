//! Command-line interface definition and repo-spec validation.

use anyhow::{bail, Result};
use clap::Parser;
use std::fmt;

/// Watch GitHub Actions workflow runs for one or more repositories in a terminal UI.
///
/// Authentication is delegated to the `gh` CLI (`gh auth token`), so make sure
/// you're logged in (`gh auth login`) if you want to see private repositories.
#[derive(Debug, Parser)]
#[command(name = "gh-actions-tui", version, about)]
pub struct Cli {
    /// Repositories to watch, in `owner/repo` form (space separated).
    #[arg(required = true, value_parser = parse_repo_spec)]
    pub repos: Vec<RepoSpec>,

    /// Number of most-recent runs to fetch per repository.
    #[arg(short, long, default_value_t = 25)]
    pub limit: u8,

    /// Auto-refresh interval, in seconds.
    #[arg(short, long, default_value_t = 30)]
    pub interval: u64,
}

/// A validated `owner/repo` pair.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RepoSpec {
    pub owner: String,
    pub repo: String,
}

impl fmt::Display for RepoSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.owner, self.repo)
    }
}

/// Parses and validates a single `owner/repo` command-line argument.
fn parse_repo_spec(raw: &str) -> Result<RepoSpec, String> {
    let mut parts = raw.splitn(2, '/');
    let owner = parts.next().unwrap_or_default().trim();
    let repo = parts.next().unwrap_or_default().trim();

    if owner.is_empty() || repo.is_empty() || repo.contains('/') {
        return Err(format!(
            "invalid repo spec '{raw}', expected format 'owner/repo'"
        ));
    }

    Ok(RepoSpec {
        owner: owner.to_string(),
        repo: repo.to_string(),
    })
}

impl Cli {
    /// Parses process arguments, returning a friendly error on failure.
    pub fn parse_args() -> Result<Self> {
        let cli = Self::try_parse()?;
        if cli.limit == 0 {
            bail!("--limit must be greater than 0");
        }
        Ok(cli)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_valid_spec() {
        let spec = parse_repo_spec("rust-lang/rust").unwrap();
        assert_eq!(spec.owner, "rust-lang");
        assert_eq!(spec.repo, "rust");
    }

    #[test]
    fn rejects_missing_slash() {
        assert!(parse_repo_spec("rust-lang").is_err());
    }

    #[test]
    fn rejects_extra_slash() {
        assert!(parse_repo_spec("a/b/c").is_err());
    }

    #[test]
    fn rejects_empty_parts() {
        assert!(parse_repo_spec("/repo").is_err());
        assert!(parse_repo_spec("owner/").is_err());
    }
}
