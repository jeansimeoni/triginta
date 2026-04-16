use chrono::{DateTime, Local, NaiveDate, Utc};

// Tuple structs are lightweight "newtypes": this is roughly like wrapping an
// `int64_t` in a dedicated typedef, except Rust keeps it type-safe so a
// `TaskId` cannot be passed where a `PomodoroId` is expected by accident.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TaskId(pub i64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ProjectId(pub i64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TagId(pub i64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FilterId(pub i64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PomodoroId(pub i64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProjectColor {
    BerryRed,
    Red,
    Orange,
    Yellow,
    OliveGreen,
    LimeGreen,
    Green,
    MintGreen,
    Teal,
    SkyBlue,
    LightBlue,
    Blue,
    Grape,
    Violet,
    Lavender,
    Magenta,
    Salmon,
    Charcoal,
    Grey,
    Taupe,
}

impl ProjectColor {
    const ALL: [Self; 20] = [
        Self::BerryRed,
        Self::Red,
        Self::Orange,
        Self::Yellow,
        Self::OliveGreen,
        Self::LimeGreen,
        Self::Green,
        Self::MintGreen,
        Self::Teal,
        Self::SkyBlue,
        Self::LightBlue,
        Self::Blue,
        Self::Grape,
        Self::Violet,
        Self::Lavender,
        Self::Magenta,
        Self::Salmon,
        Self::Charcoal,
        Self::Grey,
        Self::Taupe,
    ];

