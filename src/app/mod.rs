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
    domain::{
        DayHistorySummary, HistoryStats, SessionEntry, SessionKind, SessionOutcome, Task, TaskId,
        TaskStatus,
    },
    integrations::{DisabledTodoistProvider, TaskSyncProvider},
    storage::{Database, PomodoroRepository, TaskRepository},
    theme::ThemePalette,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskView {
    All,
    Inbox,
    Today,
    Soon,
}

impl TaskView {
    const ALL: [Self; 4] = [Self::All, Self::Inbox, Self::Today, Self::Soon];

    pub fn all() -> &'static [Self] {
        &Self::ALL
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Inbox => "Inbox",
            Self::Today => "Today",
            Self::Soon => "Soon",
        }
    }

    fn index(self) -> usize {
        match self {
            Self::All => 0,
            Self::Inbox => 1,
            Self::Today => 2,
            Self::Soon => 3,
        }
    }

    fn from_index(index: usize) -> Self {
        Self::ALL[index.min(Self::ALL.len().saturating_sub(1))]
    }

    fn next(self) -> Self {
        Self::from_index((self.index() + 1).min(Self::ALL.len().saturating_sub(1)))
    }

    fn previous(self) -> Self {
        Self::from_index(self.index().saturating_sub(1))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryPanelTab {
    Today,
    Last7Days,
}

impl HistoryPanelTab {
    pub fn next(self) -> Self {
        match self {
            Self::Today => Self::Last7Days,
            Self::Last7Days => Self::Today,
        }
    }

    pub fn previous(self) -> Self {
        self.next()
    }
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
    pub fn label(self, phase: TimerPhase) -> &'static str {
        match self {
            Self::Idle => "Ready",
            Self::Running => match phase {
                TimerPhase::Focus => "Focus",
                TimerPhase::ShortBreak | TimerPhase::LongBreak => "Break",
            },
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
    pub history_entries: Vec<SessionEntry>,
    pub today_stats: HistoryStats,
    pub weekly_summaries: Vec<DayHistorySummary>,
    pub weekly_stats: HistoryStats,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TaskInputMode {
    Create,
    Rename(TaskId),
}

#[derive(Debug, Clone)]
struct TaskInputState {
    mode: TaskInputMode,
    value: String,
    cursor: usize,
}

#[derive(Debug, Clone)]
struct TaskSearchState {
    mode: TaskSearchMode,
    query: String,
    cursor: usize,
    selected_index: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TaskSearchMode {
    TimerAssignment,
    HistoryAssignment(i64),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskInputView {
    pub title: &'static str,
    pub value: String,
    pub cursor: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeleteConfirmationView {
    pub task_id: TaskId,
    pub task_title: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskSearchResultView {
    pub task_id: TaskId,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskSearchView {
    pub title: &'static str,
    pub query: String,
    pub cursor: usize,
    pub selected_index: usize,
    pub results: Vec<TaskSearchResultView>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShortcutTip {
    pub keys: &'static str,
    pub description: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShortcutSection {
    pub title: &'static str,
    pub tips: &'static [ShortcutTip],
}

const GLOBAL_SHORTCUTS: &[ShortcutTip] = &[
    ShortcutTip {
        keys: "1-5",
        description: "focus panel",
    },
    ShortcutTip {
        keys: "Tab/S-Tab",
        description: "next/prev panel",
    },
    ShortcutTip {
        keys: "?",
        description: "help",
    },
    ShortcutTip {
        keys: "q",
        description: "quit",
    },
];

const TIMER_SHORTCUTS: &[ShortcutTip] = &[
    ShortcutTip {
        keys: "s/Space/Enter",
        description: "start/resume",
    },
    ShortcutTip {
        keys: "p",
        description: "pause",
    },
    ShortcutTip {
        keys: "x/Esc",
        description: "void/reset",
    },
    ShortcutTip {
        keys: "a",
        description: "assign task",
    },
    ShortcutTip {
        keys: "u",
        description: "clear task",
    },
];

const HISTORY_SHORTCUTS: &[ShortcutTip] = &[
    ShortcutTip {
        keys: "h/l or ←/→",
        description: "switch range",
    },
    ShortcutTip {
        keys: "j/k or ↑/↓",
        description: "move session",
    },
    ShortcutTip {
        keys: "PgUp/PgDn",
        description: "page",
    },
    ShortcutTip {
        keys: "a",
        description: "assign task",
    },
    ShortcutTip {
        keys: "u",
        description: "clear task",
    },
];

const NAVIGATION_SHORTCUTS: &[ShortcutTip] = &[
    ShortcutTip {
        keys: "j/k or ↑/↓",
        description: "change view",
    },
    ShortcutTip {
        keys: "Home/End",
        description: "jump first/last",
    },
];

const FAVORITES_SHORTCUTS: &[ShortcutTip] = &[ShortcutTip {
    keys: "1-5 / Tab",
    description: "change focus",
}];

const TASKS_SHORTCUTS: &[ShortcutTip] = &[
    ShortcutTip {
        keys: "h/l or ←/→",
        description: "switch tab",
    },
    ShortcutTip {
        keys: "j/k or ↑/↓",
        description: "move task",
    },
    ShortcutTip {
        keys: "c",
        description: "new task",
    },
    ShortcutTip {
        keys: "e/d",
        description: "rename/delete",
    },
    ShortcutTip {
        keys: "a",
        description: "assign to timer",
    },
    ShortcutTip {
        keys: "Space/x",
        description: "toggle done",
    },
];

const STATISTICS_SHORTCUTS: &[ShortcutTip] = &[ShortcutTip {
    keys: "h/l or ←/→",
    description: "switch tab",
}];

const INPUT_POPUP_SHORTCUTS: &[ShortcutTip] = &[
    ShortcutTip {
        keys: "Enter",
        description: "submit",
    },
    ShortcutTip {
        keys: "Esc",
        description: "cancel",
    },
    ShortcutTip {
        keys: "Home/End",
        description: "move cursor",
    },
    ShortcutTip {
        keys: "Backspace",
        description: "delete char",
    },
];

const SEARCH_POPUP_SHORTCUTS: &[ShortcutTip] = &[
    ShortcutTip {
        keys: "Enter",
        description: "assign selected",
    },
    ShortcutTip {
        keys: "Esc",
        description: "cancel",
    },
    ShortcutTip {
        keys: "j/k or ↑/↓",
        description: "move result",
    },
    ShortcutTip {
        keys: "Home/End",
        description: "move cursor",
    },
    ShortcutTip {
        keys: "Backspace",
        description: "delete char",
    },
];

const DELETE_CONFIRMATION_SHORTCUTS: &[ShortcutTip] = &[
    ShortcutTip {
        keys: "Enter/y",
        description: "confirm",
    },
    ShortcutTip {
        keys: "Esc/n",
        description: "cancel",
    },
];

// `App` owns the mutable runtime state for the TUI loop.
// Compared with a C program, this is the central state struct you would pass
// around to input/render functions, but here methods are attached directly to
// the type.
#[derive(Debug)]
pub struct App {
    database: Database,
    timer_settings: TimerSettings,
    active_right_panel_tab: RightPanelTab,
    active_history_panel_tab: HistoryPanelTab,
    active_task_view: TaskView,
    focused_panel: PanelFocus,
    glyph_mode: GlyphMode,
    theme: ThemePalette,
    timer: TimerState,
    history_scroll: usize,
    selected_task_id: Option<TaskId>,
    assigned_task_id: Option<TaskId>,
    active_focus_task_id: Option<TaskId>,
    task_input: Option<TaskInputState>,
    task_search: Option<TaskSearchState>,
    delete_confirmation: Option<TaskId>,
    help_open: bool,
    help_scroll: usize,
    help_viewport_lines: usize,
    should_quit: bool,
    screen_data: ScreenData,
}

impl App {
    pub fn new(
        screen_data: ScreenData,
        glyph_mode: GlyphMode,
        theme: ThemePalette,
        timer_settings: TimerSettings,
        database: Database,
    ) -> Self {
        let long_break_interval = timer_settings.long_break_interval;
        let mut app = Self {
            database,
            timer_settings,
            active_right_panel_tab: RightPanelTab::Tasks,
            active_history_panel_tab: HistoryPanelTab::Today,
            active_task_view: TaskView::All,
            focused_panel: PanelFocus::Timer,
            glyph_mode,
            theme,
            timer: TimerState::new(long_break_interval),
            history_scroll: 0,
            selected_task_id: None,
            assigned_task_id: None,
            active_focus_task_id: None,
            task_input: None,
            task_search: None,
            delete_confirmation: None,
            help_open: false,
            help_scroll: 0,
            help_viewport_lines: 0,
            should_quit: false,
            screen_data,
        };
        app.sync_task_selection();
        app
    }

    pub fn active_right_panel_tab(&self) -> RightPanelTab {
        self.active_right_panel_tab
    }

    pub fn active_history_panel_tab(&self) -> HistoryPanelTab {
        self.active_history_panel_tab
    }

    pub fn focused_panel(&self) -> PanelFocus {
        self.focused_panel
    }

    pub fn active_task_view(&self) -> TaskView {
        self.active_task_view
    }

    pub fn should_quit(&self) -> bool {
        self.should_quit
    }

    pub fn glyph_mode(&self) -> GlyphMode {
        self.glyph_mode
    }

    pub fn theme(&self) -> ThemePalette {
        self.theme
    }

    pub fn visible_tasks(&self) -> Vec<&Task> {
        self.screen_data
            .tasks
            .iter()
            .filter(|task| self.task_is_active(task) && self.task_matches_active_view(task))
            .collect()
    }

    pub fn selected_task(&self) -> Option<&Task> {
        self.selected_task_id.and_then(|task_id| {
            self.screen_data.tasks.iter().find(|task| {
                task.id == task_id
                    && self.task_is_active(task)
                    && self.task_matches_active_view(task)
            })
        })
    }

    pub fn assigned_task(&self) -> Option<&Task> {
        self.assigned_task_id.and_then(|task_id| {
            self.screen_data
                .tasks
                .iter()
                .find(|task| task.id == task_id)
        })
    }

    pub fn task_input_view(&self) -> Option<TaskInputView> {
        self.task_input.as_ref().map(|input| TaskInputView {
            title: match input.mode {
                TaskInputMode::Create => "New Task",
                TaskInputMode::Rename(_) => "Rename Task",
            },
            value: input.value.clone(),
            cursor: input.cursor,
        })
    }

    pub fn delete_confirmation_view(&self) -> Option<DeleteConfirmationView> {
        let task_id = self.delete_confirmation?;
        let task = self
            .screen_data
            .tasks
            .iter()
            .find(|task| task.id == task_id)?;
        Some(DeleteConfirmationView {
            task_id,
            task_title: task.title.clone(),
        })
    }

    pub fn task_search_view(&self) -> Option<TaskSearchView> {
        let search = self.task_search.as_ref()?;
        Some(TaskSearchView {
            title: match search.mode {
                TaskSearchMode::TimerAssignment => "Assign Task",
                TaskSearchMode::HistoryAssignment(_) => "Assign Session Task",
            },
            query: search.query.clone(),
            cursor: search.cursor,
            selected_index: search.selected_index,
            results: self
                .searchable_tasks(search.query.as_str())
                .into_iter()
                .map(|task| TaskSearchResultView {
                    task_id: task.id,
                    title: task.title.clone(),
                })
                .collect(),
        })
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

    pub fn history_scroll(&self) -> usize {
        self.history_scroll
    }

    pub fn is_help_open(&self) -> bool {
        self.help_open
    }

    pub fn help_scroll(&self) -> usize {
        self.help_scroll
    }

    pub fn app_name(&self) -> &'static str {
        "Triginta"
    }

    pub fn app_version(&self) -> &'static str {
        env!("CARGO_PKG_VERSION")
    }

    pub fn donate_label(&self) -> &'static str {
        "Donate"
    }

    pub fn focused_panel_shortcuts(&self) -> &'static [ShortcutTip] {
        match self.focused_panel {
            PanelFocus::Timer => TIMER_SHORTCUTS,
            PanelFocus::History => HISTORY_SHORTCUTS,
            PanelFocus::Navigation => NAVIGATION_SHORTCUTS,
            PanelFocus::Favorites => FAVORITES_SHORTCUTS,
            PanelFocus::RightPane => match self.active_right_panel_tab {
                RightPanelTab::Tasks => TASKS_SHORTCUTS,
                RightPanelTab::Statistics => STATISTICS_SHORTCUTS,
            },
        }
    }

    pub fn help_sections(&self) -> Vec<ShortcutSection> {
        let mut sections = vec![
            ShortcutSection {
                title: "Global",
                tips: GLOBAL_SHORTCUTS,
            },
            ShortcutSection {
                title: "Timer",
                tips: TIMER_SHORTCUTS,
            },
            ShortcutSection {
                title: "History",
                tips: HISTORY_SHORTCUTS,
            },
            ShortcutSection {
                title: "Navigation",
                tips: NAVIGATION_SHORTCUTS,
            },
            ShortcutSection {
                title: "Favorites",
                tips: FAVORITES_SHORTCUTS,
            },
            ShortcutSection {
                title: "Tasks",
                tips: TASKS_SHORTCUTS,
            },
            ShortcutSection {
                title: "Statistics",
                tips: STATISTICS_SHORTCUTS,
            },
        ];

        if self.task_input.is_some() {
            sections.push(ShortcutSection {
                title: "Task Input Popup",
                tips: INPUT_POPUP_SHORTCUTS,
            });
        }
        if self.task_search.is_some() {
            sections.push(ShortcutSection {
                title: "Task Search Popup",
                tips: SEARCH_POPUP_SHORTCUTS,
            });
        }
        if self.delete_confirmation.is_some() {
            sections.push(ShortcutSection {
                title: "Delete Confirmation",
                tips: DELETE_CONFIRMATION_SHORTCUTS,
            });
        }

        sections
    }

    pub fn help_line_count(&self) -> usize {
        let section_count = self.help_sections().len();
        self.help_sections()
            .into_iter()
            .map(|section| section.tips.len() + 1)
            .sum::<usize>()
            .saturating_add(section_count.saturating_sub(1))
    }

    pub fn sync_help_viewport(&mut self, terminal_height: u16) {
        let total_lines = self.help_line_count();
        let popup_height = if terminal_height >= 8 {
            (total_lines.saturating_add(2) as u16).min(terminal_height.saturating_sub(4))
        } else {
            (total_lines.saturating_add(2) as u16).min(terminal_height.saturating_sub(2).max(1))
        };
        self.help_viewport_lines = popup_height.saturating_sub(2).max(1) as usize;
        self.clamp_help_scroll();
    }

    fn max_help_scroll(&self) -> usize {
        self.help_line_count()
            .saturating_sub(self.help_viewport_lines.max(1))
    }

    fn clamp_help_scroll(&mut self) {
        self.help_scroll = self.help_scroll.min(self.max_help_scroll());
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

    fn task_matches_active_view(&self, _task: &Task) -> bool {
        match self.active_task_view {
            TaskView::All | TaskView::Inbox => true,
            TaskView::Today | TaskView::Soon => false,
        }
    }

    fn task_is_active(&self, task: &Task) -> bool {
        task.deleted_at.is_none()
    }

    fn visible_task_ids(&self) -> Vec<TaskId> {
        self.screen_data
            .tasks
            .iter()
            .filter(|task| self.task_is_active(task) && self.task_matches_active_view(task))
            .map(|task| task.id)
            .collect()
    }

    fn sync_task_selection(&mut self) {
        let visible_ids = self.visible_task_ids();
        self.selected_task_id = match self.selected_task_id {
            Some(selected_task_id) if visible_ids.contains(&selected_task_id) => {
                Some(selected_task_id)
            }
            _ => visible_ids.first().copied(),
        };
    }

    fn refresh_tasks(&mut self) -> Result<()> {
        self.screen_data.tasks = self.database.task_repository().list_all()?;
        if let Some(task_id) = self.assigned_task_id {
            if !self
                .screen_data
                .tasks
                .iter()
                .any(|task| task.id == task_id && self.task_is_active(task))
            {
                self.assigned_task_id = None;
            }
        }
        if let Some(task_id) = self.active_focus_task_id {
            if !self.screen_data.tasks.iter().any(|task| task.id == task_id) {
                self.active_focus_task_id = None;
            }
        }
        self.sync_task_selection();
        Ok(())
    }

    fn set_active_task_view(&mut self, view: TaskView) {
        self.active_task_view = view;
        self.sync_task_selection();
    }

    fn select_next_task_view(&mut self) {
        self.set_active_task_view(self.active_task_view.next());
    }

    fn select_previous_task_view(&mut self) {
        self.set_active_task_view(self.active_task_view.previous());
    }

    fn move_task_selection(&mut self, offset: isize) {
        let visible_ids = self.visible_task_ids();
        if visible_ids.is_empty() {
            self.selected_task_id = None;
            return;
        }

        let current_index = self
            .selected_task_id
            .and_then(|selected_task_id| {
                visible_ids
                    .iter()
                    .position(|task_id| *task_id == selected_task_id)
            })
            .unwrap_or(0);
        let next_index = (current_index as isize + offset)
            .clamp(0, visible_ids.len().saturating_sub(1) as isize)
            as usize;
        self.selected_task_id = visible_ids.get(next_index).copied();
    }

    fn searchable_tasks(&self, query: &str) -> Vec<&Task> {
        self.screen_data
            .tasks
            .iter()
            .filter(|task| self.task_is_active(task) && fuzzy_matches(query, task.title.as_str()))
            .collect()
    }

    fn open_create_task_popup(&mut self) {
        self.task_input = Some(TaskInputState {
            mode: TaskInputMode::Create,
            value: String::new(),
            cursor: 0,
        });
    }

    fn open_rename_task_popup(&mut self) {
        let Some(task) = self.selected_task().cloned() else {
            return;
        };

        self.task_input = Some(TaskInputState {
            mode: TaskInputMode::Rename(task.id),
            cursor: task.title.len(),
            value: task.title,
        });
    }

    fn move_input_cursor_home(input: &mut TaskInputState) {
        input.cursor = 0;
    }

    fn move_input_cursor_end(input: &mut TaskInputState) {
        input.cursor = input.value.len();
    }

    fn insert_input_char(input: &mut TaskInputState, character: char) {
        input.value.insert(input.cursor, character);
        input.cursor += character.len_utf8();
    }

    fn delete_input_char_before_cursor(input: &mut TaskInputState) {
        if input.cursor == 0 {
            return;
        }

        let previous_index = input.value[..input.cursor]
            .char_indices()
            .last()
            .map(|(index, _)| index)
            .unwrap_or(0);
        input.value.drain(previous_index..input.cursor);
        input.cursor = previous_index;
    }

    fn open_delete_confirmation(&mut self) {
        let Some(task_id) = self.selected_task_id else {
            return;
        };

        self.delete_confirmation = Some(task_id);
    }

    fn open_timer_task_search(&mut self) {
        self.task_search = Some(TaskSearchState {
            mode: TaskSearchMode::TimerAssignment,
            query: String::new(),
            cursor: 0,
            selected_index: 0,
        });
    }

    fn open_history_task_search(&mut self) {
        let Some(entry) = self.selected_history_focus_entry().cloned() else {
            return;
        };

        self.task_search = Some(TaskSearchState {
            mode: TaskSearchMode::HistoryAssignment(entry.id),
            query: String::new(),
            cursor: 0,
            selected_index: 0,
        });
    }

    fn toggle_selected_task_status(&mut self, now: DateTime<Local>) -> Result<()> {
        let Some(task) = self.selected_task().cloned() else {
            return Ok(());
        };

        let next_status = match task.status {
            TaskStatus::Todo => TaskStatus::Done,
            TaskStatus::Done => TaskStatus::Todo,
        };
        let completed_at = if next_status == TaskStatus::Done {
            Some(now)
        } else {
            None
        };

        self.database
            .task_repository()
            .update_status(task.id, next_status, completed_at)?;
        self.refresh_tasks()?;
        self.selected_task_id = Some(task.id);
        Ok(())
    }

    fn toggle_selected_task_assignment(&mut self) {
        let Some(task) = self.selected_task().cloned() else {
            return;
        };

        if self.assigned_task_id == Some(task.id) {
            self.assigned_task_id = None;
        } else {
            self.assigned_task_id = Some(task.id);
        }
    }

    fn clear_assigned_task(&mut self) {
        self.assigned_task_id = None;
    }

    fn clear_selected_history_task(&mut self) -> Result<()> {
        let Some(entry) = self.selected_history_focus_entry().cloned() else {
            return Ok(());
        };

        self.database
            .pomodoro_repository()
            .update_session_task(entry.id, None)?;
        self.refresh_history()?;
        Ok(())
    }

    fn selected_history_focus_entry(&self) -> Option<&SessionEntry> {
        if self.active_history_panel_tab != HistoryPanelTab::Today {
            return None;
        }

        self.screen_data
            .history_entries
            .iter()
            .filter(|entry| entry.kind == SessionKind::Focus)
            .nth(self.history_scroll)
    }

    fn begin_focus_task_if_needed(&mut self) {
        if self.timer.phase == TimerPhase::Focus && self.timer.current_phase_started_at.is_none() {
            self.active_focus_task_id = self.assigned_task_id;
        }
    }

    fn handle_task_overlay_key(&mut self, code: KeyCode, now: DateTime<Local>) -> Result<bool> {
        if let Some(mut search) = self.task_search.take() {
            match code {
                KeyCode::Esc => {}
                KeyCode::Enter => {
                    let matches = self.searchable_tasks(search.query.as_str());
                    if let Some(task) = matches.get(search.selected_index) {
                        let task_id = task.id;
                        match search.mode {
                            TaskSearchMode::TimerAssignment => {
                                self.assigned_task_id = Some(task_id);
                            }
                            TaskSearchMode::HistoryAssignment(session_id) => {
                                self.database
                                    .pomodoro_repository()
                                    .update_session_task(session_id, Some(task_id))?;
                                self.refresh_history()?;
                            }
                        }
                    } else {
                        self.task_search = Some(search);
                    }
                }
                KeyCode::Char('j') | KeyCode::Down => {
                    let last_index = self
                        .searchable_tasks(search.query.as_str())
                        .len()
                        .saturating_sub(1);
                    search.selected_index = (search.selected_index + 1).min(last_index);
                    self.task_search = Some(search);
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    search.selected_index = search.selected_index.saturating_sub(1);
                    self.task_search = Some(search);
                }
                KeyCode::Backspace => {
                    Self::delete_search_char_before_cursor(&mut search);
                    search.selected_index = 0;
                    self.task_search = Some(search);
                }
                KeyCode::Home => {
                    Self::move_search_cursor_home(&mut search);
                    self.task_search = Some(search);
                }
                KeyCode::End => {
                    Self::move_search_cursor_end(&mut search);
                    self.task_search = Some(search);
                }
                KeyCode::Char(character) => {
                    Self::insert_search_char(&mut search, character);
                    search.selected_index = 0;
                    self.task_search = Some(search);
                }
                _ => {
                    self.task_search = Some(search);
                }
            }
            return Ok(true);
        }

        if let Some(task_id) = self.delete_confirmation {
            match code {
                KeyCode::Enter | KeyCode::Char('y') => {
                    self.database.task_repository().delete(task_id)?;
                    self.delete_confirmation = None;
                    self.refresh_tasks()?;
                }
                KeyCode::Esc | KeyCode::Char('n') => {
                    self.delete_confirmation = None;
                }
                _ => {}
            }
            return Ok(true);
        }

        let Some(mut input) = self.task_input.take() else {
            return Ok(false);
        };

        match code {
            KeyCode::Esc => {}
            KeyCode::Enter => {
                let title = input.value.trim();
                if title.is_empty() {
                    self.task_input = Some(input);
                    return Ok(true);
                }

                match input.mode {
                    TaskInputMode::Create => {
                        let task = self.database.task_repository().create(title, now)?;
                        self.refresh_tasks()?;
                        self.selected_task_id = Some(task.id);
                    }
                    TaskInputMode::Rename(task_id) => {
                        self.database
                            .task_repository()
                            .update_title(task_id, title)?;
                        self.refresh_tasks()?;
                        self.selected_task_id = Some(task_id);
                    }
                }
            }
            KeyCode::Backspace => {
                Self::delete_input_char_before_cursor(&mut input);
                self.task_input = Some(input);
            }
            KeyCode::Home => {
                Self::move_input_cursor_home(&mut input);
                self.task_input = Some(input);
            }
            KeyCode::End => {
                Self::move_input_cursor_end(&mut input);
                self.task_input = Some(input);
            }
            KeyCode::Char(character) => {
                Self::insert_input_char(&mut input, character);
                self.task_input = Some(input);
            }
            _ => {
                self.task_input = Some(input);
            }
        }

        Ok(true)
    }

    fn move_search_cursor_home(search: &mut TaskSearchState) {
        search.cursor = 0;
    }

    fn move_search_cursor_end(search: &mut TaskSearchState) {
        search.cursor = search.query.len();
    }

    fn insert_search_char(search: &mut TaskSearchState, character: char) {
        search.query.insert(search.cursor, character);
        search.cursor += character.len_utf8();
    }

    fn delete_search_char_before_cursor(search: &mut TaskSearchState) {
        if search.cursor == 0 {
            return;
        }

        let previous_index = search.query[..search.cursor]
            .char_indices()
            .last()
            .map(|(index, _)| index)
            .unwrap_or(0);
        search.query.drain(previous_index..search.cursor);
        search.cursor = previous_index;
    }

    pub fn handle_key(&mut self, code: KeyCode) -> Result<()> {
        self.handle_key_at(code, Local::now())
    }

    fn handle_key_at(&mut self, code: KeyCode, now: DateTime<Local>) -> Result<()> {
        // `&mut self` is exclusive access: while this method runs, no other
        // code can also mutate the app state. This prevents a whole class of
        // aliasing bugs that are easy to create in C.
        if self.handle_task_overlay_key(code, now)? {
            return Ok(());
        }

        if self.help_open {
            match code {
                KeyCode::Esc | KeyCode::Char('?') => {
                    self.help_open = false;
                    self.help_scroll = 0;
                }
                KeyCode::Char('j') | KeyCode::Down => {
                    self.help_scroll = (self.help_scroll + 1).min(self.max_help_scroll());
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    self.help_scroll = self.help_scroll.saturating_sub(1);
                }
                KeyCode::PageDown => {
                    self.help_scroll = (self.help_scroll + self.help_viewport_lines.max(1))
                        .min(self.max_help_scroll());
                }
                KeyCode::PageUp => {
                    self.help_scroll = self
                        .help_scroll
                        .saturating_sub(self.help_viewport_lines.max(1));
                }
                KeyCode::Home => {
                    self.help_scroll = 0;
                }
                KeyCode::End => {
                    self.help_scroll = self.max_help_scroll();
                }
                _ => {}
            }
            return Ok(());
        }

        match code {
            KeyCode::Char('?') => {
                self.help_open = true;
                self.help_scroll = 0;
            }
            KeyCode::Char('c') => {
                self.open_create_task_popup();
            }
            KeyCode::Char('q') => {
                if self.timer.run_state == TimerRunState::Running {
                    self.record_voided_entry(now)?;
                    self.refresh_history()?;
                }
                self.should_quit = true;
            }
            KeyCode::Char(key) if PanelFocus::from_shortcut(key).is_some() => {
                self.focused_panel =
                    PanelFocus::from_shortcut(key).expect("focus shortcut checked");
            }
            KeyCode::Tab => {
                self.focused_panel = self.focused_panel.next();
            }
            KeyCode::BackTab => {
                self.focused_panel = self.focused_panel.previous();
            }
            KeyCode::Char('l') | KeyCode::Right if self.focused_panel == PanelFocus::RightPane => {
                self.active_right_panel_tab = self.active_right_panel_tab.next();
            }
            KeyCode::Char('h') | KeyCode::Left if self.focused_panel == PanelFocus::RightPane => {
                self.active_right_panel_tab = self.active_right_panel_tab.previous();
            }
            KeyCode::Char('l') | KeyCode::Right if self.focused_panel == PanelFocus::History => {
                self.active_history_panel_tab = self.active_history_panel_tab.next();
            }
            KeyCode::Char('h') | KeyCode::Left if self.focused_panel == PanelFocus::History => {
                self.active_history_panel_tab = self.active_history_panel_tab.previous();
            }
            KeyCode::Char('j') | KeyCode::Down if self.focused_panel == PanelFocus::History => {
                if self.active_history_panel_tab == HistoryPanelTab::Today {
                    self.scroll_history_down();
                }
            }
            KeyCode::Char('k') | KeyCode::Up if self.focused_panel == PanelFocus::History => {
                if self.active_history_panel_tab == HistoryPanelTab::Today {
                    self.scroll_history_up();
                }
            }
            KeyCode::PageDown if self.focused_panel == PanelFocus::History => {
                if self.active_history_panel_tab == HistoryPanelTab::Today {
                    self.scroll_history_page_down();
                }
            }
            KeyCode::PageUp if self.focused_panel == PanelFocus::History => {
                if self.active_history_panel_tab == HistoryPanelTab::Today {
                    self.scroll_history_page_up();
                }
            }
            KeyCode::Home if self.focused_panel == PanelFocus::History => {
                if self.active_history_panel_tab == HistoryPanelTab::Today {
                    self.history_scroll = 0;
                }
            }
            KeyCode::End if self.focused_panel == PanelFocus::History => {
                if self.active_history_panel_tab == HistoryPanelTab::Today {
                    self.history_scroll = self.max_history_scroll();
                }
            }
            KeyCode::Char('j') | KeyCode::Down if self.focused_panel == PanelFocus::Navigation => {
                self.select_next_task_view();
            }
            KeyCode::Char('k') | KeyCode::Up if self.focused_panel == PanelFocus::Navigation => {
                self.select_previous_task_view();
            }
            KeyCode::Home if self.focused_panel == PanelFocus::Navigation => {
                self.set_active_task_view(TaskView::All);
            }
            KeyCode::End if self.focused_panel == PanelFocus::Navigation => {
                self.set_active_task_view(TaskView::Soon);
            }
            KeyCode::Char('j') | KeyCode::Down
                if self.focused_panel == PanelFocus::RightPane
                    && self.active_right_panel_tab == RightPanelTab::Tasks =>
            {
                self.move_task_selection(1);
            }
            KeyCode::Char('k') | KeyCode::Up
                if self.focused_panel == PanelFocus::RightPane
                    && self.active_right_panel_tab == RightPanelTab::Tasks =>
            {
                self.move_task_selection(-1);
            }
            KeyCode::Char('e')
                if self.focused_panel == PanelFocus::RightPane
                    && self.active_right_panel_tab == RightPanelTab::Tasks =>
            {
                self.open_rename_task_popup();
            }
            KeyCode::Char('d')
                if self.focused_panel == PanelFocus::RightPane
                    && self.active_right_panel_tab == RightPanelTab::Tasks =>
            {
                self.open_delete_confirmation();
            }
            KeyCode::Char('a')
                if self.focused_panel == PanelFocus::RightPane
                    && self.active_right_panel_tab == RightPanelTab::Tasks =>
            {
                self.toggle_selected_task_assignment();
            }
            KeyCode::Char('a') if self.focused_panel == PanelFocus::Timer => {
                self.open_timer_task_search();
            }
            KeyCode::Char('u') if self.focused_panel == PanelFocus::Timer => {
                self.clear_assigned_task();
            }
            KeyCode::Char('a') if self.focused_panel == PanelFocus::History => {
                self.open_history_task_search();
            }
            KeyCode::Char('u') if self.focused_panel == PanelFocus::History => {
                self.clear_selected_history_task()?;
            }
            KeyCode::Char('x') | KeyCode::Char(' ')
                if self.focused_panel == PanelFocus::RightPane
                    && self.active_right_panel_tab == RightPanelTab::Tasks =>
            {
                self.toggle_selected_task_status(now)?;
            }
            KeyCode::Char('s') | KeyCode::Char(' ') | KeyCode::Enter
                if self.focused_panel == PanelFocus::Timer =>
            {
                self.begin_focus_task_if_needed();
                self.timer.start_or_resume(now);
            }
            KeyCode::Char('p') if self.focused_panel == PanelFocus::Timer => {
                self.timer.pause(now);
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
                } else if matches!(
                    self.timer.phase,
                    TimerPhase::ShortBreak | TimerPhase::LongBreak
                ) {
                    self.finish_break_early(now)?;
                } else {
                    self.record_voided_entry(now)?;
                    self.timer.void_current_and_prepare_next();
                    self.refresh_history()?;
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
                let next_phase = if self.timer.completed_cycles_in_round + 1
                    == self.timer_settings.long_break_interval
                {
                    TimerPhase::LongBreak
                } else {
                    TimerPhase::ShortBreak
                };

                self.database.pomodoro_repository().create(
                    self.active_focus_task_id,
                    Some(session_kind_for_phase(next_phase)),
                    started_at,
                    now,
                    duration_to_stored_minutes(self.timer_settings.pomodoro_length),
                )?;

                self.active_focus_task_id = None;
                self.timer.move_to_phase(next_phase);
                self.timer.start_or_resume(now);
                self.refresh_history()?;
            }
            TimerPhase::ShortBreak | TimerPhase::LongBreak => {
                let break_phase = self.timer.phase;
                let started_at = self
                    .timer
                    .current_phase_started_at
                    .unwrap_or(now - break_phase.duration(&self.timer_settings));
                self.database.pomodoro_repository().record_session_entry(
                    None,
                    session_kind_for_phase(break_phase),
                    SessionOutcome::Completed,
                    None,
                    started_at,
                    now,
                    self.timer.elapsed_at(now).num_seconds().max(0) as u32,
                )?;

                let completed_long_break = self.timer.phase == TimerPhase::LongBreak;
                self.timer.complete_break();
                if completed_long_break {
                    self.timer
                        .reset_round(self.timer_settings.long_break_interval);
                } else {
                    self.timer.move_to_phase(TimerPhase::Focus);
                    self.timer.prepare_next_focus_slot();
                }
                self.refresh_history()?;
            }
        }

        Ok(())
    }

    fn refresh_history(&mut self) -> Result<()> {
        let now = Local::now();
        let (started_at, ended_at) = today_bounds(now);
        let (weekly_started_at, weekly_ended_at) = last_7_days_bounds(now);
        self.screen_data.history_entries = self
            .database
            .pomodoro_repository()
            .list_day(started_at, ended_at)?;
        self.screen_data.today_stats = self
            .database
            .pomodoro_repository()
            .stats_for_day(started_at, ended_at)?;
        self.screen_data.weekly_summaries = self
            .database
            .pomodoro_repository()
            .summarize_days(weekly_started_at, weekly_ended_at)?;
        self.screen_data.weekly_stats = self
            .database
            .pomodoro_repository()
            .stats_for_day(weekly_started_at, weekly_ended_at)?;
        self.history_scroll = self.history_scroll.min(self.max_history_scroll());
        Ok(())
    }

    fn finish_break_early(&mut self, now: DateTime<Local>) -> Result<()> {
        let break_phase = self.timer.phase;
        let started_at = self.timer.current_phase_started_at.unwrap_or(now);
        self.database.pomodoro_repository().record_session_entry(
            None,
            session_kind_for_phase(break_phase),
            SessionOutcome::Completed,
            None,
            started_at,
            now,
            self.timer.elapsed_at(now).num_seconds().max(0) as u32,
        )?;

        let completed_long_break = break_phase == TimerPhase::LongBreak;
        self.timer.complete_break();
        if completed_long_break {
            self.timer
                .reset_round(self.timer_settings.long_break_interval);
        } else {
            self.timer.move_to_phase(TimerPhase::Focus);
            self.timer.prepare_next_focus_slot();
        }
        self.refresh_history()?;
        Ok(())
    }

    fn record_voided_entry(&mut self, now: DateTime<Local>) -> Result<()> {
        let duration_seconds = self.timer.elapsed_at(now).num_seconds().max(0) as u32;
        let started_at = self.timer.current_phase_started_at.unwrap_or(now);
        self.database.pomodoro_repository().record_session_entry(
            self.active_focus_task_id,
            session_kind_for_phase(self.timer.phase),
            SessionOutcome::Voided,
            None,
            started_at,
            now,
            duration_seconds,
        )?;
        if self.timer.phase == TimerPhase::Focus {
            self.active_focus_task_id = None;
        }
        Ok(())
    }

    fn max_history_scroll(&self) -> usize {
        self.screen_data
            .history_entries
            .iter()
            .filter(|entry| entry.kind == SessionKind::Focus)
            .count()
            .saturating_sub(1)
    }

    fn scroll_history_down(&mut self) {
        self.history_scroll = (self.history_scroll + 1).min(self.max_history_scroll());
    }

    fn scroll_history_up(&mut self) {
        self.history_scroll = self.history_scroll.saturating_sub(1);
    }

    fn scroll_history_page_down(&mut self) {
        self.history_scroll = (self.history_scroll + 5).min(self.max_history_scroll());
    }

    fn scroll_history_page_up(&mut self) {
        self.history_scroll = self.history_scroll.saturating_sub(5);
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
    let theme = ThemePalette::load(&paths, &config.ui.theme)?;

    info!("starting triginta");

    let database = Database::open(&paths.db_path)?;
    let now = Local::now();
    let (started_at, ended_at) = today_bounds(now);
    let (weekly_started_at, weekly_ended_at) = last_7_days_bounds(now);
    let screen_data = ScreenData {
        tasks: database.task_repository().list_all()?,
        history_entries: database
            .pomodoro_repository()
            .list_day(started_at, ended_at)?,
        today_stats: database
            .pomodoro_repository()
            .stats_for_day(started_at, ended_at)?,
        weekly_summaries: database
            .pomodoro_repository()
            .summarize_days(weekly_started_at, weekly_ended_at)?,
        weekly_stats: database
            .pomodoro_repository()
            .stats_for_day(weekly_started_at, weekly_ended_at)?,
    };

    let provider = DisabledTodoistProvider;
    info!(
        provider = provider.provider_name(),
        configured = provider.is_configured(),
        "integration boundary initialized"
    );

    let mut app = App::new(
        screen_data,
        config.ui.glyph_mode,
        theme,
        config.timer,
        database,
    );
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
        app.sync_help_viewport(terminal.size()?.height);
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

fn session_kind_for_phase(phase: TimerPhase) -> SessionKind {
    match phase {
        TimerPhase::Focus => SessionKind::Focus,
        TimerPhase::ShortBreak => SessionKind::ShortBreak,
        TimerPhase::LongBreak => SessionKind::LongBreak,
    }
}

fn today_bounds(now: DateTime<Local>) -> (DateTime<Local>, DateTime<Local>) {
    let date = now.date_naive();
    let start = date
        .and_hms_opt(0, 0, 0)
        .expect("midnight should be valid")
        .and_local_timezone(Local)
        .single()
        .expect("local midnight should be representable");
    let end = (date + chrono::Days::new(1))
        .and_hms_opt(0, 0, 0)
        .expect("midnight should be valid")
        .and_local_timezone(Local)
        .single()
        .expect("local midnight should be representable");
    (start, end)
}

fn last_7_days_bounds(now: DateTime<Local>) -> (DateTime<Local>, DateTime<Local>) {
    let (_, today_end) = today_bounds(now);
    let start_date = now.date_naive() - chrono::Days::new(6);
    let start = start_date
        .and_hms_opt(0, 0, 0)
        .expect("midnight should be valid")
        .and_local_timezone(Local)
        .single()
        .expect("local midnight should be representable");
    (start, today_end)
}

fn fuzzy_matches(query: &str, candidate: &str) -> bool {
    if query.is_empty() {
        return true;
    }

    let mut query_chars = query.chars().flat_map(char::to_lowercase);
    let mut current = query_chars.next();
    if current.is_none() {
        return true;
    }

    for candidate_char in candidate.chars().flat_map(char::to_lowercase) {
        if Some(candidate_char) == current {
            current = query_chars.next();
            if current.is_none() {
                return true;
            }
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use chrono::{Duration as ChronoDuration, Local};

    use crate::config::{AppConfig, GlyphMode, TimerSettings};
    use crate::domain::TaskStatus;
    use crate::storage::Database;
    use crate::theme::ThemePalette;

    use super::{
        App, CycleEntryState, HistoryPanelTab, PanelFocus, RightPanelTab, RunOptions, ScreenData,
        TaskView, TimerPhase, TimerRunState, apply_debug_overrides, chrono_duration,
        duration_to_stored_minutes,
    };

    fn test_app() -> App {
        App::new(
            ScreenData::default(),
            GlyphMode::NerdFonts,
            ThemePalette::load(
                &crate::config::AppPaths::from_data_dir(std::env::temp_dir())
                    .expect("paths should resolve"),
                "catppuccin-mocha",
            )
            .expect("built-in theme should load"),
            TimerSettings::default(),
            Database::open_in_memory().expect("in-memory database should open"),
        )
    }

    #[test]
    fn app_starts_running() {
        let app = test_app();
        assert!(!app.should_quit());
        assert!(!app.is_help_open());
        assert_eq!(app.active_right_panel_tab(), RightPanelTab::Tasks);
        assert_eq!(app.active_history_panel_tab(), HistoryPanelTab::Today);
        assert_eq!(app.active_task_view(), TaskView::All);
        assert_eq!(app.glyph_mode(), GlyphMode::NerdFonts);
        assert_eq!(
            app.theme(),
            ThemePalette::load(
                &crate::config::AppPaths::from_data_dir(std::env::temp_dir())
                    .expect("paths should resolve"),
                "catppuccin-mocha",
            )
            .expect("built-in theme should load")
        );
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
    fn app_switches_history_panel_tabs() {
        let mut app = test_app();
        app.handle_key(crossterm::event::KeyCode::Char('2'))
            .expect("focus should switch");

        app.handle_key(crossterm::event::KeyCode::Right)
            .expect("history tab should switch");
        assert_eq!(app.active_history_panel_tab(), HistoryPanelTab::Last7Days);

        app.handle_key(crossterm::event::KeyCode::Left)
            .expect("history tab should switch back");
        assert_eq!(app.active_history_panel_tab(), HistoryPanelTab::Today);
    }

    #[test]
    fn app_switches_task_views_from_navigation_panel() {
        let mut app = test_app();
        app.handle_key(crossterm::event::KeyCode::Char('3'))
            .expect("focus should switch");

        app.handle_key(crossterm::event::KeyCode::Down)
            .expect("task view should switch");
        assert_eq!(app.active_task_view(), TaskView::Inbox);

        app.handle_key(crossterm::event::KeyCode::End)
            .expect("task view should jump");
        assert_eq!(app.active_task_view(), TaskView::Soon);
    }

    #[test]
    fn app_creates_and_renames_task_through_popup_flow() {
        let mut app = test_app();
        app.handle_key(crossterm::event::KeyCode::Char('5'))
            .expect("focus should switch");
        app.handle_key(crossterm::event::KeyCode::Char('c'))
            .expect("popup should open");

        for character in "Write tests".chars() {
            app.handle_key(crossterm::event::KeyCode::Char(character))
                .expect("typing should succeed");
        }
        app.handle_key(crossterm::event::KeyCode::Enter)
            .expect("task should be created");

        assert_eq!(app.screen_data.tasks.len(), 1);
        assert_eq!(
            app.selected_task().expect("task should be selected").title,
            "Write tests"
        );

        app.handle_key(crossterm::event::KeyCode::Char('e'))
            .expect("rename popup should open");
        for _ in 0.."Write tests".len() {
            app.handle_key(crossterm::event::KeyCode::Backspace)
                .expect("backspace should work");
        }
        for character in "Ship tests".chars() {
            app.handle_key(crossterm::event::KeyCode::Char(character))
                .expect("typing should succeed");
        }
        app.handle_key(crossterm::event::KeyCode::Enter)
            .expect("rename should submit");

        assert_eq!(app.screen_data.tasks[0].title, "Ship tests");
    }

    #[test]
    fn input_popup_home_and_end_edit_at_cursor_position() {
        let mut app = test_app();
        app.handle_key(crossterm::event::KeyCode::Char('c'))
            .expect("popup should open");
        for character in "World".chars() {
            app.handle_key(crossterm::event::KeyCode::Char(character))
                .expect("typing should succeed");
        }
        app.handle_key(crossterm::event::KeyCode::Home)
            .expect("home should move cursor");
        for character in "Hello ".chars() {
            app.handle_key(crossterm::event::KeyCode::Char(character))
                .expect("typing should insert at start");
        }
        app.handle_key(crossterm::event::KeyCode::End)
            .expect("end should move cursor");
        app.handle_key(crossterm::event::KeyCode::Char('!'))
            .expect("typing should insert at end");
        app.handle_key(crossterm::event::KeyCode::Enter)
            .expect("submit should succeed");

        assert_eq!(app.screen_data.tasks[0].title, "Hello World!");
    }

    #[test]
    fn app_requires_delete_confirmation() {
        let mut app = test_app();
        app.handle_key(crossterm::event::KeyCode::Char('5'))
            .expect("focus should switch");
        app.handle_key(crossterm::event::KeyCode::Char('c'))
            .expect("popup should open");
        for character in "Clean inbox".chars() {
            app.handle_key(crossterm::event::KeyCode::Char(character))
                .expect("typing should succeed");
        }
        app.handle_key(crossterm::event::KeyCode::Enter)
            .expect("task should be created");

        app.handle_key(crossterm::event::KeyCode::Char('d'))
            .expect("delete dialog should open");
        app.handle_key(crossterm::event::KeyCode::Char('n'))
            .expect("delete should cancel");
        assert_eq!(app.screen_data.tasks.len(), 1);

        app.handle_key(crossterm::event::KeyCode::Char('d'))
            .expect("delete dialog should open");
        app.handle_key(crossterm::event::KeyCode::Enter)
            .expect("delete should confirm");
        assert_eq!(app.screen_data.tasks.len(), 1);
        assert!(app.screen_data.tasks[0].deleted_at.is_some());
        assert!(app.visible_tasks().is_empty());
    }

    #[test]
    fn deleted_task_stays_available_for_history_resolution() {
        let mut app = test_app();
        let now = Local::now();
        app.handle_key(crossterm::event::KeyCode::Char('5'))
            .expect("focus should switch");
        app.handle_key(crossterm::event::KeyCode::Char('c'))
            .expect("popup should open");
        for character in "Historical task".chars() {
            app.handle_key(crossterm::event::KeyCode::Char(character))
                .expect("typing should succeed");
        }
        app.handle_key(crossterm::event::KeyCode::Enter)
            .expect("task should be created");
        let task_id = app.selected_task().expect("task should be selected").id;
        app.handle_key(crossterm::event::KeyCode::Char('a'))
            .expect("assignment should toggle on");
        app.handle_key(crossterm::event::KeyCode::Char('1'))
            .expect("focus should switch");
        app.handle_key_at(crossterm::event::KeyCode::Char('s'), now)
            .expect("timer should start");
        app.handle_key_at(
            crossterm::event::KeyCode::Char('x'),
            now + ChronoDuration::seconds(5),
        )
        .expect("focus should void");

        app.handle_key(crossterm::event::KeyCode::Char('5'))
            .expect("focus should switch");
        app.handle_key(crossterm::event::KeyCode::Char('d'))
            .expect("delete dialog should open");
        app.handle_key(crossterm::event::KeyCode::Enter)
            .expect("delete should confirm");

        assert!(app.visible_tasks().is_empty());
        assert!(app.assigned_task().is_none());
        assert_eq!(app.screen_data.history_entries[0].task_id, Some(task_id));
        assert_eq!(app.screen_data.tasks[0].title, "Historical task");
        assert!(app.screen_data.tasks[0].deleted_at.is_some());
    }

    #[test]
    fn app_toggles_selected_task_status() {
        let mut app = test_app();
        let now = Local::now();
        app.handle_key(crossterm::event::KeyCode::Char('5'))
            .expect("focus should switch");
        app.handle_key(crossterm::event::KeyCode::Char('c'))
            .expect("popup should open");
        for character in "Review task flow".chars() {
            app.handle_key(crossterm::event::KeyCode::Char(character))
                .expect("typing should succeed");
        }
        app.handle_key(crossterm::event::KeyCode::Enter)
            .expect("task should be created");

        app.handle_key_at(crossterm::event::KeyCode::Char(' '), now)
            .expect("status should toggle");
        assert_eq!(app.screen_data.tasks[0].status, TaskStatus::Done);

        app.handle_key_at(crossterm::event::KeyCode::Char('x'), now)
            .expect("status should toggle");
        assert_eq!(app.screen_data.tasks[0].status, TaskStatus::Todo);
    }

    #[test]
    fn app_toggles_selected_task_as_pomodoro_assignment() {
        let mut app = test_app();
        app.handle_key(crossterm::event::KeyCode::Char('5'))
            .expect("focus should switch");
        app.handle_key(crossterm::event::KeyCode::Char('c'))
            .expect("popup should open");
        for character in "Link me".chars() {
            app.handle_key(crossterm::event::KeyCode::Char(character))
                .expect("typing should succeed");
        }
        app.handle_key(crossterm::event::KeyCode::Enter)
            .expect("task should be created");

        app.handle_key(crossterm::event::KeyCode::Char('a'))
            .expect("assignment should toggle on");
        assert_eq!(
            app.assigned_task().expect("task should be assigned").title,
            "Link me"
        );

        app.handle_key(crossterm::event::KeyCode::Char('a'))
            .expect("assignment should toggle off");
        assert!(app.assigned_task().is_none());
    }

    #[test]
    fn assigned_task_is_recorded_with_focus_session() {
        let mut app = test_app();
        let now = Local::now();
        app.handle_key(crossterm::event::KeyCode::Char('5'))
            .expect("focus should switch");
        app.handle_key(crossterm::event::KeyCode::Char('c'))
            .expect("popup should open");
        for character in "Session task".chars() {
            app.handle_key(crossterm::event::KeyCode::Char(character))
                .expect("typing should succeed");
        }
        app.handle_key(crossterm::event::KeyCode::Enter)
            .expect("task should be created");
        let task_id = app.selected_task().expect("task should be selected").id;

        app.handle_key(crossterm::event::KeyCode::Char('a'))
            .expect("assignment should toggle on");
        app.handle_key(crossterm::event::KeyCode::Char('1'))
            .expect("focus should switch");
        app.handle_key_at(crossterm::event::KeyCode::Char('s'), now)
            .expect("timer should start");
        app.handle_key_at(
            crossterm::event::KeyCode::Char('x'),
            now + ChronoDuration::seconds(5),
        )
        .expect("focus should void");

        assert_eq!(app.screen_data.history_entries.len(), 1);
        assert_eq!(app.screen_data.history_entries[0].task_id, Some(task_id));
    }

    #[test]
    fn timer_panel_can_assign_and_clear_task_via_search_popup() {
        let mut app = test_app();
        app.handle_key(crossterm::event::KeyCode::Char('c'))
            .expect("popup should open");
        for character in "Alpha task".chars() {
            app.handle_key(crossterm::event::KeyCode::Char(character))
                .expect("typing should succeed");
        }
        app.handle_key(crossterm::event::KeyCode::Enter)
            .expect("task should be created");
        app.handle_key(crossterm::event::KeyCode::Char('c'))
            .expect("popup should open");
        for character in "Beta item".chars() {
            app.handle_key(crossterm::event::KeyCode::Char(character))
                .expect("typing should succeed");
        }
        app.handle_key(crossterm::event::KeyCode::Enter)
            .expect("task should be created");

        app.handle_key(crossterm::event::KeyCode::Char('1'))
            .expect("focus should switch");
        app.handle_key(crossterm::event::KeyCode::Char('a'))
            .expect("search should open");
        for character in "bt".chars() {
            app.handle_key(crossterm::event::KeyCode::Char(character))
                .expect("typing should filter");
        }
        app.handle_key(crossterm::event::KeyCode::Enter)
            .expect("selected search result should assign");

        assert_eq!(
            app.assigned_task().expect("task should be assigned").title,
            "Beta item"
        );

        app.handle_key(crossterm::event::KeyCode::Char('u'))
            .expect("clear should succeed");
        assert!(app.assigned_task().is_none());
    }

    #[test]
    fn history_panel_can_assign_and_clear_selected_session_task() {
        let mut app = test_app();
        let now = Local::now();
        app.handle_key(crossterm::event::KeyCode::Char('c'))
            .expect("popup should open");
        for character in "Alpha task".chars() {
            app.handle_key(crossterm::event::KeyCode::Char(character))
                .expect("typing should succeed");
        }
        app.handle_key(crossterm::event::KeyCode::Enter)
            .expect("task should be created");
        let task_id = app.selected_task().expect("task should exist").id;

        app.handle_key(crossterm::event::KeyCode::Char('1'))
            .expect("focus should switch");
        app.handle_key(crossterm::event::KeyCode::Char('a'))
            .expect("timer search should open");
        app.handle_key(crossterm::event::KeyCode::Enter)
            .expect("assignment should succeed");
        app.handle_key_at(crossterm::event::KeyCode::Char('s'), now)
            .expect("timer should start");
        app.handle_key_at(
            crossterm::event::KeyCode::Char('x'),
            now + ChronoDuration::seconds(5),
        )
        .expect("focus should void");

        assert_eq!(app.screen_data.history_entries[0].task_id, Some(task_id));

        app.handle_key(crossterm::event::KeyCode::Char('2'))
            .expect("focus should switch");
        app.handle_key(crossterm::event::KeyCode::Char('u'))
            .expect("clear should succeed");
        assert_eq!(app.screen_data.history_entries[0].task_id, None);

        app.handle_key(crossterm::event::KeyCode::Char('a'))
            .expect("search should open");
        app.handle_key(crossterm::event::KeyCode::Enter)
            .expect("reassignment should succeed");
        assert_eq!(app.screen_data.history_entries[0].task_id, Some(task_id));
    }

    #[test]
    fn app_marks_quit_on_q() {
        let mut app = test_app();
        app.handle_key(crossterm::event::KeyCode::Char('q'))
            .expect("quit should succeed");
        assert!(app.should_quit());
    }

    #[test]
    fn app_toggles_help_dialog_with_question_mark_and_escape() {
        let mut app = test_app();

        app.handle_key(crossterm::event::KeyCode::Char('?'))
            .expect("help should open");
        assert!(app.is_help_open());

        app.handle_key(crossterm::event::KeyCode::Esc)
            .expect("help should close");
        assert!(!app.is_help_open());

        app.handle_key(crossterm::event::KeyCode::Char('?'))
            .expect("help should open again");
        app.handle_key(crossterm::event::KeyCode::Char('?'))
            .expect("help should close with question mark");
        assert!(!app.is_help_open());
    }

    #[test]
    fn help_dialog_supports_scrolling_and_resets_on_close() {
        let mut app = test_app();
        app.handle_key(crossterm::event::KeyCode::Char('?'))
            .expect("help should open");
        app.sync_help_viewport(12);

        app.handle_key(crossterm::event::KeyCode::Down)
            .expect("help should scroll down");
        assert_eq!(app.help_scroll(), 1);

        app.handle_key(crossterm::event::KeyCode::PageDown)
            .expect("help should page down");
        assert_eq!(app.help_scroll(), 7);

        app.handle_key(crossterm::event::KeyCode::End)
            .expect("help should jump to end");
        assert_eq!(app.help_scroll(), app.max_help_scroll());

        app.handle_key(crossterm::event::KeyCode::Up)
            .expect("help should scroll up");
        assert_eq!(app.help_scroll(), app.max_help_scroll().saturating_sub(1));

        app.handle_key(crossterm::event::KeyCode::Home)
            .expect("help should jump to top");
        assert_eq!(app.help_scroll(), 0);

        app.handle_key(crossterm::event::KeyCode::Esc)
            .expect("help should close");
        assert!(!app.is_help_open());
        assert_eq!(app.help_scroll(), 0);
    }

    #[test]
    fn quitting_while_timer_is_running_voids_current_session() {
        let mut app = test_app();
        let now = Local::now();

        app.handle_key_at(crossterm::event::KeyCode::Char('s'), now)
            .expect("timer should start");
        app.handle_key_at(
            crossterm::event::KeyCode::Char('q'),
            now + ChronoDuration::seconds(12),
        )
        .expect("quit should succeed");

        assert!(app.should_quit());
        assert_eq!(app.screen_data.today_stats.total_sessions, 1);
        assert_eq!(app.screen_data.history_entries.len(), 1);
        assert_eq!(
            app.screen_data.history_entries[0].outcome,
            crate::domain::SessionOutcome::Voided
        );
        assert_eq!(app.screen_data.history_entries[0].duration_seconds, 12);
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
        assert_eq!(app.screen_data.today_stats.total_sessions, 1);
        assert_eq!(
            app.screen_data.today_stats.total_minutes,
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
    fn voiding_during_break_completes_cycle_without_adding_extra_slot() {
        let mut app = test_app();
        let focus_started_at = Local::now() - chrono_duration(app.timer_settings.pomodoro_length);
        app.timer.phase = TimerPhase::Focus;
        app.timer.run_state = TimerRunState::Running;
        app.timer.current_phase_started_at = Some(focus_started_at);
        app.timer.running_since = Some(focus_started_at);
        app.on_tick_at(Local::now())
            .expect("tick should complete focus");

        let break_now = Local::now();
        let break_started_at = break_now - ChronoDuration::seconds(10);
        app.timer.phase = TimerPhase::ShortBreak;
        app.timer.run_state = TimerRunState::Running;
        app.timer.current_phase_started_at = Some(break_started_at);
        app.timer.running_since = Some(break_started_at);

        app.handle_key_at(crossterm::event::KeyCode::Char('x'), break_now)
            .expect("ending break early should succeed");

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
        assert_eq!(app.screen_data.today_stats.total_sessions, 1);
        assert_eq!(app.screen_data.today_stats.total_break_seconds, 10);
        assert_eq!(app.screen_data.history_entries.len(), 2);
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
