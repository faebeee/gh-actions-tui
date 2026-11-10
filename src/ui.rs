//! TUI rendering: draws the runs table, header, and footer/help bar.

use crate::app::{App, JobsScreen, LogScreen, Overlay, Row, Screen, MENU_LABELS};
use crate::status::RunOutcome;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Clear, List, ListItem, Paragraph, Row as TableRow, Table};
use ratatui::Frame;

/// Number of columns in the runs table (REPO, WORKFLOW, BRANCH, EVENT,
/// STATUS, UPDATED, #, RUN ID). Used to pad partial rows (e.g. error rows) to the
/// full width.
const COLUMN_COUNT: usize = 8;

pub fn draw(frame: &mut Frame, app: &mut App) {
    match &app.screen {
        Screen::Runs => draw_runs_screen(frame, app),
        Screen::Jobs(_) => draw_jobs_screen(frame, app),
        Screen::Log(_) => draw_log_screen(frame, app),
    }
}

fn draw_runs_screen(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // header
            Constraint::Min(3),    // table
            Constraint::Length(1), // footer
        ])
        .split(area);

    draw_header(frame, app, chunks[0]);
    draw_table(frame, app, chunks[1]);
    draw_footer(frame, app, chunks[2]);

    match app.overlay {
        Overlay::Menu { selected } => draw_menu_overlay(frame, app, area, selected),
        Overlay::ConfirmRerun => draw_confirm_overlay(frame, app, area),
        Overlay::None => {}
    }
}

fn draw_header(frame: &mut Frame, app: &App, area: Rect) {
    let refresh_text = match app.last_refresh {
        Some(t) => format!("Last refresh: {}", t.format("%Y-%m-%d %H:%M:%S UTC")),
        None => "Last refresh: never".to_string(),
    };
    let status = if app.refreshing {
        " (refreshing...)".to_string()
    } else {
        String::new()
    };

    let title = format!(
        "GitHub Actions — {} repo(s), limit {} — {}{} — Filter: {}",
        app.repos.len(),
        app.limit,
        refresh_text,
        status,
        app.filter().label()
    );

    let block = Block::default().borders(Borders::ALL).title("gh-actions-tui");
    let paragraph = Paragraph::new(title).block(block);
    frame.render_widget(paragraph, area);
}

fn draw_table(frame: &mut Frame, app: &mut App, area: Rect) {
    let header = TableRow::new(vec![
        Cell::from("REPO"),
        Cell::from("WORKFLOW"),
        Cell::from("BRANCH"),
        Cell::from("EVENT"),
        Cell::from("STATUS"),
        Cell::from("UPDATED"),
        Cell::from("#"),
        Cell::from("RUN ID"),
    ])
    .style(Style::default().add_modifier(Modifier::BOLD));

    let visible_count = app.visible_len();
    let rows: Vec<TableRow> = app
        .visible_rows()
        .map(|row| match row {
            Row::Run(run) => {
                let (label, style) = status_style(&run.status, run.conclusion.as_deref());
                TableRow::new(vec![
                    Cell::from(run.repo_full_name()),
                    Cell::from(run.workflow_name.clone()),
                    Cell::from(run.branch.clone()),
                    Cell::from(run.event.clone()),
                    Cell::from(Span::styled(label, style)),
                    Cell::from(format_relative(run.updated_at)),
                    Cell::from(format!("#{}", run.run_number)),
                    Cell::from(run.id.to_string()),
                ])
            }
            Row::Error { repo, message } => error_row(&repo.to_string(), message),
        })
        .collect();

    let widths = [
        Constraint::Percentage(20),
        Constraint::Percentage(24),
        Constraint::Percentage(14),
        Constraint::Percentage(12),
        Constraint::Percentage(12),
        Constraint::Percentage(12),
        Constraint::Percentage(6),
        Constraint::Length(12),
    ];

    let table = Table::new(rows, widths)
        .header(header)
        .block(Block::default().borders(Borders::ALL).title("Recent runs"))
        .row_highlight_style(Style::default().add_modifier(Modifier::REVERSED))
        .highlight_symbol("> ");

    if visible_count == 0 {
        let message = if !app.has_rows() {
            app.status_message
                .clone()
                .unwrap_or_else(|| "No runs to display".to_string())
        } else {
            format!("No runs match filter: {}", app.filter().label())
        };
        let block = Block::default().borders(Borders::ALL).title("Recent runs");
        let paragraph = Paragraph::new(message).block(block);
        frame.render_widget(paragraph, area);
    } else {
        frame.render_stateful_widget(table, area, app.table_state());
    }
}

