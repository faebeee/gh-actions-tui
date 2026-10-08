//! Entry point: sets up the terminal, authenticates, and runs the event loop.

mod app;
mod auth;
mod cli;
mod event;
mod github;
mod status;
mod ui;

use anyhow::Result;
use app::App;
use cli::Cli;
use crossterm::event::{DisableMouseCapture, EnableMouseCapture, KeyCode, KeyEvent, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use event::{spawn_input_reader, Event, EventBridge};
use github::GitHubClient;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use std::io::{self, Stdout};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc as tokio_mpsc;

#[tokio::main]
async fn main() -> Result<()> {
    let cli = match Cli::parse_args() {
        Ok(cli) => cli,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };

    let token = match auth::token_from_gh_cli() {
        Ok(token) => token,
        Err(e) => {
            eprintln!("Authentication error: {e:#}");
            std::process::exit(1);
        }
    };

    let client = Arc::new(GitHubClient::new(token)?);

    install_panic_hook();
    let mut terminal = setup_terminal()?;
    let result = run_app(&mut terminal, cli, client).await;
    restore_terminal(&mut terminal)?;

    if let Err(e) = result {
        eprintln!("Error: {e:#}");
        std::process::exit(1);
    }
    Ok(())
}

async fn run_app(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    cli: Cli,
    client: Arc<GitHubClient>,
) -> Result<()> {
    let mut app = App::new(cli.repos.clone(), cli.limit);

    let bridge = EventBridge::new();
    spawn_input_reader(bridge.sender());
    bridge.spawn_ticker(Duration::from_secs(cli.interval));

    let (fetch_tx, fetch_rx) = tokio_mpsc::unbounded_channel();
    bridge.spawn_fetch_bridge(fetch_rx);

    let (rerun_tx, rerun_rx) = tokio_mpsc::unbounded_channel();
    bridge.spawn_rerun_bridge(rerun_rx);

    let ctx = AppContext {
        client: &client,
        cli: &cli,
        fetch_tx: &fetch_tx,
        rerun_tx: &rerun_tx,
    };

    // Kick off the initial fetch immediately.
    trigger_fetch(ctx.client, ctx.cli, ctx.fetch_tx);

    loop {
        terminal.draw(|frame| ui::draw(frame, &mut app))?;

        if app.should_quit {
            return Ok(());
        }

        let event = match bridge.rx.recv_timeout(Duration::from_millis(250)) {
            Ok(event) => event,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
        };

        match event {
            Event::Key(key) => handle_key(key, &mut app, &ctx),
            Event::Tick => {
                app.begin_refresh();
                trigger_fetch(ctx.client, ctx.cli, ctx.fetch_tx);
            }
            Event::FetchComplete(results) => app.apply_results(results),
            Event::RerunComplete(result) => app.apply_rerun_result(result),
            Event::Resize => {}
        }
    }
}

/// Bundles the dependencies `handle_key` needs to trigger side effects
/// (fetching runs), so the function signature doesn't grow with each new
/// action.
struct AppContext<'a> {
    client: &'a Arc<GitHubClient>,
    cli: &'a Cli,
    fetch_tx: &'a tokio_mpsc::UnboundedSender<Vec<github::RepoFetchResult>>,
    rerun_tx: &'a tokio_mpsc::UnboundedSender<Result<(), String>>,
}

fn handle_key(key: KeyEvent, app: &mut App, ctx: &AppContext) {
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        app.quit();
        return;
    }

    use app::Overlay;
    match app.overlay {
        Overlay::Menu { .. } => {
            match key.code {
                KeyCode::Esc => app.close_overlay(),
                KeyCode::Down | KeyCode::Char('j') => app.menu_select_next(),
                KeyCode::Up | KeyCode::Char('k') => app.menu_select_prev(),
                KeyCode::Enter => app.menu_activate(),
                _ => {}
            }
            // If activating the menu moved us into the confirm prompt, fall
            // through so the user doesn't need a second keypress to see it
            // (no action needed here; it's just rendered next frame).
            return;
        }
        Overlay::ConfirmRerun => {
            match key.code {
                KeyCode::Char('y') | KeyCode::Enter => {
                    if let Some((owner, repo, run_id)) = app.confirm_rerun() {
                        app.set_status("Triggering rerun of failed jobs...");
                        trigger_rerun(ctx.client, owner, repo, run_id, ctx.rerun_tx);
                    }
                }
                KeyCode::Char('n') | KeyCode::Esc => app.cancel_overlay(),
                _ => {}
            }
            return;
        }
        Overlay::None => {}
    }

    match key.code {
        KeyCode::Char('q') | KeyCode::Esc => app.quit(),
        KeyCode::Down | KeyCode::Char('j') => app.select_next(),
        KeyCode::Up | KeyCode::Char('k') => app.select_prev(),
        KeyCode::Char('o') => app.open_selected(),
        KeyCode::Enter => app.open_menu(),
        KeyCode::Char('R') => app.request_rerun_direct(),
        KeyCode::Char('r') => {
            app.begin_refresh();
            trigger_fetch(ctx.client, ctx.cli, ctx.fetch_tx);
        }
        KeyCode::Char('f') => app.cycle_filter_next(),
        KeyCode::Char('F') => app.cycle_filter_prev(),
        _ => {}
    }
}

/// Spawns an async task that triggers a rerun of failed jobs for the given
/// run and sends the result back over the rerun channel.
fn trigger_rerun(
    client: &Arc<GitHubClient>,
    owner: String,
    repo: String,
    run_id: u64,
    rerun_tx: &tokio_mpsc::UnboundedSender<Result<(), String>>,
) {
    let client = Arc::clone(client);
    let tx = rerun_tx.clone();
    tokio::spawn(async move {
        let result = client
            .rerun_failed_jobs(&owner, &repo, run_id)
            .await
            .map_err(|e| format!("{e:#}"));
        let _ = tx.send(result);
    });
}

/// Spawns an async task that fetches runs for all configured repos and sends
/// the results back over the fetch channel.
fn trigger_fetch(
    client: &Arc<GitHubClient>,
    cli: &Cli,
    fetch_tx: &tokio_mpsc::UnboundedSender<Vec<github::RepoFetchResult>>,
) {
    let client = Arc::clone(client);
    let repos = cli.repos.clone();
    let limit = cli.limit;
    let tx = fetch_tx.clone();
    tokio::spawn(async move {
        let results = client.fetch_all(&repos, limit).await;
        let _ = tx.send(results);
    });
}

fn setup_terminal() -> Result<Terminal<CrosstermBackend<Stdout>>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    Ok(Terminal::new(backend)?)
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<()> {
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;
    Ok(())
}

/// Installs a panic hook that restores the terminal to a sane state before
/// printing the panic message, so a crash doesn't leave the user's shell in
/// raw/alternate-screen mode.
fn install_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture);
        default_hook(info);
    }));
}
