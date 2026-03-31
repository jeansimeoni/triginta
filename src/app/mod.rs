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
pub enum AppScreen {
    Timer,
    Tasks,
    History,
}

impl AppScreen {
    pub fn next(self) -> Self {
        match self {
            Self::Timer => Self::Tasks,
            Self::Tasks => Self::History,
            Self::History => Self::Timer,
        }
    }

    pub fn previous(self) -> Self {
        match self {
            Self::Timer => Self::History,
            Self::Tasks => Self::Timer,
            Self::History => Self::Tasks,
        }
    }

    pub fn index(self) -> usize {
        match self {
            Self::Timer => 0,
            Self::Tasks => 1,
            Self::History => 2,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ScreenData {
    pub tasks: Vec<Task>,
    pub recent_sessions: Vec<PomodoroSession>,
    pub stats: HistoryStats,
}

#[derive(Debug, Clone)]
pub struct App {
    current_screen: AppScreen,
    should_quit: bool,
    status_message: String,
    screen_data: ScreenData,
}

impl App {
    pub fn new(screen_data: ScreenData) -> Self {
        Self {
            current_screen: AppScreen::Timer,
            should_quit: false,
            status_message: "SQLite initialized. Local-first mode active.".to_string(),
            screen_data,
        }
    }

    pub fn current_screen(&self) -> AppScreen {
        self.current_screen
    }

    pub fn should_quit(&self) -> bool {
        self.should_quit
    }

    pub fn status_message(&self) -> &str {
        &self.status_message
    }

    pub fn screen_data(&self) -> &ScreenData {
        &self.screen_data
    }

    pub fn handle_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Char('q') => {
                self.should_quit = true;
                self.status_message = "Shutting down Triginta.".to_string();
            }
            KeyCode::Tab | KeyCode::Char('l') | KeyCode::Right => {
                self.current_screen = self.current_screen.next();
                self.status_message = format!("Switched to {}.", self.current_screen.label());
            }
            KeyCode::BackTab | KeyCode::Char('h') | KeyCode::Left => {
                self.current_screen = self.current_screen.previous();
                self.status_message = format!("Switched to {}.", self.current_screen.label());
            }
            _ => {}
        }
    }
}

impl AppScreen {
    fn label(self) -> &'static str {
        match self {
            Self::Timer => "Timer",
            Self::Tasks => "Tasks",
            Self::History => "History",
        }
    }
}

pub fn run() -> Result<()> {
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
    use crate::domain::HistoryStats;

    use super::{App, AppScreen, ScreenData};

    #[test]
    fn app_starts_on_timer_screen() {
        let app = App::new(ScreenData::default());
        assert_eq!(app.current_screen(), AppScreen::Timer);
        assert!(!app.should_quit());
    }

    #[test]
    fn app_switches_screens_forward_and_back() {
        let mut app = App::new(ScreenData {
            stats: HistoryStats::default(),
            ..ScreenData::default()
        });

        app.handle_key(crossterm::event::KeyCode::Tab);
        assert_eq!(app.current_screen(), AppScreen::Tasks);

        app.handle_key(crossterm::event::KeyCode::Right);
        assert_eq!(app.current_screen(), AppScreen::History);

        app.handle_key(crossterm::event::KeyCode::Left);
        assert_eq!(app.current_screen(), AppScreen::Tasks);
    }

    #[test]
    fn app_marks_quit_on_q() {
        let mut app = App::new(ScreenData::default());
        app.handle_key(crossterm::event::KeyCode::Char('q'));
        assert!(app.should_quit());
    }
}
