use chrono::{DateTime, Local};

// Tuple structs are lightweight "newtypes": this is roughly like wrapping an
// `int64_t` in a dedicated typedef, except Rust keeps it type-safe so a
// `TaskId` cannot be passed where a `PomodoroId` is expected by accident.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TaskId(pub i64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PomodoroId(pub i64);

// Enums in Rust are tagged unions built into the language.
// This is much closer to "an enum plus guaranteed exhaustive handling" than a
// plain C enum, because `match` must cover every variant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskStatus {
    Todo,
    InProgress,
    Done,
}

impl TaskStatus {
    pub fn as_str(&self) -> &'static str {
        // `&'static str` is a borrowed reference to string data baked into the
        // binary, similar in spirit to returning a pointer to a string literal
        // in C. No allocation happens here.
        match self {
            Self::Todo => "todo",
            Self::InProgress => "in_progress",
            Self::Done => "done",
        }
    }

    pub fn from_db(value: &str) -> Self {
        // `&str` is a borrowed string slice, not an owning buffer like
        // `String`. This function reads the input and returns an owned enum
        // value, so there are no lifetime issues for the caller to manage.
        match value {
            "in_progress" => Self::InProgress,
            "done" => Self::Done,
            _ => Self::Todo,
        }
    }
}

// These structs are plain data carriers, similar to C structs, but ownership
// is explicit per field. For example `title: String` means the `Task` owns the
// heap-allocated text and will free it automatically when dropped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Task {
    pub id: TaskId,
    pub title: String,
    pub status: TaskStatus,
    pub created_at: DateTime<Local>,
    pub completed_at: Option<DateTime<Local>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionKind {
    Focus,
    ShortBreak,
    LongBreak,
}

impl SessionKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Focus => "focus",
            Self::ShortBreak => "short_break",
            Self::LongBreak => "long_break",
        }
    }

    pub fn from_db(value: &str) -> Self {
        match value {
            "short_break" => Self::ShortBreak,
            "long_break" => Self::LongBreak,
            "work" | "focus" => Self::Focus,
            _ => Self::Focus,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionOutcome {
    Completed,
    Voided,
}

impl SessionOutcome {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Voided => "voided",
        }
    }

    pub fn from_db(value: &str) -> Self {
        match value {
            "voided" => Self::Voided,
            _ => Self::Completed,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionEntry {
    pub id: i64,
    pub task_id: Option<TaskId>,
    pub kind: SessionKind,
    pub outcome: SessionOutcome,
    pub next_break_kind: Option<SessionKind>,
    pub started_at: DateTime<Local>,
    pub ended_at: DateTime<Local>,
    pub duration_seconds: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HistoryStats {
    pub total_sessions: usize,
    pub total_minutes: u32,
    pub total_work_seconds: u32,
    pub total_break_seconds: u32,
    pub completed_tasks: usize,
}