    pub fn all() -> &'static [Self] {
        &Self::ALL
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::BerryRed => "berry_red",
            Self::Red => "red",
            Self::Orange => "orange",
            Self::Yellow => "yellow",
            Self::OliveGreen => "olive_green",
            Self::LimeGreen => "lime_green",
            Self::Green => "green",
            Self::MintGreen => "mint_green",
            Self::Teal => "teal",
            Self::SkyBlue => "sky_blue",
            Self::LightBlue => "light_blue",
            Self::Blue => "blue",
            Self::Grape => "grape",
            Self::Violet => "violet",
            Self::Lavender => "lavender",
            Self::Magenta => "magenta",
            Self::Salmon => "salmon",
            Self::Charcoal => "charcoal",
            Self::Grey => "grey",
            Self::Taupe => "taupe",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::BerryRed => "Berry Red",
            Self::Red => "Red",
            Self::Orange => "Orange",
            Self::Yellow => "Yellow",
            Self::OliveGreen => "Olive Green",
            Self::LimeGreen => "Lime Green",
            Self::Green => "Green",
            Self::MintGreen => "Mint Green",
            Self::Teal => "Teal",
            Self::SkyBlue => "Sky Blue",
            Self::LightBlue => "Light Blue",
            Self::Blue => "Blue",
            Self::Grape => "Grape",
            Self::Violet => "Violet",
            Self::Lavender => "Lavender",
            Self::Magenta => "Magenta",
            Self::Salmon => "Salmon",
            Self::Charcoal => "Charcoal",
            Self::Grey => "Grey",
            Self::Taupe => "Taupe",
        }
    }

    pub fn from_db(value: &str) -> Self {
        match value {
            "berry_red" => Self::BerryRed,
            "red" => Self::Red,
            "orange" => Self::Orange,
            "yellow" => Self::Yellow,
            "olive_green" => Self::OliveGreen,
            "lime_green" => Self::LimeGreen,
            "green" => Self::Green,
            "mint_green" => Self::MintGreen,
            "teal" => Self::Teal,
            "sky_blue" => Self::SkyBlue,
            "light_blue" => Self::LightBlue,
            "blue" => Self::Blue,
            "grape" => Self::Grape,
            "violet" => Self::Violet,
            "lavender" => Self::Lavender,
            "magenta" => Self::Magenta,
            "salmon" => Self::Salmon,
            "charcoal" => Self::Charcoal,
            "grey" => Self::Grey,
            "taupe" => Self::Taupe,
            _ => Self::Charcoal,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TagColor {
    BerryRed,
    Red,
    Orange,
    Yellow,
    OliveGreen,
    LimeGreen,
    Green,
    MintGreen,
    Teal,
    SkyBlue,
    LightBlue,
    Blue,
    Grape,
    Violet,
    Lavender,
    Magenta,
    Salmon,
    Charcoal,
    Grey,
    Taupe,
}

impl TagColor {
    const ALL: [Self; 20] = [
        Self::BerryRed,
        Self::Red,
        Self::Orange,
        Self::Yellow,
        Self::OliveGreen,
        Self::LimeGreen,
        Self::Green,
        Self::MintGreen,
        Self::Teal,
        Self::SkyBlue,
        Self::LightBlue,
        Self::Blue,
        Self::Grape,
        Self::Violet,
        Self::Lavender,
        Self::Magenta,
        Self::Salmon,
        Self::Charcoal,
        Self::Grey,
        Self::Taupe,
    ];

    pub fn all() -> &'static [Self] {
        &Self::ALL
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::BerryRed => "berry_red",
            Self::Red => "red",
            Self::Orange => "orange",
            Self::Yellow => "yellow",
            Self::OliveGreen => "olive_green",
            Self::LimeGreen => "lime_green",
            Self::Green => "green",
            Self::MintGreen => "mint_green",
            Self::Teal => "teal",
            Self::SkyBlue => "sky_blue",
            Self::LightBlue => "light_blue",
            Self::Blue => "blue",
            Self::Grape => "grape",
            Self::Violet => "violet",
            Self::Lavender => "lavender",
            Self::Magenta => "magenta",
            Self::Salmon => "salmon",
            Self::Charcoal => "charcoal",
            Self::Grey => "grey",
            Self::Taupe => "taupe",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::BerryRed => "Berry Red",
            Self::Red => "Red",
            Self::Orange => "Orange",
            Self::Yellow => "Yellow",
            Self::OliveGreen => "Olive Green",
            Self::LimeGreen => "Lime Green",
            Self::Green => "Green",
            Self::MintGreen => "Mint Green",
            Self::Teal => "Teal",
            Self::SkyBlue => "Sky Blue",
            Self::LightBlue => "Light Blue",
            Self::Blue => "Blue",
            Self::Grape => "Grape",
            Self::Violet => "Violet",
            Self::Lavender => "Lavender",
            Self::Magenta => "Magenta",
            Self::Salmon => "Salmon",
            Self::Charcoal => "Charcoal",
            Self::Grey => "Grey",
            Self::Taupe => "Taupe",
        }
    }

    pub fn from_db(value: &str) -> Self {
        match value {
            "berry_red" => Self::BerryRed,
            "red" => Self::Red,
            "orange" => Self::Orange,
            "yellow" => Self::Yellow,
            "olive_green" => Self::OliveGreen,
            "lime_green" => Self::LimeGreen,
            "green" => Self::Green,
            "mint_green" => Self::MintGreen,
            "teal" => Self::Teal,
            "sky_blue" => Self::SkyBlue,
            "light_blue" => Self::LightBlue,
            "blue" => Self::Blue,
            "grape" => Self::Grape,
            "violet" => Self::Violet,
            "lavender" => Self::Lavender,
            "magenta" => Self::Magenta,
            "salmon" => Self::Salmon,
            "grey" => Self::Grey,
            "taupe" => Self::Taupe,
            _ => Self::Charcoal,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FilterColor {
    BerryRed,
    Red,
    Orange,
    Yellow,
    OliveGreen,
    LimeGreen,
    Green,
    MintGreen,
    Teal,
    SkyBlue,
    LightBlue,
    Blue,
    Grape,
    Violet,
    Lavender,
    Magenta,
    Salmon,
    Charcoal,
    Grey,
    Taupe,
}

impl FilterColor {
    const ALL: [Self; 20] = [
        Self::BerryRed,
        Self::Red,
        Self::Orange,
        Self::Yellow,
        Self::OliveGreen,
        Self::LimeGreen,
        Self::Green,
        Self::MintGreen,
        Self::Teal,
        Self::SkyBlue,
        Self::LightBlue,
        Self::Blue,
        Self::Grape,
        Self::Violet,
        Self::Lavender,
        Self::Magenta,
        Self::Salmon,
        Self::Charcoal,
        Self::Grey,
        Self::Taupe,
    ];

    pub fn all() -> &'static [Self] {
        &Self::ALL
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::BerryRed => "berry_red",
            Self::Red => "red",
            Self::Orange => "orange",
            Self::Yellow => "yellow",
            Self::OliveGreen => "olive_green",
            Self::LimeGreen => "lime_green",
            Self::Green => "green",
            Self::MintGreen => "mint_green",
            Self::Teal => "teal",
            Self::SkyBlue => "sky_blue",
            Self::LightBlue => "light_blue",
            Self::Blue => "blue",
            Self::Grape => "grape",
            Self::Violet => "violet",
            Self::Lavender => "lavender",
            Self::Magenta => "magenta",
            Self::Salmon => "salmon",
            Self::Charcoal => "charcoal",
            Self::Grey => "grey",
            Self::Taupe => "taupe",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::BerryRed => "Berry Red",
            Self::Red => "Red",
            Self::Orange => "Orange",
            Self::Yellow => "Yellow",
            Self::OliveGreen => "Olive Green",
            Self::LimeGreen => "Lime Green",
            Self::Green => "Green",
            Self::MintGreen => "Mint Green",
            Self::Teal => "Teal",
            Self::SkyBlue => "Sky Blue",
            Self::LightBlue => "Light Blue",
            Self::Blue => "Blue",
            Self::Grape => "Grape",
            Self::Violet => "Violet",
            Self::Lavender => "Lavender",
            Self::Magenta => "Magenta",
            Self::Salmon => "Salmon",
            Self::Charcoal => "Charcoal",
            Self::Grey => "Grey",
            Self::Taupe => "Taupe",
        }
    }

    pub fn from_db(value: &str) -> Self {
        match value {
            "berry_red" => Self::BerryRed,
            "red" => Self::Red,
            "orange" => Self::Orange,
            "yellow" => Self::Yellow,
            "olive_green" => Self::OliveGreen,
            "lime_green" => Self::LimeGreen,
            "green" => Self::Green,
            "mint_green" => Self::MintGreen,
            "teal" => Self::Teal,
            "sky_blue" => Self::SkyBlue,
            "light_blue" => Self::LightBlue,
            "blue" => Self::Blue,
            "grape" => Self::Grape,
            "violet" => Self::Violet,
            "lavender" => Self::Lavender,
            "magenta" => Self::Magenta,
            "salmon" => Self::Salmon,
            "charcoal" => Self::Charcoal,
            "grey" => Self::Grey,
            "taupe" => Self::Taupe,
            _ => Self::Charcoal,
        }
    }
}

// Enums in Rust are tagged unions built into the language.
// This is much closer to "an enum plus guaranteed exhaustive handling" than a
// plain C enum, because `match` must cover every variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskStatus {
    Todo,
    Done,
}

