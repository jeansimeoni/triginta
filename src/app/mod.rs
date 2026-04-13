use std::time::Duration;

use anyhow::{Context, Result};
use chrono::{DateTime, Datelike, Duration as ChronoDuration, Local, NaiveDate, NaiveDateTime};
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, prelude::CrosstermBackend};
use tracing::info;

use crate::{
    config::{
        AppConfig, AppPaths, GlyphMode, TaskSortOrder, TimerSettings, init_tracing,
        load_app_config, save_app_config,
    },
    domain::{
        DayHistorySummary, HistoryStats, Project, ProjectColor, ProjectId, ProjectUpdate,
        SessionEntry, SessionKind, SessionOutcome, Task, TaskId, TaskStatus, TaskUpdate,
    },
    integrations::{DisabledTodoistProvider, TaskSyncProvider},
    storage::{Database, PomodoroRepository, ProjectRepository, TaskRepository},
    task_nlp::{next_recurring_due, parse_due_input, parse_due_time_input, parse_task_input},
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
pub enum SidebarTab {
    Navigation,
    FiltersTags,
    Projects,
}

impl SidebarTab {
    pub fn label(self) -> &'static str {
        match self {
            Self::Navigation => "Navigation",
            Self::FiltersTags => "Filters & Tags",
            Self::Projects => "Projects",
        }
    }
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
            '6' => Some(Self::Favorites),
            '7' => Some(Self::RightPane),
            '3' | '4' | '5' => Some(Self::Navigation),
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
    pub projects: Vec<Project>,
    pub history_entries: Vec<SessionEntry>,
    pub today_stats: HistoryStats,
    pub weekly_summaries: Vec<DayHistorySummary>,
    pub weekly_stats: HistoryStats,
}

#[derive(Debug, Clone)]
struct TaskInputState {
    value: String,
    cursor: usize,
    project_id: ProjectId,
    suggestion_index: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TaskEditorField {
    Title,
    Project,
    DueDate,
    DueTime,
    Recurrence,
}

impl TaskEditorField {
    const ALL: [Self; 5] = [
        Self::Title,
        Self::Project,
        Self::DueDate,
        Self::DueTime,
        Self::Recurrence,
    ];

