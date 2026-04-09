use std::time::Duration;

use anyhow::{Context, Result};
use chrono::{DateTime, Duration as ChronoDuration, Local};
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, prelude::CrosstermBackend};
use tracing::info;

use crate::{
    config::{AppConfig, AppPaths, GlyphMode, TimerSettings, init_tracing, load_app_config},
    domain::{HistoryStats, PomodoroSession, Task},
    integrations::{DisabledTodoistProvider, TaskSyncProvider},
    storage::{Database, PomodoroRepository, TaskRepository},
    ui,
};

const TICK_RATE: Duration = Duration::from_millis(250);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RunOptions {
    pub force_ascii: bool,
    pub force_short_timer: bool,
}

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelFocus {
    Timer,
    History,
    Navigation,
    Favorites,
    RightPane,
}

impl PanelFocus {
    pub fn next(self) -> Self {
        match self {
            Self::Timer => Self::History,
            Self::History => Self::Navigation,
            Self::Navigation => Self::Favorites,
            Self::Favorites => Self::RightPane,
            Self::RightPane => Self::Timer,
        }
    }

    pub fn previous(self) -> Self {
        match self {
            Self::Timer => Self::RightPane,
            Self::History => Self::Timer,
            Self::Navigation => Self::History,
            Self::Favorites => Self::Navigation,
            Self::RightPane => Self::Favorites,
        }
    }

