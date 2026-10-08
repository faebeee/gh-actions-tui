//! Application state: holds the current set of runs, selection, and status.

use crate::cli::RepoSpec;
use crate::github::{JobInfo, RepoFetchResult, RunInfo};
use crate::status::RunOutcome;
use chrono::{DateTime, Utc};
use ratatui::text::Text;
use ratatui::widgets::TableState;

/// A row of data displayed in the runs table, or an inline error for a repo
/// that failed to fetch.
#[derive(Debug, Clone)]
pub enum Row {
    Run(RunInfo),
    Error { repo: RepoSpec, message: String },
}

/// A status filter applied to the runs table. Cycled through via the `f`/`F`
/// keys; `Row::Error` entries always remain visible regardless of the active
/// filter, since they signal a fetch problem rather than a run status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StatusFilter {
    #[default]
    All,
    Success,
    Failure,
    InProgress,
}

impl StatusFilter {
    /// Whether a run's status/conclusion matches this filter.
    fn matches(&self, status: &str, conclusion: Option<&str>) -> bool {
        let outcome = RunOutcome::classify(status, conclusion);
        match self {
            StatusFilter::All => true,
            StatusFilter::Success => outcome == RunOutcome::Success,
            StatusFilter::Failure => outcome == RunOutcome::Failure,
            StatusFilter::InProgress => outcome == RunOutcome::InProgress,
        }
    }

    /// The next filter in the cycle (used by the `f` key).
    pub fn next(self) -> Self {
        match self {
            StatusFilter::All => StatusFilter::Success,
            StatusFilter::Success => StatusFilter::Failure,
            StatusFilter::Failure => StatusFilter::InProgress,
            StatusFilter::InProgress => StatusFilter::All,
        }
    }

    /// The previous filter in the cycle (used by the `F` key).
    pub fn prev(self) -> Self {
        match self {
            StatusFilter::All => StatusFilter::InProgress,
            StatusFilter::Success => StatusFilter::All,
            StatusFilter::Failure => StatusFilter::Success,
            StatusFilter::InProgress => StatusFilter::Failure,
        }
    }

    /// A short human-readable label for display in the UI.
    pub fn label(&self) -> &'static str {
        match self {
            StatusFilter::All => "All",
            StatusFilter::Success => "Success",
            StatusFilter::Failure => "Failure",
            StatusFilter::InProgress => "In Progress",
        }
    }
}

/// The three actions available in the per-row action menu.
pub const MENU_ITEM_COUNT: usize = 3;

/// Labels for the per-row action menu, in index order.
pub const MENU_LABELS: [&str; MENU_ITEM_COUNT] =
    ["Open in browser", "Rerun failed jobs", "View logs"];

/// An overlay shown on top of the table: either the per-row action menu, or
/// a confirmation prompt before rerunning failed jobs. Mutually exclusive
/// with normal table navigation while active.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Overlay {
    #[default]
    None,
    Menu {
        selected: usize,
    },
    ConfirmRerun,
}

/// The result of activating a menu item: most actions are handled entirely
/// within `App`, but some (like fetching jobs for the logs view) need
/// `main.rs` to dispatch an async API call, so the identity of the run is
/// returned here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuAction {
    /// No further action needed (handled synchronously, e.g. opened the
    /// browser, or moved into the rerun confirm prompt).
    None,
    /// The user chose "View logs"; `main.rs` should fetch the run's jobs
    /// for `(owner, repo, run_id)`.
    ViewLogsRequested { owner: String, repo: String, run_id: u64 },
}

/// The full-screen view currently shown, replacing the runs table. Distinct
/// from [`Overlay`], which is a popup shown on top of the table; `Screen`
/// variants other than `Runs` take over the entire frame.
#[derive(Default)]
pub enum Screen {
    #[default]
    Runs,
    Jobs(JobsScreen),
    Log(LogScreen),
}

/// State for the "Jobs" screen: lists the jobs (and their steps) for a
/// single workflow run.
pub struct JobsScreen {
    pub owner: String,
    pub repo: String,
    pub run_id: u64,
    pub workflow_name: String,
    pub run_number: i64,
    pub jobs: Vec<JobInfo>,
    pub selected: usize,
    pub loading: bool,
    pub error: Option<String>,
}

impl JobsScreen {
    fn new(owner: String, repo: String, run_id: u64, workflow_name: String, run_number: i64) -> Self {
        Self {
            owner,
            repo,
            run_id,
            workflow_name,
            run_number,
            jobs: Vec::new(),
            selected: 0,
            loading: true,
            error: None,
        }
    }
}

