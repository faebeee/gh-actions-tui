//! GitHub API access: builds the octocrab client and fetches workflow runs.

use crate::cli::RepoSpec;
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use octocrab::Octocrab;

/// A simplified, UI-friendly view of a single workflow run.
#[derive(Debug, Clone)]
pub struct RunInfo {
    pub owner: String,
    pub repo: String,
    pub workflow_name: String,
    pub branch: String,
    pub run_number: i64,
    pub event: String,
    pub status: String,
    pub conclusion: Option<String>,
    pub updated_at: DateTime<Utc>,
    pub html_url: String,
}

impl RunInfo {
    pub fn repo_full_name(&self) -> String {
        format!("{}/{}", self.owner, self.repo)
    }
}

/// The outcome of fetching runs for a single repository: either the list of
/// runs, or an error message to display inline (so one bad repo doesn't
/// crash the whole app).
pub enum RepoFetchResult {
    Ok(Vec<RunInfo>),
    Err { repo: RepoSpec, message: String },
}

/// Thin wrapper around an [`Octocrab`] client scoped to this app's needs.
pub struct GitHubClient {
    octocrab: Octocrab,
}

impl GitHubClient {
    /// Builds a client authenticated with the given personal access token.
    pub fn new(token: String) -> Result<Self> {
        let octocrab = Octocrab::builder()
            .personal_token(token)
            .build()
            .context("failed to build GitHub API client")?;
        Ok(Self { octocrab })
    }

    /// Fetches the most recent workflow runs for every repo, concurrently.
    /// Each repo's result is isolated so a single failure (e.g. no access)
    /// doesn't abort the others.
    pub async fn fetch_all(&self, repos: &[RepoSpec], limit: u8) -> Vec<RepoFetchResult> {
        let futures = repos
            .iter()
            .map(|repo| self.fetch_one(repo.clone(), limit));
        futures::future::join_all(futures).await
    }

    async fn fetch_one(&self, repo: RepoSpec, limit: u8) -> RepoFetchResult {
        let per_page = limit.min(100);
        let result = self
            .octocrab
            .workflows(&repo.owner, &repo.repo)
            .list_all_runs()
            .per_page(per_page)
            .page(1u32)
            .send()
            .await;

        match result {
            Ok(page) => {
                let runs = page
                    .items
                    .into_iter()
                    .take(limit as usize)
                    .map(|run| RunInfo {
                        owner: repo.owner.clone(),
                        repo: repo.repo.clone(),
                        workflow_name: run.name,
                        branch: run.head_branch,
                        run_number: run.run_number,
                        event: run.event,
                        status: run.status,
                        conclusion: run.conclusion,
                        updated_at: run.updated_at,
                        html_url: run.html_url.to_string(),
                    })
                    .collect();
                RepoFetchResult::Ok(runs)
            }
            Err(err) => RepoFetchResult::Err {
                repo,
                message: describe_error(&err),
            },
        }
    }
}

/// Renders an octocrab error into a short, human-readable message.
fn describe_error(err: &octocrab::Error) -> String {
    match err {
        octocrab::Error::GitHub { source, .. } => source.message.clone(),
        other => other.to_string(),
    }
}
