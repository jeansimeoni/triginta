use chrono::{DateTime, Local};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TaskId(pub i64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PomodoroId(pub i64);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskStatus {
    Todo,
    InProgress,
    Done,
}

impl TaskStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Todo => "todo",
            Self::InProgress => "in_progress",
            Self::Done => "done",
        }
    }

    pub fn from_db(value: &str) -> Self {
        match value {
            "in_progress" => Self::InProgress,
            "done" => Self::Done,
            _ => Self::Todo,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Task {
    pub id: TaskId,
    pub title: String,
    pub status: TaskStatus,
    pub created_at: DateTime<Local>,
    pub completed_at: Option<DateTime<Local>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PomodoroSession {
    pub id: PomodoroId,
    pub task_id: Option<TaskId>,
    pub started_at: DateTime<Local>,
    pub ended_at: Option<DateTime<Local>>,
    pub duration_minutes: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HistoryStats {
    pub total_sessions: usize,
    pub total_minutes: u32,
    pub completed_tasks: usize,
}
