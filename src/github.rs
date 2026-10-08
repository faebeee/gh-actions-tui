//! GitHub API access: builds the octocrab client and fetches workflow runs.

use crate::cli::RepoSpec;
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use http::Uri;
use http_body_util::BodyExt;
use octocrab::Octocrab;

/// A simplified, UI-friendly view of a single workflow run.
#[derive(Debug, Clone)]
pub struct RunInfo {
    pub id: u64,
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

/// A simplified, UI-friendly view of a single job within a workflow run.
#[derive(Debug, Clone)]
pub struct JobInfo {
    pub id: u64,
    pub name: String,
    pub status: String,
    pub conclusion: Option<String>,
    pub steps: Vec<StepInfo>,
}

/// A simplified, UI-friendly view of a single step within a job.
#[derive(Debug, Clone)]
pub struct StepInfo {
    pub number: i64,
    pub name: String,
    pub status: String,
    pub conclusion: Option<String>,
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

    /// Triggers a re-run of only the failed jobs in a workflow run.
    ///
    /// Uses a raw POST since octocrab has no built-in helper for this
    /// endpoint (only `cancel_workflow_run` is wrapped).
    pub async fn rerun_failed_jobs(&self, owner: &str, repo: &str, run_id: u64) -> Result<()> {
        let route = format!("/repos/{owner}/{repo}/actions/runs/{run_id}/rerun-failed-jobs");
        let uri: Uri = route
            .parse()
            .context("failed to build rerun-failed-jobs URI")?;
        octocrab::map_github_error(
            self.octocrab
                ._post(uri, None::<&()>)
                .await
                .context("failed to send rerun-failed-jobs request")?,
        )
        .await
        .map(|_: http::Response<_>| ())
        .context("GitHub rejected the rerun-failed-jobs request")
    }

    /// Fetches the jobs (with their steps) for a single workflow run.
    pub async fn fetch_jobs(&self, owner: &str, repo: &str, run_id: u64) -> Result<Vec<JobInfo>> {
        let page = self
            .octocrab
            .workflows(owner, repo)
            .list_jobs(run_id.into())
            .filter(octocrab::params::workflows::Filter::All)
            .per_page(100)
            .send()
            .await
            .context("failed to list jobs for run")?;

        Ok(page
            .items
            .into_iter()
            .map(|job| JobInfo {
                id: job.id.into_inner(),
                name: job.name,
                status: enum_to_raw_string(&job.status),
                conclusion: job.conclusion.as_ref().map(enum_to_raw_string),
                steps: job
                    .steps
                    .into_iter()
                    .map(|step| StepInfo {
                        number: step.number,
                        name: step.name,
                        status: enum_to_raw_string(&step.status),
                        conclusion: step.conclusion.as_ref().map(enum_to_raw_string),
                    })
                    .collect(),
            })
            .collect())
    }

    /// Fetches the raw log text for a single job, following GitHub's
    /// redirect to the underlying blob storage. Returns the raw bytes
    /// (including ANSI escape codes) so the caller can render them with
    /// their color/style intact (e.g. via `ansi-to-tui`).
    pub async fn fetch_job_log(&self, owner: &str, repo: &str, job_id: u64) -> Result<Vec<u8>> {
        let route = format!("/repos/{owner}/{repo}/actions/jobs/{job_id}/logs");
        let uri: Uri = route.parse().context("failed to build job logs URI")?;
        let response = self
            .octocrab
            ._get(uri)
            .await
            .context("failed to request job logs")?;
        let response = self
            .octocrab
            .follow_location_to_data(response)
            .await
            .context("failed to follow job logs redirect")?;
        let bytes = response
            .into_body()
            .collect()
            .await
            .context("failed to read job logs body")?
            .to_bytes();
        Ok(bytes.to_vec())
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
                        id: run.id.into_inner(),
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

/// Converts a `#[serde(rename_all = "snake_case")]` enum (e.g. octocrab's
/// `Status`/`Conclusion`) into the raw snake_case string GitHub's API uses,
/// so callers can reuse `RunOutcome::classify` without a manual mapping
/// that could drift out of sync with octocrab's variants.
fn enum_to_raw_string<T: serde::Serialize>(value: &T) -> String {
    match serde_json::to_value(value) {
        Ok(serde_json::Value::String(s)) => s,
        _ => "unknown".to_string(),
    }
}