/// State for the "Log" screen: the raw (ANSI-colored) log text for a single
/// job, with a scroll position.
pub struct LogScreen {
    pub owner: String,
    pub repo: String,
    pub job_id: u64,
    pub job_name: String,
    pub text: Text<'static>,
    pub line_count: usize,
    pub scroll: u16,
    pub loading: bool,
    pub error: Option<String>,
    /// The `Jobs` screen this log was opened from, so `Esc` can return to
    /// it exactly as it was (without re-fetching).
    jobs_screen: Box<JobsScreen>,
}

impl LogScreen {
    fn new(job_id: u64, job_name: String, jobs_screen: JobsScreen) -> Self {
        Self {
            owner: jobs_screen.owner.clone(),
            repo: jobs_screen.repo.clone(),
            job_id,
            job_name,
            text: Text::default(),
            line_count: 0,
            scroll: 0,
            loading: true,
            error: None,
            jobs_screen: Box::new(jobs_screen),
        }
    }
}

/// Overall application state, updated by the event loop and rendered by the
/// UI layer.
///
/// `rows`, `filter`, `visible`, and `table_state` are kept private: they
/// have interdependent invariants (the cached `visible` index list must
/// always reflect the current `rows`/`filter`, and `table_state`'s
/// selection must always be within bounds of `visible`), which are only
/// maintained if all mutation goes through `App`'s methods.
pub struct App {
    pub repos: Vec<RepoSpec>,
    pub limit: u8,
    rows: Vec<Row>,
    filter: StatusFilter,
    /// Cached indices into `rows` that pass the active `filter`. Recomputed
    /// by `refresh_visible` whenever `rows` or `filter` changes, so
    /// navigation and rendering share one consistent snapshot instead of
    /// each recomputing the filter independently.
    visible: Vec<usize>,
    table_state: TableState,
    pub last_refresh: Option<DateTime<Utc>>,
    pub refreshing: bool,
    pub status_message: Option<String>,
    pub should_quit: bool,
    pub overlay: Overlay,
    pub screen: Screen,
}

impl App {
    pub fn new(repos: Vec<RepoSpec>, limit: u8) -> Self {
        Self {
            repos,
            limit,
            rows: Vec::new(),
            filter: StatusFilter::default(),
            visible: Vec::new(),
            table_state: TableState::default(),
            last_refresh: None,
            refreshing: false,
            status_message: Some("Loading...".to_string()),
            should_quit: false,
            overlay: Overlay::None,
            screen: Screen::Runs,
        }
    }

    /// The currently active status filter.
    pub fn filter(&self) -> StatusFilter {
        self.filter
    }

    /// Mutable access to the table's selection/scroll state, for rendering.
    pub fn table_state(&mut self) -> &mut TableState {
        &mut self.table_state
    }

    /// Whether there are any rows at all, ignoring the active filter.
    pub fn has_rows(&self) -> bool {
        !self.rows.is_empty()
    }

