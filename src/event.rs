//! Input polling and periodic tick/refresh event generation.
//!
//! Terminal input is polled on a background thread (crossterm's blocking
//! `event::read`) and forwarded over a channel, multiplexed with a refresh
//! ticker and the async fetch-result channel. This keeps the render loop
//! simple: it just awaits the next [`Event`].

use crate::github::RepoFetchResult;
use crossterm::event::{self as ct_event, Event as CtEvent, KeyEvent, KeyEventKind};
use std::sync::mpsc;
use std::time::Duration;
use tokio::sync::mpsc as tokio_mpsc;

/// Events consumed by the main loop.
pub enum Event {
    /// A key was pressed.
    Key(KeyEvent),
    /// The refresh interval elapsed; time to re-fetch.
    Tick,
    /// Background fetch results arrived.
    FetchComplete(Vec<RepoFetchResult>),
    /// The terminal was resized.
    Resize,
}

/// Spawns a background OS thread that blocks on terminal input and forwards
/// key/resize events into the given synchronous channel sender.
pub fn spawn_input_reader(tx: mpsc::Sender<Event>) {
    std::thread::spawn(move || loop {
        match ct_event::poll(Duration::from_millis(100)) {
            Ok(true) => match ct_event::read() {
                Ok(CtEvent::Key(key)) if key.kind == KeyEventKind::Press => {
                    if tx.send(Event::Key(key)).is_err() {
                        return;
                    }
                }
                Ok(CtEvent::Resize(_, _)) => {
                    if tx.send(Event::Resize).is_err() {
                        return;
                    }
                }
                Ok(_) => {}
                Err(_) => return,
            },
            Ok(false) => {}
            Err(_) => return,
        }
    });
}

/// A handle for driving the app's tick timer and receiving fetch results,
/// bridged into the same std `mpsc::Receiver` used for input events.
pub struct EventBridge {
    pub rx: mpsc::Receiver<Event>,
    tx: mpsc::Sender<Event>,
}

impl EventBridge {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        Self { rx, tx }
    }

    pub fn sender(&self) -> mpsc::Sender<Event> {
        self.tx.clone()
    }

    /// Spawns the tokio task that periodically signals a tick (triggering a
    /// refresh) on the given interval.
    pub fn spawn_ticker(&self, interval: Duration) {
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            ticker.tick().await; // first tick fires immediately; skip it since we fetch on startup
            loop {
                ticker.tick().await;
                if tx.send(Event::Tick).is_err() {
                    return;
                }
            }
        });
    }

    /// Bridges an async tokio channel (used by the fetch task to report
    /// completion) into the sync channel consumed by the main loop.
    pub fn spawn_fetch_bridge(&self, mut fetch_rx: tokio_mpsc::UnboundedReceiver<Vec<RepoFetchResult>>) {
        let tx = self.tx.clone();
        tokio::spawn(async move {
            while let Some(results) = fetch_rx.recv().await {
                if tx.send(Event::FetchComplete(results)).is_err() {
                    return;
                }
            }
        });
    }
}

impl Default for EventBridge {
    fn default() -> Self {
        Self::new()
    }
}