impl TaskStatus {
    pub fn as_str(&self) -> &'static str {
        // `&'static str` is a borrowed reference to string data baked into the
        // binary, similar in spirit to returning a pointer to a string literal
        // in C. No allocation happens here.
        match self {
            Self::Todo => "todo",
            Self::Done => "done",
        }
    }

    pub fn from_db(value: &str) -> Self {
        // `&str` is a borrowed string slice, not an owning buffer like
        // `String`. This function reads the input and returns an owned enum
        // value, so there are no lifetime issues for the caller to manage.
        match value {
            "done" => Self::Done,
            _ => Self::Todo,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TaskPriority {
    P1,
    P2,
    P3,
    P4,
}

impl TaskPriority {
    pub fn level(self) -> u8 {
        match self {
            Self::P1 => 1,
            Self::P2 => 2,
            Self::P3 => 3,
            Self::P4 => 4,
        }
    }

    pub fn from_level(level: u8) -> Self {
        match level {
            1 => Self::P1,
            2 => Self::P2,
            3 => Self::P3,
            _ => Self::P4,
        }
    }

    pub fn from_db(value: i64) -> Self {
        match value {
            1 => Self::P1,
            2 => Self::P2,
            3 => Self::P3,
            4 => Self::P4,
            _ => Self::P4,
        }
    }

    pub fn to_db(self) -> i64 {
        i64::from(self.level())
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::P1 => "P1",
            Self::P2 => "P2",
            Self::P3 => "P3",
            Self::P4 => "P4",
        }
    }
}

impl Default for TaskPriority {
    fn default() -> Self {
        Self::P4
    }
}

// These structs are plain data carriers, similar to C structs, but ownership
// is explicit per field. For example `title: String` means the `Task` owns the
// heap-allocated text and will free it automatically when dropped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskDue {
    pub date: NaiveDate,
    pub datetime: Option<DateTime<Utc>>,
    pub timezone: Option<String>,
    pub string: String,
    pub is_recurring: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Task {
    pub id: TaskId,
    pub project_id: ProjectId,
    pub parent_task_id: Option<TaskId>,
    pub child_order: i64,
    pub title: String,
    pub description: String,
    pub status: TaskStatus,
    pub priority: TaskPriority,
    pub created_at: DateTime<Local>,
    pub completed_at: Option<DateTime<Local>>,
    pub deleted_at: Option<DateTime<Local>>,
    pub due: Option<TaskDue>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskUpdate {
    pub title: String,
    pub description: String,
    pub project_id: ProjectId,
    pub parent_task_id: Option<TaskId>,
    pub priority: TaskPriority,
    pub due: Option<TaskDue>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Project {
    pub id: ProjectId,
    pub name: String,
    pub parent_project_id: Option<ProjectId>,
    pub color: ProjectColor,
    pub is_favorite: bool,
    pub is_inbox: bool,
    pub child_order: i64,
    pub created_at: DateTime<Local>,
    pub deleted_at: Option<DateTime<Local>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectUpdate {
    pub name: String,
    pub parent_project_id: Option<ProjectId>,
    pub color: ProjectColor,
    pub is_favorite: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tag {
    pub id: TagId,
    pub name: String,
    pub color: TagColor,
    pub is_favorite: bool,
    pub item_order: i64,
    pub created_at: DateTime<Local>,
    pub deleted_at: Option<DateTime<Local>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagUpdate {
    pub name: String,
    pub color: TagColor,
    pub is_favorite: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Filter {
    pub id: FilterId,
    pub name: String,
    pub query: String,
    pub color: FilterColor,
    pub is_favorite: bool,
    pub item_order: i64,
    pub created_at: DateTime<Local>,
    pub deleted_at: Option<DateTime<Local>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilterUpdate {
    pub name: String,
    pub query: String,
    pub color: FilterColor,
    pub is_favorite: bool,
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
    pub notes: String,
    pub kind: SessionKind,
    pub outcome: SessionOutcome,
    pub next_break_kind: Option<SessionKind>,
    pub started_at: DateTime<Local>,
    pub ended_at: DateTime<Local>,
    pub duration_seconds: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DayHistorySummary {
    pub day: chrono::NaiveDate,
    pub completed_sessions: usize,
    pub voided_sessions: usize,
    pub focus_seconds: u32,
    pub break_seconds: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FocusDaySummary {
    pub day: chrono::NaiveDate,
    pub focus_seconds: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FocusHourSummary {
    pub hour: u8,
    pub focus_seconds: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HistoryStats {
    pub total_sessions: usize,
    pub total_minutes: u32,
    pub total_work_seconds: u32,
    pub total_break_seconds: u32,
    pub completed_tasks: usize,
}