    fn index(self) -> usize {
        match self {
            Self::Title => 0,
            Self::Project => 1,
            Self::DueDate => 2,
            Self::DueTime => 3,
            Self::Recurrence => 4,
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

#[derive(Debug, Clone)]
struct TaskEditorState {
    task_id: TaskId,
    title_input: String,
    title_cursor: usize,
    project_input: String,
    project_cursor: usize,
    project_id: ProjectId,
    suggestion_index: usize,
    due_date_input: String,
    due_date_cursor: usize,
    due_time_input: String,
    due_time_cursor: usize,
    recurrence_input: String,
    recurrence_cursor: usize,
    due_natural: String,
    due_from_title: bool,
    focused_field: TaskEditorField,
    calendar: Option<CalendarState>,
}

#[derive(Debug, Clone)]
struct CalendarState {
    display_date: NaiveDate,
    selected_date: NaiveDate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProjectEditorField {
    Name,
    Parent,
    Color,
    Favorite,
}

impl ProjectEditorField {
    const ALL: [Self; 4] = [Self::Name, Self::Parent, Self::Color, Self::Favorite];

    fn index(self) -> usize {
        match self {
            Self::Name => 0,
            Self::Parent => 1,
            Self::Color => 2,
            Self::Favorite => 3,
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

#[derive(Debug, Clone)]
struct TaskSearchState {
    mode: TaskSearchMode,
    query: String,
    cursor: usize,
    selected_index: usize,
}

#[derive(Debug, Clone)]
struct ProjectEditorState {
    project_id: Option<ProjectId>,
    name_input: String,
    name_cursor: usize,
    parent_input: String,
    parent_cursor: usize,
    color_index: usize,
    is_favorite: bool,
    suggestion_index: usize,
    focused_field: ProjectEditorField,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TaskSortPopupState {
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
    pub project_name: String,
    pub show_project_assignment: bool,
    pub project_suggestions: Vec<String>,
    pub selected_project_suggestion: usize,
    pub due_preview: Option<TaskDuePreviewView>,
    pub preview_panel: FormPreviewPanelView,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CalendarPickerView {
    pub display_date: NaiveDate,
    pub selected_date: NaiveDate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TaskEditorFocusView {
    pub title: bool,
    pub project: bool,
    pub due_date: bool,
    pub due_time: bool,
    pub recurrence: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskEditorView {
    pub title: &'static str,
    pub title_value: String,
    pub title_cursor: usize,
    pub project_value: String,
    pub project_cursor: usize,
    pub project_suggestions: Vec<String>,
    pub selected_project_suggestion: usize,
    pub due_date_value: String,
    pub due_date_cursor: usize,
    pub due_time_value: String,
    pub due_time_cursor: usize,
    pub recurrence_value: String,
    pub recurrence_cursor: usize,
    pub focus: TaskEditorFocusView,
    pub due_preview: Option<TaskDuePreviewView>,
    pub calendar: Option<CalendarPickerView>,
    pub preview_panel: FormPreviewPanelView,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskDuePreviewView {
    pub date: NaiveDate,
    pub datetime: Option<NaiveDateTime>,
    pub string: String,
    pub is_recurring: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreviewLineView {
    KeyValue {
        label: String,
        value: String,
        emphasized: bool,
        dimmed: bool,
    },
    Text {
        text: String,
        dimmed: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormPreviewPanelView {
    pub preview_lines: Vec<PreviewLineView>,
    pub tips: Vec<String>,
}

impl PreviewLineView {
    fn key_value(label: &str, value: impl Into<String>) -> Self {
        Self::KeyValue {
            label: label.to_string(),
            value: value.into(),
            emphasized: false,
            dimmed: false,
        }
    }

    fn emphasized_key_value(label: &str, value: impl Into<String>) -> Self {
        Self::KeyValue {
            label: label.to_string(),
            value: value.into(),
            emphasized: true,
            dimmed: false,
        }
    }

    fn dimmed_key_value(label: &str, value: impl Into<String>) -> Self {
        Self::KeyValue {
            label: label.to_string(),
            value: value.into(),
            emphasized: false,
            dimmed: true,
        }
    }

    fn text(text: impl Into<String>) -> Self {
        Self::Text {
            text: text.into(),
            dimmed: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedTaskDraft {
    cleaned_title: String,
    due: Option<crate::domain::TaskDue>,
    project_id: ProjectId,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectTreeRowView {
    pub project_id: Option<ProjectId>,
    pub name: String,
    pub depth: usize,
    pub tree_prefix: String,
    pub is_favorite: bool,
    pub color: Option<ProjectColor>,
    pub task_count: usize,
    pub is_selected: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectEditorFocusView {
    pub name: bool,
    pub parent: bool,
    pub color: bool,
    pub favorite: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectEditorView {
    pub title: &'static str,
    pub name_value: String,
    pub name_cursor: usize,
    pub parent_value: String,
    pub parent_cursor: usize,
    pub parsed_name: String,
    pub parent_label: Option<String>,
    pub parent_suggestions: Vec<String>,
    pub selected_parent_suggestion: usize,
    pub color_label: String,
    pub color_value: ProjectColor,
    pub is_favorite: bool,
    pub focus: ProjectEditorFocusView,
    pub preview_panel: FormPreviewPanelView,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectDeleteConfirmationView {
    pub project_id: ProjectId,
    pub project_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskSortOptionView {
    pub label: &'static str,
    pub is_active: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskSortPopupView {
    pub title: &'static str,
    pub selected_index: usize,
    pub options: Vec<TaskSortOptionView>,
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
        keys: "1-7",
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
        keys: "PgUp/PgDn",
        description: "page",
    },
    ShortcutTip {
        keys: "Home/End",
        description: "jump first/last",
    },
];

const FILTERS_TAGS_SHORTCUTS: &[ShortcutTip] = &[ShortcutTip {
    keys: "3/4/5",
    description: "switch tab",
}];

const PROJECTS_SHORTCUTS: &[ShortcutTip] = &[
    ShortcutTip {
        keys: "j/k or ↑/↓",
        description: "move project",
    },
    ShortcutTip {
        keys: "PgUp/PgDn",
        description: "page",
    },
    ShortcutTip {
        keys: "Home",
        description: "all projects",
    },
    ShortcutTip {
        keys: "C/e/d",
        description: "new/edit/delete project",
    },
    ShortcutTip {
        keys: "f",
        description: "toggle favorite",
    },
    ShortcutTip {
        keys: "c",
        description: "new task here",
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
        description: "edit/delete",
    },
    ShortcutTip {
        keys: "a",
        description: "assign to timer",
    },
    ShortcutTip {
        keys: "Space",
        description: "toggle done",
    },
    ShortcutTip {
        keys: "o/f",
        description: "sort/filter",
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

const EDITOR_POPUP_SHORTCUTS: &[ShortcutTip] = &[
    ShortcutTip {
        keys: "Enter",
        description: "submit",
    },
    ShortcutTip {
        keys: "Esc",
        description: "cancel/close calendar",
    },
    ShortcutTip {
        keys: "Tab/S-Tab",
        description: "next/prev field",
    },
    ShortcutTip {
        keys: "F1-F5",
        description: "jump to field",
    },
    ShortcutTip {
        keys: "j/k or ↑/↓",
        description: "project suggestions",
    },
    ShortcutTip {
        keys: "F6",
        description: "open calendar",
    },
    ShortcutTip {
        keys: "F7",
        description: "clear due",
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

const PROJECT_EDITOR_SHORTCUTS: &[ShortcutTip] = &[
    ShortcutTip {
        keys: "Enter",
        description: "save",
    },
    ShortcutTip {
        keys: "Esc",
        description: "cancel",
    },
    ShortcutTip {
        keys: "Tab/S-Tab",
        description: "next/prev field",
    },
    ShortcutTip {
        keys: "F1-F4",
        description: "jump to field",
    },
    ShortcutTip {
        keys: "h/l or j/k",
        description: "change value",
    },
];

const PROJECT_DELETE_CONFIRMATION_SHORTCUTS: &[ShortcutTip] = &[
    ShortcutTip {
        keys: "Enter/y",
        description: "confirm",
    },
    ShortcutTip {
        keys: "Esc/n",
        description: "cancel",
    },
];

const SORT_POPUP_SHORTCUTS: &[ShortcutTip] = &[
    ShortcutTip {
        keys: "Enter",
        description: "apply sort",
    },
    ShortcutTip {
        keys: "Esc",
        description: "cancel",
    },
    ShortcutTip {
        keys: "j/k or ↑/↓",
        description: "move option",
    },
];

// `App` owns the mutable runtime state for the TUI loop.
// Compared with a C program, this is the central state struct you would pass
// around to input/render functions, but here methods are attached directly to
// the type.
#[derive(Debug)]
pub struct App {
    database: Database,
    config: AppConfig,
    config_paths: Option<AppPaths>,
    timer_settings: TimerSettings,
    active_right_panel_tab: RightPanelTab,
    active_history_panel_tab: HistoryPanelTab,
    active_sidebar_tab: SidebarTab,
    active_task_view: TaskView,
    focused_panel: PanelFocus,
    glyph_mode: GlyphMode,
    theme: ThemePalette,
    timer: TimerState,
    history_scroll: usize,
    selected_task_id: Option<TaskId>,
    selected_project_id: Option<ProjectId>,
    assigned_task_id: Option<TaskId>,
    active_focus_task_id: Option<TaskId>,
    task_input: Option<TaskInputState>,
    task_editor: Option<TaskEditorState>,
    project_editor: Option<ProjectEditorState>,
    task_search: Option<TaskSearchState>,
    task_sort_popup: Option<TaskSortPopupState>,
    delete_confirmation: Option<TaskId>,
    project_delete_confirmation: Option<ProjectId>,
    help_open: bool,
    help_scroll: usize,
    help_viewport_lines: usize,
    should_quit: bool,
    screen_data: ScreenData,
}

impl App {
    fn task_input_parse(&self, raw: &str, fallback_project_id: ProjectId) -> ParsedTaskDraft {
        let (without_project_tokens, project_id) =
            self.extract_project_reference(raw, fallback_project_id);
        let parsed = parse_task_input(without_project_tokens.as_str(), self.today());
        ParsedTaskDraft {
            cleaned_title: parsed.cleaned_title,
            due: parsed.due,
            project_id,
        }
    }

    pub fn new(
        screen_data: ScreenData,
        config: AppConfig,
        config_paths: Option<AppPaths>,
        theme: ThemePalette,
        database: Database,
    ) -> Self {
        let glyph_mode = config.ui.glyph_mode;
        let timer_settings = config.timer.clone();
        let long_break_interval = timer_settings.long_break_interval;
        let mut app = Self {
            database,
            config,
            config_paths,
            timer_settings,
            active_right_panel_tab: RightPanelTab::Tasks,
            active_history_panel_tab: HistoryPanelTab::Today,
            active_sidebar_tab: SidebarTab::Navigation,
            active_task_view: TaskView::All,
            focused_panel: PanelFocus::Timer,
            glyph_mode,
            theme,
            timer: TimerState::new(long_break_interval),
            history_scroll: 0,
            selected_task_id: None,
            selected_project_id: None,
            assigned_task_id: None,
            active_focus_task_id: None,
            task_input: None,
            task_editor: None,
            project_editor: None,
            task_search: None,
            task_sort_popup: None,
            delete_confirmation: None,
            project_delete_confirmation: None,
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

    pub fn active_sidebar_tab(&self) -> SidebarTab {
        self.active_sidebar_tab
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
        let mut tasks = self
            .screen_data
            .tasks
            .iter()
            .filter(|task| self.task_is_visible(task))
            .collect::<Vec<_>>();
        tasks.sort_by(|left, right| self.compare_tasks(left, right));
        tasks
    }

    pub fn selected_task(&self) -> Option<&Task> {
        self.selected_task_id.and_then(|task_id| {
            self.screen_data
                .tasks
                .iter()
                .find(|task| task.id == task_id && self.task_is_visible(task))
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

    pub fn task_details_task(&self) -> Option<&Task> {
        match self.focused_panel {
            PanelFocus::RightPane if self.active_right_panel_tab == RightPanelTab::Tasks => {
                self.selected_task()
            }
            PanelFocus::Timer => self.assigned_task(),
            PanelFocus::History => self.selected_history_task(),
            _ => None,
        }
    }

    pub fn task_input_view(&self) -> Option<TaskInputView> {
        self.task_input.as_ref().map(|input| {
            let project_suggestions = self
                .active_project_query(input.value.as_str(), input.cursor)
                .map(|(_, _, query)| {
                    self.project_suggestions(query.as_str())
                        .into_iter()
                        .take(4)
                        .map(|project| project.name.clone())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let due_preview = self
                .task_input_parse(input.value.as_str(), input.project_id)
                .due
                .map(|due| TaskDuePreviewView {
                    date: due.date,
                    datetime: due.datetime,
                    string: due.string,
                    is_recurring: due.is_recurring,
                });
            let project_name = self
                .project_name(input.project_id)
                .unwrap_or("Inbox")
                .to_string();
            let show_project_assignment = input.project_id != self.inbox_project_id();

            TaskInputView {
                title: "New Task",
                value: input.value.clone(),
                cursor: input.cursor,
                project_name: project_name.clone(),
                show_project_assignment,
                project_suggestions: project_suggestions.clone(),
                selected_project_suggestion: input
                    .suggestion_index
                    .min(project_suggestions.len().saturating_sub(1)),
                due_preview: due_preview.clone(),
                preview_panel: Self::task_input_preview_panel(
                    show_project_assignment,
                    project_name.as_str(),
                    due_preview.as_ref(),
                ),
            }
        })
    }

    pub fn task_editor_view(&self) -> Option<TaskEditorView> {
        self.task_editor.as_ref().map(|editor| {
            let current_project_name = self.project_name(editor.project_id).unwrap_or("Inbox");
            let show_project_suggestions = editor.focused_field == TaskEditorField::Project
                && !editor.project_input.trim().is_empty()
                && !editor
                    .project_input
                    .trim()
                    .eq_ignore_ascii_case(current_project_name);
            let project_suggestions = if show_project_suggestions {
                self.project_suggestions(editor.project_input.as_str())
                    .into_iter()
                    .take(4)
                    .map(|project| project.name.clone())
                    .collect::<Vec<_>>()
            } else {
                Vec::new()
            };
            let focus = TaskEditorFocusView {
                title: editor.focused_field == TaskEditorField::Title,
                project: editor.focused_field == TaskEditorField::Project,
                due_date: editor.focused_field == TaskEditorField::DueDate,
                due_time: editor.focused_field == TaskEditorField::DueTime,
                recurrence: editor.focused_field == TaskEditorField::Recurrence,
            };
            let due_preview = self.editor_due_preview(editor);
            TaskEditorView {
                title: "Edit Task",
                title_value: editor.title_input.clone(),
                title_cursor: editor.title_cursor,
                project_value: editor.project_input.clone(),
                project_cursor: editor.project_cursor,
                project_suggestions: project_suggestions.clone(),
                selected_project_suggestion: editor
                    .suggestion_index
                    .min(project_suggestions.len().saturating_sub(1)),
                due_date_value: editor.due_date_input.clone(),
                due_date_cursor: editor.due_date_cursor,
                due_time_value: editor.due_time_input.clone(),
                due_time_cursor: editor.due_time_cursor,
                recurrence_value: editor.recurrence_input.clone(),
                recurrence_cursor: editor.recurrence_cursor,
                focus,
                due_preview: due_preview.clone(),
                calendar: editor.calendar.as_ref().map(|calendar| CalendarPickerView {
                    display_date: calendar.display_date,
                    selected_date: calendar.selected_date,
                }),
                preview_panel: Self::task_editor_preview_panel(
                    editor.project_input.as_str(),
                    due_preview.as_ref(),
                    editor.focused_field,
                ),
            }
        })
    }

    pub fn project_editor_view(&self) -> Option<ProjectEditorView> {
        let editor = self.project_editor.as_ref()?;
        let inline_parent_query = self
            .active_project_query(editor.name_input.as_str(), editor.name_cursor)
            .map(|(_, _, query)| query);
        let direct_parent_query = self.active_parent_field_query(editor.parent_input.as_str());
        let (parsed_name, extracted_parent_id) =
            self.extract_project_reference(editor.name_input.as_str(), ProjectId(0));
        let inline_parent_id = if extracted_parent_id == ProjectId(0) {
            None
        } else {
            Some(extracted_parent_id)
        };
        let direct_parent_id =
            self.resolve_project_parent_input(editor.parent_input.as_str(), editor.project_id);
        let parent_label = direct_parent_id
            .or(inline_parent_id)
            .and_then(|project_id| self.project_name(project_id).map(str::to_string));
        let parent_suggestions = match editor.focused_field {
            ProjectEditorField::Parent => {
                let should_show = direct_parent_query
                    .map(|query| {
                        let resolved = self.resolve_project_parent_input(query, editor.project_id);
                        resolved
                            .and_then(|project_id| self.project_name(project_id))
                            .map(|name| !query.eq_ignore_ascii_case(name))
                            .unwrap_or(true)
                    })
                    .unwrap_or(false);
                if should_show {
                    direct_parent_query
                        .map(|query| {
                            self.project_parent_suggestions(query, editor.project_id)
                                .into_iter()
                                .take(4)
                                .map(|project| project.name.clone())
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default()
                } else {
                    Vec::new()
                }
            }
            ProjectEditorField::Name => inline_parent_query
                .map(|query| {
                    self.project_parent_suggestions(query.as_str(), editor.project_id)
                        .into_iter()
                        .take(4)
                        .map(|project| project.name.clone())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default(),
            _ => Vec::new(),
        };
        let color = ProjectColor::all()
            .get(editor.color_index)
            .copied()
            .unwrap_or(ProjectColor::Charcoal);

        Some(ProjectEditorView {
            title: if editor.project_id.is_some() {
                "Edit Project"
            } else {
                "New Project"
            },
            name_value: editor.name_input.clone(),
            name_cursor: editor.name_cursor,
            parent_value: editor.parent_input.clone(),
            parent_cursor: editor.parent_cursor,
            parsed_name,
            parent_label: parent_label.clone(),
            parent_suggestions: parent_suggestions.clone(),
            selected_parent_suggestion: editor.suggestion_index,
            color_label: color.label().to_string(),
            color_value: color,
            is_favorite: editor.is_favorite,
            focus: ProjectEditorFocusView {
                name: editor.focused_field == ProjectEditorField::Name,
                parent: editor.focused_field == ProjectEditorField::Parent,
                color: editor.focused_field == ProjectEditorField::Color,
                favorite: editor.focused_field == ProjectEditorField::Favorite,
            },
            preview_panel: Self::project_editor_preview_panel(
                parent_label.as_deref(),
                editor.focused_field,
            ),
        })
    }

    fn task_input_preview_panel(
        show_project_assignment: bool,
        project_name: &str,
        due_preview: Option<&TaskDuePreviewView>,
    ) -> FormPreviewPanelView {
        let mut preview_lines = Vec::new();
        if show_project_assignment {
            preview_lines.push(PreviewLineView::key_value("Project", project_name));
        }
        if let Some(due) = due_preview {
            preview_lines.push(PreviewLineView::key_value(
                "Due Date",
                due.date.format("%Y-%m-%d").to_string(),
            ));
            if let Some(datetime) = due.datetime {
                preview_lines.push(PreviewLineView::key_value(
                    "Due Time",
                    datetime.format("%H:%M").to_string(),
                ));
            }
            preview_lines.push(PreviewLineView::key_value(
                "Recurring",
                if due.is_recurring { "yes" } else { "no" },
            ));
            let normalized = due
                .datetime
                .map(|datetime| {
                    format!(
                        "{} {}",
                        due.date.format("%Y-%m-%d"),
                        datetime.format("%H:%M")
                    )
                })
                .unwrap_or_else(|| due.date.format("%Y-%m-%d").to_string());
            if due.string.to_ascii_lowercase() != normalized {
                preview_lines.push(PreviewLineView::dimmed_key_value(
                    "From",
                    due.string.as_str(),
                ));
            }
        }

        FormPreviewPanelView {
            preview_lines,
            tips: vec!["Press # for selecting a project".to_string()],
        }
    }

    fn task_editor_preview_panel(
        project_value: &str,
        due_preview: Option<&TaskDuePreviewView>,
        focused_field: TaskEditorField,
    ) -> FormPreviewPanelView {
        let mut preview_lines = vec![PreviewLineView::key_value("Project", project_value)];
        if let Some(due) = due_preview {
            preview_lines.push(PreviewLineView::emphasized_key_value(
                "Summary",
                due.string.as_str(),
            ));
            preview_lines.push(PreviewLineView::key_value(
                "Date",
                due.date.format("%Y-%m-%d").to_string(),
            ));
            let time_value = due
                .datetime
                .map(|datetime| datetime.format("%H:%M").to_string())
                .unwrap_or_else(|| "-".to_string());
            preview_lines.push(PreviewLineView::key_value("Time", time_value));
            preview_lines.push(PreviewLineView::key_value(
                "Recurring",
                if due.is_recurring { "yes" } else { "no" },
            ));
        } else {
            preview_lines.push(PreviewLineView::key_value("Summary", "no due date"));
        }

        let tips = match focused_field {
            TaskEditorField::Title => vec!["Press # for selecting a project".to_string()],
            TaskEditorField::Project => {
                vec!["Type in Project to fuzzy-match and use Enter/Tab to accept".to_string()]
            }
            TaskEditorField::DueDate => {
                vec!["Type YYYY-MM-DD or use F6 to pick from calendar".to_string()]
            }
            TaskEditorField::DueTime => {
                vec!["Type HH:MM (24h) or leave empty for all-day due date".to_string()]
            }
            TaskEditorField::Recurrence => {
                vec!["Type recurrence phrases like: every monday at 9am".to_string()]
            }
        };

        FormPreviewPanelView {
            preview_lines,
            tips,
        }
    }

    fn project_editor_preview_panel(
        parent_label: Option<&str>,
        focused_field: ProjectEditorField,
    ) -> FormPreviewPanelView {
        let mut preview_lines = Vec::new();
        if let Some(parent_label) = parent_label {
            preview_lines.push(PreviewLineView::text(format!("Parent: {parent_label}")));
        }
        let tips = match focused_field {
            ProjectEditorField::Name => vec!["Press # for selecting a parent project".to_string()],
            ProjectEditorField::Parent => {
                vec!["Type to fuzzy-match a parent project and use Enter/Tab to accept".to_string()]
            }
            ProjectEditorField::Color => vec!["Use ←/→ or h/l to change the color".to_string()],
            ProjectEditorField::Favorite => vec!["Use ←/→ or h/l to toggle favorite".to_string()],
        };

        FormPreviewPanelView {
            preview_lines,
            tips,
        }
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

    pub fn project_delete_confirmation_view(&self) -> Option<ProjectDeleteConfirmationView> {
        let project_id = self.project_delete_confirmation?;
        let project = self
            .screen_data
            .projects
            .iter()
            .find(|project| project.id == project_id)?;
        Some(ProjectDeleteConfirmationView {
            project_id,
            project_name: project.name.clone(),
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

    pub fn task_sort_popup_view(&self) -> Option<TaskSortPopupView> {
        let popup = self.task_sort_popup?;
        Some(TaskSortPopupView {
            title: "Sort Tasks",
            selected_index: popup.selected_index,
            options: TaskSortOrder::all()
                .iter()
                .map(|sort_order| TaskSortOptionView {
                    label: sort_order.label(),
                    is_active: *sort_order == self.config.ui.task_list_sort,
                })
                .collect(),
        })
    }

    pub fn task_sort_order(&self) -> TaskSortOrder {
        self.config.ui.task_list_sort
    }

    pub fn hides_completed_tasks(&self) -> bool {
        self.config.ui.hide_completed_tasks
    }

    pub fn screen_data(&self) -> &ScreenData {
        // Returning `&ScreenData` lends read-only access to the caller.
        // No copy is made, and the borrow checker ensures the reference cannot
        // outlive `self`.
        &self.screen_data
    }

    pub fn project_tree_rows(&self) -> Vec<ProjectTreeRowView> {
        let mut rows = vec![ProjectTreeRowView {
            project_id: None,
            name: "All Projects".to_string(),
            depth: 0,
            tree_prefix: String::new(),
            is_favorite: false,
            color: None,
            task_count: self.tasks_for_project_filter(None),
            is_selected: self.selected_project_id.is_none(),
        }];
        self.append_project_tree_rows(&mut rows, None, 0, &[]);
        rows
    }

    pub fn task_count_for_view(&self, view: TaskView) -> usize {
        self.screen_data
            .tasks
            .iter()
            .filter(|task| self.task_is_counted_for_view(task, view))
            .count()
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
        if self.task_sort_popup.is_some() {
            return SORT_POPUP_SHORTCUTS;
        }
        if self.task_editor.is_some() {
            return EDITOR_POPUP_SHORTCUTS;
        }
        if self.project_editor.is_some() {
            return PROJECT_EDITOR_SHORTCUTS;
        }
        if self.task_input.is_some() {
            return INPUT_POPUP_SHORTCUTS;
        }
        if self.project_delete_confirmation.is_some() {
            return PROJECT_DELETE_CONFIRMATION_SHORTCUTS;
        }
        match self.focused_panel {
            PanelFocus::Timer => TIMER_SHORTCUTS,
            PanelFocus::History => HISTORY_SHORTCUTS,
            PanelFocus::Navigation => match self.active_sidebar_tab {
                SidebarTab::Navigation => NAVIGATION_SHORTCUTS,
                SidebarTab::FiltersTags => FILTERS_TAGS_SHORTCUTS,
                SidebarTab::Projects => PROJECTS_SHORTCUTS,
            },
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
                title: "Filters & Tags",
                tips: FILTERS_TAGS_SHORTCUTS,
            },
            ShortcutSection {
                title: "Projects",
                tips: PROJECTS_SHORTCUTS,
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
        if self.task_editor.is_some() {
            sections.push(ShortcutSection {
                title: "Task Editor",
                tips: EDITOR_POPUP_SHORTCUTS,
            });
        }
        if self.project_editor.is_some() {
            sections.push(ShortcutSection {
                title: "Project Editor",
                tips: PROJECT_EDITOR_SHORTCUTS,
            });
        }
        if self.task_search.is_some() {
            sections.push(ShortcutSection {
                title: "Task Search Popup",
                tips: SEARCH_POPUP_SHORTCUTS,
            });
        }
        if self.task_sort_popup.is_some() {
            sections.push(ShortcutSection {
                title: "Task Sort Popup",
                tips: SORT_POPUP_SHORTCUTS,
            });
        }
        if self.delete_confirmation.is_some() {
            sections.push(ShortcutSection {
                title: "Delete Confirmation",
                tips: DELETE_CONFIRMATION_SHORTCUTS,
            });
        }
        if self.project_delete_confirmation.is_some() {
            sections.push(ShortcutSection {
                title: "Project Delete Confirmation",
                tips: PROJECT_DELETE_CONFIRMATION_SHORTCUTS,
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

    fn today(&self) -> NaiveDate {
        Local::now().date_naive()
    }

    fn editor_due_preview(&self, editor: &TaskEditorState) -> Option<TaskDuePreviewView> {
        Self::build_due_from_editor(editor, self.today())
            .ok()
            .flatten()
            .map(|due| TaskDuePreviewView {
                date: due.date,
                datetime: due.datetime,
                string: due.string,
                is_recurring: due.is_recurring,
            })
    }

    fn build_due_from_editor(
        editor: &TaskEditorState,
        reference_date: NaiveDate,
    ) -> Result<Option<crate::domain::TaskDue>> {
        let date_text = editor.due_date_input.trim();
        let time_text = editor.due_time_input.trim();
        let recurrence_text = editor.recurrence_input.trim();

        if date_text.is_empty() && time_text.is_empty() && recurrence_text.is_empty() {
            return Ok(None);
        }

        let recurring_due = if recurrence_text.is_empty() {
            None
        } else {
            let due = parse_due_input(recurrence_text, reference_date)
                .filter(|due| due.is_recurring)
                .context("recurring pattern must use a Todoist-style recurring phrase")?;
            Some(due)
        };

        let date = if date_text.is_empty() {
            recurring_due
                .as_ref()
                .map(|due| due.date)
                .context("due date is required when a due time is set")?
        } else {
            NaiveDate::parse_from_str(date_text, "%Y-%m-%d")
                .ok()
                .or_else(|| parse_due_input(date_text, reference_date).map(|due| due.date))
                .with_context(|| format!("invalid due date: {date_text}"))?
        };

        let datetime = if time_text.is_empty() {
            if date_text.is_empty() {
                recurring_due.as_ref().and_then(|due| due.datetime)
            } else {
                None
            }
        } else {
            let time = chrono::NaiveTime::parse_from_str(time_text, "%H:%M")
                .ok()
                .or_else(|| parse_due_time_input(time_text))
                .with_context(|| format!("invalid due time: {time_text}"))?;
            Some(NaiveDateTime::new(date, time))
        };

        let string = if !editor.due_natural.trim().is_empty() {
            editor.due_natural.trim().to_string()
        } else if !recurrence_text.is_empty() {
            recurrence_text.to_string()
        } else if let Some(datetime) = datetime {
            format!(
                "{} at {}",
                date.format("%Y-%m-%d"),
                datetime.format("%H:%M")
            )
        } else {
            date.format("%Y-%m-%d").to_string()
        };

        Ok(Some(crate::domain::TaskDue {
            date,
            datetime,
            string,
            is_recurring: recurring_due.is_some(),
        }))
    }

    fn focus_editor_field(editor: &mut TaskEditorState, field: TaskEditorField) {
        editor.focused_field = field;
    }

    fn open_editor_calendar(editor: &mut TaskEditorState, reference_date: NaiveDate) {
        let selected_date = NaiveDate::parse_from_str(editor.due_date_input.trim(), "%Y-%m-%d")
            .ok()
            .or_else(|| {
                Self::build_due_from_editor(editor, reference_date)
                    .ok()
                    .flatten()
                    .map(|due| due.date)
            })
            .unwrap_or(reference_date);
        let display_date = selected_date.with_day(1).unwrap_or(selected_date);
        editor.calendar = Some(CalendarState {
            display_date,
            selected_date,
        });
    }

    fn task_matches_active_view(&self, task: &Task) -> bool {
        self.task_matches_view(task, self.active_task_view)
    }

    fn task_is_active(&self, task: &Task) -> bool {
        task.deleted_at.is_none()
            && self
                .project_by_id(task.project_id)
                .map(|project| project.deleted_at.is_none())
                .unwrap_or(false)
    }

    fn task_is_visible(&self, task: &Task) -> bool {
        self.task_is_active(task)
            && self.task_matches_active_view(task)
            && self.task_matches_selected_project(task)
            && (!self.config.ui.hide_completed_tasks || task.status != TaskStatus::Done)
    }

    fn task_matches_selected_project(&self, task: &Task) -> bool {
        self.selected_project_id
            .map(|project_id| self.project_is_in_subtree(task.project_id, project_id))
            .unwrap_or(true)
    }

    fn task_matches_view(&self, task: &Task, view: TaskView) -> bool {
        match view {
            TaskView::All => true,
            TaskView::Inbox => self
                .project_by_id(task.project_id)
                .map(|project| project.is_inbox)
                .unwrap_or(false),
            TaskView::Today => task.due.as_ref().map(|due| due.date) == Some(self.today()),
            TaskView::Soon => task
                .due
                .as_ref()
                .map(|due| due.date > self.today())
                .unwrap_or(false),
        }
    }

    fn task_is_counted_for_view(&self, task: &Task, view: TaskView) -> bool {
        self.task_is_active(task)
            && self.task_matches_view(task, view)
            && (!self.config.ui.hide_completed_tasks || task.status != TaskStatus::Done)
    }

    fn tasks_for_project_filter(&self, project_id: Option<ProjectId>) -> usize {
        self.screen_data
            .tasks
            .iter()
            .filter(|task| {
                self.task_is_active(task)
                    && self.task_matches_view(task, self.active_task_view)
                    && project_id
                        .map(|selected| self.project_is_in_subtree(task.project_id, selected))
                        .unwrap_or(true)
                    && (!self.config.ui.hide_completed_tasks || task.status != TaskStatus::Done)
            })
            .count()
    }

    fn project_by_id(&self, project_id: ProjectId) -> Option<&Project> {
        self.screen_data
            .projects
            .iter()
            .find(|project| project.id == project_id)
    }

    fn project_name(&self, project_id: ProjectId) -> Option<&str> {
        self.project_by_id(project_id)
            .map(|project| project.name.as_str())
    }

    fn resolve_project_input(
        &self,
        query: &str,
        fallback_project_id: Option<ProjectId>,
    ) -> ProjectId {
        let normalized = query.trim().to_lowercase();
        if normalized.is_empty() {
            return fallback_project_id.unwrap_or_else(|| self.inbox_project_id());
        }

        self.screen_data
            .projects
            .iter()
            .filter(|project| project.deleted_at.is_none())
            .find(|project| project.name.to_lowercase() == normalized)
            .map(|project| project.id)
            .or_else(|| {
                self.screen_data
                    .projects
                    .iter()
                    .filter(|project| project.deleted_at.is_none())
                    .find(|project| fuzzy_matches(normalized.as_str(), project.name.as_str()))
                    .map(|project| project.id)
            })
            .unwrap_or_else(|| fallback_project_id.unwrap_or_else(|| self.inbox_project_id()))
    }

    fn matched_project_prefix(
        &self,
        query: &str,
        excluded_project_id: Option<ProjectId>,
    ) -> Option<(&Project, usize)> {
        let normalized_query = query.trim_start();
        self.screen_data
            .projects
            .iter()
            .filter(|project| {
                project.deleted_at.is_none() && Some(project.id) != excluded_project_id
            })
            .filter_map(|project| {
                let name = project.name.as_str();
                let query_prefix = normalized_query.get(..name.len())?;
                if !query_prefix.eq_ignore_ascii_case(name) {
                    return None;
                }
                let next = normalized_query[name.len()..].chars().next();
                if next.is_some_and(|character| !character.is_whitespace()) {
                    return None;
                }
                Some((project, name.len()))
            })
            .max_by_key(|(_, length)| *length)
    }

    fn extract_project_reference(
        &self,
        raw: &str,
        fallback_project_id: ProjectId,
    ) -> (String, ProjectId) {
        let Some(start) = raw.rfind('#') else {
            return (raw.trim().to_string(), fallback_project_id);
        };
        if start > 0 && !raw[..start].chars().last().is_some_and(char::is_whitespace) {
            return (raw.trim().to_string(), fallback_project_id);
        }
        let query = raw[start + 1..].trim();
        if query.is_empty() {
            return (raw.trim().to_string(), fallback_project_id);
        }
        if let Some((project, matched_length)) = self.matched_project_prefix(query, None) {
            let remainder = query[matched_length..].trim_start();
            let cleaned = if raw[..start].trim().is_empty() {
                remainder.to_string()
            } else if remainder.is_empty() {
                raw[..start].trim_end().to_string()
            } else {
                format!("{} {}", raw[..start].trim_end(), remainder)
            };
            return (cleaned.trim().to_string(), project.id);
        }
        let project_id = self.resolve_project_input(query, Some(fallback_project_id));
        let cleaned = raw[..start].trim_end().to_string();
        (cleaned, project_id)
    }

    fn inbox_project_id(&self) -> ProjectId {
        self.screen_data
            .projects
            .iter()
            .find(|project| project.is_inbox)
            .map(|project| project.id)
            .expect("inbox project should exist")
    }

    fn project_is_in_subtree(&self, project_id: ProjectId, root_project_id: ProjectId) -> bool {
        let mut current = Some(project_id);
        while let Some(candidate) = current {
            if candidate == root_project_id {
                return true;
            }
            current = self
                .project_by_id(candidate)
                .and_then(|project| project.parent_project_id);
        }
        false
    }

    fn project_children(&self, parent_project_id: Option<ProjectId>) -> Vec<&Project> {
        let mut projects = self
            .screen_data
            .projects
            .iter()
            .filter(|project| {
                project.deleted_at.is_none()
                    && project.parent_project_id == parent_project_id
                    && !project.is_inbox
            })
            .collect::<Vec<_>>();
        projects.sort_by(|left, right| {
            left.child_order
                .cmp(&right.child_order)
                .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
                .then_with(|| left.id.0.cmp(&right.id.0))
        });
        projects
    }

    fn append_project_tree_rows(
        &self,
        rows: &mut Vec<ProjectTreeRowView>,
        parent_project_id: Option<ProjectId>,
        depth: usize,
        ancestor_has_more: &[bool],
    ) {
        let mut children = self.project_children(parent_project_id);
        if parent_project_id.is_none() {
            if let Some(inbox) = self
                .screen_data
                .projects
                .iter()
                .find(|project| project.is_inbox && project.deleted_at.is_none())
            {
                children.insert(0, inbox);
            }
        }

        let total_children = children.len();
        for (index, project) in children.into_iter().enumerate() {
            let is_last = index + 1 == total_children;
            let tree_prefix = if depth == 0 {
                String::new()
            } else {
                let mut prefix = String::new();
                for has_more in ancestor_has_more {
                    prefix.push_str(if *has_more { "│ " } else { "  " });
                }
                prefix.push_str(if is_last { "└ " } else { "├ " });
                prefix
            };
            rows.push(ProjectTreeRowView {
                project_id: Some(project.id),
                name: project.name.clone(),
                depth,
                tree_prefix,
                is_favorite: project.is_favorite,
                color: Some(project.color),
                task_count: self.tasks_for_project_filter(Some(project.id)),
                is_selected: self.selected_project_id == Some(project.id),
            });
            let next_ancestor = if parent_project_id.is_none() {
                Vec::new()
            } else {
                let mut value = ancestor_has_more.to_vec();
                value.push(!is_last);
                value
            };
            self.append_project_tree_rows(rows, Some(project.id), depth + 1, &next_ancestor);
        }
    }

    fn project_suggestions(&self, query: &str) -> Vec<&Project> {
        let normalized = query.trim();
        if normalized.is_empty() {
            return Vec::new();
        }

        let mut matches = self
            .screen_data
            .projects
            .iter()
            .filter(|project| project.deleted_at.is_none())
            .filter(|project| fuzzy_matches(normalized, project.name.as_str()))
            .collect::<Vec<_>>();
        matches.sort_by(|left, right| {
            let left_name = left.name.to_lowercase();
            let right_name = right.name.to_lowercase();
            left_name
                .starts_with(&normalized.to_lowercase())
                .cmp(&right_name.starts_with(&normalized.to_lowercase()))
                .reverse()
                .then_with(|| left_name.cmp(&right_name))
        });
        matches
    }

    fn active_project_query(&self, value: &str, cursor: usize) -> Option<(usize, usize, String)> {
        let safe_cursor = cursor.min(value.len());
        let before_cursor = &value[..safe_cursor];
        let token_start = before_cursor.rfind('#')?;
        if token_start > 0
            && !value[..token_start]
                .chars()
                .last()
                .is_some_and(char::is_whitespace)
        {
            return None;
        }
        let query = value[token_start + 1..safe_cursor].trim_start();
        if query.is_empty() || query.contains('\n') {
            return None;
        }
        if let Some((_, matched_length)) = self.matched_project_prefix(query, None) {
            let remainder = query[matched_length..].trim_start();
            if remainder.is_empty() {
                return None;
            }
        }
        Some((token_start, safe_cursor, query.to_string()))
    }

    fn accept_task_input_project_suggestion(&self, input: &mut TaskInputState) -> bool {
        let Some((start, end, query)) =
            self.active_project_query(input.value.as_str(), input.cursor)
        else {
            return false;
        };
        let suggestions = self.project_suggestions(query.as_str());
        let Some(project) = suggestions
            .get(
                input
                    .suggestion_index
                    .min(suggestions.len().saturating_sub(1)),
            )
            .copied()
        else {
            return false;
        };

        input.project_id = project.id;
        input
            .value
            .replace_range(start..end, format!("#{} ", project.name).as_str());
        input.cursor = (start + project.name.len() + 2).min(input.value.len());
        input.suggestion_index = 0;
        while input.cursor < input.value.len() && input.value[input.cursor..].starts_with(' ') {
            input.value.remove(input.cursor);
        }
        true
    }

    fn accept_project_editor_parent_suggestion(&self, editor: &mut ProjectEditorState) -> bool {
        let Some((start, end, query)) =
            self.active_project_query(editor.name_input.as_str(), editor.name_cursor)
        else {
            return false;
        };
        let suggestions = self.project_parent_suggestions(query.as_str(), editor.project_id);
        let Some(project) = suggestions
            .get(
                editor
                    .suggestion_index
                    .min(suggestions.len().saturating_sub(1)),
            )
            .copied()
        else {
            return false;
        };

        editor
            .name_input
            .replace_range(start..end, format!("#{} ", project.name).as_str());
        editor.name_cursor = (start + project.name.len() + 2).min(editor.name_input.len());
        editor.suggestion_index = 0;
        true
    }

    fn accept_project_editor_parent_field_suggestion(
        &self,
        editor: &mut ProjectEditorState,
    ) -> bool {
        let Some(query) = self.active_parent_field_query(editor.parent_input.as_str()) else {
            return false;
        };
        let suggestions = self.project_parent_suggestions(query, editor.project_id);
        let Some(project) = suggestions
            .get(
                editor
                    .suggestion_index
                    .min(suggestions.len().saturating_sub(1)),
            )
            .copied()
        else {
            return false;
        };
        if editor
            .parent_input
            .trim()
            .eq_ignore_ascii_case(project.name.as_str())
        {
            return false;
        }

        editor.parent_input = project.name.clone();
        editor.parent_cursor = editor.parent_input.len();
        editor.suggestion_index = 0;
        true
    }

    fn compare_tasks(&self, left: &Task, right: &Task) -> std::cmp::Ordering {
        match self.config.ui.task_list_sort {
            TaskSortOrder::DueAsc => self
                .compare_task_due(left, right, false)
                .then_with(|| self.compare_task_title(left, right))
                .then_with(|| right.created_at.cmp(&left.created_at))
                .then_with(|| right.id.0.cmp(&left.id.0)),
            TaskSortOrder::DueDesc => self
                .compare_task_due(left, right, true)
                .then_with(|| self.compare_task_title(left, right))
                .then_with(|| right.created_at.cmp(&left.created_at))
                .then_with(|| right.id.0.cmp(&left.id.0)),
            TaskSortOrder::TitleAsc => self
                .compare_task_title(left, right)
                .then_with(|| right.created_at.cmp(&left.created_at))
                .then_with(|| right.id.0.cmp(&left.id.0)),
            TaskSortOrder::TitleDesc => self
                .compare_task_title(right, left)
                .then_with(|| right.created_at.cmp(&left.created_at))
                .then_with(|| right.id.0.cmp(&left.id.0)),
            TaskSortOrder::CreatedNewest => right
                .created_at
                .cmp(&left.created_at)
                .then_with(|| self.compare_task_title(left, right))
                .then_with(|| right.id.0.cmp(&left.id.0)),
            TaskSortOrder::CreatedOldest => left
                .created_at
                .cmp(&right.created_at)
                .then_with(|| self.compare_task_title(left, right))
                .then_with(|| left.id.0.cmp(&right.id.0)),
        }
    }

    fn compare_task_due(&self, left: &Task, right: &Task, descending: bool) -> std::cmp::Ordering {
        match (&left.due, &right.due) {
            (Some(left_due), Some(right_due)) => {
                let left_key = (left_due.date, left_due.datetime);
                let right_key = (right_due.date, right_due.datetime);
                if descending {
                    right_key.cmp(&left_key)
                } else {
                    left_key.cmp(&right_key)
                }
            }
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        }
    }

    fn compare_task_title(&self, left: &Task, right: &Task) -> std::cmp::Ordering {
        left.title
            .to_lowercase()
            .cmp(&right.title.to_lowercase())
            .then_with(|| left.title.cmp(&right.title))
    }

    fn has_existing_recurring_successor(
        &self,
        original_task_id: TaskId,
        title: &str,
        due: &crate::domain::TaskDue,
    ) -> bool {
        self.screen_data.tasks.iter().any(|task| {
            task.id != original_task_id
                && task.deleted_at.is_none()
                && task.title == title
                && task.due.as_ref() == Some(due)
        })
    }

    fn visible_task_ids(&self) -> Vec<TaskId> {
        self.visible_tasks()
            .into_iter()
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

    fn sync_project_selection(&mut self) {
        if let Some(project_id) = self.selected_project_id {
            if self
                .screen_data
                .projects
                .iter()
                .any(|project| project.id == project_id && project.deleted_at.is_none())
            {
                return;
            }
            self.selected_project_id = None;
        }
    }

    fn refresh_tasks(&mut self) -> Result<()> {
        self.screen_data.tasks = self.database.task_repository().list_all()?;
        self.screen_data.projects = self.database.project_repository().list_all()?;
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
        self.sync_project_selection();
        self.sync_task_selection();
        Ok(())
    }

    fn persist_ui_preferences(&self) -> Result<()> {
        if let Some(paths) = &self.config_paths {
            save_app_config(paths, &self.config)?;
        }
        Ok(())
    }

    fn set_active_task_view(&mut self, view: TaskView) {
        self.active_task_view = view;
        self.sync_task_selection();
    }

    fn open_task_sort_popup(&mut self) {
        let selected_index = TaskSortOrder::all()
            .iter()
            .position(|sort_order| *sort_order == self.config.ui.task_list_sort)
            .unwrap_or(0);
        self.task_sort_popup = Some(TaskSortPopupState { selected_index });
    }

    fn toggle_hide_completed_tasks(&mut self) -> Result<()> {
        self.config.ui.hide_completed_tasks = !self.config.ui.hide_completed_tasks;
        self.persist_ui_preferences()?;
        self.sync_task_selection();
        Ok(())
    }

    fn apply_task_sort_order(&mut self, sort_order: TaskSortOrder) -> Result<()> {
        self.config.ui.task_list_sort = sort_order;
        self.persist_ui_preferences()?;
        self.sync_task_selection();
        Ok(())
    }

    fn select_next_task_view(&mut self) {
        self.set_active_task_view(self.active_task_view.next());
    }

    fn select_previous_task_view(&mut self) {
        self.set_active_task_view(self.active_task_view.previous());
    }

    fn move_task_view_selection(&mut self, offset: isize) {
        let all = TaskView::all();
        if all.is_empty() {
            return;
        }
        let current_index = all
            .iter()
            .position(|view| *view == self.active_task_view)
            .unwrap_or(0);
        let next_index = (current_index as isize + offset)
            .clamp(0, all.len().saturating_sub(1) as isize) as usize;
        self.set_active_task_view(all[next_index]);
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
            value: String::new(),
            cursor: 0,
            project_id: self
                .selected_project_id
                .unwrap_or_else(|| self.inbox_project_id()),
            suggestion_index: 0,
        });
    }

    fn open_edit_task_popup(&mut self) {
        let Some(task) = self.selected_task().cloned() else {
            return;
        };

        let (due_date_input, due_time_input, recurrence_input, due_natural) =
            if let Some(due) = &task.due {
                (
                    due.date.format("%Y-%m-%d").to_string(),
                    due.datetime
                        .map(|datetime| datetime.format("%H:%M").to_string())
                        .unwrap_or_default(),
                    if due.is_recurring {
                        due.string.clone()
                    } else {
                        String::new()
                    },
                    due.string.clone(),
                )
            } else {
                (String::new(), String::new(), String::new(), String::new())
            };

        let due_date_cursor = due_date_input.len();
        let due_time_cursor = due_time_input.len();
        let recurrence_cursor = recurrence_input.len();

        self.task_editor = Some(TaskEditorState {
            task_id: task.id,
            title_cursor: task.title.len(),
            title_input: task.title,
            project_input: self
                .project_name(task.project_id)
                .unwrap_or("Inbox")
                .to_string(),
            project_cursor: self.project_name(task.project_id).unwrap_or("Inbox").len(),
            project_id: task.project_id,
            suggestion_index: 0,
            due_date_input,
            due_date_cursor,
            due_time_input,
            due_time_cursor,
            recurrence_input,
            recurrence_cursor,
            due_natural,
            due_from_title: false,
            focused_field: TaskEditorField::Title,
            calendar: None,
        });
    }

    fn move_input_cursor_home(input: &mut TaskInputState) {
        input.cursor = 0;
    }

    fn move_input_cursor_left(input: &mut TaskInputState) {
        if input.cursor == 0 {
            return;
        }
        input.cursor = input.value[..input.cursor]
            .char_indices()
            .last()
            .map(|(index, _)| index)
            .unwrap_or(0);
    }

    fn move_input_cursor_right(input: &mut TaskInputState) {
        if input.cursor >= input.value.len() {
            return;
        }
        let next = input.value[input.cursor..]
            .char_indices()
            .nth(1)
            .map(|(offset, _)| input.cursor + offset)
            .unwrap_or(input.value.len());
        input.cursor = next;
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

    fn editor_value_mut(editor: &mut TaskEditorState) -> (&mut String, &mut usize) {
        match editor.focused_field {
            TaskEditorField::Title => (&mut editor.title_input, &mut editor.title_cursor),
            TaskEditorField::Project => (&mut editor.project_input, &mut editor.project_cursor),
            TaskEditorField::DueDate => (&mut editor.due_date_input, &mut editor.due_date_cursor),
            TaskEditorField::DueTime => (&mut editor.due_time_input, &mut editor.due_time_cursor),
            TaskEditorField::Recurrence => {
                (&mut editor.recurrence_input, &mut editor.recurrence_cursor)
            }
        }
    }

    fn move_editor_cursor_home(editor: &mut TaskEditorState) {
        let (_, cursor) = Self::editor_value_mut(editor);
        *cursor = 0;
    }

    fn move_editor_cursor_left(editor: &mut TaskEditorState) {
        let (value, cursor) = Self::editor_value_mut(editor);
        if *cursor == 0 {
            return;
        }
        *cursor = value[..*cursor]
            .char_indices()
            .last()
            .map(|(index, _)| index)
            .unwrap_or(0);
    }

    fn move_editor_cursor_right(editor: &mut TaskEditorState) {
        let (value, cursor) = Self::editor_value_mut(editor);
        if *cursor >= value.len() {
            return;
        }
        *cursor = value[*cursor..]
            .char_indices()
            .nth(1)
            .map(|(offset, _)| *cursor + offset)
            .unwrap_or(value.len());
    }

    fn move_editor_cursor_end(editor: &mut TaskEditorState) {
        let (value, cursor) = Self::editor_value_mut(editor);
        *cursor = value.len();
    }

    fn insert_editor_char(
        editor: &mut TaskEditorState,
        character: char,
        reference_date: NaiveDate,
    ) {
        let (value, cursor) = Self::editor_value_mut(editor);
        value.insert(*cursor, character);
        *cursor += character.len_utf8();
        Self::after_editor_text_change(editor, reference_date);
    }

    fn delete_editor_char_before_cursor(editor: &mut TaskEditorState, reference_date: NaiveDate) {
        let (value, cursor) = Self::editor_value_mut(editor);
        if *cursor == 0 {
            return;
        }

        let previous_index = value[..*cursor]
            .char_indices()
            .last()
            .map(|(index, _)| index)
            .unwrap_or(0);
        value.drain(previous_index..*cursor);
        *cursor = previous_index;
        Self::after_editor_text_change(editor, reference_date);
    }

    fn after_editor_text_change(editor: &mut TaskEditorState, reference_date: NaiveDate) {
        match editor.focused_field {
            TaskEditorField::Title => Self::sync_editor_due_from_title(editor, reference_date),
            TaskEditorField::Project => {}
            TaskEditorField::DueDate | TaskEditorField::DueTime => {
                editor.due_from_title = false;
                if !editor.recurrence_input.trim().is_empty() {
                    editor.due_natural = editor.recurrence_input.trim().to_string();
                } else {
                    editor.due_natural.clear();
                }
            }
            TaskEditorField::Recurrence => {
                editor.due_from_title = false;
                Self::sync_editor_due_from_recurrence(editor, reference_date);
            }
        }
    }

    fn sync_editor_due_from_title(editor: &mut TaskEditorState, reference_date: NaiveDate) {
        let parsed = parse_task_input(editor.title_input.as_str(), reference_date);
        if let Some(due) = parsed.due {
            editor.due_date_input = due.date.format("%Y-%m-%d").to_string();
            editor.due_time_input = due
                .datetime
                .map(|datetime| datetime.format("%H:%M").to_string())
                .unwrap_or_default();
            editor.recurrence_input = if due.is_recurring {
                due.string.clone()
            } else {
                String::new()
            };
            editor.due_natural = due.string;
            editor.due_from_title = true;
            return;
        }

        if editor.due_from_title {
            Self::clear_editor_due(editor);
        }
    }

    fn sync_editor_due_from_recurrence(editor: &mut TaskEditorState, reference_date: NaiveDate) {
        let recurrence = editor.recurrence_input.trim();
        if recurrence.is_empty() {
            editor.due_natural.clear();
            return;
        }

        let parsed = parse_task_input(format!("Placeholder {recurrence}").as_str(), reference_date);
        let Some(due) = parsed.due else {
            return;
        };
        if !due.is_recurring {
            return;
        }

        editor.due_date_input = due.date.format("%Y-%m-%d").to_string();
        editor.due_natural = recurrence.to_string();
        if let Some(datetime) = due.datetime {
            editor.due_time_input = datetime.format("%H:%M").to_string();
        } else if editor.due_time_input.trim().is_empty() {
            editor.due_time_input.clear();
        }
    }

    fn clear_editor_due(editor: &mut TaskEditorState) {
        editor.due_date_input.clear();
        editor.due_date_cursor = 0;
        editor.due_time_input.clear();
        editor.due_time_cursor = 0;
        editor.recurrence_input.clear();
        editor.recurrence_cursor = 0;
        editor.due_natural.clear();
        editor.due_from_title = false;
        editor.calendar = None;
    }

    fn shift_calendar_month(date: NaiveDate, months: i32) -> NaiveDate {
        let shifted = if months >= 0 {
            date.checked_add_months(chrono::Months::new(months as u32))
        } else {
            date.checked_sub_months(chrono::Months::new(months.unsigned_abs()))
        };
        shifted.unwrap_or(date)
    }

    fn move_calendar_selection(editor: &mut TaskEditorState, days: i64) {
        let Some(calendar) = editor.calendar.as_mut() else {
            return;
        };

        let next = calendar
            .selected_date
            .checked_add_signed(ChronoDuration::days(days))
            .unwrap_or(calendar.selected_date);
        calendar.selected_date = next;
        calendar.display_date = next.with_day(1).unwrap_or(next);
    }

    fn shift_calendar_page(editor: &mut TaskEditorState, months: i32) {
        let Some(calendar) = editor.calendar.as_mut() else {
            return;
        };

        calendar.display_date = Self::shift_calendar_month(calendar.display_date, months)
            .with_day(1)
            .unwrap_or(calendar.display_date);
        calendar.selected_date = Self::shift_calendar_month(calendar.selected_date, months);
    }

    fn apply_calendar_selection(editor: &mut TaskEditorState) {
        let Some(calendar) = editor.calendar.take() else {
            return;
        };

        editor.due_date_input = calendar.selected_date.format("%Y-%m-%d").to_string();
        editor.due_date_cursor = editor.due_date_input.len();
        editor.due_from_title = false;
        if editor.recurrence_input.trim().is_empty() {
            editor.due_natural.clear();
        }
    }

    fn submit_task_editor(
        &mut self,
        editor: TaskEditorState,
        now: DateTime<Local>,
    ) -> Result<bool> {
        let project_from_field =
            self.resolve_project_input(editor.project_input.as_str(), Some(editor.project_id));
        let parsed = self.task_input_parse(editor.title_input.as_str(), project_from_field);
        if parsed.cleaned_title.is_empty() {
            self.task_editor = Some(editor);
            return Ok(true);
        }

        let due = match Self::build_due_from_editor(&editor, now.date_naive()) {
            Ok(due) => due,
            Err(_) => {
                self.task_editor = Some(editor);
                return Ok(true);
            }
        };

        self.database.task_repository().update(
            editor.task_id,
            &TaskUpdate {
                title: parsed.cleaned_title,
                project_id: parsed.project_id,
                due,
            },
        )?;
        self.refresh_tasks()?;
        self.selected_task_id = Some(editor.task_id);
        Ok(true)
    }

    fn open_delete_confirmation(&mut self) {
        let Some(task_id) = self.selected_task_id else {
            return;
        };

        self.delete_confirmation = Some(task_id);
    }

    fn project_parent_suggestions(
        &self,
        query: &str,
        project_id: Option<ProjectId>,
    ) -> Vec<&Project> {
        let normalized = query.trim().trim_start_matches('#');
        if normalized.is_empty() {
            return Vec::new();
        }

        let mut matches = self
            .screen_data
            .projects
            .iter()
            .filter(|project| project.deleted_at.is_none() && !project.is_inbox)
            .filter(|project| Some(project.id) != project_id)
            .filter(|project| fuzzy_matches(normalized, project.name.as_str()))
            .collect::<Vec<_>>();
        let normalized_lower = normalized.to_lowercase();
        matches.sort_by(|left, right| {
            let left_lower = left.name.to_lowercase();
            let right_lower = right.name.to_lowercase();
            left_lower
                .starts_with(normalized_lower.as_str())
                .cmp(&right_lower.starts_with(normalized_lower.as_str()))
                .reverse()
                .then_with(|| left_lower.cmp(&right_lower))
        });
        matches
    }

    fn active_parent_field_query<'a>(&self, value: &'a str) -> Option<&'a str> {
        let query = value.trim();
        if query.is_empty() { None } else { Some(query) }
    }

    fn resolve_project_parent_input(
        &self,
        query: &str,
        project_id: Option<ProjectId>,
    ) -> Option<ProjectId> {
        let normalized = query.trim();
        if normalized.is_empty() {
            return None;
        }
        self.project_parent_suggestions(normalized, project_id)
            .first()
            .copied()
            .map(|project| project.id)
    }

    fn next_project_editor_field(&self, editor: &ProjectEditorState) -> ProjectEditorField {
        editor.focused_field.next()
    }

    fn previous_project_editor_field(&self, editor: &ProjectEditorState) -> ProjectEditorField {
        editor.focused_field.previous()
    }

    fn focus_project_editor_field(
        &self,
        editor: &mut ProjectEditorState,
        field: ProjectEditorField,
    ) {
        editor.focused_field = field;
        editor.suggestion_index = 0;
    }

    fn project_editor_has_parent_without_name(&self, editor: &ProjectEditorState) -> bool {
        let (clean_name, parent_project_id) =
            self.extract_project_reference(editor.name_input.as_str(), ProjectId(0));
        parent_project_id != ProjectId(0) && clean_name.trim().is_empty()
    }

    fn open_create_project_popup(&mut self) {
        let parent_input = self
            .selected_project_id
            .and_then(|selected| self.project_name(selected))
            .unwrap_or("")
            .to_string();
        self.project_editor = Some(ProjectEditorState {
            project_id: None,
            name_input: String::new(),
            name_cursor: 0,
            parent_input: parent_input.clone(),
            parent_cursor: parent_input.len(),
            color_index: ProjectColor::all()
                .iter()
                .position(|color| *color == ProjectColor::Charcoal)
                .unwrap_or(0),
            is_favorite: false,
            suggestion_index: 0,
            focused_field: ProjectEditorField::Name,
        });
    }

    fn open_edit_project_popup(&mut self) {
        let Some(project_id) = self.selected_project_id else {
            return;
        };
        let Some(project) = self.project_by_id(project_id).cloned() else {
            return;
        };
        let color_index = ProjectColor::all()
            .iter()
            .position(|color| *color == project.color)
            .unwrap_or(0);
        self.project_editor = Some(ProjectEditorState {
            project_id: Some(project.id),
            name_input: project.name.clone(),
            name_cursor: project.name.len(),
            parent_input: project
                .parent_project_id
                .and_then(|parent_id| self.project_name(parent_id))
                .unwrap_or("")
                .to_string(),
            parent_cursor: project
                .parent_project_id
                .and_then(|parent_id| self.project_name(parent_id))
                .unwrap_or("")
                .len(),
            color_index,
            is_favorite: project.is_favorite,
            suggestion_index: 0,
            focused_field: ProjectEditorField::Name,
        });
    }

    fn open_project_delete_confirmation(&mut self) {
        let Some(project_id) = self.selected_project_id else {
            return;
        };
        if self
            .project_by_id(project_id)
            .is_some_and(|project| !project.is_inbox)
        {
            self.project_delete_confirmation = Some(project_id);
        }
    }

    fn move_project_selection(&mut self, offset: isize) {
        let rows = self.project_tree_rows();
        if rows.is_empty() {
            self.selected_project_id = None;
            return;
        }
        let current_index = rows
            .iter()
            .position(|row| row.project_id == self.selected_project_id)
            .unwrap_or(0);
        let next_index = (current_index as isize + offset)
            .clamp(0, rows.len().saturating_sub(1) as isize) as usize;
        self.selected_project_id = rows[next_index].project_id;
    }

    fn toggle_selected_project_favorite(&mut self) -> Result<()> {
        let Some(project_id) = self.selected_project_id else {
            return Ok(());
        };
        let Some(project) = self.project_by_id(project_id).cloned() else {
            return Ok(());
        };
        if project.is_inbox {
            return Ok(());
        }
        self.database.project_repository().update(
            project_id,
            &ProjectUpdate {
                name: project.name,
                parent_project_id: project.parent_project_id,
                color: project.color,
                is_favorite: !project.is_favorite,
            },
        )?;
        self.refresh_tasks()?;
        Ok(())
    }

    fn submit_project_editor(
        &mut self,
        editor: ProjectEditorState,
        now: DateTime<Local>,
    ) -> Result<()> {
        let (clean_name, inline_parent_project_id) =
            self.extract_project_reference(editor.name_input.as_str(), ProjectId(0));
        let name = clean_name.trim();
        if name.is_empty() {
            self.project_editor = Some(editor);
            return Ok(());
        }
        let parent_project_id = if inline_parent_project_id != ProjectId(0) {
            Some(inline_parent_project_id)
        } else if !editor.parent_input.trim().is_empty() {
            self.resolve_project_parent_input(editor.parent_input.as_str(), editor.project_id)
        } else {
            None
        };
        let color = ProjectColor::all()
            .get(editor.color_index)
            .copied()
            .unwrap_or(ProjectColor::Charcoal);
        if let Some(project_id) = editor.project_id {
            self.database.project_repository().update(
                project_id,
                &ProjectUpdate {
                    name: name.to_string(),
                    parent_project_id,
                    color,
                    is_favorite: editor.is_favorite,
                },
            )?;
            self.selected_project_id = Some(project_id);
        } else {
            let project = self.database.project_repository().create(
                name,
                parent_project_id,
                color,
                editor.is_favorite,
                now,
            )?;
            self.selected_project_id = Some(project.id);
        }
        self.refresh_tasks()?;
        Ok(())
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

        let next_recurring_task = if next_status == TaskStatus::Done {
            match task.due.as_ref() {
                Some(due) if due.is_recurring => match next_recurring_due(due, now) {
                    Some(next_due) => {
                        if self.has_existing_recurring_successor(
                            task.id,
                            task.title.as_str(),
                            &next_due,
                        ) {
                            info!(
                                task_id = task.id.0,
                                title = task.title.as_str(),
                                next_due = next_due.string.as_str(),
                                next_due_date = %next_due.date,
                                "completed recurring task; existing next occurrence already present"
                            );
                            None
                        } else {
                            let next_task = self.database.task_repository().create(
                                task.title.as_str(),
                                task.project_id,
                                Some(&next_due),
                                now,
                            )?;
                            info!(
                                task_id = task.id.0,
                                title = task.title.as_str(),
                                next_task_id = next_task.id.0,
                                next_due = next_due.string.as_str(),
                                next_due_date = %next_due.date,
                                "completed recurring task and created next occurrence"
                            );
                            Some(next_task)
                        }
                    }
                    None => {
                        info!(
                            task_id = task.id.0,
                            title = task.title.as_str(),
                            recurrence = due.string.as_str(),
                            "completed recurring task but could not resolve next occurrence"
                        );
                        None
                    }
                },
                _ => None,
            }
        } else {
            None
        };

        self.refresh_tasks()?;
        if let Some(next_task) = next_recurring_task {
            if self.active_focus_task_id == Some(task.id) {
                self.active_focus_task_id = Some(next_task.id);
            }
            let next_task_visible = self
                .screen_data
                .tasks
                .iter()
                .find(|candidate| candidate.id == next_task.id)
                .map(|candidate| {
                    self.task_is_active(candidate) && self.task_matches_active_view(candidate)
                })
                .unwrap_or(false);
            self.selected_task_id = Some(if next_task_visible {
                next_task.id
            } else {
                task.id
            });
        } else {
            self.selected_task_id = Some(task.id);
        }
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

    fn selected_history_task(&self) -> Option<&Task> {
        let task_id = self.selected_history_focus_entry()?.task_id?;
        self.screen_data
            .tasks
            .iter()
            .find(|task| task.id == task_id)
    }

    fn begin_focus_task_if_needed(&mut self) {
        if self.timer.phase == TimerPhase::Focus && self.timer.current_phase_started_at.is_none() {
            self.active_focus_task_id = self.assigned_task_id;
        }
    }

    fn handle_task_overlay_key(&mut self, code: KeyCode, now: DateTime<Local>) -> Result<bool> {
        if let Some(mut popup) = self.task_sort_popup.take() {
            match code {
                KeyCode::Esc | KeyCode::Char('o') => {}
                KeyCode::Enter => {
                    if let Some(sort_order) =
                        TaskSortOrder::all().get(popup.selected_index).copied()
                    {
                        self.apply_task_sort_order(sort_order)?;
                    }
                }
                KeyCode::Char('j') | KeyCode::Down => {
                    let last_index = TaskSortOrder::all().len().saturating_sub(1);
                    popup.selected_index = (popup.selected_index + 1).min(last_index);
                    self.task_sort_popup = Some(popup);
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    popup.selected_index = popup.selected_index.saturating_sub(1);
                    self.task_sort_popup = Some(popup);
                }
                _ => {
                    self.task_sort_popup = Some(popup);
                }
            }
            return Ok(true);
        }

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
                KeyCode::Left => {
                    Self::move_search_cursor_left(&mut search);
                    self.task_search = Some(search);
                }
                KeyCode::Right => {
                    Self::move_search_cursor_right(&mut search);
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

        if let Some(project_id) = self.project_delete_confirmation {
            match code {
                KeyCode::Enter | KeyCode::Char('y') => {
                    self.database.project_repository().delete(project_id, now)?;
                    self.project_delete_confirmation = None;
                    self.refresh_tasks()?;
                }
                KeyCode::Esc | KeyCode::Char('n') => {
                    self.project_delete_confirmation = None;
                }
                _ => {}
            }
            return Ok(true);
        }

        if let Some(mut editor) = self.project_editor.take() {
            match code {
                KeyCode::Esc => {}
                KeyCode::Enter => {
                    if editor.focused_field == ProjectEditorField::Name
                        && self.accept_project_editor_parent_suggestion(&mut editor)
                    {
                        self.project_editor = Some(editor);
                        return Ok(true);
                    }
                    if editor.focused_field == ProjectEditorField::Parent
                        && self.accept_project_editor_parent_field_suggestion(&mut editor)
                    {
                        self.project_editor = Some(editor);
                        return Ok(true);
                    }
                    self.submit_project_editor(editor, now)?;
                }
                KeyCode::Tab => {
                    if editor.focused_field == ProjectEditorField::Name {
                        if self.accept_project_editor_parent_suggestion(&mut editor) {
                            self.project_editor = Some(editor);
                            return Ok(true);
                        }
                        if self.project_editor_has_parent_without_name(&editor) {
                            if !editor.name_input.ends_with(' ') {
                                editor.name_input.push(' ');
                            }
                            editor.name_cursor = editor.name_input.len();
                            self.project_editor = Some(editor);
                            return Ok(true);
                        }
                    }
                    if editor.focused_field == ProjectEditorField::Parent
                        && self.accept_project_editor_parent_field_suggestion(&mut editor)
                    {
                        self.project_editor = Some(editor);
                        return Ok(true);
                    }
                    editor.focused_field = self.next_project_editor_field(&editor);
                    editor.suggestion_index = 0;
                    self.project_editor = Some(editor);
                }
                KeyCode::BackTab => {
                    editor.focused_field = self.previous_project_editor_field(&editor);
                    editor.suggestion_index = 0;
                    self.project_editor = Some(editor);
                }
                KeyCode::F(1) => {
                    self.focus_project_editor_field(&mut editor, ProjectEditorField::Name);
                    self.project_editor = Some(editor);
                }
                KeyCode::F(2) => {
                    self.focus_project_editor_field(&mut editor, ProjectEditorField::Parent);
                    self.project_editor = Some(editor);
                }
                KeyCode::F(3) => {
                    self.focus_project_editor_field(&mut editor, ProjectEditorField::Color);
                    self.project_editor = Some(editor);
                }
                KeyCode::F(4) => {
                    self.focus_project_editor_field(&mut editor, ProjectEditorField::Favorite);
                    self.project_editor = Some(editor);
                }
                KeyCode::Backspace
                    if matches!(
                        editor.focused_field,
                        ProjectEditorField::Name | ProjectEditorField::Parent
                    ) =>
                {
                    let (value, cursor) = if editor.focused_field == ProjectEditorField::Name {
                        (&mut editor.name_input, &mut editor.name_cursor)
                    } else {
                        (&mut editor.parent_input, &mut editor.parent_cursor)
                    };
                    if *cursor > 0 {
                        let previous_index = value[..*cursor]
                            .char_indices()
                            .last()
                            .map(|(index, _)| index)
                            .unwrap_or(0);
                        value.drain(previous_index..*cursor);
                        *cursor = previous_index;
                    }
                    editor.suggestion_index = 0;
                    self.project_editor = Some(editor);
                }
                KeyCode::Home
                    if matches!(
                        editor.focused_field,
                        ProjectEditorField::Name | ProjectEditorField::Parent
                    ) =>
                {
                    if editor.focused_field == ProjectEditorField::Name {
                        editor.name_cursor = 0;
                    } else {
                        editor.parent_cursor = 0;
                    }
                    self.project_editor = Some(editor);
                }
                KeyCode::Left
                    if matches!(
                        editor.focused_field,
                        ProjectEditorField::Name | ProjectEditorField::Parent
                    ) =>
                {
                    let (value, cursor) = if editor.focused_field == ProjectEditorField::Name {
                        (&editor.name_input, &mut editor.name_cursor)
                    } else {
                        (&editor.parent_input, &mut editor.parent_cursor)
                    };
                    if *cursor > 0 {
                        *cursor = value[..*cursor]
                            .char_indices()
                            .last()
                            .map(|(index, _)| index)
                            .unwrap_or(0);
                    }
                    self.project_editor = Some(editor);
                }
                KeyCode::Right
                    if matches!(
                        editor.focused_field,
                        ProjectEditorField::Name | ProjectEditorField::Parent
                    ) =>
                {
                    let (value, cursor) = if editor.focused_field == ProjectEditorField::Name {
                        (&editor.name_input, &mut editor.name_cursor)
                    } else {
                        (&editor.parent_input, &mut editor.parent_cursor)
                    };
                    if *cursor < value.len() {
                        *cursor = value[*cursor..]
                            .char_indices()
                            .nth(1)
                            .map(|(offset, _)| *cursor + offset)
                            .unwrap_or(value.len());
                    }
                    self.project_editor = Some(editor);
                }
                KeyCode::End
                    if matches!(
                        editor.focused_field,
                        ProjectEditorField::Name | ProjectEditorField::Parent
                    ) =>
                {
                    if editor.focused_field == ProjectEditorField::Name {
                        editor.name_cursor = editor.name_input.len();
                    } else {
                        editor.parent_cursor = editor.parent_input.len();
                    }
                    self.project_editor = Some(editor);
                }
                KeyCode::Left | KeyCode::Char('h') | KeyCode::Up | KeyCode::Char('k')
                    if matches!(
                        editor.focused_field,
                        ProjectEditorField::Color | ProjectEditorField::Favorite
                    ) =>
                {
                    match editor.focused_field {
                        ProjectEditorField::Color => {
                            editor.color_index = editor.color_index.saturating_sub(1);
                        }
                        ProjectEditorField::Favorite => {
                            editor.is_favorite = !editor.is_favorite;
                        }
                        ProjectEditorField::Parent => {}
                        ProjectEditorField::Name => {}
                    }
                    self.project_editor = Some(editor);
                }
                KeyCode::Right | KeyCode::Char('l') | KeyCode::Down | KeyCode::Char('j')
                    if matches!(
                        editor.focused_field,
                        ProjectEditorField::Color | ProjectEditorField::Favorite
                    ) =>
                {
                    match editor.focused_field {
                        ProjectEditorField::Color => {
                            editor.color_index = (editor.color_index + 1)
                                .min(ProjectColor::all().len().saturating_sub(1));
                        }
                        ProjectEditorField::Favorite => {
                            editor.is_favorite = !editor.is_favorite;
                        }
                        ProjectEditorField::Parent => {}
                        ProjectEditorField::Name => {}
                    }
                    self.project_editor = Some(editor);
                }
                KeyCode::Char(character) if editor.focused_field == ProjectEditorField::Name => {
                    if character == '#' && self.project_editor_has_parent_without_name(&editor) {
                        editor.name_input.clear();
                        editor.name_cursor = 0;
                    }
                    if character == '#'
                        && editor.name_cursor > 0
                        && editor.name_input[..editor.name_cursor]
                            .chars()
                            .last()
                            .is_some_and(|previous| !previous.is_whitespace())
                    {
                        editor.name_input.insert(editor.name_cursor, ' ');
                        editor.name_cursor += 1;
                    }
                    editor.name_input.insert(editor.name_cursor, character);
                    editor.name_cursor += character.len_utf8();
                    editor.suggestion_index = 0;
                    self.project_editor = Some(editor);
                }
                KeyCode::Char(character) if editor.focused_field == ProjectEditorField::Parent => {
                    editor.parent_input.insert(editor.parent_cursor, character);
                    editor.parent_cursor += character.len_utf8();
                    editor.suggestion_index = 0;
                    self.project_editor = Some(editor);
                }
                KeyCode::Down if editor.focused_field == ProjectEditorField::Name => {
                    if let Some((_, _, query)) =
                        self.active_project_query(editor.name_input.as_str(), editor.name_cursor)
                    {
                        let last_index = self
                            .project_parent_suggestions(query.as_str(), editor.project_id)
                            .len()
                            .saturating_sub(1);
                        editor.suggestion_index = (editor.suggestion_index + 1).min(last_index);
                    }
                    self.project_editor = Some(editor);
                }
                KeyCode::Down if editor.focused_field == ProjectEditorField::Parent => {
                    if let Some(query) =
                        self.active_parent_field_query(editor.parent_input.as_str())
                    {
                        let last_index = self
                            .project_parent_suggestions(query, editor.project_id)
                            .len()
                            .saturating_sub(1);
                        editor.suggestion_index = (editor.suggestion_index + 1).min(last_index);
                    }
                    self.project_editor = Some(editor);
                }
                KeyCode::Up if editor.focused_field == ProjectEditorField::Name => {
                    editor.suggestion_index = editor.suggestion_index.saturating_sub(1);
                    self.project_editor = Some(editor);
                }
                KeyCode::Up if editor.focused_field == ProjectEditorField::Parent => {
                    editor.suggestion_index = editor.suggestion_index.saturating_sub(1);
                    self.project_editor = Some(editor);
                }
                _ => {
                    self.project_editor = Some(editor);
                }
            }
            return Ok(true);
        }

        if let Some(mut editor) = self.task_editor.take() {
            if editor.calendar.is_some() {
                match code {
                    KeyCode::Esc => {
                        editor.calendar = None;
                    }
                    KeyCode::Enter => {
                        Self::apply_calendar_selection(&mut editor);
                    }
                    KeyCode::Left | KeyCode::Char('h') => {
                        Self::move_calendar_selection(&mut editor, -1);
                    }
                    KeyCode::Right | KeyCode::Char('l') => {
                        Self::move_calendar_selection(&mut editor, 1);
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        Self::move_calendar_selection(&mut editor, -7);
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        Self::move_calendar_selection(&mut editor, 7);
                    }
                    KeyCode::PageUp => {
                        Self::shift_calendar_page(&mut editor, -1);
                    }
                    KeyCode::PageDown => {
                        Self::shift_calendar_page(&mut editor, 1);
                    }
                    _ => {}
                }
                self.task_editor = Some(editor);
                return Ok(true);
            }

            match code {
                KeyCode::Esc => {}
                KeyCode::Enter => {
                    if editor.focused_field == TaskEditorField::Project {
                        let query = editor.project_input.trim();
                        if !query.is_empty() {
                            let suggestions = self.project_suggestions(query);
                            if let Some(project) = suggestions
                                .get(
                                    editor
                                        .suggestion_index
                                        .min(suggestions.len().saturating_sub(1)),
                                )
                                .copied()
                            {
                                let already_selected = editor.project_id == project.id
                                    && editor
                                        .project_input
                                        .trim()
                                        .eq_ignore_ascii_case(project.name.as_str());
                                if !already_selected {
                                    editor.project_id = project.id;
                                    editor.project_input = project.name.clone();
                                    editor.project_cursor = editor.project_input.len();
                                    editor.suggestion_index = 0;
                                    self.task_editor = Some(editor);
                                    return Ok(true);
                                }
                            }
                        }
                    }
                    return self.submit_task_editor(editor, now);
                }
                KeyCode::Tab => {
                    if editor.focused_field == TaskEditorField::Project {
                        let query = editor.project_input.trim();
                        if !query.is_empty() {
                            let suggestions = self.project_suggestions(query);
                            if let Some(project) = suggestions
                                .get(
                                    editor
                                        .suggestion_index
                                        .min(suggestions.len().saturating_sub(1)),
                                )
                                .copied()
                            {
                                let already_selected = editor.project_id == project.id
                                    && editor
                                        .project_input
                                        .trim()
                                        .eq_ignore_ascii_case(project.name.as_str());
                                if !already_selected {
                                    editor.project_id = project.id;
                                    editor.project_input = project.name.clone();
                                    editor.project_cursor = editor.project_input.len();
                                    editor.suggestion_index = 0;
                                    self.task_editor = Some(editor);
                                    return Ok(true);
                                }
                            }
                        }
                    }
                    editor.focused_field = editor.focused_field.next();
                    editor.suggestion_index = 0;
                    self.task_editor = Some(editor);
                }
                KeyCode::BackTab => {
                    editor.focused_field = editor.focused_field.previous();
                    editor.suggestion_index = 0;
                    self.task_editor = Some(editor);
                }
                KeyCode::F(1) => {
                    Self::focus_editor_field(&mut editor, TaskEditorField::Title);
                    editor.suggestion_index = 0;
                    self.task_editor = Some(editor);
                }
                KeyCode::F(2) => {
                    Self::focus_editor_field(&mut editor, TaskEditorField::Project);
                    editor.suggestion_index = 0;
                    self.task_editor = Some(editor);
                }
                KeyCode::F(3) => {
                    Self::focus_editor_field(&mut editor, TaskEditorField::DueDate);
                    editor.suggestion_index = 0;
                    self.task_editor = Some(editor);
                }
                KeyCode::F(4) => {
                    Self::focus_editor_field(&mut editor, TaskEditorField::DueTime);
                    editor.suggestion_index = 0;
                    self.task_editor = Some(editor);
                }
                KeyCode::F(5) => {
                    Self::focus_editor_field(&mut editor, TaskEditorField::Recurrence);
                    editor.suggestion_index = 0;
                    self.task_editor = Some(editor);
                }
                KeyCode::F(7) => {
                    Self::clear_editor_due(&mut editor);
                    self.task_editor = Some(editor);
                }
                KeyCode::Down | KeyCode::Char('j')
                    if editor.focused_field == TaskEditorField::Project =>
                {
                    let last_index = self
                        .project_suggestions(editor.project_input.as_str())
                        .len()
                        .saturating_sub(1);
                    editor.suggestion_index = (editor.suggestion_index + 1).min(last_index);
                    self.task_editor = Some(editor);
                }
                KeyCode::Up | KeyCode::Char('k')
                    if editor.focused_field == TaskEditorField::Project =>
                {
                    editor.suggestion_index = editor.suggestion_index.saturating_sub(1);
                    self.task_editor = Some(editor);
                }
                KeyCode::F(6) if editor.focused_field == TaskEditorField::DueDate => {
                    Self::open_editor_calendar(&mut editor, now.date_naive());
                    self.task_editor = Some(editor);
                }
                KeyCode::Home => {
                    Self::move_editor_cursor_home(&mut editor);
                    self.task_editor = Some(editor);
                }
                KeyCode::Left => {
                    Self::move_editor_cursor_left(&mut editor);
                    self.task_editor = Some(editor);
                }
                KeyCode::Right => {
                    Self::move_editor_cursor_right(&mut editor);
                    self.task_editor = Some(editor);
                }
                KeyCode::End => {
                    Self::move_editor_cursor_end(&mut editor);
                    self.task_editor = Some(editor);
                }
                KeyCode::Backspace => {
                    Self::delete_editor_char_before_cursor(&mut editor, now.date_naive());
                    editor.suggestion_index = 0;
                    self.task_editor = Some(editor);
                }
                KeyCode::Char(character) => {
                    Self::insert_editor_char(&mut editor, character, now.date_naive());
                    editor.suggestion_index = 0;
                    self.task_editor = Some(editor);
                }
                _ => {
                    self.task_editor = Some(editor);
                }
            }
            return Ok(true);
        }

        let Some(mut input) = self.task_input.take() else {
            return Ok(false);
        };

        match code {
            KeyCode::Esc => {}
            KeyCode::Tab => {
                let _ = self.accept_task_input_project_suggestion(&mut input);
                self.task_input = Some(input);
            }
            KeyCode::Down => {
                if let Some((_, _, query)) =
                    self.active_project_query(input.value.as_str(), input.cursor)
                {
                    let last_index = self
                        .project_suggestions(query.as_str())
                        .len()
                        .saturating_sub(1);
                    input.suggestion_index = (input.suggestion_index + 1).min(last_index);
                }
                self.task_input = Some(input);
            }
            KeyCode::Up => {
                input.suggestion_index = input.suggestion_index.saturating_sub(1);
                self.task_input = Some(input);
            }
            KeyCode::Enter => {
                if self.accept_task_input_project_suggestion(&mut input) {
                    self.task_input = Some(input);
                    return Ok(true);
                }
                let parsed = self.task_input_parse(input.value.as_str(), input.project_id);
                if parsed.cleaned_title.is_empty() {
                    self.task_input = Some(input);
                    return Ok(true);
                }

                let task = self.database.task_repository().create(
                    parsed.cleaned_title.as_str(),
                    parsed.project_id,
                    parsed.due.as_ref(),
                    now,
                )?;
                self.refresh_tasks()?;
                self.selected_task_id = Some(task.id);
            }
            KeyCode::Backspace => {
                Self::delete_input_char_before_cursor(&mut input);
                input.suggestion_index = 0;
                self.task_input = Some(input);
            }
            KeyCode::Home => {
                Self::move_input_cursor_home(&mut input);
                input.suggestion_index = 0;
                self.task_input = Some(input);
            }
            KeyCode::Left => {
                Self::move_input_cursor_left(&mut input);
                input.suggestion_index = 0;
                self.task_input = Some(input);
            }
            KeyCode::Right => {
                Self::move_input_cursor_right(&mut input);
                input.suggestion_index = 0;
                self.task_input = Some(input);
            }
            KeyCode::End => {
                Self::move_input_cursor_end(&mut input);
                input.suggestion_index = 0;
                self.task_input = Some(input);
            }
            KeyCode::Char(character) => {
                Self::insert_input_char(&mut input, character);
                input.suggestion_index = 0;
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

    fn move_search_cursor_left(search: &mut TaskSearchState) {
        if search.cursor == 0 {
            return;
        }
        search.cursor = search.query[..search.cursor]
            .char_indices()
            .last()
            .map(|(index, _)| index)
            .unwrap_or(0);
    }

    fn move_search_cursor_right(search: &mut TaskSearchState) {
        if search.cursor >= search.query.len() {
            return;
        }
        search.cursor = search.query[search.cursor..]
            .char_indices()
            .nth(1)
            .map(|(offset, _)| search.cursor + offset)
            .unwrap_or(search.query.len());
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
            KeyCode::Char('3') => {
                self.focused_panel = PanelFocus::Navigation;
                self.active_sidebar_tab = SidebarTab::Navigation;
            }
            KeyCode::Char('4') => {
                self.focused_panel = PanelFocus::Navigation;
                self.active_sidebar_tab = SidebarTab::FiltersTags;
            }
            KeyCode::Char('5') => {
                self.focused_panel = PanelFocus::Navigation;
                self.active_sidebar_tab = SidebarTab::Projects;
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
                match self.active_sidebar_tab {
                    SidebarTab::Navigation => self.select_next_task_view(),
                    SidebarTab::FiltersTags => {}
                    SidebarTab::Projects => self.move_project_selection(1),
                }
            }
            KeyCode::Char('k') | KeyCode::Up if self.focused_panel == PanelFocus::Navigation => {
                match self.active_sidebar_tab {
                    SidebarTab::Navigation => self.select_previous_task_view(),
                    SidebarTab::FiltersTags => {}
                    SidebarTab::Projects => self.move_project_selection(-1),
                }
            }
            KeyCode::PageDown if self.focused_panel == PanelFocus::Navigation => {
                const SIDEBAR_PAGE_STEP: isize = 5;
                match self.active_sidebar_tab {
                    SidebarTab::Navigation => self.move_task_view_selection(SIDEBAR_PAGE_STEP),
                    SidebarTab::FiltersTags => {}
                    SidebarTab::Projects => self.move_project_selection(SIDEBAR_PAGE_STEP),
                }
            }
            KeyCode::PageUp if self.focused_panel == PanelFocus::Navigation => {
                const SIDEBAR_PAGE_STEP: isize = 5;
                match self.active_sidebar_tab {
                    SidebarTab::Navigation => self.move_task_view_selection(-SIDEBAR_PAGE_STEP),
                    SidebarTab::FiltersTags => {}
                    SidebarTab::Projects => self.move_project_selection(-SIDEBAR_PAGE_STEP),
                }
            }
            KeyCode::Home if self.focused_panel == PanelFocus::Navigation => {
                match self.active_sidebar_tab {
                    SidebarTab::Navigation => self.set_active_task_view(TaskView::All),
                    SidebarTab::FiltersTags => {}
                    SidebarTab::Projects => self.selected_project_id = None,
                }
            }
            KeyCode::End if self.focused_panel == PanelFocus::Navigation => {
                match self.active_sidebar_tab {
                    SidebarTab::Navigation => self.set_active_task_view(TaskView::Soon),
                    SidebarTab::FiltersTags => {}
                    SidebarTab::Projects => {
                        self.selected_project_id = self
                            .project_tree_rows()
                            .last()
                            .and_then(|row| row.project_id)
                    }
                }
            }
            KeyCode::Char('C')
                if self.focused_panel == PanelFocus::Navigation
                    && self.active_sidebar_tab == SidebarTab::Projects =>
            {
                self.open_create_project_popup();
            }
            KeyCode::Char('e')
                if self.focused_panel == PanelFocus::Navigation
                    && self.active_sidebar_tab == SidebarTab::Projects =>
            {
                self.open_edit_project_popup();
            }
            KeyCode::Char('d')
                if self.focused_panel == PanelFocus::Navigation
                    && self.active_sidebar_tab == SidebarTab::Projects =>
            {
                self.open_project_delete_confirmation();
            }
            KeyCode::Char('f')
                if self.focused_panel == PanelFocus::Navigation
                    && self.active_sidebar_tab == SidebarTab::Projects =>
            {
                self.toggle_selected_project_favorite()?;
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
                self.open_edit_task_popup();
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
            KeyCode::Char('o')
                if self.focused_panel == PanelFocus::RightPane
                    && self.active_right_panel_tab == RightPanelTab::Tasks =>
            {
                self.open_task_sort_popup();
            }
            KeyCode::Char('f')
                if self.focused_panel == PanelFocus::RightPane
                    && self.active_right_panel_tab == RightPanelTab::Tasks =>
            {
                self.toggle_hide_completed_tasks()?;
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
            KeyCode::Char(' ')
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
        projects: database.project_repository().list_all()?,
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

    let mut app = App::new(screen_data, config, Some(paths.clone()), theme, database);
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
    use chrono::{Duration as ChronoDuration, Local, NaiveDate};

    use crate::config::{AppConfig, GlyphMode, TaskSortOrder, TimerSettings};
    use crate::domain::{ProjectColor, ProjectId, TaskDue, TaskId, TaskStatus};
    use crate::storage::{Database, ProjectRepository, TaskRepository};
    use crate::task_nlp::parse_task_input;
    use crate::theme::ThemePalette;

    use super::{
        App, CycleEntryState, HistoryPanelTab, PanelFocus, PreviewLineView, RightPanelTab,
        RunOptions, ScreenData, TaskEditorField, TaskEditorState, TaskView, TimerPhase,
        TimerRunState, apply_debug_overrides, chrono_duration, duration_to_stored_minutes,
    };

    fn assert_key_value_preview_line(
        line: &PreviewLineView,
        expected_label: &str,
        expected_value: &str,
    ) {
        match line {
            PreviewLineView::KeyValue { label, value, .. } => {
                assert_eq!(label, expected_label);
                assert_eq!(value, expected_value);
            }
            _ => panic!("expected key/value preview line"),
        }
    }

    fn test_app() -> App {
        let config = AppConfig::default();
        let database = Database::open_in_memory().expect("in-memory database should open");
        App::new(
            ScreenData {
                tasks: database
                    .task_repository()
                    .list_all()
                    .expect("tasks should load"),
                projects: database
                    .project_repository()
                    .list_all()
                    .expect("projects should load"),
                ..ScreenData::default()
            },
            config,
            None,
            ThemePalette::load(
                &crate::config::AppPaths::from_data_dir(std::env::temp_dir())
                    .expect("paths should resolve"),
                "catppuccin-mocha",
            )
            .expect("built-in theme should load"),
            database,
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
        app.handle_key(crossterm::event::KeyCode::Char('7'))
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
    fn app_creates_and_edits_task_title_through_form_flow() {
        let mut app = test_app();
        app.handle_key(crossterm::event::KeyCode::Char('7'))
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
            .expect("editor should open");
        for _ in 0.."Write tests".len() {
            app.handle_key(crossterm::event::KeyCode::Backspace)
                .expect("backspace should work");
        }
        for character in "Ship tests".chars() {
            app.handle_key(crossterm::event::KeyCode::Char(character))
                .expect("typing should succeed");
        }
        app.handle_key(crossterm::event::KeyCode::Enter)
            .expect("edit should submit");

        assert_eq!(app.screen_data.tasks[0].title, "Ship tests");
    }

    #[test]
    fn task_editor_updates_due_from_title_nlp() {
        let mut app = test_app();
        let tomorrow = app.today() + chrono::Days::new(1);

        app.handle_key(crossterm::event::KeyCode::Char('7'))
            .expect("focus should switch");
        app.handle_key(crossterm::event::KeyCode::Char('c'))
            .expect("popup should open");
        for character in "Write tests".chars() {
            app.handle_key(crossterm::event::KeyCode::Char(character))
                .expect("typing should succeed");
        }
        app.handle_key(crossterm::event::KeyCode::Enter)
            .expect("task should be created");

        app.handle_key(crossterm::event::KeyCode::Char('e'))
            .expect("editor should open");
        for character in " tomorrow at 3pm".chars() {
            app.handle_key(crossterm::event::KeyCode::Char(character))
                .expect("typing should succeed");
        }

        let editor = app.task_editor_view().expect("editor should be visible");
        let due_preview = editor.due_preview.expect("due preview should be visible");
        assert_eq!(due_preview.date, tomorrow);
        assert_eq!(
            due_preview.datetime,
            Some(tomorrow.and_hms_opt(15, 0, 0).expect("valid time"))
        );

        app.handle_key(crossterm::event::KeyCode::Enter)
            .expect("edit should submit");

        let task = app.selected_task().expect("task should still exist");
        assert_eq!(task.title, "Write tests");
        assert_eq!(
            task.due,
            Some(TaskDue {
                date: tomorrow,
                datetime: Some(tomorrow.and_hms_opt(15, 0, 0).expect("valid time")),
                string: "tomorrow at 3pm".to_string(),
                is_recurring: false,
            })
        );
    }

    #[test]
    fn task_editor_can_clear_due() {
        let mut app = test_app();

        app.handle_key(crossterm::event::KeyCode::Char('7'))
            .expect("focus should switch");
        app.handle_key(crossterm::event::KeyCode::Char('c'))
            .expect("popup should open");
        for character in "Write tests tomorrow".chars() {
            app.handle_key(crossterm::event::KeyCode::Char(character))
                .expect("typing should succeed");
        }
        app.handle_key(crossterm::event::KeyCode::Enter)
            .expect("task should be created");
        assert!(
            app.selected_task()
                .expect("task should exist")
                .due
                .is_some()
        );

        app.handle_key(crossterm::event::KeyCode::Char('e'))
            .expect("editor should open");
        app.handle_key(crossterm::event::KeyCode::F(7))
            .expect("clear due should succeed");
        app.handle_key(crossterm::event::KeyCode::Enter)
            .expect("edit should submit");

        assert_eq!(app.selected_task().expect("task should exist").due, None);
    }

    #[test]
    fn task_editor_calendar_sets_due_date() {
        let mut app = test_app();
        let today = app.today();

        app.handle_key(crossterm::event::KeyCode::Char('7'))
            .expect("focus should switch");
        app.handle_key(crossterm::event::KeyCode::Char('c'))
            .expect("popup should open");
        for character in "Write tests".chars() {
            app.handle_key(crossterm::event::KeyCode::Char(character))
                .expect("typing should succeed");
        }
        app.handle_key(crossterm::event::KeyCode::Enter)
            .expect("task should be created");

        app.handle_key(crossterm::event::KeyCode::Char('e'))
            .expect("editor should open");
        app.handle_key(crossterm::event::KeyCode::F(3))
            .expect("focus should switch to due date");
        app.handle_key(crossterm::event::KeyCode::F(6))
            .expect("calendar should open");
        assert!(
            app.task_editor_view()
                .expect("editor should be visible")
                .calendar
                .is_some()
        );

        app.handle_key(crossterm::event::KeyCode::Right)
            .expect("calendar should move selection");
        app.handle_key(crossterm::event::KeyCode::Enter)
            .expect("calendar selection should apply");
        app.handle_key(crossterm::event::KeyCode::Enter)
            .expect("edit should submit");

        assert_eq!(
            app.selected_task().expect("task should exist").due,
            Some(TaskDue {
                date: today + chrono::Days::new(1),
                datetime: None,
                string: (today + chrono::Days::new(1))
                    .format("%Y-%m-%d")
                    .to_string(),
                is_recurring: false,
            })
        );
    }

    #[test]
    fn task_editor_recurrence_field_updates_due_pattern() {
        let mut app = test_app();
        let expected_due = parse_task_input("Placeholder every monday at 9am", app.today())
            .due
            .expect("recurrence should parse");

        app.handle_key(crossterm::event::KeyCode::Char('7'))
            .expect("focus should switch");
        app.handle_key(crossterm::event::KeyCode::Char('c'))
            .expect("popup should open");
        for character in "Write tests".chars() {
            app.handle_key(crossterm::event::KeyCode::Char(character))
                .expect("typing should succeed");
        }
        app.handle_key(crossterm::event::KeyCode::Enter)
            .expect("task should be created");

        app.handle_key(crossterm::event::KeyCode::Char('e'))
            .expect("editor should open");
        app.handle_key(crossterm::event::KeyCode::F(5))
            .expect("focus should switch to recurrence");
        for character in "every monday at 9am".chars() {
            app.handle_key(crossterm::event::KeyCode::Char(character))
                .expect("typing should succeed");
        }
        app.handle_key(crossterm::event::KeyCode::Enter)
            .expect("edit should submit");

        assert_eq!(
            app.selected_task().expect("task should exist").due,
            Some(expected_due)
        );
    }

    #[test]
    fn task_editor_recurrence_phrase_updates_due_time() {
        let mut editor = TaskEditorState {
            task_id: TaskId(1),
            title_input: "Write tests".to_string(),
            title_cursor: "Write tests".len(),
            project_input: "Inbox".to_string(),
            project_cursor: "Inbox".len(),
            project_id: ProjectId(1),
            suggestion_index: 0,
            due_date_input: "2026-04-13".to_string(),
            due_date_cursor: "2026-04-13".len(),
            due_time_input: "09:00".to_string(),
            due_time_cursor: "09:00".len(),
            recurrence_input: "every tuesday at 10am".to_string(),
            recurrence_cursor: "every tuesday at 10am".len(),
            due_natural: "every monday at 9am".to_string(),
            due_from_title: false,
            focused_field: TaskEditorField::Recurrence,
            calendar: None,
        };

        App::sync_editor_due_from_recurrence(
            &mut editor,
            NaiveDate::from_ymd_opt(2026, 4, 10).expect("valid date"),
        );

        assert_eq!(editor.due_date_input, "2026-04-14");
        assert_eq!(editor.due_time_input, "10:00");
        assert_eq!(editor.due_natural, "every tuesday at 10am");
    }

    #[test]
    fn task_editor_due_fields_accept_natural_language() {
        let mut app = test_app();
        let expected_date = app.today() + chrono::Days::new(1);

        app.handle_key(crossterm::event::KeyCode::Char('7'))
            .expect("focus should switch");
        app.handle_key(crossterm::event::KeyCode::Char('c'))
            .expect("popup should open");
        for character in "Finish this".chars() {
            app.handle_key(crossterm::event::KeyCode::Char(character))
                .expect("typing should succeed");
        }
        app.handle_key(crossterm::event::KeyCode::Enter)
            .expect("task should be created");

        app.handle_key(crossterm::event::KeyCode::Char('e'))
            .expect("editor should open");
        app.handle_key(crossterm::event::KeyCode::F(3))
            .expect("focus should switch to due date");
        for _ in 0.."YYYY-MM-DD".len() {
            app.handle_key(crossterm::event::KeyCode::Backspace).ok();
        }
        for character in "tomorrow".chars() {
            app.handle_key(crossterm::event::KeyCode::Char(character))
                .expect("date typing should succeed");
        }
        app.handle_key(crossterm::event::KeyCode::F(4))
            .expect("focus should switch to due time");
        for character in "3pm".chars() {
            app.handle_key(crossterm::event::KeyCode::Char(character))
                .expect("time typing should succeed");
        }
        app.handle_key(crossterm::event::KeyCode::Enter)
            .expect("edit should submit");

        assert_eq!(
            app.selected_task().expect("task should exist").due,
            Some(TaskDue {
                date: expected_date,
                datetime: Some(expected_date.and_hms_opt(15, 0, 0).expect("valid time")),
                string: format!("{} at 15:00", expected_date.format("%Y-%m-%d")),
                is_recurring: false,
            })
        );
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
    fn create_popup_extracts_due_date_preview_and_stores_due() {
        let mut app = test_app();
        let today = app.today();

        app.handle_key(crossterm::event::KeyCode::Char('c'))
            .expect("popup should open");
        for character in "Ship report tomorrow".chars() {
            app.handle_key(crossterm::event::KeyCode::Char(character))
                .expect("typing should succeed");
        }

        let input = app.task_input_view().expect("input popup should be open");
        let due_preview = input.due_preview.expect("due preview should be visible");
        assert_eq!(due_preview.string, "tomorrow");
        assert_eq!(due_preview.date, today + chrono::Days::new(1));
        assert_eq!(due_preview.datetime, None);

        app.handle_key(crossterm::event::KeyCode::Enter)
            .expect("submit should succeed");

        let task = app.selected_task().expect("task should be selected");
        assert_eq!(task.title, "Ship report");
        assert_eq!(
            task.due,
            Some(TaskDue {
                date: today + chrono::Days::new(1),
                datetime: None,
                string: "tomorrow".to_string(),
                is_recurring: false,
            })
        );
    }

    #[test]
    fn create_popup_extracts_due_time_preview_and_stores_datetime() {
        let mut app = test_app();
        let tomorrow = app.today() + chrono::Days::new(1);

        app.handle_key(crossterm::event::KeyCode::Char('c'))
            .expect("popup should open");
        for character in "Ship report tomorrow at 3pm".chars() {
            app.handle_key(crossterm::event::KeyCode::Char(character))
                .expect("typing should succeed");
        }

        let input = app.task_input_view().expect("input popup should be open");
        let due_preview = input.due_preview.expect("due preview should be visible");
        assert_eq!(due_preview.string, "tomorrow at 3pm");
        assert_eq!(due_preview.date, tomorrow);
        assert_eq!(
            due_preview.datetime,
            Some(tomorrow.and_hms_opt(15, 0, 0).expect("valid time"))
        );

        app.handle_key(crossterm::event::KeyCode::Enter)
            .expect("submit should succeed");

        let task = app.selected_task().expect("task should be selected");
        assert_eq!(task.title, "Ship report");
        assert_eq!(
            task.due,
            Some(TaskDue {
                date: tomorrow,
                datetime: Some(tomorrow.and_hms_opt(15, 0, 0).expect("valid time")),
                string: "tomorrow at 3pm".to_string(),
                is_recurring: false,
            })
        );
    }

    #[test]
    fn task_input_preview_panel_includes_due_preview_and_contextual_tip() {
        let mut app = test_app();
        let tomorrow = app.today() + chrono::Days::new(1);
        app.handle_key(crossterm::event::KeyCode::Char('c'))
            .expect("popup should open");
        for character in "Ship report tomorrow at 3pm".chars() {
            app.handle_key(crossterm::event::KeyCode::Char(character))
                .expect("typing should succeed");
        }

        let input = app.task_input_view().expect("input popup should be open");
        assert_eq!(input.preview_panel.tips.len(), 1);
        assert_eq!(
            input.preview_panel.tips[0],
            "Press # for selecting a project"
        );
        assert!(input.preview_panel.preview_lines.len() >= 3);
        assert_key_value_preview_line(
            &input.preview_panel.preview_lines[0],
            "Due Date",
            tomorrow.format("%Y-%m-%d").to_string().as_str(),
        );
        assert_key_value_preview_line(&input.preview_panel.preview_lines[1], "Due Time", "15:00");
        assert_key_value_preview_line(&input.preview_panel.preview_lines[2], "Recurring", "no");
    }

    #[test]
    fn task_editor_preview_panel_switches_tip_by_active_field() {
        let mut app = test_app();
        app.handle_key(crossterm::event::KeyCode::Char('7'))
            .expect("focus should switch");
        app.handle_key(crossterm::event::KeyCode::Char('c'))
            .expect("popup should open");
        for character in "Prepare deck tomorrow".chars() {
            app.handle_key(crossterm::event::KeyCode::Char(character))
                .expect("typing should succeed");
        }
        app.handle_key(crossterm::event::KeyCode::Enter)
            .expect("submit should succeed");
        app.handle_key(crossterm::event::KeyCode::Char('e'))
            .expect("editor should open");

        let editor = app.task_editor_view().expect("editor should be visible");
        assert_eq!(
            editor.preview_panel.tips,
            vec!["Press # for selecting a project".to_string()]
        );

        app.handle_key(crossterm::event::KeyCode::F(3))
            .expect("focus should switch");
        let editor = app.task_editor_view().expect("editor should be visible");
        assert_eq!(
            editor.preview_panel.tips,
            vec!["Type YYYY-MM-DD or use F6 to pick from calendar".to_string()]
        );

        app.handle_key(crossterm::event::KeyCode::F(5))
            .expect("focus should switch");
        let editor = app.task_editor_view().expect("editor should be visible");
        assert_eq!(
            editor.preview_panel.tips,
            vec!["Type recurrence phrases like: every monday at 9am".to_string()]
        );
    }

    #[test]
    fn project_editor_preview_panel_switches_tip_by_active_field() {
        let mut app = test_app();
        app.handle_key(crossterm::event::KeyCode::Char('5'))
            .expect("focus should switch");
        app.handle_key(crossterm::event::KeyCode::Char('C'))
            .expect("editor should open");

        let editor = app
            .project_editor_view()
            .expect("project editor should be visible");
        assert_eq!(
            editor.preview_panel.tips,
            vec!["Press # for selecting a parent project".to_string()]
        );

        app.handle_key(crossterm::event::KeyCode::F(3))
            .expect("focus should switch");
        let editor = app
            .project_editor_view()
            .expect("project editor should be visible");
        assert_eq!(
            editor.preview_panel.tips,
            vec!["Use ←/→ or h/l to change the color".to_string()]
        );

        app.handle_key(crossterm::event::KeyCode::F(4))
            .expect("focus should switch");
        let editor = app
            .project_editor_view()
            .expect("project editor should be visible");
        assert_eq!(
            editor.preview_panel.tips,
            vec!["Use ←/→ or h/l to toggle favorite".to_string()]
        );
    }

    #[test]
    fn create_popup_enter_accepts_project_suggestion_before_submitting_task() {
        let mut app = test_app();
        let project = app
            .database
            .project_repository()
            .create(
                "Child Project 02",
                None,
                ProjectColor::Blue,
                false,
                Local::now(),
            )
            .expect("project should create");
        app.refresh_tasks().expect("tasks should refresh");

        app.handle_key(crossterm::event::KeyCode::Char('c'))
            .expect("popup should open");
        for character in "Draft spec #Chi".chars() {
            app.handle_key(crossterm::event::KeyCode::Char(character))
                .expect("typing should succeed");
        }

        let input = app.task_input_view().expect("input popup should be open");
        assert!(!input.project_suggestions.is_empty());

        app.handle_key(crossterm::event::KeyCode::Enter)
            .expect("enter should accept suggestion first");
        let input = app
            .task_input_view()
            .expect("input popup should stay open after suggestion accept");
        assert_eq!(input.project_name, "Child Project 02");
        assert_eq!(app.visible_tasks().len(), 0);

        app.handle_key(crossterm::event::KeyCode::Enter)
            .expect("second enter should submit task");
        let task = app.selected_task().expect("task should be selected");
        assert_eq!(task.title, "Draft spec");
        assert_eq!(task.project_id, project.id);
    }

    #[test]
    fn task_views_filter_by_due_date() {
        let mut app = test_app();
        let today = app.today();
        let repository = app.database.task_repository();
        let inbox_project_id = app.inbox_project_id();

        repository
            .create("Inbox task", inbox_project_id, None, Local::now())
            .expect("inbox task should create");
        repository
            .create(
                "Today task",
                inbox_project_id,
                Some(&TaskDue {
                    date: today,
                    datetime: None,
                    string: "today".to_string(),
                    is_recurring: false,
                }),
                Local::now(),
            )
            .expect("today task should create");
        repository
            .create(
                "Soon task",
                inbox_project_id,
                Some(&TaskDue {
                    date: today + chrono::Days::new(2),
                    datetime: None,
                    string: "next week".to_string(),
                    is_recurring: false,
                }),
                Local::now(),
            )
            .expect("soon task should create");

        app.refresh_tasks().expect("tasks should refresh");

        app.set_active_task_view(TaskView::All);
        assert_eq!(app.visible_tasks().len(), 3);

        app.set_active_task_view(TaskView::Inbox);
        let inbox_titles = app
            .visible_tasks()
            .into_iter()
            .map(|task| task.title.as_str())
            .collect::<Vec<_>>();
        assert_eq!(inbox_titles, vec!["Today task", "Soon task", "Inbox task"]);

        app.set_active_task_view(TaskView::Today);
        let today_titles = app
            .visible_tasks()
            .into_iter()
            .map(|task| task.title.as_str())
            .collect::<Vec<_>>();
        assert_eq!(today_titles, vec!["Today task"]);

        app.set_active_task_view(TaskView::Soon);
        let soon_titles = app
            .visible_tasks()
            .into_iter()
            .map(|task| task.title.as_str())
            .collect::<Vec<_>>();
        assert_eq!(soon_titles, vec!["Soon task"]);
    }

    #[test]
    fn selected_project_filters_visible_tasks_by_subtree() {
        let mut app = test_app();
        let parent = app
            .database
            .project_repository()
            .create("Work", None, ProjectColor::Blue, false, Local::now())
            .expect("parent project should create");
        let child = app
            .database
            .project_repository()
            .create(
                "Client",
                Some(parent.id),
                ProjectColor::Teal,
                false,
                Local::now(),
            )
            .expect("child project should create");
        let inbox_project_id = app.inbox_project_id();
        let repository = app.database.task_repository();
        repository
            .create("Inbox task", inbox_project_id, None, Local::now())
            .expect("inbox task should create");
        repository
            .create("Parent task", parent.id, None, Local::now())
            .expect("parent task should create");
        repository
            .create("Child task", child.id, None, Local::now())
            .expect("child task should create");

        app.refresh_tasks().expect("tasks should refresh");
        app.selected_project_id = Some(parent.id);

        let titles = app
            .visible_tasks()
            .into_iter()
            .map(|task| task.title.as_str())
            .collect::<Vec<_>>();
        assert_eq!(titles, vec!["Child task", "Parent task"]);
    }

    #[test]
    fn project_editor_uses_inline_parent_autocomplete_and_field_shortcuts() {
        let mut app = test_app();

        app.handle_key(crossterm::event::KeyCode::Char('5'))
            .expect("focus should switch");
        app.handle_key(crossterm::event::KeyCode::Char('C'))
            .expect("project editor should open");

        let editor = app
            .project_editor_view()
            .expect("project editor should be visible");
        assert!(editor.focus.name);
        assert!(editor.parent_suggestions.is_empty());

        app.handle_key(crossterm::event::KeyCode::Tab)
            .expect("focus should switch");
        let editor = app
            .project_editor_view()
            .expect("project editor should be visible");
        assert!(editor.focus.parent);

        app.handle_key(crossterm::event::KeyCode::BackTab)
            .expect("focus should switch back");
        let editor = app
            .project_editor_view()
            .expect("project editor should be visible");
        assert!(editor.focus.name);

        app.handle_key(crossterm::event::KeyCode::F(2))
            .expect("focus should jump");
        let editor = app
            .project_editor_view()
            .expect("project editor should be visible");
        assert!(editor.focus.parent);

        app.handle_key(crossterm::event::KeyCode::F(3))
            .expect("focus should jump");
        let editor = app
            .project_editor_view()
            .expect("project editor should be visible");
        assert!(editor.focus.color);

        app.handle_key(crossterm::event::KeyCode::F(4))
            .expect("focus should jump");
        let editor = app
            .project_editor_view()
            .expect("project editor should be visible");
        assert!(editor.focus.favorite);
    }

    #[test]
    fn project_editor_persists_parent_selected_from_inline_autocomplete() {
        let mut app = test_app();
        let parent = app
            .database
            .project_repository()
            .create(
                "Test Project 01",
                None,
                ProjectColor::Blue,
                false,
                Local::now(),
            )
            .expect("parent project should create");
        app.refresh_tasks().expect("tasks should refresh");

        app.handle_key(crossterm::event::KeyCode::Char('5'))
            .expect("focus should switch");
        app.handle_key(crossterm::event::KeyCode::Char('C'))
            .expect("project editor should open");

        for character in "Child Project 03 #Test".chars() {
            app.handle_key(crossterm::event::KeyCode::Char(character))
                .expect("typing should succeed");
        }

        app.handle_key(crossterm::event::KeyCode::Tab)
            .expect("tab should accept parent suggestion");
        let editor = app
            .project_editor_view()
            .expect("project editor should remain visible");
        assert_eq!(editor.parent_label.as_deref(), Some("Test Project 01"));
        app.handle_key(crossterm::event::KeyCode::Enter)
            .expect("project should be created");

        let created = app
            .screen_data
            .projects
            .iter()
            .find(|project| project.name == "Child Project 03")
            .expect("child project should exist");
        assert_eq!(created.parent_project_id, Some(parent.id));
    }

    #[test]
    fn edit_project_popup_shows_parent_in_parent_field_not_name_field() {
        let mut app = test_app();
        let parent = app
            .database
            .project_repository()
            .create("Parent A", None, ProjectColor::Blue, false, Local::now())
            .expect("parent should create");
        let child = app
            .database
            .project_repository()
            .create(
                "Child A",
                Some(parent.id),
                ProjectColor::Charcoal,
                false,
                Local::now(),
            )
            .expect("child should create");
        app.refresh_tasks().expect("tasks should refresh");
        app.selected_project_id = Some(child.id);

        app.handle_key(crossterm::event::KeyCode::Char('5'))
            .expect("focus should switch");
        app.handle_key(crossterm::event::KeyCode::Char('e'))
            .expect("edit popup should open");

        let editor = app
            .project_editor_view()
            .expect("project editor should be visible");
        assert_eq!(editor.name_value, "Child A");
        assert_eq!(editor.parent_value, "Parent A");
    }

    #[test]
    fn task_editor_accepts_project_suggestion_without_hash() {
        let mut app = test_app();
        let target = app
            .database
            .project_repository()
            .create(
                "Website Revamp",
                None,
                ProjectColor::Blue,
                false,
                Local::now(),
            )
            .expect("project should create");
        app.refresh_tasks().expect("tasks should refresh");
        app.handle_key(crossterm::event::KeyCode::Char('7'))
            .expect("focus should switch");

        app.handle_key(crossterm::event::KeyCode::Char('c'))
            .expect("popup should open");
        for character in "Refactor auth".chars() {
            app.handle_key(crossterm::event::KeyCode::Char(character))
                .expect("typing should succeed");
        }
        app.handle_key(crossterm::event::KeyCode::Enter)
            .expect("task should create");

        app.handle_key(crossterm::event::KeyCode::Char('e'))
            .expect("editor should open");
        app.handle_key(crossterm::event::KeyCode::F(2))
            .expect("project field should focus");
        let editor = app.task_editor_view().expect("editor should stay open");
        assert!(editor.focus.project);
        assert_eq!(editor.project_value, "Inbox");
        for _ in 0.."Inbox".len() {
            app.handle_key(crossterm::event::KeyCode::Backspace)
                .expect("project text should clear");
        }
        for character in "web".chars() {
            app.handle_key(crossterm::event::KeyCode::Char(character))
                .expect("typing should succeed");
        }
        let editor = app.task_editor_view().expect("editor should stay open");
        assert_eq!(editor.project_value, "web");
        assert!(!app.project_suggestions("web").is_empty());

        app.handle_key(crossterm::event::KeyCode::Enter)
            .expect("enter should accept suggestion");
        let editor = app.task_editor_view().expect("editor should stay open");
        assert_eq!(editor.project_value, "Website Revamp");
        app.handle_key(crossterm::event::KeyCode::Enter)
            .expect("enter should save");

        assert_eq!(
            app.selected_task().expect("task should exist").project_id,
            target.id
        );
    }

    #[test]
    fn project_editor_tab_after_exact_parent_match_keeps_name_field_for_inline_name_entry() {
        let mut app = test_app();
        let parent = app
            .database
            .project_repository()
            .create(
                "Child Project 01",
                None,
                ProjectColor::Blue,
                false,
                Local::now(),
            )
            .expect("parent project should create");
        app.refresh_tasks().expect("tasks should refresh");

        app.handle_key(crossterm::event::KeyCode::Char('5'))
            .expect("focus should switch");
        app.handle_key(crossterm::event::KeyCode::Char('C'))
            .expect("project editor should open");

        for character in "#Child Project 01".chars() {
            app.handle_key(crossterm::event::KeyCode::Char(character))
                .expect("typing should succeed");
        }

        app.handle_key(crossterm::event::KeyCode::Tab)
            .expect("tab should keep inline parent workflow");
        for character in " Another Project 01".chars() {
            app.handle_key(crossterm::event::KeyCode::Char(character))
                .expect("typing should keep editing name");
        }
        app.handle_key(crossterm::event::KeyCode::Enter)
            .expect("project should be created");

        let created = app
            .screen_data
            .projects
            .iter()
            .find(|project| project.name == "Another Project 01")
            .expect("child project should exist");
        assert_eq!(created.parent_project_id, Some(parent.id));
    }

    #[test]
    fn project_editor_persists_parent_when_name_is_typed_before_hash_reference() {
        let mut app = test_app();
        let parent = app
            .database
            .project_repository()
            .create(
                "Child Project 01",
                None,
                ProjectColor::Blue,
                false,
                Local::now(),
            )
            .expect("parent project should create");
        app.refresh_tasks().expect("tasks should refresh");

        app.handle_key(crossterm::event::KeyCode::Char('5'))
            .expect("focus should switch");
        app.handle_key(crossterm::event::KeyCode::Char('C'))
            .expect("project editor should open");
        for character in "Another Project 01 #Child Project 01".chars() {
            app.handle_key(crossterm::event::KeyCode::Char(character))
                .expect("typing should succeed");
        }
        app.handle_key(crossterm::event::KeyCode::Enter)
            .expect("project should be created");

        let created = app
            .screen_data
            .projects
            .iter()
            .find(|project| project.name == "Another Project 01")
            .expect("child project should exist");
        assert_eq!(created.parent_project_id, Some(parent.id));
    }

    #[test]
    fn project_editor_persists_parent_when_name_is_typed_before_hash_autocomplete() {
        let mut app = test_app();
        let parent = app
            .database
            .project_repository()
            .create(
                "Child Project 01",
                None,
                ProjectColor::Blue,
                false,
                Local::now(),
            )
            .expect("parent project should create");
        app.refresh_tasks().expect("tasks should refresh");

        app.handle_key(crossterm::event::KeyCode::Char('5'))
            .expect("focus should switch");
        app.handle_key(crossterm::event::KeyCode::Char('C'))
            .expect("project editor should open");
        for character in "Another Project 01 #Child".chars() {
            app.handle_key(crossterm::event::KeyCode::Char(character))
                .expect("typing should succeed");
        }
        app.handle_key(crossterm::event::KeyCode::Tab)
            .expect("tab should accept parent");
        app.handle_key(crossterm::event::KeyCode::Enter)
            .expect("project should be created");

        let created = app
            .screen_data
            .projects
            .iter()
            .find(|project| project.name == "Another Project 01")
            .expect("child project should exist");
        assert_eq!(created.parent_project_id, Some(parent.id));
    }

    #[test]
    fn project_editor_autoinserts_space_before_hash_reference_after_name_text() {
        let mut app = test_app();
        let parent = app
            .database
            .project_repository()
            .create(
                "Child Project 01",
                None,
                ProjectColor::Blue,
                false,
                Local::now(),
            )
            .expect("parent project should create");
        app.refresh_tasks().expect("tasks should refresh");

        app.handle_key(crossterm::event::KeyCode::Char('5'))
            .expect("focus should switch");
        app.handle_key(crossterm::event::KeyCode::Char('C'))
            .expect("project editor should open");
        for character in "Another Project 01#Child".chars() {
            app.handle_key(crossterm::event::KeyCode::Char(character))
                .expect("typing should succeed");
        }
        app.handle_key(crossterm::event::KeyCode::Tab)
            .expect("tab should accept parent");
        app.handle_key(crossterm::event::KeyCode::Enter)
            .expect("project should be created");

        let created = app
            .screen_data
            .projects
            .iter()
            .find(|project| project.name == "Another Project 01")
            .expect("child project should exist");
        assert_eq!(created.parent_project_id, Some(parent.id));
    }

    #[test]
    fn project_editor_hash_prefix_then_name_picks_exact_child_not_ancestor() {
        let mut app = test_app();
        let test_parent = app
            .database
            .project_repository()
            .create(
                "Test Project 01",
                None,
                ProjectColor::Blue,
                false,
                Local::now(),
            )
            .expect("test parent should create");
        let child = app
            .database
            .project_repository()
            .create(
                "Child Project 01",
                Some(test_parent.id),
                ProjectColor::Teal,
                false,
                Local::now(),
            )
            .expect("child should create");
        app.refresh_tasks().expect("tasks should refresh");

        app.handle_key(crossterm::event::KeyCode::Char('5'))
            .expect("focus should switch");
        app.handle_key(crossterm::event::KeyCode::Char('C'))
            .expect("project editor should open");
        for character in "#Child Project 01 Another Project 01".chars() {
            app.handle_key(crossterm::event::KeyCode::Char(character))
                .expect("typing should succeed");
        }
        app.handle_key(crossterm::event::KeyCode::Enter)
            .expect("project should be created");

        let created = app
            .screen_data
            .projects
            .iter()
            .find(|project| project.name == "Another Project 01")
            .expect("new project should exist");
        assert_eq!(created.parent_project_id, Some(child.id));
    }

    #[test]
    fn project_editor_hash_prefix_then_name_ignores_prefilled_selected_project_context() {
        let mut app = test_app();
        let test_parent = app
            .database
            .project_repository()
            .create(
                "Test Project 01",
                None,
                ProjectColor::Blue,
                false,
                Local::now(),
            )
            .expect("test parent should create");
        let child = app
            .database
            .project_repository()
            .create(
                "Child Project 01",
                Some(test_parent.id),
                ProjectColor::Teal,
                false,
                Local::now(),
            )
            .expect("child should create");
        app.refresh_tasks().expect("tasks should refresh");
        app.selected_project_id = Some(test_parent.id);

        app.handle_key(crossterm::event::KeyCode::Char('5'))
            .expect("focus should switch");
        app.handle_key(crossterm::event::KeyCode::Char('C'))
            .expect("project editor should open");
        for character in "#Child Project 01 Another Project 01".chars() {
            app.handle_key(crossterm::event::KeyCode::Char(character))
                .expect("typing should succeed");
        }
        app.handle_key(crossterm::event::KeyCode::Enter)
            .expect("project should be created");

        let created = app
            .screen_data
            .projects
            .iter()
            .find(|project| project.name == "Another Project 01")
            .expect("new project should exist");
        assert_eq!(created.parent_project_id, Some(child.id));
    }

    #[test]
    fn project_editor_create_prefills_selected_project_and_all_starts_empty() {
        let mut app = test_app();
        let parent = app
            .database
            .project_repository()
            .create(
                "Selected Parent",
                None,
                ProjectColor::Blue,
                false,
                Local::now(),
            )
            .expect("project should create");
        app.refresh_tasks().expect("tasks should refresh");

        app.handle_key(crossterm::event::KeyCode::Char('5'))
            .expect("focus should switch");

        app.selected_project_id = None;
        app.handle_key(crossterm::event::KeyCode::Char('C'))
            .expect("project editor should open");
        let editor = app
            .project_editor_view()
            .expect("project editor should be visible");
        assert_eq!(editor.name_value, "");
        app.handle_key(crossterm::event::KeyCode::Esc)
            .expect("editor should close");

        app.selected_project_id = Some(parent.id);
        app.handle_key(crossterm::event::KeyCode::Char('C'))
            .expect("project editor should open");
        let editor = app
            .project_editor_view()
            .expect("project editor should be visible");
        assert_eq!(editor.name_value, "");
        assert_eq!(editor.parent_value, "Selected Parent");
    }

    #[test]
    fn project_tree_rows_render_clear_branch_prefixes_for_nested_projects() {
        let mut app = test_app();
        let test_parent = app
            .database
            .project_repository()
            .create(
                "Test Project 01",
                None,
                ProjectColor::Blue,
                false,
                Local::now(),
            )
            .expect("test parent should create");
        let child_one = app
            .database
            .project_repository()
            .create(
                "Child Project 01",
                Some(test_parent.id),
                ProjectColor::Teal,
                false,
                Local::now(),
            )
            .expect("child 1 should create");
        let child_two = app
            .database
            .project_repository()
            .create(
                "Child Project 02",
                Some(test_parent.id),
                ProjectColor::SkyBlue,
                false,
                Local::now(),
            )
            .expect("child 2 should create");
        app.database
            .project_repository()
            .create(
                "Another Project 01",
                Some(child_one.id),
                ProjectColor::Charcoal,
                false,
                Local::now(),
            )
            .expect("nested child 1 should create");
        app.database
            .project_repository()
            .create(
                "Another Project 02",
                Some(child_two.id),
                ProjectColor::Charcoal,
                false,
                Local::now(),
            )
            .expect("nested child 2 should create");
        app.refresh_tasks().expect("tasks should refresh");

        let rows = app.project_tree_rows();
        let child_one_row = rows
            .iter()
            .find(|row| row.name == "Child Project 01")
            .expect("child 1 row should exist");
        let child_two_row = rows
            .iter()
            .find(|row| row.name == "Child Project 02")
            .expect("child 2 row should exist");
        let nested_one_row = rows
            .iter()
            .find(|row| row.name == "Another Project 01")
            .expect("nested child 1 row should exist");
        let nested_two_row = rows
            .iter()
            .find(|row| row.name == "Another Project 02")
            .expect("nested child 2 row should exist");

        assert_eq!(child_one_row.tree_prefix, "├ ");
        assert_eq!(child_two_row.tree_prefix, "└ ");
        assert_eq!(nested_one_row.tree_prefix, "│ └ ");
        assert_eq!(nested_two_row.tree_prefix, "  └ ");
    }

    #[test]
    fn project_tree_rows_render_clear_branch_prefixes_for_deeper_levels() {
        let mut app = test_app();
        let root = app
            .database
            .project_repository()
            .create("Root", None, ProjectColor::Blue, false, Local::now())
            .expect("root should create");
        let child_a = app
            .database
            .project_repository()
            .create(
                "Child A",
                Some(root.id),
                ProjectColor::Teal,
                false,
                Local::now(),
            )
            .expect("child A should create");
        let _child_b = app
            .database
            .project_repository()
            .create(
                "Child B",
                Some(root.id),
                ProjectColor::SkyBlue,
                false,
                Local::now(),
            )
            .expect("child B should create");
        let grand_a = app
            .database
            .project_repository()
            .create(
                "Grand A",
                Some(child_a.id),
                ProjectColor::Charcoal,
                false,
                Local::now(),
            )
            .expect("grand A should create");
        app.database
            .project_repository()
            .create(
                "Great A",
                Some(grand_a.id),
                ProjectColor::Grey,
                false,
                Local::now(),
            )
            .expect("great A should create");
        app.refresh_tasks().expect("tasks should refresh");

        let rows = app.project_tree_rows();
        let child_a_row = rows
            .iter()
            .find(|row| row.name == "Child A")
            .expect("child A row should exist");
        let child_b_row = rows
            .iter()
            .find(|row| row.name == "Child B")
            .expect("child B row should exist");
        let grand_a_row = rows
            .iter()
            .find(|row| row.name == "Grand A")
            .expect("grand A row should exist");
        let great_a_row = rows
            .iter()
            .find(|row| row.name == "Great A")
            .expect("great A row should exist");

        assert_eq!(child_a_row.tree_prefix, "├ ");
        assert_eq!(child_b_row.tree_prefix, "└ ");
        assert_eq!(grand_a_row.tree_prefix, "│ └ ");
        assert_eq!(great_a_row.tree_prefix, "│   └ ");
    }

    #[test]
    fn task_list_hides_completed_tasks_by_default_and_can_toggle_them() {
        let mut app = test_app();
        let repository = app.database.task_repository();
        let inbox_project_id = app.inbox_project_id();
        let now = Local::now();
        let created = repository
            .create("Completed task", inbox_project_id, None, now)
            .expect("task should create");
        repository
            .update_status(created.id, TaskStatus::Done, Some(now))
            .expect("status should update");
        app.refresh_tasks().expect("tasks should refresh");
        app.handle_key(crossterm::event::KeyCode::Char('7'))
            .expect("focus should switch");

        assert!(app.visible_tasks().is_empty());

        app.handle_key(crossterm::event::KeyCode::Char('f'))
            .expect("filter should toggle");

        let titles = app
            .visible_tasks()
            .into_iter()
            .map(|task| task.title.as_str())
            .collect::<Vec<_>>();
        assert_eq!(titles, vec!["Completed task"]);
    }

    #[test]
    fn task_list_sorts_by_due_then_title_by_default() {
        let mut app = test_app();
        let repository = app.database.task_repository();
        let inbox_project_id = app.inbox_project_id();
        let now = Local::now();
        let today = app.today();

        repository
            .create(
                "Zulu",
                inbox_project_id,
                Some(&TaskDue {
                    date: today,
                    datetime: None,
                    string: "today".to_string(),
                    is_recurring: false,
                }),
                now,
            )
            .expect("task should create");
        repository
            .create(
                "Alpha",
                inbox_project_id,
                Some(&TaskDue {
                    date: today,
                    datetime: None,
                    string: "today".to_string(),
                    is_recurring: false,
                }),
                now,
            )
            .expect("task should create");
        repository
            .create("Inbox", inbox_project_id, None, now)
            .expect("task should create");

        app.refresh_tasks().expect("tasks should refresh");

        let titles = app
            .visible_tasks()
            .into_iter()
            .map(|task| task.title.as_str())
            .collect::<Vec<_>>();
        assert_eq!(titles, vec!["Alpha", "Zulu", "Inbox"]);
    }

    #[test]
    fn task_sort_popup_applies_selected_sort() {
        let mut app = test_app();
        let repository = app.database.task_repository();
        let inbox_project_id = app.inbox_project_id();
        let now = Local::now();

        repository
            .create("Bravo", inbox_project_id, None, now)
            .expect("task should create");
        repository
            .create("Alpha", inbox_project_id, None, now)
            .expect("task should create");
        app.refresh_tasks().expect("tasks should refresh");
        app.handle_key(crossterm::event::KeyCode::Char('7'))
            .expect("focus should switch");

        app.handle_key(crossterm::event::KeyCode::Char('o'))
            .expect("sort popup should open");
        app.handle_key(crossterm::event::KeyCode::Down)
            .expect("popup selection should move");
        app.handle_key(crossterm::event::KeyCode::Down)
            .expect("popup selection should move");
        app.handle_key(crossterm::event::KeyCode::Enter)
            .expect("sort should apply");

        assert_eq!(app.task_sort_order(), TaskSortOrder::TitleAsc);
        let titles = app
            .visible_tasks()
            .into_iter()
            .map(|task| task.title.as_str())
            .collect::<Vec<_>>();
        assert_eq!(titles, vec!["Alpha", "Bravo"]);
    }

    #[test]
    fn app_requires_delete_confirmation() {
        let mut app = test_app();
        app.handle_key(crossterm::event::KeyCode::Char('7'))
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
        app.handle_key(crossterm::event::KeyCode::Char('7'))
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

        app.handle_key(crossterm::event::KeyCode::Char('7'))
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
        app.handle_key(crossterm::event::KeyCode::Char('7'))
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
        assert!(app.selected_task().is_none());

        app.handle_key(crossterm::event::KeyCode::Char('f'))
            .expect("filter should toggle");
        app.handle_key_at(crossterm::event::KeyCode::Char(' '), now)
            .expect("status should toggle");
        assert_eq!(app.screen_data.tasks[0].status, TaskStatus::Todo);
    }

    #[test]
    fn app_completes_recurring_task_and_creates_next_occurrence() {
        let mut app = test_app();
        let now = Local::now();

        app.handle_key(crossterm::event::KeyCode::Char('7'))
            .expect("focus should switch");
        app.handle_key(crossterm::event::KeyCode::Char('c'))
            .expect("popup should open");
        for character in "Review metrics every day at 9am".chars() {
            app.handle_key(crossterm::event::KeyCode::Char(character))
                .expect("typing should succeed");
        }
        app.handle_key(crossterm::event::KeyCode::Enter)
            .expect("task should be created");

        let original_task_id = app.selected_task().expect("task should exist").id;
        let original_due = app
            .selected_task()
            .expect("task should exist")
            .due
            .clone()
            .expect("due should exist");

        app.handle_key_at(crossterm::event::KeyCode::Char(' '), now)
            .expect("status should toggle");

        assert_eq!(app.screen_data.tasks.len(), 2);

        let completed_task = app
            .screen_data
            .tasks
            .iter()
            .find(|task| task.id == original_task_id)
            .expect("completed task should still exist");
        assert_eq!(completed_task.status, TaskStatus::Done);

        let next_task = app
            .screen_data
            .tasks
            .iter()
            .find(|task| task.id != original_task_id)
            .expect("next recurring task should exist");
        assert_eq!(next_task.title, "Review metrics");
        assert_eq!(next_task.status, TaskStatus::Todo);
        assert!(next_task.due.as_ref().expect("next due should exist").date > original_due.date);
        assert_eq!(app.selected_task_id, Some(next_task.id));
    }

    #[test]
    fn app_does_not_create_duplicate_recurring_successor() {
        let mut app = test_app();
        let now = Local::now();

        app.handle_key(crossterm::event::KeyCode::Char('7'))
            .expect("focus should switch");
        app.handle_key(crossterm::event::KeyCode::Char('c'))
            .expect("popup should open");
        for character in "Review metrics every day at 9am".chars() {
            app.handle_key(crossterm::event::KeyCode::Char(character))
                .expect("typing should succeed");
        }
        app.handle_key(crossterm::event::KeyCode::Enter)
            .expect("task should be created");

        let original_task_id = app.selected_task().expect("task should exist").id;

        app.handle_key_at(crossterm::event::KeyCode::Char(' '), now)
            .expect("status should toggle");
        assert_eq!(app.screen_data.tasks.len(), 2);

        app.selected_task_id = Some(original_task_id);
        app.handle_key_at(crossterm::event::KeyCode::Char(' '), now)
            .expect("status should toggle back");
        app.handle_key_at(crossterm::event::KeyCode::Char(' '), now)
            .expect("status should toggle again");

        assert_eq!(app.screen_data.tasks.len(), 2);
        let successor_count = app
            .screen_data
            .tasks
            .iter()
            .filter(|task| task.id != original_task_id && task.deleted_at.is_none())
            .count();
        assert_eq!(successor_count, 1);
    }

    #[test]
    fn app_completes_editor_created_daily_recurring_task() {
        let mut app = test_app();
        let now = Local::now();

        app.handle_key(crossterm::event::KeyCode::Char('7'))
            .expect("focus should switch");
        app.handle_key(crossterm::event::KeyCode::Char('c'))
            .expect("popup should open");
        for character in "Get this done".chars() {
            app.handle_key(crossterm::event::KeyCode::Char(character))
                .expect("typing should succeed");
        }
        app.handle_key(crossterm::event::KeyCode::Enter)
            .expect("task should be created");

        app.handle_key(crossterm::event::KeyCode::Char('e'))
            .expect("editor should open");
        app.handle_key(crossterm::event::KeyCode::F(5))
            .expect("focus should switch to recurrence");
        for character in "every day".chars() {
            app.handle_key(crossterm::event::KeyCode::Char(character))
                .expect("typing should succeed");
        }
        app.handle_key(crossterm::event::KeyCode::Enter)
            .expect("edit should submit");

        let original_task_id = app.selected_task().expect("task should exist").id;
        app.handle_key_at(crossterm::event::KeyCode::Char(' '), now)
            .expect("status should toggle");

        assert_eq!(app.screen_data.tasks.len(), 2);
        assert!(
            app.screen_data
                .tasks
                .iter()
                .any(|task| task.id != original_task_id
                    && task.title == "Get this done"
                    && task.status == TaskStatus::Todo
                    && task.due.as_ref().is_some_and(|due| due.is_recurring))
        );
    }

    #[test]
    fn app_toggles_selected_task_as_pomodoro_assignment() {
        let mut app = test_app();
        app.handle_key(crossterm::event::KeyCode::Char('7'))
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
    fn task_details_follow_focused_panel_source() {
        let mut app = test_app();
        app.handle_key(crossterm::event::KeyCode::Char('7'))
            .expect("focus should switch");
        app.handle_key(crossterm::event::KeyCode::Char('c'))
            .expect("popup should open");
        for character in "Details task".chars() {
            app.handle_key(crossterm::event::KeyCode::Char(character))
                .expect("typing should succeed");
        }
        app.handle_key(crossterm::event::KeyCode::Enter)
            .expect("task should be created");

        assert_eq!(
            app.task_details_task()
                .expect("task details should exist")
                .title,
            "Details task"
        );

        app.handle_key(crossterm::event::KeyCode::Char('1'))
            .expect("focus should switch");
        assert!(app.task_details_task().is_none());

        app.handle_key(crossterm::event::KeyCode::Char('7'))
            .expect("focus should switch");
        app.handle_key(crossterm::event::KeyCode::Char('a'))
            .expect("assignment should toggle");
        app.handle_key(crossterm::event::KeyCode::Char('1'))
            .expect("focus should switch");

        assert_eq!(
            app.task_details_task()
                .expect("assigned task should be shown")
                .title,
            "Details task"
        );
    }

    #[test]
    fn history_focus_shows_selected_session_task_details() {
        let mut app = test_app();
        let now = Local::now();

        app.handle_key(crossterm::event::KeyCode::Char('7'))
            .expect("focus should switch");
        app.handle_key(crossterm::event::KeyCode::Char('c'))
            .expect("popup should open");
        for character in "History task".chars() {
            app.handle_key(crossterm::event::KeyCode::Char(character))
                .expect("typing should succeed");
        }
        app.handle_key(crossterm::event::KeyCode::Enter)
            .expect("task should be created");
        app.handle_key(crossterm::event::KeyCode::Char('a'))
            .expect("assignment should toggle");
        app.handle_key(crossterm::event::KeyCode::Char('1'))
            .expect("focus should switch");
        app.handle_key_at(crossterm::event::KeyCode::Char('s'), now)
            .expect("timer should start");
        app.handle_key_at(
            crossterm::event::KeyCode::Char('x'),
            now + ChronoDuration::seconds(5),
        )
        .expect("timer should void");
        app.handle_key(crossterm::event::KeyCode::Char('2'))
            .expect("focus should switch");

        assert_eq!(
            app.task_details_task()
                .expect("history-linked task should be shown")
                .title,
            "History task"
        );
    }

    #[test]
    fn assigned_task_is_recorded_with_focus_session() {
        let mut app = test_app();
        let now = Local::now();
        app.handle_key(crossterm::event::KeyCode::Char('7'))
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