    pub fn from_shortcut(key: char) -> Option<Self> {
        match key {
            '1' => Some(Self::Timer),
            '2' => Some(Self::History),
            '3' => Some(Self::Navigation),
            '4' => Some(Self::Favorites),
            '5' => Some(Self::RightPane),
            _ => None,
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Timer => "Timer",
            Self::History => "Daily History",
            Self::Navigation => "Navigation",
            Self::Favorites => "Favorites",
            Self::RightPane => "Right Pane",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimerPhase {
    Focus,
    ShortBreak,
    LongBreak,
}

impl TimerPhase {
    pub fn label(self) -> &'static str {
        match self {
            Self::Focus => "Pomodoro",
            Self::ShortBreak => "Short Break",
            Self::LongBreak => "Long Break",
        }
    }

    pub fn duration(self, settings: &TimerSettings) -> ChronoDuration {
        match self {
            Self::Focus => chrono_duration(settings.pomodoro_length),
            Self::ShortBreak => chrono_duration(settings.short_break_length),
            Self::LongBreak => chrono_duration(settings.long_break_length),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimerRunState {
    Idle,
    Running,
    Paused,
}

impl TimerRunState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Idle => "Ready",
            Self::Running => "Running",
            Self::Paused => "Paused",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CycleEntryState {
    NotStarted,
    Running,
    Break,
    Completed,
    Voided,
}

#[derive(Debug, Clone)]
pub struct TimerState {
    phase: TimerPhase,
    run_state: TimerRunState,
    elapsed: ChronoDuration,
    current_phase_started_at: Option<DateTime<Local>>,
    running_since: Option<DateTime<Local>>,
    completed_cycles_in_round: u32,
    current_cycle_index: usize,
    cycle_entries: Vec<CycleEntryState>,
}

impl TimerState {
    fn new(long_break_interval: u32) -> Self {
        Self {
            phase: TimerPhase::Focus,
            run_state: TimerRunState::Idle,
            elapsed: ChronoDuration::zero(),
            current_phase_started_at: None,
            running_since: None,
            completed_cycles_in_round: 0,
            current_cycle_index: 0,
            cycle_entries: vec![CycleEntryState::NotStarted; long_break_interval as usize],
        }
    }

    fn elapsed_at(&self, now: DateTime<Local>) -> ChronoDuration {
        match (self.run_state, self.running_since) {
            (TimerRunState::Running, Some(running_since)) => self.elapsed + (now - running_since),
            _ => self.elapsed,
        }
    }

    fn duration(&self, settings: &TimerSettings) -> ChronoDuration {
        self.phase.duration(settings)
    }

    fn remaining_at(&self, now: DateTime<Local>, settings: &TimerSettings) -> ChronoDuration {
        let remaining = self.duration(settings) - self.elapsed_at(now);
        if remaining < ChronoDuration::zero() {
            ChronoDuration::zero()
        } else {
            remaining
        }
    }

    fn progress_at(&self, now: DateTime<Local>, settings: &TimerSettings) -> f64 {
        let total_ms = self.duration(settings).num_milliseconds();
        if total_ms <= 0 {
            return 0.0;
        }

        let elapsed_ms = self.elapsed_at(now).num_milliseconds().clamp(0, total_ms);
        elapsed_ms as f64 / total_ms as f64
    }

    fn start_or_resume(&mut self, now: DateTime<Local>) {
        if self.run_state == TimerRunState::Running {
            return;
        }

        self.ensure_current_entry();
        if self.current_phase_started_at.is_none() {
            self.current_phase_started_at = Some(now);
        }

        self.running_since = Some(now);
        self.run_state = TimerRunState::Running;
        if let Some(current) = self.cycle_entries.get_mut(self.current_cycle_index) {
            *current = match self.phase {
                TimerPhase::Focus => CycleEntryState::Running,
                TimerPhase::ShortBreak | TimerPhase::LongBreak => CycleEntryState::Break,
            };
        }
    }

    fn pause(&mut self, now: DateTime<Local>) {
        if self.run_state != TimerRunState::Running {
            return;
        }

        self.elapsed = self.elapsed_at(now);
        self.running_since = None;
        self.run_state = TimerRunState::Paused;
    }

    fn reset_to_focus(&mut self) {
        self.phase = TimerPhase::Focus;
        self.run_state = TimerRunState::Idle;
        self.elapsed = ChronoDuration::zero();
        self.current_phase_started_at = None;
        self.running_since = None;
    }

    fn move_to_phase(&mut self, phase: TimerPhase) {
        self.phase = phase;
        self.run_state = TimerRunState::Idle;
        self.elapsed = ChronoDuration::zero();
        self.current_phase_started_at = None;
        self.running_since = None;
    }

    fn ensure_current_entry(&mut self) {
        if self.current_cycle_index >= self.cycle_entries.len() {
            self.cycle_entries.push(CycleEntryState::NotStarted);
        }
    }

    fn void_current_and_prepare_next(&mut self) {
        self.ensure_current_entry();
        if let Some(current) = self.cycle_entries.get_mut(self.current_cycle_index) {
            *current = CycleEntryState::Voided;
        }
        self.reset_to_focus();
        self.cycle_entries.push(CycleEntryState::NotStarted);
        self.current_cycle_index += 1;
    }

    fn complete_break(&mut self) {
        self.ensure_current_entry();
        if let Some(current) = self.cycle_entries.get_mut(self.current_cycle_index) {
            *current = CycleEntryState::Completed;
        }
        self.completed_cycles_in_round += 1;
    }

    fn prepare_next_focus_slot(&mut self) {
        self.current_cycle_index += 1;
        self.ensure_current_entry();
    }

    fn reset_round(&mut self, long_break_interval: u32) {
        self.completed_cycles_in_round = 0;
        self.current_cycle_index = 0;
        self.cycle_entries = vec![CycleEntryState::NotStarted; long_break_interval as usize];
        self.reset_to_focus();
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TimerView {
    pub phase: TimerPhase,
    pub run_state: TimerRunState,
    pub elapsed: ChronoDuration,
    pub remaining: ChronoDuration,
    pub progress: f64,
    pub cycle_entries: Vec<CycleEntryState>,
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
#[derive(Debug)]
pub struct App {
    database: Database,
    timer_settings: TimerSettings,
    active_right_panel_tab: RightPanelTab,
    focused_panel: PanelFocus,
    glyph_mode: GlyphMode,
    timer: TimerState,
    should_quit: bool,
    status_message: String,
    screen_data: ScreenData,
}

impl App {
    pub fn new(
        screen_data: ScreenData,
        glyph_mode: GlyphMode,
        timer_settings: TimerSettings,
        database: Database,
    ) -> Self {
        let long_break_interval = timer_settings.long_break_interval;
        Self {
            database,
            timer_settings,
            active_right_panel_tab: RightPanelTab::Tasks,
            focused_panel: PanelFocus::Timer,
            glyph_mode,
            timer: TimerState::new(long_break_interval),
            should_quit: false,
            status_message: "SQLite initialized. Local-first mode active.".to_string(),
            screen_data,
        }
    }

    pub fn active_right_panel_tab(&self) -> RightPanelTab {
        self.active_right_panel_tab
    }

    pub fn focused_panel(&self) -> PanelFocus {
        self.focused_panel
    }

    pub fn should_quit(&self) -> bool {
        self.should_quit
    }

    pub fn glyph_mode(&self) -> GlyphMode {
        self.glyph_mode
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

    pub fn timer_settings(&self) -> &TimerSettings {
        &self.timer_settings
    }

    pub fn timer_view(&self) -> TimerView {
        self.timer_view_at(Local::now())
    }

    fn timer_view_at(&self, now: DateTime<Local>) -> TimerView {
        TimerView {
            phase: self.timer.phase,
            run_state: self.timer.run_state,
            elapsed: self.timer.elapsed_at(now),
            remaining: self.timer.remaining_at(now, &self.timer_settings),
            progress: self.timer.progress_at(now, &self.timer_settings),
            cycle_entries: self.timer.cycle_entries.clone(),
        }
    }

    pub fn handle_key(&mut self, code: KeyCode) -> Result<()> {
        self.handle_key_at(code, Local::now())
    }

    fn handle_key_at(&mut self, code: KeyCode, now: DateTime<Local>) -> Result<()> {
        // `&mut self` is exclusive access: while this method runs, no other
        // code can also mutate the app state. This prevents a whole class of
        // aliasing bugs that are easy to create in C.
        match code {
            KeyCode::Char('q') => {
                self.should_quit = true;
                self.status_message = "Shutting down Triginta.".to_string();
            }
            KeyCode::Char(key) if PanelFocus::from_shortcut(key).is_some() => {
                self.focused_panel =
                    PanelFocus::from_shortcut(key).expect("focus shortcut checked");
                self.status_message = format!("Focused {} panel.", self.focused_panel.title());
            }
            KeyCode::Tab => {
                self.focused_panel = self.focused_panel.next();
                self.status_message = format!("Focused {} panel.", self.focused_panel.title());
            }
            KeyCode::BackTab => {
                self.focused_panel = self.focused_panel.previous();
                self.status_message = format!("Focused {} panel.", self.focused_panel.title());
            }
            KeyCode::Char('l') | KeyCode::Right if self.focused_panel == PanelFocus::RightPane => {
                self.active_right_panel_tab = self.active_right_panel_tab.next();
                self.status_message = match self.active_right_panel_tab {
                    RightPanelTab::Tasks => "Switched right panel to tasks.".to_string(),
                    RightPanelTab::Statistics => "Switched right panel to statistics.".to_string(),
                };
            }
            KeyCode::Char('h') | KeyCode::Left if self.focused_panel == PanelFocus::RightPane => {
                self.active_right_panel_tab = self.active_right_panel_tab.previous();
                self.status_message = match self.active_right_panel_tab {
                    RightPanelTab::Tasks => "Switched right panel to tasks.".to_string(),
                    RightPanelTab::Statistics => "Switched right panel to statistics.".to_string(),
                };
            }
            KeyCode::Char('s') | KeyCode::Char(' ') | KeyCode::Enter
                if self.focused_panel == PanelFocus::Timer =>
            {
                self.timer.start_or_resume(now);
                self.status_message = format!("{} started.", self.timer.phase.label());
            }
            KeyCode::Char('p') if self.focused_panel == PanelFocus::Timer => {
                self.timer.pause(now);
                self.status_message = format!("{} paused.", self.timer.phase.label());
            }
            KeyCode::Char('x') | KeyCode::Esc if self.focused_panel == PanelFocus::Timer => {
                let current_cycle_state = self
                    .timer
                    .cycle_entries
                    .get(self.timer.current_cycle_index)
                    .copied()
                    .unwrap_or(CycleEntryState::NotStarted);
                if self.timer.run_state == TimerRunState::Idle
                    && self.timer.phase == TimerPhase::Focus
                    && current_cycle_state == CycleEntryState::NotStarted
                {
                    self.status_message = "Timer already ready.".to_string();
                } else {
                    self.timer.void_current_and_prepare_next();
                    self.status_message =
                        "Current pomodoro voided. Next pomodoro ready.".to_string();
                }
            }
            _ => {}
        }

        Ok(())
    }

    pub fn on_tick(&mut self) -> Result<()> {
        self.on_tick_at(Local::now())
    }

    fn on_tick_at(&mut self, now: DateTime<Local>) -> Result<()> {
        if self.timer.run_state != TimerRunState::Running {
            return Ok(());
        }

        if self.timer.elapsed_at(now) < self.timer.duration(&self.timer_settings) {
            return Ok(());
        }

        match self.timer.phase {
            TimerPhase::Focus => {
                let started_at = self
                    .timer
                    .current_phase_started_at
                    .unwrap_or(now - chrono_duration(self.timer_settings.pomodoro_length));
                self.database.pomodoro_repository().create(
                    None,
                    started_at,
                    now,
                    duration_to_stored_minutes(self.timer_settings.pomodoro_length),
                )?;

                let next_phase = if self.timer.completed_cycles_in_round + 1
                    == self.timer_settings.long_break_interval
                {
                    TimerPhase::LongBreak
                } else {
                    TimerPhase::ShortBreak
                };

                self.timer.move_to_phase(next_phase);
                self.timer.start_or_resume(now);
                self.refresh_history()?;
                self.status_message = format!(
                    "Pomodoro complete. {} started automatically.",
                    next_phase.label()
                );
            }
            TimerPhase::ShortBreak | TimerPhase::LongBreak => {
                let completed_long_break = self.timer.phase == TimerPhase::LongBreak;
                self.timer.complete_break();
                if completed_long_break {
                    self.timer
                        .reset_round(self.timer_settings.long_break_interval);
                } else {
                    self.timer.move_to_phase(TimerPhase::Focus);
                    self.timer.prepare_next_focus_slot();
                }
                self.status_message = "Break complete. Pomodoro ready.".to_string();
            }
        }

        Ok(())
    }

    fn refresh_history(&mut self) -> Result<()> {
        self.screen_data.recent_sessions = self.database.pomodoro_repository().list_recent(10)?;
        self.screen_data.stats = self.database.pomodoro_repository().stats()?;
        Ok(())
    }
}

pub fn run(options: RunOptions) -> Result<()> {
    // Startup is written as a straight-line sequence of fallible operations.
    // The `?` operator keeps this readable: each step either succeeds and
    // continues, or returns early with an error.
    let paths = AppPaths::resolve()?;
    paths.ensure_dirs()?;
    let _tracing_guard = init_tracing(&paths)?;
    let mut config = load_app_config(&paths)?;
    apply_debug_overrides(&mut config, options);

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

    let mut app = App::new(screen_data, config.ui.glyph_mode, config.timer, database);
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
        app.on_tick()?;
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
                app.handle_key(key.code)?;
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

fn apply_debug_overrides(config: &mut AppConfig, options: RunOptions) {
    if options.force_ascii {
        config.ui.glyph_mode = GlyphMode::Ascii;
    }
    if options.force_short_timer {
        config.timer = TimerSettings::short_timer_preset();
    }
}

fn chrono_duration(duration: Duration) -> ChronoDuration {
    ChronoDuration::from_std(duration).expect("timer duration should fit in chrono duration")
}

fn duration_to_stored_minutes(duration: Duration) -> u32 {
    duration.as_secs().div_ceil(60) as u32
}

#[cfg(test)]
mod tests {
    use chrono::{Duration as ChronoDuration, Local};

    use crate::config::{AppConfig, GlyphMode, TimerSettings};
    use crate::storage::Database;

    use super::{
        App, CycleEntryState, PanelFocus, RightPanelTab, RunOptions, ScreenData, TimerPhase,
        TimerRunState, apply_debug_overrides, chrono_duration, duration_to_stored_minutes,
    };

    fn test_app() -> App {
        App::new(
            ScreenData::default(),
            GlyphMode::NerdFonts,
            TimerSettings::default(),
            Database::open_in_memory().expect("in-memory database should open"),
        )
    }

    #[test]
    fn app_starts_running() {
        let app = test_app();
        assert!(!app.should_quit());
        assert_eq!(app.active_right_panel_tab(), RightPanelTab::Tasks);
        assert_eq!(app.glyph_mode(), GlyphMode::NerdFonts);
        assert_eq!(app.focused_panel(), PanelFocus::Timer);
        assert_eq!(app.timer_view().phase, TimerPhase::Focus);
        assert_eq!(app.timer_view().run_state, TimerRunState::Idle);
        assert_eq!(
            app.timer_view().cycle_entries,
            vec![
                CycleEntryState::NotStarted,
                CycleEntryState::NotStarted,
                CycleEntryState::NotStarted,
                CycleEntryState::NotStarted
            ]
        );
    }

    #[test]
    fn app_switches_right_panel_tabs() {
        let mut app = test_app();
        app.handle_key(crossterm::event::KeyCode::Char('5'))
            .expect("focus should switch");

        app.handle_key(crossterm::event::KeyCode::Right)
            .expect("right panel tab should switch");
        assert_eq!(app.active_right_panel_tab(), RightPanelTab::Statistics);

        app.handle_key(crossterm::event::KeyCode::Left)
            .expect("right panel tab should switch back");
        assert_eq!(app.active_right_panel_tab(), RightPanelTab::Tasks);
    }

    #[test]
    fn app_marks_quit_on_q() {
        let mut app = test_app();
        app.handle_key(crossterm::event::KeyCode::Char('q'))
            .expect("quit should succeed");
        assert!(app.should_quit());
    }

    #[test]
    fn timer_start_pause_and_reset_work_from_timer_panel() {
        let mut app = test_app();
        let now = Local::now();

        app.handle_key_at(crossterm::event::KeyCode::Char('s'), now)
            .expect("timer should start");
        assert_eq!(app.timer.run_state, TimerRunState::Running);

        app.handle_key_at(
            crossterm::event::KeyCode::Char('p'),
            now + ChronoDuration::minutes(3),
        )
        .expect("timer should pause");
        assert_eq!(app.timer.run_state, TimerRunState::Paused);
        assert!(app.timer.elapsed >= ChronoDuration::minutes(3));

        app.handle_key_at(crossterm::event::KeyCode::Char('x'), now)
            .expect("timer should reset");
        assert_eq!(app.timer.phase, TimerPhase::Focus);
        assert_eq!(app.timer.run_state, TimerRunState::Idle);
        assert_eq!(app.timer.elapsed, ChronoDuration::zero());
        assert_eq!(
            app.timer.cycle_entries,
            vec![
                CycleEntryState::Voided,
                CycleEntryState::NotStarted,
                CycleEntryState::NotStarted,
                CycleEntryState::NotStarted,
                CycleEntryState::NotStarted
            ]
        );
    }

    #[test]
    fn completed_pomodoro_transitions_to_break_without_completing_cycle() {
        let mut app = test_app();
        let started_at = Local::now() - chrono_duration(app.timer_settings.pomodoro_length);
        app.timer.phase = TimerPhase::Focus;
        app.timer.run_state = TimerRunState::Running;
        app.timer.current_phase_started_at = Some(started_at);
        app.timer.running_since = Some(started_at);

        app.on_tick_at(Local::now())
            .expect("tick should complete phase");

        assert_eq!(app.timer.phase, TimerPhase::ShortBreak);
        assert_eq!(app.timer.run_state, TimerRunState::Running);
        assert_eq!(app.screen_data.stats.total_sessions, 1);
        assert_eq!(
            app.screen_data.stats.total_minutes,
            duration_to_stored_minutes(app.timer_settings.pomodoro_length)
        );
        assert_eq!(
            app.timer.cycle_entries,
            vec![
                CycleEntryState::Break,
                CycleEntryState::NotStarted,
                CycleEntryState::NotStarted,
                CycleEntryState::NotStarted
            ]
        );
    }

    #[test]
    fn completed_break_marks_cycle_complete_and_prepares_next_slot() {
        let mut app = test_app();
        let started_at = Local::now() - chrono_duration(app.timer_settings.short_break_length);
        app.timer.phase = TimerPhase::ShortBreak;
        app.timer.run_state = TimerRunState::Running;
        app.timer.current_phase_started_at = Some(started_at);
        app.timer.running_since = Some(started_at);
        app.timer.cycle_entries = vec![
            CycleEntryState::Break,
            CycleEntryState::NotStarted,
            CycleEntryState::NotStarted,
            CycleEntryState::NotStarted,
        ];

        app.on_tick_at(Local::now())
            .expect("tick should complete break");

        assert_eq!(app.timer.phase, TimerPhase::Focus);
        assert_eq!(app.timer.run_state, TimerRunState::Idle);
        assert_eq!(
            app.timer.cycle_entries,
            vec![
                CycleEntryState::Completed,
                CycleEntryState::NotStarted,
                CycleEntryState::NotStarted,
                CycleEntryState::NotStarted
            ]
        );
    }

    #[test]
    fn debug_short_timer_override_replaces_timer_settings() {
        let mut config = AppConfig::default();
        apply_debug_overrides(
            &mut config,
            RunOptions {
                force_ascii: false,
                force_short_timer: true,
            },
        );

        assert_eq!(config.timer, TimerSettings::short_timer_preset());
    }
}