/// Builds a table row for a repo-level fetch error, padded to
/// `COLUMN_COUNT` cells so the table stays aligned.
fn error_row(repo: &str, message: &str) -> TableRow<'static> {
    let mut cells = vec![
        Cell::from(repo.to_string()),
        Cell::from(Span::styled(
            format!("error: {message}"),
            Style::default().fg(Color::Red),
        )),
    ];
    cells.resize_with(COLUMN_COUNT, || Cell::from(""));
    TableRow::new(cells)
}

fn draw_footer(frame: &mut Frame, app: &App, area: Rect) {
    let help = "↑/k up  ↓/j down  Enter menu  o open  R rerun failed  r refresh  f/F filter  q quit";
    let text = match &app.status_message {
        Some(msg) if app.refreshing => format!("{help}  |  {msg}"),
        Some(msg) => format!("{help}  |  {msg}"),
        None => help.to_string(),
    };
    let paragraph = Paragraph::new(Line::from(text));
    frame.render_widget(paragraph, area);
}

/// Returns a `Rect` centered within `area`, sized to `percent_x`/`percent_y`
/// of its width/height.
fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vertical[1])[1]
}

/// Draws the per-row action menu (triggered by `Enter` on a selected run).
fn draw_menu_overlay(frame: &mut Frame, app: &App, area: Rect, selected: usize) {
    let run_failed = app.selected_run_failed();

    let items: Vec<ListItem> = MENU_LABELS
        .iter()
        .enumerate()
        .map(|(i, label)| {
            let disabled = i == 1 && !run_failed;
            let style = if disabled {
                Style::default().fg(Color::DarkGray)
            } else {
                Style::default()
            };
            let text = if disabled {
                format!("{label} (no failed jobs)")
            } else {
                label.to_string()
            };
            ListItem::new(Span::styled(text, style))
        })
        .collect();

    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title("Actions"))
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
        .highlight_symbol("> ");

    let rect = centered_rect(40, 30, area);
    let mut state = ratatui::widgets::ListState::default();
    state.select(Some(selected));

    frame.render_widget(Clear, rect);
    frame.render_stateful_widget(list, rect, &mut state);
}

/// Draws the confirmation prompt before rerunning failed jobs.
fn draw_confirm_overlay(frame: &mut Frame, app: &App, area: Rect) {
    let message = match app.selected_run() {
        Some(run) => format!(
            "Rerun failed jobs for {} #{}?\n\n(y) confirm   (n) cancel",
            run.workflow_name, run.run_number
        ),
        None => "No run selected.\n\n(n) cancel".to_string(),
    };

    let block = Block::default().borders(Borders::ALL).title("Confirm rerun");
    let paragraph = Paragraph::new(message).block(block);

    let rect = centered_rect(50, 30, area);
    frame.render_widget(Clear, rect);
    frame.render_widget(paragraph, rect);
}

/// Draws the full-screen "Jobs" view: the list of jobs (and their steps)
/// for a single workflow run.
fn draw_jobs_screen(frame: &mut Frame, app: &App) {
    let Screen::Jobs(jobs) = &app.screen else {
        return;
    };
    let area = frame.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // header
            Constraint::Min(3),    // jobs list
            Constraint::Length(1), // footer
        ])
        .split(area);

    let title = format!(
        "Jobs — {} #{} (run #{}) — {}/{}",
        jobs.workflow_name, jobs.run_number, jobs.run_id, jobs.owner, jobs.repo
    );
    let header = Paragraph::new(title).block(Block::default().borders(Borders::ALL).title("gh-actions-tui"));
    frame.render_widget(header, chunks[0]);

    draw_jobs_body(frame, jobs, chunks[1]);

    let help = "↑/k up  ↓/j down  Enter view log  Esc back";
    let text = match &jobs.error {
        Some(e) => format!("{help}  |  error: {e}"),
        None if jobs.loading => format!("{help}  |  Loading jobs..."),
        None => help.to_string(),
    };
    frame.render_widget(Paragraph::new(Line::from(text)), chunks[2]);
}