    /// The rows that pass the active status filter, in display order.
    pub fn visible_rows(&self) -> impl Iterator<Item = &Row> + '_ {
        self.visible.iter().map(move |&i| &self.rows[i])
    }

    /// Number of rows that pass the active status filter.
    pub fn visible_len(&self) -> usize {
        self.visible.len()
    }

    /// Recomputes the cached `visible` index list from `rows` and `filter`,
    /// then clamps the selection to stay within bounds. Must be called
    /// after any change to `rows` or `filter`.
    fn refresh_visible(&mut self) {
        self.visible = self
            .rows
            .iter()
            .enumerate()
            .filter(|(_, row)| match row {
                Row::Run(run) => self.filter.matches(&run.status, run.conclusion.as_deref()),
                Row::Error { .. } => true,
            })
            .map(|(i, _)| i)
            .collect();
        self.clamp_selection();
    }

    /// Cycles to the next status filter and refreshes the visible set.
    pub fn cycle_filter_next(&mut self) {
        self.filter = self.filter.next();
        self.refresh_visible();
    }

    /// Cycles to the previous status filter and refreshes the visible set.
    pub fn cycle_filter_prev(&mut self) {
        self.filter = self.filter.prev();
        self.refresh_visible();
    }

    /// Marks a refresh as started (updates status/spinner state).
    pub fn begin_refresh(&mut self) {
        self.refreshing = true;
    }

    /// Applies freshly fetched results, replacing the current rows and
    /// re-sorting by most-recently-updated first.
    pub fn apply_results(&mut self, results: Vec<RepoFetchResult>) {
        let mut rows = Vec::new();
        for result in results {
            match result {
                RepoFetchResult::Ok(runs) => rows.extend(runs.into_iter().map(Row::Run)),
                RepoFetchResult::Err { repo, message } => {
                    rows.push(Row::Error { repo, message })
                }
            }
        }
        rows.sort_by_key(|a| std::cmp::Reverse(a.sort_key()));

        self.rows = rows;
        self.refreshing = false;
        self.last_refresh = Some(Utc::now());
        self.status_message = None;
        self.refresh_visible();
    }

    pub fn set_status(&mut self, message: impl Into<String>) {
        self.status_message = Some(message.into());
    }

    /// The currently selected row, if any, as a `RunInfo` (i.e. not an
    /// error row). Public so the UI layer can render overlay text (e.g.
    /// the rerun confirmation prompt).
    pub fn selected_run(&self) -> Option<&RunInfo> {
        let i = self.table_state.selected()?;
        let &row_idx = self.visible.get(i)?;
        match self.rows.get(row_idx) {
            Some(Row::Run(run)) => Some(run),
            _ => None,
        }
    }

    /// Opens the currently selected row's run URL in the system's default
    /// browser, if the selected row is a valid run (not an error row).
    pub fn open_selected(&mut self) {
        let Some(run) = self.selected_run() else {
            self.set_status("No run URL available for this row");
            return;
        };
        match open::that(&run.html_url) {
            Ok(()) => self.set_status(format!("Opened {}", run.html_url)),
            Err(e) => self.set_status(format!("Failed to open browser: {e}")),
        }
    }

    /// Whether the currently selected run's outcome is a failure (i.e. it
    /// has failed jobs eligible for a rerun). Public so the UI layer can
    /// dim/disable the "Rerun failed jobs" menu item accordingly.
    pub fn selected_run_failed(&self) -> bool {
        self.selected_run()
            .map(|run| {
                RunOutcome::classify(&run.status, run.conclusion.as_deref()) == RunOutcome::Failure
            })
            .unwrap_or(false)
    }

    /// Opens the per-row action menu for the selected row. No-op if the
    /// selected row isn't a valid run (e.g. an error row, or nothing
    /// selected).
    pub fn open_menu(&mut self) {
        if self.selected_run().is_none() {
            return;
        }
        self.overlay = Overlay::Menu { selected: 0 };
    }

    /// Closes any active overlay (menu or confirm prompt) without taking
    /// action.
    pub fn close_overlay(&mut self) {
        self.overlay = Overlay::None;
    }

    /// Moves the menu selection to the next item, wrapping around.
    pub fn menu_select_next(&mut self) {
        if let Overlay::Menu { selected } = &mut self.overlay {
            *selected = (*selected + 1) % MENU_ITEM_COUNT;
        }
    }

    /// Moves the menu selection to the previous item, wrapping around.
    pub fn menu_select_prev(&mut self) {
        if let Overlay::Menu { selected } = &mut self.overlay {
            *selected = (*selected + MENU_ITEM_COUNT - 1) % MENU_ITEM_COUNT;
        }
    }

    /// Activates the currently selected menu item.
    pub fn menu_activate(&mut self) -> MenuAction {
        let Overlay::Menu { selected } = self.overlay else {
            return MenuAction::None;
        };
        match selected {
            0 => {
                self.overlay = Overlay::None;
                self.open_selected();
                MenuAction::None
            }
            1 => {
                self.request_rerun_direct();
                MenuAction::None
            }
            2 => self.request_view_logs(),
            _ => unreachable!("menu selection out of range"),
        }
    }

    /// Requests a rerun of failed jobs for the selected row, bypassing the
    /// menu (used by the direct `R` shortcut, and by the menu's "Rerun
    /// failed jobs" item). Only proceeds to the confirmation prompt if the
    /// selected run actually has a failed outcome.
    pub fn request_rerun_direct(&mut self) {
        if self.selected_run().is_none() {
            self.overlay = Overlay::None;
            self.set_status("No run selected");
            return;
        }
        if !self.selected_run_failed() {
            self.overlay = Overlay::None;
            self.set_status("No failed jobs to rerun");
            return;
        }
        self.overlay = Overlay::ConfirmRerun;
    }

    /// Confirms the pending rerun, returning the `(owner, repo, run_id)`
    /// needed to make the API call, and closing the overlay. Returns
    /// `None` if there's no confirm prompt active or the selected run is
    /// no longer valid.
    pub fn confirm_rerun(&mut self) -> Option<(String, String, u64)> {
        if self.overlay != Overlay::ConfirmRerun {
            return None;
        }
        self.overlay = Overlay::None;
        let run = self.selected_run()?;
        Some((run.owner.clone(), run.repo.clone(), run.id))
    }

    /// Cancels the pending rerun confirmation without taking action.
    pub fn cancel_overlay(&mut self) {
        self.close_overlay();
    }

    /// Applies the result of a rerun-failed-jobs API call, updating the
    /// status message.
    pub fn apply_rerun_result(&mut self, result: Result<(), String>) {
        match result {
            Ok(()) => self.set_status("Rerun of failed jobs triggered"),
            Err(e) => self.set_status(format!("Failed to trigger rerun: {e}")),
        }
    }

    /// Requests the jobs for the selected run, closing the menu and
    /// transitioning to the `Jobs` screen (shown in a loading state until
    /// `apply_jobs_result` is called). Returns the `(owner, repo, run_id)`
    /// needed for `main.rs` to dispatch the async fetch, or `MenuAction::None`
    /// if there's no valid selected run.
    fn request_view_logs(&mut self) -> MenuAction {
        self.overlay = Overlay::None;
        let Some(run) = self.selected_run() else {
            self.set_status("No run selected");
            return MenuAction::None;
        };
        let (owner, repo, run_id) = (run.owner.clone(), run.repo.clone(), run.id);
        self.screen = Screen::Jobs(JobsScreen::new(
            owner.clone(),
            repo.clone(),
            run_id,
            run.workflow_name.clone(),
            run.run_number,
        ));
        MenuAction::ViewLogsRequested {
            owner,
            repo,
            run_id,
        }
    }

    /// Applies the result of a jobs-list fetch to the active `Jobs` screen.
    /// No-op if the screen has since changed (e.g. the user navigated back).
    pub fn apply_jobs_result(&mut self, result: Result<Vec<JobInfo>, String>) {
        if let Screen::Jobs(jobs) = &mut self.screen {
            jobs.loading = false;
            match result {
                Ok(fetched) => {
                    jobs.jobs = fetched;
                    jobs.selected = 0;
                }
                Err(e) => jobs.error = Some(e),
            }
        }
    }

    /// Moves the jobs-screen selection to the next job, wrapping around.
    pub fn jobs_select_next(&mut self) {
        if let Screen::Jobs(jobs) = &mut self.screen {
            if !jobs.jobs.is_empty() {
                jobs.selected = (jobs.selected + 1) % jobs.jobs.len();
            }
        }
    }

    /// Moves the jobs-screen selection to the previous job, wrapping around.
    pub fn jobs_select_prev(&mut self) {
        if let Screen::Jobs(jobs) = &mut self.screen {
            if !jobs.jobs.is_empty() {
                jobs.selected = (jobs.selected + jobs.jobs.len() - 1) % jobs.jobs.len();
            }
        }
    }

    /// Requests the raw log for the currently selected job in the `Jobs`
    /// screen, transitioning to the `Log` screen (loading state). Returns
    /// the `(owner, repo, job_id)` needed for `main.rs` to dispatch the
    /// async fetch, or `None` if there's no valid job selected (in which
    /// case the screen is left unchanged).
    pub fn request_view_log(&mut self) -> Option<(String, String, u64)> {
        let Screen::Jobs(jobs) = &self.screen else {
            return None;
        };
        jobs.jobs.get(jobs.selected)?;
        // Take ownership of the current `Jobs` screen so it can be stashed
        // inside the new `Log` screen (and restored by `back_to_jobs`).
        let Screen::Jobs(jobs) = std::mem::replace(&mut self.screen, Screen::Runs) else {
            unreachable!("checked above");
        };
        let job = jobs.jobs[jobs.selected].clone();
        let owner = jobs.owner.clone();
        let repo = jobs.repo.clone();
        let job_id = job.id;
        self.screen = Screen::Log(LogScreen::new(job_id, job.name.clone(), jobs));
        Some((owner, repo, job_id))
    }

    /// Applies the result of a job-log fetch to the active `Log` screen.
    /// Converts the raw bytes (including ANSI escape codes) into a styled
    /// [`Text`] via `ansi-to-tui`, and scrolls to the first line containing
    /// GitHub's `##[error]` marker, if any. No-op if the screen has since
    /// changed.
    pub fn apply_log_result(&mut self, result: Result<Vec<u8>, String>) {
        use ansi_to_tui::IntoText;

        if let Screen::Log(log) = &mut self.screen {
            log.loading = false;
            match result {
                Ok(bytes) => match bytes.into_text() {
                    Ok(text) => {
                        log.line_count = text.lines.len();
                        log.scroll = text
                            .lines
                            .iter()
                            .position(|line| {
                                line.spans
                                    .iter()
                                    .any(|span| span.content.contains("##[error]"))
                            })
                            .map(|i| i as u16)
                            .unwrap_or(0);
                        log.text = text;
                    }
                    Err(e) => log.error = Some(format!("Failed to parse log: {e}")),
                },
                Err(e) => log.error = Some(e),
            }
        }
    }

    /// Scrolls the log view down by `n` lines, clamped to the last line.
    pub fn log_scroll_down(&mut self, n: u16) {
        if let Screen::Log(log) = &mut self.screen {
            let max = log.line_count.saturating_sub(1) as u16;
            log.scroll = log.scroll.saturating_add(n).min(max);
        }
    }

    /// Scrolls the log view up by `n` lines, clamped to the top.
    pub fn log_scroll_up(&mut self, n: u16) {
        if let Screen::Log(log) = &mut self.screen {
            log.scroll = log.scroll.saturating_sub(n);
        }
    }

    /// Jumps the log view to the first line.
    pub fn log_scroll_to_top(&mut self) {
        if let Screen::Log(log) = &mut self.screen {
            log.scroll = 0;
        }
    }

    /// Jumps the log view to the last line.
    pub fn log_scroll_to_bottom(&mut self) {
        if let Screen::Log(log) = &mut self.screen {
            log.scroll = log.line_count.saturating_sub(1) as u16;
        }
    }

    /// Returns from the `Jobs` or `Log` screen back to the runs table.
    pub fn back_to_runs(&mut self) {
        self.screen = Screen::Runs;
    }

    /// Returns from the `Log` screen back to the `Jobs` screen it was
    /// opened from, exactly as it was (no re-fetch). No-op if not
    /// currently on the `Log` screen.
    pub fn back_to_jobs(&mut self) {
        if let Screen::Log(_) = &self.screen {
            let Screen::Log(log) = std::mem::replace(&mut self.screen, Screen::Runs) else {
                unreachable!("checked above");
            };
            self.screen = Screen::Jobs(*log.jobs_screen);
        }
    }

    pub fn quit(&mut self) {
        self.should_quit = true;
    }

    pub fn select_next(&mut self) {
        if self.visible.is_empty() {
            return;
        }
        let next = match self.table_state.selected() {
            Some(i) if i + 1 < self.visible.len() => i + 1,
            Some(_) => self.visible.len() - 1,
            None => 0,
        };
        self.table_state.select(Some(next));
    }

    pub fn select_prev(&mut self) {
        if self.visible.is_empty() {
            return;
        }
        let prev = match self.table_state.selected() {
            Some(i) if i > 0 => i - 1,
            _ => 0,
        };
        self.table_state.select(Some(prev));
    }

    fn clamp_selection(&mut self) {
        let len = self.visible.len();
        if len == 0 {
            self.table_state.select(None);
        } else if self.table_state.selected().is_none() {
            self.table_state.select(Some(0));
        } else if let Some(i) = self.table_state.selected() {
            if i >= len {
                self.table_state.select(Some(len - 1));
            }
        }
    }
}

