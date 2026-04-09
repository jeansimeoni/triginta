use std::time::Duration;

use anyhow::{Context, Result};
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, prelude::CrosstermBackend};
use tracing::info;

use crate::{
    config::{AppPaths, init_tracing},
    domain::{HistoryStats, PomodoroSession, Task},
    integrations::{DisabledTodoistProvider, TaskSyncProvider},
    storage::{Database, PomodoroRepository, TaskRepository},
    ui,
};

const TICK_RATE: Duration = Duration::from_millis(250);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RightPanelTab {
    Tasks,
    Statistics,
}

impl RightPanelTab {
    pub fn next(self) -> Self {
        match self {
            Self::Tasks => Self::Statistics,
            Self::Statistics => Self::Tasks,
        }
    }

    pub fn previous(self) -> Self {
        match self {
            Self::Tasks => Self::Statistics,
            Self::Statistics => Self::Tasks,
        }
    }

    pub fn index(self) -> usize {
        match self {
            Self::Tasks => 0,
            Self::Statistics => 1,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ScreenData {
    pub tasks: Vec<Task>,
    pub recent_sessions: Vec<PomodoroSession>,
    pub stats: HistoryStats,
}

// `App` owns the mutable runtime state for the TUI loop.
// Compared with a C program, this is the central state struct you would pass
// around to input/render functions, but here methods are attached directly to
// the type.
#[derive(Debug, Clone)]
pub struct App {
    active_right_panel_tab: RightPanelTab,
    should_quit: bool,
    status_message: String,
    screen_data: ScreenData,
}

impl App {
    pub fn new(screen_data: ScreenData) -> Self {
        Self {
            active_right_panel_tab: RightPanelTab::Tasks,
            should_quit: false,
            status_message: "SQLite initialized. Local-first mode active.".to_string(),
            screen_data,
        }
    }

    pub fn active_right_panel_tab(&self) -> RightPanelTab {
        self.active_right_panel_tab
    }

    pub fn should_quit(&self) -> bool {
        self.should_quit
    }

    pub fn status_message(&self) -> &str {
        &self.status_message
    }

    pub fn screen_data(&self) -> &ScreenData {
        // Returning `&ScreenData` lends read-only access to the caller.
        // No copy is made, and the borrow checker ensures the reference cannot
        // outlive `self`.
        &self.screen_data
    }

    pub fn handle_key(&mut self, code: KeyCode) {
        // `&mut self` is exclusive access: while this method runs, no other
        // code can also mutate the app state. This prevents a whole class of
        // aliasing bugs that are easy to create in C.
        match code {
            KeyCode::Char('q') => {
                self.should_quit = true;
                self.status_message = "Shutting down Triginta.".to_string();
            }
            KeyCode::Tab | KeyCode::Char('l') | KeyCode::Right => {
                self.active_right_panel_tab = self.active_right_panel_tab.next();
                self.status_message = match self.active_right_panel_tab {
                    RightPanelTab::Tasks => "Switched right panel to tasks.".to_string(),
                    RightPanelTab::Statistics => "Switched right panel to statistics.".to_string(),
                };
            }
            KeyCode::BackTab | KeyCode::Char('h') | KeyCode::Left => {
                self.active_right_panel_tab = self.active_right_panel_tab.previous();
                self.status_message = match self.active_right_panel_tab {
                    RightPanelTab::Tasks => "Switched right panel to tasks.".to_string(),
                    RightPanelTab::Statistics => "Switched right panel to statistics.".to_string(),
                };
            }
            _ => {}
        }
    }
}

pub fn run() -> Result<()> {
    // Startup is written as a straight-line sequence of fallible operations.
    // The `?` operator keeps this readable: each step either succeeds and
    // continues, or returns early with an error.
    let paths = AppPaths::resolve()?;
    paths.ensure_dirs()?;
    let _tracing_guard = init_tracing(&paths)?;

    info!("starting triginta");

    let database = Database::open(&paths.db_path)?;
    let screen_data = ScreenData {
        tasks: database.task_repository().list_all()?,
        recent_sessions: database.pomodoro_repository().list_recent(10)?,
        stats: database.pomodoro_repository().stats()?,
    };

    let provider = DisabledTodoistProvider;
    info!(
        provider = provider.provider_name(),
        configured = provider.is_configured(),
        "integration boundary initialized"
    );

    let mut app = App::new(screen_data);
    let mut terminal = setup_terminal()?;

    let result = run_event_loop(&mut terminal, &mut app);
    // Terminal state must be restored even if the event loop returned an error.
    // This is the same concern as putting tty cleanup in a `goto cleanup` path
    // in C, just expressed more directly.
    restore_terminal(&mut terminal)?;
    result
}

fn run_event_loop(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    app: &mut App,
) -> Result<()> {
    while !app.should_quit() {
        terminal
            .draw(|frame| ui::render(frame, app))
            .context("failed to draw terminal frame")?;

        // `poll` waits up to `TICK_RATE`; if an event exists we read it.
        // The `let ... else` form is a concise "if not a key event, continue"
        // branch without nesting the main control flow.
        if event::poll(TICK_RATE).context("failed to poll for terminal events")? {
            let Event::Key(key) = event::read().context("failed to read terminal event")? else {
                continue;
            };

            if key.kind == KeyEventKind::Press {
                app.handle_key(key.code);
            }
        }
    }

    Ok(())
}

fn setup_terminal() -> Result<Terminal<CrosstermBackend<std::io::Stdout>>> {
    // The return type looks verbose because the concrete terminal backend type
    // is spelled out explicitly. Rust often prefers exact types over hidden
    // pointers, especially in lower-level code.
    enable_raw_mode().context("failed to enable raw mode")?;
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen).context("failed to enter alternate screen")?;
    let backend = CrosstermBackend::new(stdout);
    Terminal::new(backend).context("failed to initialize terminal")
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>) -> Result<()> {
    disable_raw_mode().context("failed to disable raw mode")?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)
        .context("failed to leave alternate screen")?;
    terminal.show_cursor().context("failed to show cursor")
}

#[cfg(test)]
mod tests {
    use super::{App, RightPanelTab, ScreenData};

    #[test]
    fn app_starts_running() {
        let app = App::new(ScreenData::default());
        assert!(!app.should_quit());
        assert_eq!(app.active_right_panel_tab(), RightPanelTab::Tasks);
    }

    #[test]
    fn app_switches_right_panel_tabs() {
        let mut app = App::new(ScreenData::default());

        app.handle_key(crossterm::event::KeyCode::Tab);
        assert_eq!(app.active_right_panel_tab(), RightPanelTab::Statistics);

        app.handle_key(crossterm::event::KeyCode::Left);
        assert_eq!(app.active_right_panel_tab(), RightPanelTab::Tasks);
    }

    #[test]
    fn app_marks_quit_on_q() {
        let mut app = App::new(ScreenData::default());
        app.handle_key(crossterm::event::KeyCode::Char('q'));
        assert!(app.should_quit());
    }
}
