//! TUI rendering: draws the runs table, header, and footer/help bar.

use crate::app::{App, Row};
use crate::status::RunOutcome;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row as TableRow, Table};
use ratatui::Frame;

/// Number of columns in the runs table (REPO, WORKFLOW, BRANCH, EVENT,
/// STATUS, UPDATED, #). Used to pad partial rows (e.g. error rows) to the
/// full width.
const COLUMN_COUNT: usize = 7;

pub fn draw(frame: &mut Frame, app: &mut App) {
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
    let help = "↑/k up  ↓/j down  o/Enter open in browser  r refresh  f/F filter  q quit";
    let text = match &app.status_message {
        Some(msg) if app.refreshing => format!("{help}  |  {msg}"),
        Some(msg) => format!("{help}  |  {msg}"),
        None => help.to_string(),
    };
    let paragraph = Paragraph::new(Line::from(text));
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