impl Row {
    /// Sort key: errors sort using epoch (oldest), so real runs (usually
    /// more interesting) surface first when mixed sorting occurs; within
    /// runs, sorts by updated_at.
    fn sort_key(&self) -> DateTime<Utc> {
        match self {
            Row::Run(run) => run.updated_at,
            Row::Error { .. } => DateTime::<Utc>::MIN_UTC,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::github::StepInfo;

    /// Builds a minimal `RunInfo` for tests, with only `status`/`conclusion`
    /// varying between cases.
    fn run(status: &str, conclusion: Option<&str>) -> RunInfo {
        RunInfo {
            id: 1,
            owner: "owner".to_string(),
            repo: "repo".to_string(),
            workflow_name: "CI".to_string(),
            branch: "main".to_string(),
            run_number: 1,
            event: "push".to_string(),
            status: status.to_string(),
            conclusion: conclusion.map(str::to_string),
            updated_at: Utc::now(),
            html_url: "https://github.com/owner/repo/actions/runs/1".to_string(),
        }
    }

    fn repo_spec() -> RepoSpec {
        RepoSpec {
            owner: "owner".to_string(),
            repo: "repo".to_string(),
        }
    }

    #[test]
    fn status_filter_cycles_forward_and_wraps() {
        assert_eq!(StatusFilter::All.next(), StatusFilter::Success);
        assert_eq!(StatusFilter::Success.next(), StatusFilter::Failure);
        assert_eq!(StatusFilter::Failure.next(), StatusFilter::InProgress);
        assert_eq!(StatusFilter::InProgress.next(), StatusFilter::All);
    }

    #[test]
    fn status_filter_cycles_backward_and_wraps() {
        assert_eq!(StatusFilter::All.prev(), StatusFilter::InProgress);
        assert_eq!(StatusFilter::InProgress.prev(), StatusFilter::Failure);
        assert_eq!(StatusFilter::Failure.prev(), StatusFilter::Success);
        assert_eq!(StatusFilter::Success.prev(), StatusFilter::All);
    }

    #[test]
    fn status_filter_matches_expected_outcomes() {
        assert!(StatusFilter::All.matches("in_progress", None));
        assert!(StatusFilter::Success.matches("completed", Some("success")));
        assert!(!StatusFilter::Success.matches("completed", Some("failure")));
        assert!(StatusFilter::Failure.matches("completed", Some("timed_out")));
        assert!(StatusFilter::InProgress.matches("queued", None));
        assert!(!StatusFilter::InProgress.matches("completed", Some("success")));
    }

    #[test]
    fn visible_rows_always_includes_errors() {
        let mut app = App::new(vec![], 10);
        app.apply_results(vec![
            RepoFetchResult::Ok(vec![run("completed", Some("success"))]),
            RepoFetchResult::Err {
                repo: repo_spec(),
                message: "boom".to_string(),
            },
        ]);

        app.cycle_filter_next(); // -> Success (still matches the one success run + the error)
        assert_eq!(app.visible_len(), 2);

        app.cycle_filter_next(); // -> Failure (no runs match, but error still shows)
        assert_eq!(app.visible_len(), 1);
        assert!(matches!(app.visible_rows().next(), Some(Row::Error { .. })));
    }

    #[test]
    fn visible_rows_filters_runs_by_status() {
        let mut app = App::new(vec![], 10);
        app.apply_results(vec![RepoFetchResult::Ok(vec![
            run("completed", Some("success")),
            run("completed", Some("failure")),
            run("in_progress", None),
        ])]);
        assert_eq!(app.visible_len(), 3);

        app.cycle_filter_next(); // -> Success
        assert_eq!(app.filter(), StatusFilter::Success);
        assert_eq!(app.visible_len(), 1);

        app.cycle_filter_next(); // -> Failure
        assert_eq!(app.visible_len(), 1);

        app.cycle_filter_next(); // -> InProgress
        assert_eq!(app.visible_len(), 1);

        app.cycle_filter_next(); // -> All
        assert_eq!(app.filter(), StatusFilter::All);
        assert_eq!(app.visible_len(), 3);
    }

    #[test]
    fn selection_clamps_when_filter_shrinks_visible_set() {
        let mut app = App::new(vec![], 10);
        app.apply_results(vec![RepoFetchResult::Ok(vec![
            run("completed", Some("success")),
            run("completed", Some("success")),
            run("completed", Some("failure")),
        ])]);
        // Select the last row (index 2, the failure run).
        app.table_state.select(Some(2));

        app.cycle_filter_next(); // -> Success: only 2 rows now visible (indices 0..2)
        assert_eq!(app.visible_len(), 2);
        // Selection must be clamped back within bounds.
        assert!(app.table_state.selected().unwrap() < app.visible_len());
    }

    #[test]
    fn selection_is_none_when_nothing_visible() {
        let mut app = App::new(vec![], 10);
        app.apply_results(vec![RepoFetchResult::Ok(vec![run(
            "completed",
            Some("success"),
        )])]);
        app.cycle_filter_next(); // -> Success: matches
        app.cycle_filter_next(); // -> Failure: nothing matches, no errors either
        assert_eq!(app.visible_len(), 0);
        assert_eq!(app.table_state.selected(), None);
    }

    #[test]
    fn open_menu_is_noop_on_error_row() {
        let mut app = App::new(vec![], 10);
        app.apply_results(vec![RepoFetchResult::Err {
            repo: repo_spec(),
            message: "boom".to_string(),
        }]);
        app.table_state.select(Some(0));

        app.open_menu();
        assert_eq!(app.overlay, Overlay::None);
    }

    #[test]
    fn menu_navigation_wraps_between_three_items() {
        let mut app = App::new(vec![], 10);
        app.apply_results(vec![RepoFetchResult::Ok(vec![run(
            "completed",
            Some("success"),
        )])]);
        app.table_state.select(Some(0));

        app.open_menu();
        assert_eq!(app.overlay, Overlay::Menu { selected: 0 });

        app.menu_select_next();
        assert_eq!(app.overlay, Overlay::Menu { selected: 1 });

        app.menu_select_next();
        assert_eq!(app.overlay, Overlay::Menu { selected: 2 });

        app.menu_select_next();
        assert_eq!(app.overlay, Overlay::Menu { selected: 0 });

        app.menu_select_prev();
        assert_eq!(app.overlay, Overlay::Menu { selected: 2 });
    }

    #[test]
    fn rerun_is_blocked_when_run_has_not_failed() {
        let mut app = App::new(vec![], 10);
        app.apply_results(vec![RepoFetchResult::Ok(vec![run(
            "completed",
            Some("success"),
        )])]);
        app.table_state.select(Some(0));

        app.request_rerun_direct();
        assert_eq!(app.overlay, Overlay::None);
        assert_eq!(
            app.status_message.as_deref(),
            Some("No failed jobs to rerun")
        );
    }

    #[test]
    fn rerun_direct_opens_confirm_prompt_when_run_failed() {
        let mut app = App::new(vec![], 10);
        app.apply_results(vec![RepoFetchResult::Ok(vec![run(
            "completed",
            Some("failure"),
        )])]);
        app.table_state.select(Some(0));

        app.request_rerun_direct();
        assert_eq!(app.overlay, Overlay::ConfirmRerun);
    }

    #[test]
    fn confirm_rerun_returns_run_identity_and_closes_overlay() {
        let mut app = App::new(vec![], 10);
        app.apply_results(vec![RepoFetchResult::Ok(vec![run(
            "completed",
            Some("failure"),
        )])]);
        app.table_state.select(Some(0));
        app.request_rerun_direct();

        let result = app.confirm_rerun();
        assert_eq!(
            result,
            Some(("owner".to_string(), "repo".to_string(), 1))
        );
        assert_eq!(app.overlay, Overlay::None);
    }

    #[test]
    fn cancel_overlay_closes_confirm_prompt_without_result() {
        let mut app = App::new(vec![], 10);
        app.apply_results(vec![RepoFetchResult::Ok(vec![run(
            "completed",
            Some("failure"),
        )])]);
        app.table_state.select(Some(0));
        app.request_rerun_direct();

        app.cancel_overlay();
        assert_eq!(app.overlay, Overlay::None);
    }

    fn job(id: u64, name: &str, conclusion: Option<&str>) -> JobInfo {
        JobInfo {
            id,
            name: name.to_string(),
            status: "completed".to_string(),
            conclusion: conclusion.map(str::to_string),
            steps: vec![StepInfo {
                number: 1,
                name: "Run tests".to_string(),
                status: "completed".to_string(),
                conclusion: conclusion.map(str::to_string),
            }],
        }
    }

    #[test]
    fn view_logs_menu_item_requests_jobs_and_switches_screen() {
        let mut app = App::new(vec![], 10);
        app.apply_results(vec![RepoFetchResult::Ok(vec![run(
            "completed",
            Some("failure"),
        )])]);
        app.table_state.select(Some(0));
        app.open_menu();
        app.menu_select_next(); // -> Rerun failed jobs
        app.menu_select_next(); // -> View logs

        let action = app.menu_activate();
        assert_eq!(
            action,
            MenuAction::ViewLogsRequested {
                owner: "owner".to_string(),
                repo: "repo".to_string(),
                run_id: 1,
            }
        );
        assert!(matches!(app.screen, Screen::Jobs(_)));
        assert_eq!(app.overlay, Overlay::None);
    }

    #[test]
    fn apply_jobs_result_populates_jobs_screen() {
        let mut app = App::new(vec![], 10);
        app.apply_results(vec![RepoFetchResult::Ok(vec![run(
            "completed",
            Some("failure"),
        )])]);
        app.table_state.select(Some(0));
        app.open_menu();
        app.menu_select_next();
        app.menu_select_next();
        app.menu_activate();

        app.apply_jobs_result(Ok(vec![job(1, "build", Some("failure"))]));

        let Screen::Jobs(jobs) = &app.screen else {
            panic!("expected Jobs screen");
        };
        assert!(!jobs.loading);
        assert_eq!(jobs.jobs.len(), 1);
        assert_eq!(jobs.jobs[0].name, "build");
    }

    #[test]
    fn jobs_select_next_prev_wrap() {
        let mut app = App::new(vec![], 10);
        app.apply_results(vec![RepoFetchResult::Ok(vec![run(
            "completed",
            Some("failure"),
        )])]);
        app.table_state.select(Some(0));
        app.open_menu();
        app.menu_select_next();
        app.menu_select_next();
        app.menu_activate();
        app.apply_jobs_result(Ok(vec![
            job(1, "build", Some("failure")),
            job(2, "test", Some("success")),
        ]));

        app.jobs_select_next();
        let Screen::Jobs(jobs) = &app.screen else {
            panic!("expected Jobs screen");
        };
        assert_eq!(jobs.selected, 1);

        app.jobs_select_next();
        let Screen::Jobs(jobs) = &app.screen else {
            panic!("expected Jobs screen");
        };
        assert_eq!(jobs.selected, 0);

        app.jobs_select_prev();
        let Screen::Jobs(jobs) = &app.screen else {
            panic!("expected Jobs screen");
        };
        assert_eq!(jobs.selected, 1);
    }

    #[test]
    fn request_view_log_switches_to_log_screen_and_back_restores_jobs() {
        let mut app = App::new(vec![], 10);
        app.apply_results(vec![RepoFetchResult::Ok(vec![run(
            "completed",
            Some("failure"),
        )])]);
        app.table_state.select(Some(0));
        app.open_menu();
        app.menu_select_next();
        app.menu_select_next();
        app.menu_activate();
        app.apply_jobs_result(Ok(vec![job(42, "build", Some("failure"))]));

        let result = app.request_view_log();
        assert_eq!(
            result,
            Some(("owner".to_string(), "repo".to_string(), 42))
        );
        assert!(matches!(app.screen, Screen::Log(_)));

        app.back_to_jobs();
        let Screen::Jobs(jobs) = &app.screen else {
            panic!("expected Jobs screen");
        };
        assert_eq!(jobs.jobs.len(), 1);
        assert_eq!(jobs.jobs[0].id, 42);
    }

    #[test]
    fn apply_log_result_scrolls_to_first_error_marker() {
        let mut app = App::new(vec![], 10);
        app.apply_results(vec![RepoFetchResult::Ok(vec![run(
            "completed",
            Some("failure"),
        )])]);
        app.table_state.select(Some(0));
        app.open_menu();
        app.menu_select_next();
        app.menu_select_next();
        app.menu_activate();
        app.apply_jobs_result(Ok(vec![job(1, "build", Some("failure"))]));
        app.request_view_log();

        let log_bytes = b"line one\nline two\n##[error]it broke\nline four".to_vec();
        app.apply_log_result(Ok(log_bytes));

        let Screen::Log(log) = &app.screen else {
            panic!("expected Log screen");
        };
        assert!(!log.loading);
        assert_eq!(log.scroll, 2);
        assert_eq!(log.line_count, 4);
    }

    #[test]
    fn apply_log_result_defaults_to_top_when_no_error_marker() {
        let mut app = App::new(vec![], 10);
        app.apply_results(vec![RepoFetchResult::Ok(vec![run(
            "completed",
            Some("success"),
        )])]);
        app.table_state.select(Some(0));
        app.open_menu();
        app.menu_select_next();
        app.menu_select_next();
        app.menu_activate();
        app.apply_jobs_result(Ok(vec![job(1, "build", Some("success"))]));
        app.request_view_log();

        app.apply_log_result(Ok(b"all good\nnothing to see".to_vec()));

        let Screen::Log(log) = &app.screen else {
            panic!("expected Log screen");
        };
        assert_eq!(log.scroll, 0);
    }

    #[test]
    fn back_to_runs_resets_screen() {
        let mut app = App::new(vec![], 10);
        app.apply_results(vec![RepoFetchResult::Ok(vec![run(
            "completed",
            Some("failure"),
        )])]);
        app.table_state.select(Some(0));
        app.open_menu();
        app.menu_select_next();
        app.menu_select_next();
        app.menu_activate();

        app.back_to_runs();
        assert!(matches!(app.screen, Screen::Runs));
    }
}