/// Renders the job list (with nested, dimmed step rows) for the jobs
/// screen body.
fn draw_jobs_body(frame: &mut Frame, jobs: &JobsScreen, area: Rect) {
    if jobs.loading {
        let p = Paragraph::new("Loading jobs...").block(Block::default().borders(Borders::ALL).title("Jobs"));
        frame.render_widget(p, area);
        return;
    }
    if jobs.jobs.is_empty() {
        let message = jobs
            .error
            .clone()
            .unwrap_or_else(|| "No jobs found for this run".to_string());
        let p = Paragraph::new(message).block(Block::default().borders(Borders::ALL).title("Jobs"));
        frame.render_widget(p, area);
        return;
    }

    let mut items: Vec<ListItem> = Vec::new();
    // Tracks, for each rendered list item, which job index it belongs to,
    // so we can highlight all lines (job + its steps) of the selected job.
    let mut owning_job: Vec<usize> = Vec::new();

    for (job_idx, job) in jobs.jobs.iter().enumerate() {
        let (label, style) = status_style(&job.status, job.conclusion.as_deref());
        items.push(ListItem::new(Span::styled(
            format!("{label} {}", job.name),
            style,
        )));
        owning_job.push(job_idx);

        // Defensively sort by step number in case the API doesn't return
        // them in order.
        let mut steps = job.steps.clone();
        steps.sort_by_key(|step| step.number);
        for step in &steps {
            let (step_label, step_style) = status_style(&step.status, step.conclusion.as_deref());
            items.push(ListItem::new(Span::styled(
                format!("    {step_label} {}", step.name),
                step_style.add_modifier(Modifier::DIM),
            )));
            owning_job.push(job_idx);
        }
    }

    let list = List::new(items).block(Block::default().borders(Borders::ALL).title("Jobs"));

    // Highlight every line that belongs to the selected job.
    let mut state = ratatui::widgets::ListState::default();
    if let Some(first_line) = owning_job.iter().position(|&j| j == jobs.selected) {
        state.select(Some(first_line));
    }
    let list = list.highlight_style(Style::default().add_modifier(Modifier::REVERSED));

    frame.render_stateful_widget(list, area, &mut state);
}

/// Draws the full-screen "Log" view: the raw (ANSI-colored) log for a
/// single job.
fn draw_log_screen(frame: &mut Frame, app: &App) {
    let Screen::Log(log) = &app.screen else {
        return;
    };
    let area = frame.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // header
            Constraint::Min(3),    // log body
            Constraint::Length(1), // footer
        ])
        .split(area);

    let title = format!(
        "Log — {} (job #{}) — {}/{}",
        log.job_name, log.job_id, log.owner, log.repo
    );
    let header = Paragraph::new(title).block(Block::default().borders(Borders::ALL).title("gh-actions-tui"));
    frame.render_widget(header, chunks[0]);

    draw_log_body(frame, log, chunks[1]);

    let help = "↑/k ↓/j scroll  PgUp/PgDn page  g/G top/bottom  Esc back";
    let text = match &log.error {
        Some(e) => format!("{help}  |  error: {e}"),
        None if log.loading => format!("{help}  |  Loading log..."),
        None => format!("{help}  |  line {}/{}", log.scroll + 1, log.line_count),
    };
    frame.render_widget(Paragraph::new(Line::from(text)), chunks[2]);
}

fn draw_log_body(frame: &mut Frame, log: &LogScreen, area: Rect) {
    if log.loading {
        let p = Paragraph::new("Loading log...").block(Block::default().borders(Borders::ALL).title("Log"));
        frame.render_widget(p, area);
        return;
    }
    if let Some(e) = &log.error {
        let p = Paragraph::new(format!("Failed to load log: {e}"))
            .block(Block::default().borders(Borders::ALL).title("Log"));
        frame.render_widget(p, area);
        return;
    }

    let paragraph = Paragraph::new(log.text.clone())
        .block(Block::default().borders(Borders::ALL).title("Log"))
        .scroll((log.scroll, 0));
    frame.render_widget(paragraph, area);
}

/// Maps a run's status/conclusion to a display label and color.
fn status_style(status: &str, conclusion: Option<&str>) -> (String, Style) {
    let outcome = RunOutcome::classify(status, conclusion);
    let color = match outcome {
        RunOutcome::Success | RunOutcome::Completed => Color::Green,
        RunOutcome::Failure => Color::Red,
        RunOutcome::Cancelled => Color::DarkGray,
        RunOutcome::Skipped => Color::Gray,
        RunOutcome::InProgress => Color::Yellow,
        RunOutcome::Unknown => Color::White,
    };
    let label = conclusion.unwrap_or(status);
    (format!("● {label}"), Style::default().fg(color))
}

/// Formats a UTC timestamp as a short relative time (e.g. "5m ago").
fn format_relative(t: chrono::DateTime<chrono::Utc>) -> String {
    let now = chrono::Utc::now();
    let delta = now.signed_duration_since(t);

    if delta.num_seconds() < 60 {
        format!("{}s ago", delta.num_seconds().max(0))
    } else if delta.num_minutes() < 60 {
        format!("{}m ago", delta.num_minutes())
    } else if delta.num_hours() < 24 {
        format!("{}h ago", delta.num_hours())
    } else {
        format!("{}d ago", delta.num_days())
    }
}
