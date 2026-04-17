use std::path::Path;

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Local, NaiveDate, Utc};
use rusqlite::{Connection, OptionalExtension, params};

use crate::domain::{
    DayHistorySummary, Filter, FilterColor, FilterId, FilterUpdate, FocusDaySummary,
    FocusHourSummary, HistoryStats, Project, ProjectColor, ProjectId, ProjectUpdate, Section,
    SectionId, SectionUpdate, SessionEntry, SessionKind, SessionOutcome, Tag, TagColor, TagId,
    TagUpdate, Task, TaskDue, TaskId, TaskPriority, TaskStatus, TaskUpdate,
};

// Keeping the schema as a string literal makes bootstrap simple for this early
// vertical slice. `execute_batch` sends the whole script to SQLite at once,
// which is similar to feeding a schema file to sqlite3 in a C program.
const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS projects (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL,
    parent_project_id INTEGER,
    color TEXT NOT NULL DEFAULT 'charcoal',
    is_favorite INTEGER NOT NULL DEFAULT 0,
    is_inbox INTEGER NOT NULL DEFAULT 0,
    child_order INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    synced_at TEXT,
    todoist_id TEXT,
    deleted_at TEXT,
    FOREIGN KEY(parent_project_id) REFERENCES projects(id)
);

CREATE TABLE IF NOT EXISTS tasks (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id INTEGER,
    section_id INTEGER,
    parent_task_id INTEGER,
    child_order INTEGER NOT NULL DEFAULT 0,
    title TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    status TEXT NOT NULL DEFAULT 'todo',
    priority INTEGER NOT NULL DEFAULT 4,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    synced_at TEXT,
    todoist_id TEXT,
    completed_at TEXT,
    deleted_at TEXT,
    due_date TEXT,
    due_datetime_utc TEXT,
    due_timezone TEXT,
    due_string TEXT,
    due_is_recurring INTEGER NOT NULL DEFAULT 0,
    FOREIGN KEY(project_id) REFERENCES projects(id),
    FOREIGN KEY(section_id) REFERENCES sections(id),
    FOREIGN KEY(parent_task_id) REFERENCES tasks(id)
);

CREATE TABLE IF NOT EXISTS sections (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id INTEGER NOT NULL,
    name TEXT NOT NULL,
    section_order INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    synced_at TEXT,
    todoist_id TEXT,
    deleted_at TEXT,
    FOREIGN KEY(project_id) REFERENCES projects(id)
);

CREATE TABLE IF NOT EXISTS tags (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL,
    color TEXT NOT NULL DEFAULT 'charcoal',
    is_favorite INTEGER NOT NULL DEFAULT 0,
    item_order INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    synced_at TEXT,
    todoist_id TEXT,
    deleted_at TEXT
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_tags_name_active
    ON tags(lower(name))
    WHERE deleted_at IS NULL;

CREATE TABLE IF NOT EXISTS filters (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL,
    query TEXT NOT NULL,
    color TEXT NOT NULL DEFAULT 'charcoal',
    is_favorite INTEGER NOT NULL DEFAULT 0,
    item_order INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    synced_at TEXT,
    todoist_id TEXT,
    deleted_at TEXT
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_filters_name_active
    ON filters(lower(name))
    WHERE deleted_at IS NULL;

CREATE TABLE IF NOT EXISTS task_tags (
    task_id INTEGER NOT NULL,
    tag_id INTEGER NOT NULL,
    PRIMARY KEY (task_id, tag_id),
    FOREIGN KEY(task_id) REFERENCES tasks(id),
    FOREIGN KEY(tag_id) REFERENCES tags(id)
);

CREATE TABLE IF NOT EXISTS pomodoros (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id INTEGER,
    started_at TEXT NOT NULL,
    ended_at TEXT,
    duration_minutes INTEGER NOT NULL,
    FOREIGN KEY(task_id) REFERENCES tasks(id)
);

CREATE TABLE IF NOT EXISTS session_history (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id INTEGER,
    notes TEXT NOT NULL DEFAULT '',
    kind TEXT NOT NULL,
    outcome TEXT NOT NULL,
    next_break_kind TEXT,
    started_at TEXT NOT NULL,
    ended_at TEXT NOT NULL,
    duration_seconds INTEGER NOT NULL,
    FOREIGN KEY(task_id) REFERENCES tasks(id)
);

CREATE TABLE IF NOT EXISTS app_metadata (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS sync_state (
    provider TEXT PRIMARY KEY,
    sync_token TEXT,
    last_synced_at TEXT,
    last_status TEXT,
    last_error TEXT,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS sync_outbox (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    provider TEXT NOT NULL,
    entity_type TEXT NOT NULL,
    entity_local_id INTEGER NOT NULL,
    op_kind TEXT NOT NULL,
    payload TEXT NOT NULL,
    op_timestamp_utc TEXT NOT NULL,
    attempts INTEGER NOT NULL DEFAULT 0,
    last_error TEXT,
    error_code TEXT,
    next_attempt_at TEXT,
    last_attempt_at TEXT,
    created_at TEXT NOT NULL
);
"#;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncStateRecord {
    pub provider: String,
    pub sync_token: Option<String>,
    pub last_synced_at: Option<String>,
    pub last_status: Option<String>,
    pub last_error: Option<String>,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncOutboxEntry {
    pub id: i64,
    pub provider: String,
    pub entity_type: String,
    pub entity_local_id: i64,
    pub op_kind: String,
    pub payload: String,
    pub op_timestamp_utc: String,
    pub attempts: i64,
    pub last_error: Option<String>,
    pub error_code: Option<String>,
    pub next_attempt_at: Option<String>,
    pub last_attempt_at: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncTaskSnapshot {
    pub local_id: i64,
    pub todoist_id: Option<String>,
    pub project_todoist_id: Option<String>,
    pub project_is_inbox: bool,
    pub section_todoist_id: Option<String>,
    pub parent_todoist_id: Option<String>,
    pub title: String,
    pub description: String,
    pub priority: i64,
    pub due_date: Option<NaiveDate>,
    pub due_datetime_utc: Option<String>,
    pub due_timezone: Option<String>,
    pub due_string: Option<String>,
    pub labels: Vec<String>,
    pub deleted_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncProjectSnapshot {
    pub local_id: i64,
    pub todoist_id: Option<String>,
    pub parent_todoist_id: Option<String>,
    pub has_parent_project: bool,
    pub name: String,
    pub color: String,
    pub is_favorite: bool,
    pub deleted_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncSectionSnapshot {
    pub local_id: i64,
    pub todoist_id: Option<String>,
    pub project_todoist_id: Option<String>,
    pub name: String,
    pub deleted_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncTagSnapshot {
    pub local_id: i64,
    pub todoist_id: Option<String>,
    pub name: String,
    pub color: String,
    pub is_favorite: bool,
    pub deleted_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncFilterSnapshot {
    pub local_id: i64,
    pub todoist_id: Option<String>,
    pub name: String,
    pub query: String,
    pub color: String,
    pub is_favorite: bool,
    pub deleted_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncEntitySnapshot {
    Task(SyncTaskSnapshot),
    Project(SyncProjectSnapshot),
    Section(SyncSectionSnapshot),
    Tag(SyncTagSnapshot),
    Filter(SyncFilterSnapshot),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncApplyOutcome {
    Created,
    Updated,
    Skipped,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteProjectRecord {
    pub todoist_id: String,
    pub parent_todoist_id: Option<String>,
    pub name: String,
    pub color: String,
    pub is_favorite: bool,
    pub is_inbox: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteSectionRecord {
    pub todoist_id: String,
    pub project_todoist_id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteTagRecord {
    pub todoist_id: String,
    pub name: String,
    pub color: String,
    pub is_favorite: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteFilterRecord {
    pub todoist_id: String,
    pub name: String,
    pub query: String,
    pub color: String,
    pub is_favorite: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteTaskRecord {
    pub todoist_id: String,
    pub project_todoist_id: Option<String>,
    pub section_todoist_id: Option<String>,
    pub parent_todoist_id: Option<String>,
    pub content: String,
    pub description: String,
    pub priority: i64,
    pub labels: Vec<String>,
    pub due_date: Option<NaiveDate>,
    pub due_datetime_utc: Option<String>,
    pub due_timezone: Option<String>,
    pub due_string: Option<String>,
    pub due_is_recurring: bool,
    pub completed_at: Option<String>,
}

// These traits define the storage-facing API the rest of the app relies on.
// This separation matters because it keeps the higher layers talking in domain
// terms (`Task`, `PomodoroSession`) instead of raw SQL concepts.
pub trait TaskRepository {
    fn list_all(&self) -> Result<Vec<Task>>;
    fn create(
        &self,
        title: &str,
        project_id: ProjectId,
        due: Option<&TaskDue>,
        now: DateTime<Local>,
    ) -> Result<Task>;
    fn update(&self, task_id: TaskId, update: &TaskUpdate) -> Result<Task>;
    fn update_status(
        &self,
        task_id: TaskId,
        status: TaskStatus,
        completed_at: Option<DateTime<Local>>,
    ) -> Result<Task>;
    fn move_within_parent(&self, task_id: TaskId, direction: isize) -> Result<()>;
    fn delete(&self, task_id: TaskId) -> Result<()>;
}

pub trait ProjectRepository {
    fn list_all(&self) -> Result<Vec<Project>>;
    fn inbox_project_id(&self) -> Result<ProjectId>;
    fn create(
        &self,
        name: &str,
        parent_project_id: Option<ProjectId>,
        color: ProjectColor,
        is_favorite: bool,
        now: DateTime<Local>,
    ) -> Result<Project>;
    fn update(&self, project_id: ProjectId, update: &ProjectUpdate) -> Result<Project>;
    fn move_within_parent(&self, project_id: ProjectId, direction: isize) -> Result<()>;
    fn delete(&self, project_id: ProjectId, now: DateTime<Local>) -> Result<()>;
}

pub trait SectionRepository {
    fn list_all(&self) -> Result<Vec<Section>>;
    fn create(&self, project_id: ProjectId, name: &str, now: DateTime<Local>) -> Result<Section>;
    fn update(&self, section_id: SectionId, update: &SectionUpdate) -> Result<Section>;
    fn move_within_project(&self, section_id: SectionId, direction: isize) -> Result<()>;
    fn delete(&self, section_id: SectionId, now: DateTime<Local>) -> Result<()>;
}

pub trait TagRepository {
    fn list_all(&self) -> Result<Vec<Tag>>;
    fn create(
        &self,
        name: &str,
        color: TagColor,
        is_favorite: bool,
        now: DateTime<Local>,
    ) -> Result<Tag>;
    fn update(&self, tag_id: TagId, update: &TagUpdate) -> Result<Tag>;
    fn move_within_list(&self, tag_id: TagId, direction: isize) -> Result<()>;
    fn delete(&self, tag_id: TagId, now: DateTime<Local>) -> Result<()>;
    fn list_task_tag_links(&self) -> Result<Vec<(TaskId, TagId)>>;
    fn replace_task_tags(&self, task_id: TaskId, tag_ids: &[TagId]) -> Result<()>;
}

pub trait FilterRepository {
    fn list_all(&self) -> Result<Vec<Filter>>;
    fn create(
        &self,
        name: &str,
        query: &str,
        color: FilterColor,
        is_favorite: bool,
        now: DateTime<Local>,
    ) -> Result<Filter>;
    fn update(&self, filter_id: FilterId, update: &FilterUpdate) -> Result<Filter>;
    fn move_within_list(&self, filter_id: FilterId, direction: isize) -> Result<()>;
    fn delete(&self, filter_id: FilterId, now: DateTime<Local>) -> Result<()>;
}

pub trait PomodoroRepository {
    fn list_day(
        &self,
        started_at: DateTime<Local>,
        ended_at: DateTime<Local>,
    ) -> Result<Vec<SessionEntry>>;
    fn stats_for_day(
        &self,
        started_at: DateTime<Local>,
        ended_at: DateTime<Local>,
    ) -> Result<HistoryStats>;
    fn summarize_days(
        &self,
        started_at: DateTime<Local>,
        ended_at: DateTime<Local>,
    ) -> Result<Vec<DayHistorySummary>>;
    fn summarize_completed_focus_days(
        &self,
        started_at: DateTime<Local>,
        ended_at: DateTime<Local>,
    ) -> Result<Vec<FocusDaySummary>>;
    fn summarize_completed_focus_hours(
        &self,
        started_at: DateTime<Local>,
        ended_at: DateTime<Local>,
    ) -> Result<Vec<FocusHourSummary>>;
    fn create(
        &self,
        task_id: Option<TaskId>,
        notes: &str,
        next_break_kind: Option<SessionKind>,
        started_at: DateTime<Local>,
        ended_at: DateTime<Local>,
        duration_minutes: u32,
    ) -> Result<SessionEntry>;
    fn record_session_entry(
        &self,
        task_id: Option<TaskId>,
        notes: &str,
        kind: SessionKind,
        outcome: SessionOutcome,
        next_break_kind: Option<SessionKind>,
        started_at: DateTime<Local>,
        ended_at: DateTime<Local>,
        duration_seconds: u32,
    ) -> Result<SessionEntry>;
    fn update_session_task(&self, session_id: i64, task_id: Option<TaskId>) -> Result<()>;
    fn update_session_notes(&self, session_id: i64, notes: &str) -> Result<()>;
}

pub trait SyncRepository {
    fn get_state(&self, provider: &str) -> Result<Option<SyncStateRecord>>;
    fn upsert_state(&self, state: &SyncStateRecord) -> Result<()>;
    fn list_outbox(&self, provider: &str, limit: i64) -> Result<Vec<SyncOutboxEntry>>;
    fn list_ready_outbox(
        &self,
        provider: &str,
        limit: i64,
        ready_before_utc: &str,
    ) -> Result<Vec<SyncOutboxEntry>>;
    fn enqueue_outbox(
        &self,
        provider: &str,
        entity_type: &str,
        entity_local_id: i64,
        op_kind: &str,
        payload: &str,
        op_timestamp_utc: &str,
    ) -> Result<i64>;
    fn enqueue_bootstrap_outbox(&self, provider: &str, op_timestamp_utc: &str) -> Result<usize>;
    fn mark_outbox_delivered(&self, entry_id: i64) -> Result<()>;
    fn mark_outbox_failed(
        &self,
        entry_id: i64,
        error: &str,
        error_code: Option<&str>,
        next_attempt_at: Option<&str>,
        last_attempt_at: &str,
    ) -> Result<()>;
    fn load_entity_snapshot(
        &self,
        entity_type: &str,
        entity_local_id: i64,
    ) -> Result<Option<SyncEntitySnapshot>>;
    fn set_entity_todoist_id(
        &self,
        entity_type: &str,
        entity_local_id: i64,
        todoist_id: &str,
        synced_at_utc: &str,
    ) -> Result<()>;
    fn mark_entity_synced(
        &self,
        entity_type: &str,
        entity_local_id: i64,
        synced_at_utc: &str,
    ) -> Result<()>;
    fn apply_remote_project(
        &self,
        remote: &RemoteProjectRecord,
        synced_at_utc: &str,
        dry_run: bool,
    ) -> Result<SyncApplyOutcome>;
    fn apply_remote_section(
        &self,
        remote: &RemoteSectionRecord,
        synced_at_utc: &str,
        dry_run: bool,
    ) -> Result<SyncApplyOutcome>;
    fn apply_remote_tag(
        &self,
        remote: &RemoteTagRecord,
        synced_at_utc: &str,
        dry_run: bool,
    ) -> Result<SyncApplyOutcome>;
    fn apply_remote_filter(
        &self,
        remote: &RemoteFilterRecord,
        synced_at_utc: &str,
        dry_run: bool,
    ) -> Result<SyncApplyOutcome>;
    fn apply_remote_task(
        &self,
        remote: &RemoteTaskRecord,
        synced_at_utc: &str,
        dry_run: bool,
    ) -> Result<SyncApplyOutcome>;
}

#[derive(Debug)]
pub struct Database {
    connection: Connection,
}

impl Database {
    pub fn open(path: &Path) -> Result<Self> {
        // `Connection` is owned by `Database`, so Rust guarantees the SQLite
        // handle lives at least as long as any repository borrowed from it.
        let connection = Connection::open(path)
            .with_context(|| format!("failed to open database at {}", path.display()))?;
        let database = Self { connection };
        database.initialize()?;
        Ok(database)
    }

    pub fn open_in_memory() -> Result<Self> {
        let connection =
            Connection::open_in_memory().context("failed to open in-memory database")?;
        let database = Self { connection };
        database.initialize()?;
        Ok(database)
    }

    fn initialize(&self) -> Result<()> {
        // `&self` means initialization can use the connection without taking
        // ownership of the `Database`. The caller still owns the database after
        // this method returns.
        self.connection
            .execute_batch(SCHEMA)
            .context("failed to initialize database schema")?;
        self.ensure_tasks_column("deleted_at", "ALTER TABLE tasks ADD COLUMN deleted_at TEXT")?;
        self.ensure_tasks_column(
            "project_id",
            "ALTER TABLE tasks ADD COLUMN project_id INTEGER",
        )?;
        self.ensure_tasks_column("due_date", "ALTER TABLE tasks ADD COLUMN due_date TEXT")?;
        self.ensure_tasks_column(
            "due_datetime_utc",
            "ALTER TABLE tasks ADD COLUMN due_datetime_utc TEXT",
        )?;
        self.ensure_tasks_column(
            "due_timezone",
            "ALTER TABLE tasks ADD COLUMN due_timezone TEXT",
        )?;
        self.ensure_tasks_column("due_string", "ALTER TABLE tasks ADD COLUMN due_string TEXT")?;
        self.ensure_tasks_column(
            "due_is_recurring",
            "ALTER TABLE tasks ADD COLUMN due_is_recurring INTEGER NOT NULL DEFAULT 0",
        )?;
        self.ensure_tasks_column(
            "priority",
            "ALTER TABLE tasks ADD COLUMN priority INTEGER NOT NULL DEFAULT 4",
        )?;
        self.ensure_tasks_column(
            "description",
            "ALTER TABLE tasks ADD COLUMN description TEXT NOT NULL DEFAULT ''",
        )?;
        self.ensure_tasks_column(
            "parent_task_id",
            "ALTER TABLE tasks ADD COLUMN parent_task_id INTEGER",
        )?;
        self.ensure_tasks_column(
            "child_order",
            "ALTER TABLE tasks ADD COLUMN child_order INTEGER NOT NULL DEFAULT 0",
        )?;
        self.ensure_tasks_column(
            "section_id",
            "ALTER TABLE tasks ADD COLUMN section_id INTEGER",
        )?;
        self.ensure_tasks_column(
            "updated_at",
            "ALTER TABLE tasks ADD COLUMN updated_at TEXT NOT NULL DEFAULT ''",
        )?;
        self.ensure_tasks_column("synced_at", "ALTER TABLE tasks ADD COLUMN synced_at TEXT")?;
        self.ensure_tasks_column("todoist_id", "ALTER TABLE tasks ADD COLUMN todoist_id TEXT")?;
        self.ensure_projects_column(
            "updated_at",
            "ALTER TABLE projects ADD COLUMN updated_at TEXT NOT NULL DEFAULT ''",
        )?;
        self.ensure_projects_column(
            "synced_at",
            "ALTER TABLE projects ADD COLUMN synced_at TEXT",
        )?;
        self.ensure_projects_column(
            "todoist_id",
            "ALTER TABLE projects ADD COLUMN todoist_id TEXT",
        )?;
        self.ensure_sections_table()?;
        self.ensure_sections_column(
            "updated_at",
            "ALTER TABLE sections ADD COLUMN updated_at TEXT NOT NULL DEFAULT ''",
        )?;
        self.ensure_sections_column(
            "synced_at",
            "ALTER TABLE sections ADD COLUMN synced_at TEXT",
        )?;
        self.ensure_sections_column(
            "todoist_id",
            "ALTER TABLE sections ADD COLUMN todoist_id TEXT",
        )?;
        self.ensure_tags_column(
            "updated_at",
            "ALTER TABLE tags ADD COLUMN updated_at TEXT NOT NULL DEFAULT ''",
        )?;
        self.ensure_tags_column("synced_at", "ALTER TABLE tags ADD COLUMN synced_at TEXT")?;
        self.ensure_tags_column("todoist_id", "ALTER TABLE tags ADD COLUMN todoist_id TEXT")?;
        self.ensure_filters_column(
            "updated_at",
            "ALTER TABLE filters ADD COLUMN updated_at TEXT NOT NULL DEFAULT ''",
        )?;
        self.ensure_filters_column("synced_at", "ALTER TABLE filters ADD COLUMN synced_at TEXT")?;
        self.ensure_filters_column(
            "todoist_id",
            "ALTER TABLE filters ADD COLUMN todoist_id TEXT",
        )?;
        self.ensure_sync_tables()?;
        self.ensure_sync_outbox_column(
            "error_code",
            "ALTER TABLE sync_outbox ADD COLUMN error_code TEXT",
        )?;
        self.ensure_sync_outbox_column(
            "next_attempt_at",
            "ALTER TABLE sync_outbox ADD COLUMN next_attempt_at TEXT",
        )?;
        self.ensure_sync_outbox_column(
            "last_attempt_at",
            "ALTER TABLE sync_outbox ADD COLUMN last_attempt_at TEXT",
        )?;
        self.backfill_sync_timestamps()?;
        self.ensure_task_indexes()?;
        self.ensure_sync_indexes()?;
        self.ensure_session_history_column(
            "notes",
            "ALTER TABLE session_history ADD COLUMN notes TEXT NOT NULL DEFAULT ''",
        )?;
        self.connection
            .execute(
                "INSERT OR IGNORE INTO app_metadata(key, value) VALUES (?1, ?2)",
                params!["schema_version", "1"],
            )
            .context("failed to initialize app metadata")?;
        let inbox_project_id = self.ensure_inbox_project()?;
        self.assign_tasks_to_inbox(inbox_project_id)?;
        self.normalize_task_child_order()?;
        Ok(())
    }

    fn ensure_tasks_column(&self, column_name: &str, alter_sql: &str) -> Result<()> {
        self.ensure_table_column("tasks", column_name, alter_sql)
    }

    fn ensure_projects_column(&self, column_name: &str, alter_sql: &str) -> Result<()> {
        self.ensure_table_column("projects", column_name, alter_sql)
    }

    fn ensure_sections_column(&self, column_name: &str, alter_sql: &str) -> Result<()> {
        self.ensure_table_column("sections", column_name, alter_sql)
    }

    fn ensure_tags_column(&self, column_name: &str, alter_sql: &str) -> Result<()> {
        self.ensure_table_column("tags", column_name, alter_sql)
    }

    fn ensure_filters_column(&self, column_name: &str, alter_sql: &str) -> Result<()> {
        self.ensure_table_column("filters", column_name, alter_sql)
    }

    fn ensure_sync_outbox_column(&self, column_name: &str, alter_sql: &str) -> Result<()> {
        self.ensure_table_column("sync_outbox", column_name, alter_sql)
    }

    fn ensure_table_column(
        &self,
        table_name: &str,
        column_name: &str,
        alter_sql: &str,
    ) -> Result<()> {
        let mut statement = self
            .connection
            .prepare(format!("PRAGMA table_info({table_name})").as_str())
            .with_context(|| format!("failed to inspect {table_name} schema"))?;
        let columns = statement
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<std::result::Result<Vec<_>, _>>()
            .with_context(|| format!("failed to read {table_name} schema"))?;

        if columns.iter().any(|column| column == column_name) {
            return Ok(());
        }

        self.connection
            .execute(alter_sql, [])
            .with_context(|| format!("failed to add {column_name} column to {table_name}"))?;
        Ok(())
    }

    fn ensure_session_history_column(&self, column_name: &str, alter_sql: &str) -> Result<()> {
        let mut statement = self
            .connection
            .prepare("PRAGMA table_info(session_history)")
            .context("failed to inspect session history schema")?;
        let columns = statement
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<std::result::Result<Vec<_>, _>>()
            .context("failed to read session history schema")?;

        if columns.iter().any(|column| column == column_name) {
            return Ok(());
        }

        self.connection
            .execute(alter_sql, [])
            .with_context(|| format!("failed to add {column_name} column to session_history"))?;
        Ok(())
    }

    fn ensure_task_indexes(&self) -> Result<()> {
        self.connection.execute(
            "CREATE INDEX IF NOT EXISTS idx_tasks_parent_task_id
             ON tasks(parent_task_id)",
            [],
        )?;
        self.connection.execute(
            "CREATE INDEX IF NOT EXISTS idx_tasks_parent_child_order
             ON tasks(parent_task_id, child_order, created_at, id)",
            [],
        )?;
        self.connection.execute(
            "CREATE INDEX IF NOT EXISTS idx_tasks_section_id
             ON tasks(section_id)",
            [],
        )?;
        self.connection.execute(
            "CREATE INDEX IF NOT EXISTS idx_sections_project_order
             ON sections(project_id, section_order, created_at, id)",
            [],
        )?;
        Ok(())
    }

    fn ensure_sync_indexes(&self) -> Result<()> {
        self.connection.execute(
            "CREATE UNIQUE INDEX IF NOT EXISTS idx_projects_todoist_id_active
             ON projects(todoist_id)
             WHERE deleted_at IS NULL AND todoist_id IS NOT NULL",
            [],
        )?;
        self.connection.execute(
            "CREATE UNIQUE INDEX IF NOT EXISTS idx_sections_todoist_id_active
             ON sections(todoist_id)
             WHERE deleted_at IS NULL AND todoist_id IS NOT NULL",
            [],
        )?;
        self.connection.execute(
            "CREATE UNIQUE INDEX IF NOT EXISTS idx_tasks_todoist_id_active
             ON tasks(todoist_id)
             WHERE deleted_at IS NULL AND todoist_id IS NOT NULL",
            [],
        )?;
        self.connection.execute(
            "CREATE UNIQUE INDEX IF NOT EXISTS idx_tags_todoist_id_active
             ON tags(todoist_id)
             WHERE deleted_at IS NULL AND todoist_id IS NOT NULL",
            [],
        )?;
        self.connection.execute(
            "CREATE UNIQUE INDEX IF NOT EXISTS idx_filters_todoist_id_active
             ON filters(todoist_id)
             WHERE deleted_at IS NULL AND todoist_id IS NOT NULL",
            [],
        )?;
        self.connection.execute(
            "CREATE INDEX IF NOT EXISTS idx_sync_outbox_provider_id
             ON sync_outbox(provider, id)",
            [],
        )?;
        Ok(())
    }

    fn ensure_sync_tables(&self) -> Result<()> {
        self.connection.execute(
            "CREATE TABLE IF NOT EXISTS sync_state (
                provider TEXT PRIMARY KEY,
                sync_token TEXT,
                last_synced_at TEXT,
                last_status TEXT,
                last_error TEXT,
                updated_at TEXT NOT NULL
            )",
            [],
        )?;
        self.connection.execute(
            "CREATE TABLE IF NOT EXISTS sync_outbox (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                provider TEXT NOT NULL,
                entity_type TEXT NOT NULL,
                entity_local_id INTEGER NOT NULL,
                op_kind TEXT NOT NULL,
                payload TEXT NOT NULL,
                op_timestamp_utc TEXT NOT NULL,
                attempts INTEGER NOT NULL DEFAULT 0,
                last_error TEXT,
                error_code TEXT,
                next_attempt_at TEXT,
                last_attempt_at TEXT,
                created_at TEXT NOT NULL
            )",
            [],
        )?;
        Ok(())
    }

    fn backfill_sync_timestamps(&self) -> Result<()> {
        self.connection.execute(
            "UPDATE tasks SET updated_at = created_at WHERE updated_at = ''",
            [],
        )?;
        self.connection.execute(
            "UPDATE projects SET updated_at = created_at WHERE updated_at = ''",
            [],
        )?;
        self.connection.execute(
            "UPDATE sections SET updated_at = created_at WHERE updated_at = ''",
            [],
        )?;
        self.connection.execute(
            "UPDATE tags SET updated_at = created_at WHERE updated_at = ''",
            [],
        )?;
        self.connection.execute(
            "UPDATE filters SET updated_at = created_at WHERE updated_at = ''",
            [],
        )?;
        Ok(())
    }

    fn ensure_sections_table(&self) -> Result<()> {
        self.connection.execute(
            "CREATE TABLE IF NOT EXISTS sections (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                project_id INTEGER NOT NULL,
                name TEXT NOT NULL,
                section_order INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                synced_at TEXT,
                todoist_id TEXT,
                deleted_at TEXT,
                FOREIGN KEY(project_id) REFERENCES projects(id)
            )",
            [],
        )?;
        Ok(())
    }

    fn normalize_task_child_order(&self) -> Result<()> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT id, parent_task_id
                 FROM tasks
                 WHERE deleted_at IS NULL
                 ORDER BY parent_task_id ASC, created_at ASC, id ASC",
            )
            .context("failed to prepare task ordering normalization query")?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    TaskId(row.get::<_, i64>(0)?),
                    row.get::<_, Option<i64>>(1)?.map(TaskId),
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()
            .context("failed to load tasks for ordering normalization")?;

        let mut last_parent: Option<TaskId> = None;
        let mut order: i64 = 1;
        let mut update = self
            .connection
            .prepare("UPDATE tasks SET child_order = ?1 WHERE id = ?2")
            .context("failed to prepare task ordering update statement")?;
        for (task_id, parent_task_id) in rows {
            if parent_task_id != last_parent {
                order = 1;
                last_parent = parent_task_id;
            }
            update
                .execute(params![order, task_id.0])
                .with_context(|| format!("failed to normalize child_order for {}", task_id.0))?;
            order += 1;
        }
        Ok(())
    }

    pub fn task_repository(&self) -> SqliteTaskRepository<'_> {
        // The repository borrows the connection instead of cloning or moving
        // it. The lifetime parameter (`'_`) is Rust's way of saying "this
        // repository cannot outlive the `Database` it came from."
        SqliteTaskRepository {
            connection: &self.connection,
        }
    }

    pub fn pomodoro_repository(&self) -> SqlitePomodoroRepository<'_> {
        SqlitePomodoroRepository {
            connection: &self.connection,
        }
    }

    pub fn project_repository(&self) -> SqliteProjectRepository<'_> {
        SqliteProjectRepository {
            connection: &self.connection,
        }
    }

    pub fn tag_repository(&self) -> SqliteTagRepository<'_> {
        SqliteTagRepository {
            connection: &self.connection,
        }
    }

    pub fn section_repository(&self) -> SqliteSectionRepository<'_> {
        SqliteSectionRepository {
            connection: &self.connection,
        }
    }

    pub fn filter_repository(&self) -> SqliteFilterRepository<'_> {
        SqliteFilterRepository {
            connection: &self.connection,
        }
    }

    pub fn sync_repository(&self) -> SqliteSyncRepository<'_> {
        SqliteSyncRepository {
            connection: &self.connection,
        }
    }

    fn ensure_inbox_project(&self) -> Result<ProjectId> {
        if let Some(project_id) = self
            .connection
            .query_row(
                "SELECT id FROM projects WHERE is_inbox = 1 ORDER BY id LIMIT 1",
                [],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .context("failed to load inbox project")?
        {
            return Ok(ProjectId(project_id));
        }

        self.connection
            .execute(
                "INSERT INTO projects(name, parent_project_id, color, is_favorite, is_inbox, child_order, created_at, updated_at, synced_at, todoist_id, deleted_at)
                 VALUES (?1, NULL, ?2, 0, 1, 0, ?3, ?4, NULL, NULL, NULL)",
                params![
                    "Inbox",
                    ProjectColor::Charcoal.as_str(),
                    Local::now(),
                    Local::now()
                ],
            )
            .context("failed to create inbox project")?;
        Ok(ProjectId(self.connection.last_insert_rowid()))
    }

    fn assign_tasks_to_inbox(&self, inbox_project_id: ProjectId) -> Result<()> {
        self.connection
            .execute(
                "UPDATE tasks SET project_id = ?1 WHERE project_id IS NULL",
                params![inbox_project_id.0],
            )
            .context("failed to assign tasks to inbox project")?;
        Ok(())
    }
}

fn now_utc_rfc3339() -> String {
    Utc::now().to_rfc3339()
}

fn enqueue_sync_outbox(
    connection: &Connection,
    provider: &str,
    entity_type: &str,
    entity_local_id: i64,
    op_kind: &str,
    payload: &str,
    op_timestamp_utc: &str,
) -> Result<()> {
    connection.execute(
        "INSERT INTO sync_outbox(provider, entity_type, entity_local_id, op_kind, payload, op_timestamp_utc, attempts, last_error, error_code, next_attempt_at, last_attempt_at, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, NULL, NULL, ?7, NULL, ?8)",
        params![
            provider,
            entity_type,
            entity_local_id,
            op_kind,
            payload,
            op_timestamp_utc,
            op_timestamp_utc,
            op_timestamp_utc,
        ],
    )?;
    Ok(())
}

pub struct SqliteSyncRepository<'a> {
    connection: &'a Connection,
}

impl SyncRepository for SqliteSyncRepository<'_> {
    fn get_state(&self, provider: &str) -> Result<Option<SyncStateRecord>> {
        self.connection
            .query_row(
                "SELECT provider, sync_token, last_synced_at, last_status, last_error, updated_at
                 FROM sync_state
                 WHERE provider = ?1",
                params![provider],
                |row| {
                    Ok(SyncStateRecord {
                        provider: row.get(0)?,
                        sync_token: row.get(1)?,
                        last_synced_at: row.get(2)?,
                        last_status: row.get(3)?,
                        last_error: row.get(4)?,
                        updated_at: row.get(5)?,
                    })
                },
            )
            .optional()
            .context("failed to load sync state")
    }

    fn upsert_state(&self, state: &SyncStateRecord) -> Result<()> {
        self.connection.execute(
            "INSERT INTO sync_state(provider, sync_token, last_synced_at, last_status, last_error, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(provider) DO UPDATE SET
               sync_token = excluded.sync_token,
               last_synced_at = excluded.last_synced_at,
               last_status = excluded.last_status,
               last_error = excluded.last_error,
               updated_at = excluded.updated_at",
            params![
                state.provider,
                state.sync_token,
                state.last_synced_at,
                state.last_status,
                state.last_error,
                state.updated_at
            ],
        )?;
        Ok(())
    }

    fn list_outbox(&self, provider: &str, limit: i64) -> Result<Vec<SyncOutboxEntry>> {
        let mut statement = self.connection.prepare(
            "SELECT id, provider, entity_type, entity_local_id, op_kind, payload, op_timestamp_utc, attempts, last_error, error_code, next_attempt_at, last_attempt_at, created_at
             FROM sync_outbox
             WHERE provider = ?1
             ORDER BY id ASC
             LIMIT ?2",
        )?;
        let rows = statement.query_map(params![provider, limit], |row| {
            Ok(SyncOutboxEntry {
                id: row.get(0)?,
                provider: row.get(1)?,
                entity_type: row.get(2)?,
                entity_local_id: row.get(3)?,
                op_kind: row.get(4)?,
                payload: row.get(5)?,
                op_timestamp_utc: row.get(6)?,
                attempts: row.get(7)?,
                last_error: row.get(8)?,
                error_code: row.get(9)?,
                next_attempt_at: row.get(10)?,
                last_attempt_at: row.get(11)?,
                created_at: row.get(12)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .context("failed to list sync outbox entries")
    }

    fn list_ready_outbox(
        &self,
        provider: &str,
        limit: i64,
        ready_before_utc: &str,
    ) -> Result<Vec<SyncOutboxEntry>> {
        let mut statement = self.connection.prepare(
            "SELECT id, provider, entity_type, entity_local_id, op_kind, payload, op_timestamp_utc, attempts, last_error, error_code, next_attempt_at, last_attempt_at, created_at
             FROM sync_outbox
             WHERE provider = ?1
               AND (next_attempt_at IS NULL OR next_attempt_at <= ?2)
             ORDER BY id ASC
             LIMIT ?3",
        )?;
        let rows = statement.query_map(params![provider, ready_before_utc, limit], |row| {
            Ok(SyncOutboxEntry {
                id: row.get(0)?,
                provider: row.get(1)?,
                entity_type: row.get(2)?,
                entity_local_id: row.get(3)?,
                op_kind: row.get(4)?,
                payload: row.get(5)?,
                op_timestamp_utc: row.get(6)?,
                attempts: row.get(7)?,
                last_error: row.get(8)?,
                error_code: row.get(9)?,
                next_attempt_at: row.get(10)?,
                last_attempt_at: row.get(11)?,
                created_at: row.get(12)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .context("failed to list ready sync outbox entries")
    }

    fn enqueue_outbox(
        &self,
        provider: &str,
        entity_type: &str,
        entity_local_id: i64,
        op_kind: &str,
        payload: &str,
        op_timestamp_utc: &str,
    ) -> Result<i64> {
        enqueue_sync_outbox(
            self.connection,
            provider,
            entity_type,
            entity_local_id,
            op_kind,
            payload,
            op_timestamp_utc,
        )?;
        Ok(self.connection.last_insert_rowid())
    }

    fn enqueue_bootstrap_outbox(&self, provider: &str, op_timestamp_utc: &str) -> Result<usize> {
        let mut total = 0usize;
        for (entity_type, table) in [
            ("project", "projects"),
            ("section", "sections"),
            ("tag", "tags"),
            ("filter", "filters"),
            ("task", "tasks"),
        ] {
            let extra_where = if entity_type == "project" {
                " AND COALESCE(t.is_inbox, 0) = 0"
            } else {
                ""
            };
            total += self.connection.execute(
                format!(
                    "INSERT INTO sync_outbox(provider, entity_type, entity_local_id, op_kind, payload, op_timestamp_utc, attempts, created_at)
                     SELECT ?1, ?2, t.id, 'update', '{{}}', ?3, 0, ?3
                     FROM {table} t
                     WHERE t.todoist_id IS NULL
                       AND t.deleted_at IS NULL
                       {extra_where}
                       AND NOT EXISTS (
                           SELECT 1 FROM sync_outbox o
                           WHERE o.provider = ?1
                             AND o.entity_type = ?2
                             AND o.entity_local_id = t.id
                       )"
                )
                .as_str(),
                params![provider, entity_type, op_timestamp_utc],
            )?;
        }
        Ok(total)
    }

    fn mark_outbox_delivered(&self, entry_id: i64) -> Result<()> {
        self.connection
            .execute("DELETE FROM sync_outbox WHERE id = ?1", params![entry_id])?;
        Ok(())
    }

    fn mark_outbox_failed(
        &self,
        entry_id: i64,
        error: &str,
        error_code: Option<&str>,
        next_attempt_at: Option<&str>,
        last_attempt_at: &str,
    ) -> Result<()> {
        self.connection.execute(
            "UPDATE sync_outbox
             SET attempts = attempts + 1,
                 last_error = ?1,
                 error_code = ?2,
                 next_attempt_at = ?3,
                 last_attempt_at = ?4
             WHERE id = ?5",
            params![
                error,
                error_code,
                next_attempt_at,
                last_attempt_at,
                entry_id
            ],
        )?;
        Ok(())
    }

    fn load_entity_snapshot(
        &self,
        entity_type: &str,
        entity_local_id: i64,
    ) -> Result<Option<SyncEntitySnapshot>> {
        match entity_type {
            "task" => {
                let task = self
                    .connection
                    .query_row(
                        "SELECT tasks.id, tasks.todoist_id, projects.todoist_id, sections.todoist_id, parent.todoist_id,
                                projects.is_inbox, tasks.title, tasks.description, tasks.priority,
                                tasks.due_date, tasks.due_datetime_utc, tasks.due_timezone, tasks.due_string, tasks.deleted_at
                         FROM tasks
                         LEFT JOIN projects ON projects.id = tasks.project_id
                         LEFT JOIN sections ON sections.id = tasks.section_id
                         LEFT JOIN tasks AS parent ON parent.id = tasks.parent_task_id
                         WHERE tasks.id = ?1",
                        params![entity_local_id],
                        |row| {
                            Ok(SyncTaskSnapshot {
                                local_id: row.get(0)?,
                                todoist_id: row.get(1)?,
                                project_todoist_id: row.get(2)?,
                                section_todoist_id: row.get(3)?,
                                parent_todoist_id: row.get(4)?,
                                project_is_inbox: row.get::<_, Option<i64>>(5)?.unwrap_or(0) != 0,
                                title: row.get(6)?,
                                description: row.get(7)?,
                                priority: row.get(8)?,
                                due_date: row.get(9)?,
                                due_datetime_utc: row.get(10)?,
                                due_timezone: row.get(11)?,
                                due_string: row.get(12)?,
                                deleted_at: row.get(13)?,
                                labels: Vec::new(),
                            })
                        },
                    )
                    .optional()?;
                let Some(mut task) = task else {
                    return Ok(None);
                };
                let mut statement = self.connection.prepare(
                    "SELECT tags.name
                     FROM task_tags
                     INNER JOIN tags ON tags.id = task_tags.tag_id
                     WHERE task_tags.task_id = ?1 AND tags.deleted_at IS NULL
                     ORDER BY tags.item_order ASC, tags.name COLLATE NOCASE ASC, tags.id ASC",
                )?;
                let rows = statement.query_map(params![entity_local_id], |row| row.get(0))?;
                task.labels = rows.collect::<std::result::Result<Vec<String>, _>>()?;
                Ok(Some(SyncEntitySnapshot::Task(task)))
            }
            "project" => self
                .connection
                .query_row(
                    "SELECT p.id, p.todoist_id, parent.todoist_id, p.parent_project_id, p.name, p.color, p.is_favorite, p.deleted_at
                     FROM projects p
                     LEFT JOIN projects parent ON parent.id = p.parent_project_id
                     WHERE p.id = ?1",
                    params![entity_local_id],
                    |row| {
                        Ok(SyncEntitySnapshot::Project(SyncProjectSnapshot {
                            local_id: row.get(0)?,
                            todoist_id: row.get(1)?,
                            parent_todoist_id: row.get(2)?,
                            has_parent_project: row.get::<_, Option<i64>>(3)?.is_some(),
                            name: row.get(4)?,
                            color: row.get(5)?,
                            is_favorite: row.get::<_, i64>(6)? != 0,
                            deleted_at: row.get(7)?,
                        }))
                    },
                )
                .optional()
                .context("failed to load sync project snapshot"),
            "section" => self
                .connection
                .query_row(
                    "SELECT sections.id, sections.todoist_id, projects.todoist_id, sections.name, sections.deleted_at
                     FROM sections
                     LEFT JOIN projects ON projects.id = sections.project_id
                     WHERE sections.id = ?1",
                    params![entity_local_id],
                    |row| {
                        Ok(SyncEntitySnapshot::Section(SyncSectionSnapshot {
                            local_id: row.get(0)?,
                            todoist_id: row.get(1)?,
                            project_todoist_id: row.get(2)?,
                            name: row.get(3)?,
                            deleted_at: row.get(4)?,
                        }))
                    },
                )
                .optional()
                .context("failed to load sync section snapshot"),
            "tag" => self
                .connection
                .query_row(
                    "SELECT id, todoist_id, name, color, is_favorite, deleted_at
                     FROM tags
                     WHERE id = ?1",
                    params![entity_local_id],
                    |row| {
                        Ok(SyncEntitySnapshot::Tag(SyncTagSnapshot {
                            local_id: row.get(0)?,
                            todoist_id: row.get(1)?,
                            name: row.get(2)?,
                            color: row.get(3)?,
                            is_favorite: row.get::<_, i64>(4)? != 0,
                            deleted_at: row.get(5)?,
                        }))
                    },
                )
                .optional()
                .context("failed to load sync tag snapshot"),
            "filter" => self
                .connection
                .query_row(
                    "SELECT id, todoist_id, name, query, color, is_favorite, deleted_at
                     FROM filters
                     WHERE id = ?1",
                    params![entity_local_id],
                    |row| {
                        Ok(SyncEntitySnapshot::Filter(SyncFilterSnapshot {
                            local_id: row.get(0)?,
                            todoist_id: row.get(1)?,
                            name: row.get(2)?,
                            query: row.get(3)?,
                            color: row.get(4)?,
                            is_favorite: row.get::<_, i64>(5)? != 0,
                            deleted_at: row.get(6)?,
                        }))
                    },
                )
                .optional()
                .context("failed to load sync filter snapshot"),
            _ => Ok(None),
        }
    }

    fn set_entity_todoist_id(
        &self,
        entity_type: &str,
        entity_local_id: i64,
        todoist_id: &str,
        synced_at_utc: &str,
    ) -> Result<()> {
        match entity_type {
            "task" => {
                self.connection.execute(
                    "UPDATE tasks SET todoist_id = ?1, synced_at = ?2 WHERE id = ?3",
                    params![todoist_id, synced_at_utc, entity_local_id],
                )?;
            }
            "project" => {
                self.connection.execute(
                    "UPDATE projects SET todoist_id = ?1, synced_at = ?2 WHERE id = ?3",
                    params![todoist_id, synced_at_utc, entity_local_id],
                )?;
            }
            "section" => {
                self.connection.execute(
                    "UPDATE sections SET todoist_id = ?1, synced_at = ?2 WHERE id = ?3",
                    params![todoist_id, synced_at_utc, entity_local_id],
                )?;
            }
            "tag" => {
                self.connection.execute(
                    "UPDATE tags SET todoist_id = ?1, synced_at = ?2 WHERE id = ?3",
                    params![todoist_id, synced_at_utc, entity_local_id],
                )?;
            }
            "filter" => {
                self.connection.execute(
                    "UPDATE filters SET todoist_id = ?1, synced_at = ?2 WHERE id = ?3",
                    params![todoist_id, synced_at_utc, entity_local_id],
                )?;
            }
            _ => {}
        };
        Ok(())
    }

    fn mark_entity_synced(
        &self,
        entity_type: &str,
        entity_local_id: i64,
        synced_at_utc: &str,
    ) -> Result<()> {
        match entity_type {
            "task" => {
                self.connection.execute(
                    "UPDATE tasks SET synced_at = ?1 WHERE id = ?2",
                    params![synced_at_utc, entity_local_id],
                )?;
            }
            "project" => {
                self.connection.execute(
                    "UPDATE projects SET synced_at = ?1 WHERE id = ?2",
                    params![synced_at_utc, entity_local_id],
                )?;
            }
            "section" => {
                self.connection.execute(
                    "UPDATE sections SET synced_at = ?1 WHERE id = ?2",
                    params![synced_at_utc, entity_local_id],
                )?;
            }
            "tag" => {
                self.connection.execute(
                    "UPDATE tags SET synced_at = ?1 WHERE id = ?2",
                    params![synced_at_utc, entity_local_id],
                )?;
            }
            "filter" => {
                self.connection.execute(
                    "UPDATE filters SET synced_at = ?1 WHERE id = ?2",
                    params![synced_at_utc, entity_local_id],
                )?;
            }
            _ => {}
        };
        Ok(())
    }

    fn apply_remote_project(
        &self,
        remote: &RemoteProjectRecord,
        synced_at_utc: &str,
        dry_run: bool,
    ) -> Result<SyncApplyOutcome> {
        let now_local = Local::now();
        let parent_local_id = match remote.parent_todoist_id.as_deref() {
            Some(todoist_id) => self.lookup_local_id_by_todoist("projects", todoist_id)?,
            None => None,
        };
        let mapped = self
            .connection
            .query_row(
                "SELECT id, updated_at, synced_at
                 FROM projects
                 WHERE todoist_id = ?1
                 LIMIT 1",
                params![remote.todoist_id],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, DateTime<Local>>(1)?,
                        row.get::<_, Option<String>>(2)?,
                    ))
                },
            )
            .optional()?;

        if let Some((local_id, updated_at, local_synced_at)) = mapped {
            if local_changed_since_sync(updated_at, local_synced_at.as_deref()) {
                return Ok(SyncApplyOutcome::Skipped);
            }
            if dry_run {
                return Ok(SyncApplyOutcome::Updated);
            }
            self.connection.execute(
                "UPDATE projects
                 SET name = ?1, parent_project_id = ?2, color = ?3, is_favorite = ?4,
                     is_inbox = ?5, updated_at = ?6, synced_at = ?7, deleted_at = NULL
                 WHERE id = ?8",
                params![
                    remote.name,
                    parent_local_id,
                    remote.color,
                    if remote.is_favorite { 1_i64 } else { 0_i64 },
                    if remote.is_inbox { 1_i64 } else { 0_i64 },
                    now_local,
                    synced_at_utc,
                    local_id
                ],
            )?;
            return Ok(SyncApplyOutcome::Updated);
        }

        if let Some(existing_local_id) =
            self.match_unmapped_project_by_name_and_parent(remote.name.as_str(), parent_local_id)?
        {
            if dry_run {
                return Ok(SyncApplyOutcome::Updated);
            }
            self.connection.execute(
                "UPDATE projects
                 SET todoist_id = ?1, name = ?2, parent_project_id = ?3, color = ?4, is_favorite = ?5,
                     is_inbox = ?6, updated_at = ?7, synced_at = ?8, deleted_at = NULL
                 WHERE id = ?9",
                params![
                    remote.todoist_id,
                    remote.name,
                    parent_local_id,
                    remote.color,
                    if remote.is_favorite { 1_i64 } else { 0_i64 },
                    if remote.is_inbox { 1_i64 } else { 0_i64 },
                    now_local,
                    synced_at_utc,
                    existing_local_id
                ],
            )?;
            return Ok(SyncApplyOutcome::Updated);
        }

        if dry_run {
            return Ok(SyncApplyOutcome::Created);
        }
        let child_order = self.next_project_child_order(parent_local_id)?;
        self.connection.execute(
            "INSERT INTO projects(name, parent_project_id, color, is_favorite, is_inbox, child_order, created_at, updated_at, synced_at, todoist_id, deleted_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, NULL)",
            params![
                remote.name,
                parent_local_id,
                remote.color,
                if remote.is_favorite { 1_i64 } else { 0_i64 },
                if remote.is_inbox { 1_i64 } else { 0_i64 },
                child_order,
                now_local,
                now_local,
                synced_at_utc,
                remote.todoist_id
            ],
        )?;
        Ok(SyncApplyOutcome::Created)
    }

    fn apply_remote_section(
        &self,
        remote: &RemoteSectionRecord,
        synced_at_utc: &str,
        dry_run: bool,
    ) -> Result<SyncApplyOutcome> {
        let Some(project_local_id) =
            self.lookup_local_id_by_todoist("projects", remote.project_todoist_id.as_str())?
        else {
            return Ok(SyncApplyOutcome::Skipped);
        };
        let now_local = Local::now();
        let mapped = self
            .connection
            .query_row(
                "SELECT id, updated_at, synced_at
                 FROM sections
                 WHERE todoist_id = ?1
                 LIMIT 1",
                params![remote.todoist_id],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, DateTime<Local>>(1)?,
                        row.get::<_, Option<String>>(2)?,
                    ))
                },
            )
            .optional()?;

        if let Some((local_id, updated_at, local_synced_at)) = mapped {
            if local_changed_since_sync(updated_at, local_synced_at.as_deref()) {
                return Ok(SyncApplyOutcome::Skipped);
            }
            if dry_run {
                return Ok(SyncApplyOutcome::Updated);
            }
            self.connection.execute(
                "UPDATE sections
                 SET project_id = ?1, name = ?2, updated_at = ?3, synced_at = ?4, deleted_at = NULL
                 WHERE id = ?5",
                params![
                    project_local_id,
                    remote.name,
                    now_local,
                    synced_at_utc,
                    local_id
                ],
            )?;
            return Ok(SyncApplyOutcome::Updated);
        }

        if dry_run {
            return Ok(SyncApplyOutcome::Created);
        }
        let section_order = self.next_section_order(ProjectId(project_local_id))?;
        self.connection.execute(
            "INSERT INTO sections(project_id, name, section_order, created_at, updated_at, synced_at, todoist_id, deleted_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL)",
            params![
                project_local_id,
                remote.name,
                section_order,
                now_local,
                now_local,
                synced_at_utc,
                remote.todoist_id
            ],
        )?;
        Ok(SyncApplyOutcome::Created)
    }

    fn apply_remote_tag(
        &self,
        remote: &RemoteTagRecord,
        synced_at_utc: &str,
        dry_run: bool,
    ) -> Result<SyncApplyOutcome> {
        let now_local = Local::now();
        let mapped = self
            .connection
            .query_row(
                "SELECT id, updated_at, synced_at
                 FROM tags
                 WHERE todoist_id = ?1
                 LIMIT 1",
                params![remote.todoist_id],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, DateTime<Local>>(1)?,
                        row.get::<_, Option<String>>(2)?,
                    ))
                },
            )
            .optional()?;

        if let Some((local_id, updated_at, local_synced_at)) = mapped {
            if local_changed_since_sync(updated_at, local_synced_at.as_deref()) {
                return Ok(SyncApplyOutcome::Skipped);
            }
            if dry_run {
                return Ok(SyncApplyOutcome::Updated);
            }
            self.connection.execute(
                "UPDATE tags
                 SET name = ?1, color = ?2, is_favorite = ?3,
                     updated_at = ?4, synced_at = ?5, deleted_at = NULL
                 WHERE id = ?6",
                params![
                    remote.name,
                    remote.color,
                    if remote.is_favorite { 1_i64 } else { 0_i64 },
                    now_local,
                    synced_at_utc,
                    local_id
                ],
            )?;
            return Ok(SyncApplyOutcome::Updated);
        }

        if let Some(existing_local_id) = self.match_unmapped_tag_by_name(remote.name.as_str())? {
            if dry_run {
                return Ok(SyncApplyOutcome::Updated);
            }
            self.connection.execute(
                "UPDATE tags
                 SET todoist_id = ?1, name = ?2, color = ?3, is_favorite = ?4,
                     updated_at = ?5, synced_at = ?6, deleted_at = NULL
                 WHERE id = ?7",
                params![
                    remote.todoist_id,
                    remote.name,
                    remote.color,
                    if remote.is_favorite { 1_i64 } else { 0_i64 },
                    now_local,
                    synced_at_utc,
                    existing_local_id
                ],
            )?;
            return Ok(SyncApplyOutcome::Updated);
        }

        if dry_run {
            return Ok(SyncApplyOutcome::Created);
        }
        let item_order = self.next_tag_item_order()?;
        self.connection.execute(
            "INSERT INTO tags(name, color, is_favorite, item_order, created_at, updated_at, synced_at, todoist_id, deleted_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, NULL)",
            params![
                remote.name,
                remote.color,
                if remote.is_favorite { 1_i64 } else { 0_i64 },
                item_order,
                now_local,
                now_local,
                synced_at_utc,
                remote.todoist_id
            ],
        )?;
        Ok(SyncApplyOutcome::Created)
    }

    fn apply_remote_filter(
        &self,
        remote: &RemoteFilterRecord,
        synced_at_utc: &str,
        dry_run: bool,
    ) -> Result<SyncApplyOutcome> {
        let now_local = Local::now();
        let mapped = self
            .connection
            .query_row(
                "SELECT id, updated_at, synced_at
                 FROM filters
                 WHERE todoist_id = ?1
                 LIMIT 1",
                params![remote.todoist_id],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, DateTime<Local>>(1)?,
                        row.get::<_, Option<String>>(2)?,
                    ))
                },
            )
            .optional()?;

        if let Some((local_id, updated_at, local_synced_at)) = mapped {
            if local_changed_since_sync(updated_at, local_synced_at.as_deref()) {
                return Ok(SyncApplyOutcome::Skipped);
            }
            if dry_run {
                return Ok(SyncApplyOutcome::Updated);
            }
            self.connection.execute(
                "UPDATE filters
                 SET name = ?1, query = ?2, color = ?3, is_favorite = ?4,
                     updated_at = ?5, synced_at = ?6, deleted_at = NULL
                 WHERE id = ?7",
                params![
                    remote.name,
                    remote.query,
                    remote.color,
                    if remote.is_favorite { 1_i64 } else { 0_i64 },
                    now_local,
                    synced_at_utc,
                    local_id
                ],
            )?;
            return Ok(SyncApplyOutcome::Updated);
        }

        if dry_run {
            return Ok(SyncApplyOutcome::Created);
        }
        let item_order = self.next_filter_item_order()?;
        self.connection.execute(
            "INSERT INTO filters(name, query, color, is_favorite, item_order, created_at, updated_at, synced_at, todoist_id, deleted_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, NULL)",
            params![
                remote.name,
                remote.query,
                remote.color,
                if remote.is_favorite { 1_i64 } else { 0_i64 },
                item_order,
                now_local,
                now_local,
                synced_at_utc,
                remote.todoist_id
            ],
        )?;
        Ok(SyncApplyOutcome::Created)
    }

    fn apply_remote_task(
        &self,
        remote: &RemoteTaskRecord,
        synced_at_utc: &str,
        dry_run: bool,
    ) -> Result<SyncApplyOutcome> {
        let now_local = Local::now();
        let project_local_id = match remote.project_todoist_id.as_deref() {
            Some(project_todoist_id) => {
                self.lookup_local_id_by_todoist("projects", project_todoist_id)?
            }
            None => Some(self.inbox_project_local_id()?),
        };
        if remote.project_todoist_id.is_some() && project_local_id.is_none() {
            return Ok(SyncApplyOutcome::Skipped);
        }
        let project_local_id =
            project_local_id.context("resolved remote project mapping is missing unexpectedly")?;
        let section_local_id = match remote.section_todoist_id.as_deref() {
            Some(todoist_id) => self.lookup_local_id_by_todoist("sections", todoist_id)?,
            None => None,
        };
        let parent_local_id = match remote.parent_todoist_id.as_deref() {
            Some(todoist_id) => self.lookup_local_id_by_todoist("tasks", todoist_id)?,
            None => None,
        };
        let status = if remote.completed_at.is_some() {
            TaskStatus::Done.as_str()
        } else {
            TaskStatus::Todo.as_str()
        };

        let mapped = self
            .connection
            .query_row(
                "SELECT id, updated_at, synced_at
                 FROM tasks
                 WHERE todoist_id = ?1
                 LIMIT 1",
                params![remote.todoist_id],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, DateTime<Local>>(1)?,
                        row.get::<_, Option<String>>(2)?,
                    ))
                },
            )
            .optional()?;

        if let Some((local_id, updated_at, local_synced_at)) = mapped {
            if local_changed_since_sync(updated_at, local_synced_at.as_deref()) {
                return Ok(SyncApplyOutcome::Skipped);
            }
            if dry_run {
                return Ok(SyncApplyOutcome::Updated);
            }
            self.connection.execute(
                "UPDATE tasks
                 SET project_id = ?1, section_id = ?2, parent_task_id = ?3,
                     title = ?4, description = ?5, status = ?6, completed_at = ?7, priority = ?8,
                     due_date = ?9, due_datetime_utc = ?10, due_timezone = ?11, due_string = ?12, due_is_recurring = ?13,
                     updated_at = ?14, synced_at = ?15, deleted_at = NULL
                 WHERE id = ?16",
                params![
                    project_local_id,
                    section_local_id,
                    parent_local_id,
                    remote.content,
                    remote.description,
                    status,
                    remote.completed_at,
                    remote.priority,
                    remote.due_date,
                    remote.due_datetime_utc,
                    remote.due_timezone,
                    remote.due_string,
                    if remote.due_is_recurring { 1_i64 } else { 0_i64 },
                    now_local,
                    synced_at_utc,
                    local_id
                ],
            )?;
            self.replace_task_tags_by_names(TaskId(local_id), remote.labels.as_slice())?;
            return Ok(SyncApplyOutcome::Updated);
        }

        if dry_run {
            return Ok(SyncApplyOutcome::Created);
        }
        let child_order = self.next_task_child_order(parent_local_id.map(TaskId))?;
        self.connection.execute(
            "INSERT INTO tasks(project_id, section_id, parent_task_id, child_order, title, description, status, priority, created_at, updated_at, synced_at, todoist_id, completed_at, deleted_at, due_date, due_datetime_utc, due_timezone, due_string, due_is_recurring)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, NULL, ?14, ?15, ?16, ?17, ?18)",
            params![
                project_local_id,
                section_local_id,
                parent_local_id,
                child_order,
                remote.content,
                remote.description,
                status,
                remote.priority,
                now_local,
                now_local,
                synced_at_utc,
                remote.todoist_id,
                remote.completed_at,
                remote.due_date,
                remote.due_datetime_utc,
                remote.due_timezone,
                remote.due_string,
                if remote.due_is_recurring { 1_i64 } else { 0_i64 },
            ],
        )?;
        let local_id = self.connection.last_insert_rowid();
        self.replace_task_tags_by_names(TaskId(local_id), remote.labels.as_slice())?;
        Ok(SyncApplyOutcome::Created)
    }
}

impl SqliteSyncRepository<'_> {
    fn lookup_local_id_by_todoist(&self, table: &str, todoist_id: &str) -> Result<Option<i64>> {
        self.connection
            .query_row(
                format!("SELECT id FROM {table} WHERE todoist_id = ?1 LIMIT 1").as_str(),
                params![todoist_id],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .with_context(|| format!("failed to lookup local id in {table} by todoist id"))
    }

    fn inbox_project_local_id(&self) -> Result<i64> {
        if let Some(id) = self
            .connection
            .query_row(
                "SELECT id FROM projects WHERE is_inbox = 1 AND deleted_at IS NULL ORDER BY id LIMIT 1",
                [],
                |row| row.get::<_, i64>(0),
            )
            .optional()?
        {
            return Ok(id);
        }

        let fallback = self
            .connection
            .query_row(
                "SELECT id
                 FROM projects
                 WHERE deleted_at IS NULL
                   AND lower(name) = 'inbox'
                 ORDER BY id
                 LIMIT 1",
                [],
                |row| row.get::<_, i64>(0),
            )
            .optional()?;
        if let Some(id) = fallback {
            self.connection.execute(
                "UPDATE projects SET is_inbox = 1 WHERE id = ?1",
                params![id],
            )?;
            return Ok(id);
        }

        self.connection
            .execute(
                "INSERT INTO projects(name, parent_project_id, color, is_favorite, is_inbox, child_order, created_at, updated_at, synced_at, todoist_id, deleted_at)
                 VALUES ('Inbox', NULL, 'charcoal', 0, 1, 0, ?1, ?1, NULL, NULL, NULL)",
                params![Local::now()],
            )
            .context("failed to create fallback inbox project for remote task apply")?;
        Ok(self.connection.last_insert_rowid())
    }

    fn match_unmapped_tag_by_name(&self, name: &str) -> Result<Option<i64>> {
        let mut statement = self.connection.prepare(
            "SELECT id
             FROM tags
             WHERE todoist_id IS NULL
               AND deleted_at IS NULL
               AND lower(name) = lower(?1)",
        )?;
        let rows = statement.query_map(params![name], |row| row.get::<_, i64>(0))?;
        let ids = rows.collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(if ids.len() == 1 { Some(ids[0]) } else { None })
    }

    fn match_unmapped_project_by_name_and_parent(
        &self,
        name: &str,
        parent_project_id: Option<i64>,
    ) -> Result<Option<i64>> {
        let mut statement = self.connection.prepare(
            "SELECT id
             FROM projects
             WHERE todoist_id IS NULL
               AND deleted_at IS NULL
               AND lower(name) = lower(?1)
               AND parent_project_id IS ?2",
        )?;
        let rows =
            statement.query_map(params![name, parent_project_id], |row| row.get::<_, i64>(0))?;
        let ids = rows.collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(if ids.len() == 1 { Some(ids[0]) } else { None })
    }

    fn next_project_child_order(&self, parent_project_id: Option<i64>) -> Result<i64> {
        self.connection
            .query_row(
                "SELECT COALESCE(MAX(child_order), -1) + 1
                 FROM projects
                 WHERE deleted_at IS NULL
                   AND parent_project_id IS ?1",
                params![parent_project_id],
                |row| row.get::<_, i64>(0),
            )
            .context("failed to compute remote project child order")
    }

    fn next_section_order(&self, project_id: ProjectId) -> Result<i64> {
        self.connection
            .query_row(
                "SELECT COALESCE(MAX(section_order), 0) + 1
                 FROM sections
                 WHERE project_id = ?1 AND deleted_at IS NULL",
                params![project_id.0],
                |row| row.get::<_, i64>(0),
            )
            .context("failed to compute remote section order")
    }

    fn next_tag_item_order(&self) -> Result<i64> {
        self.connection
            .query_row(
                "SELECT COALESCE(MAX(item_order), -1) + 1
                 FROM tags
                 WHERE deleted_at IS NULL",
                [],
                |row| row.get::<_, i64>(0),
            )
            .context("failed to compute remote tag order")
    }

    fn next_filter_item_order(&self) -> Result<i64> {
        self.connection
            .query_row(
                "SELECT COALESCE(MAX(item_order), -1) + 1
                 FROM filters
                 WHERE deleted_at IS NULL",
                [],
                |row| row.get::<_, i64>(0),
            )
            .context("failed to compute remote filter order")
    }

    fn next_task_child_order(&self, parent_task_id: Option<TaskId>) -> Result<i64> {
        if let Some(parent_task_id) = parent_task_id {
            self.connection
                .query_row(
                    "SELECT COALESCE(MAX(child_order), 0) + 1
                     FROM tasks
                     WHERE deleted_at IS NULL AND parent_task_id = ?1",
                    params![parent_task_id.0],
                    |row| row.get::<_, i64>(0),
                )
                .context("failed to compute remote task child order")
        } else {
            self.connection
                .query_row(
                    "SELECT COALESCE(MAX(child_order), 0) + 1
                     FROM tasks
                     WHERE deleted_at IS NULL AND parent_task_id IS NULL",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .context("failed to compute remote task root order")
        }
    }

    fn replace_task_tags_by_names(&self, task_id: TaskId, tag_names: &[String]) -> Result<()> {
        let transaction = self.connection.unchecked_transaction()?;
        transaction.execute(
            "DELETE FROM task_tags WHERE task_id = ?1",
            params![task_id.0],
        )?;
        if !tag_names.is_empty() {
            let mut insert = transaction.prepare(
                "INSERT OR IGNORE INTO task_tags(task_id, tag_id)
                 SELECT ?1, id FROM tags WHERE deleted_at IS NULL AND lower(name) = lower(?2)",
            )?;
            for name in tag_names {
                insert.execute(params![task_id.0, name])?;
            }
        }
        transaction.commit()?;
        Ok(())
    }
}

fn local_changed_since_sync(updated_at: DateTime<Local>, synced_at_utc: Option<&str>) -> bool {
    let Some(synced_at_utc) = synced_at_utc else {
        return true;
    };
    let Ok(parsed_synced) = DateTime::parse_from_rfc3339(synced_at_utc) else {
        return true;
    };
    let synced_local = parsed_synced.with_timezone(&Local);
    updated_at > synced_local
}

pub struct SqliteTaskRepository<'a> {
    connection: &'a Connection,
}

impl TaskRepository for SqliteTaskRepository<'_> {
    fn list_all(&self) -> Result<Vec<Task>> {
        // `prepare` compiles SQL once and `query_map` walks each row through a
        // closure. The closure is conceptually similar to a row-to-struct
        // callback in C, but its return type is checked by the compiler.
        let mut statement = self.connection.prepare(
            "SELECT id, project_id, section_id, parent_task_id, child_order, title, description, status, priority, created_at, completed_at, deleted_at, due_date, due_datetime_utc, due_timezone, due_string, due_is_recurring
             FROM tasks
             ORDER BY created_at DESC, id DESC",
        )?;

        let rows = statement.query_map([], |row| {
            Ok(Task {
                id: TaskId(row.get(0)?),
                project_id: ProjectId(row.get(1)?),
                section_id: row.get::<_, Option<i64>>(2)?.map(SectionId),
                parent_task_id: row.get::<_, Option<i64>>(3)?.map(TaskId),
                child_order: row.get(4)?,
                title: row.get(5)?,
                description: row.get(6)?,
                status: TaskStatus::from_db(row.get::<_, String>(7)?.as_str()),
                priority: TaskPriority::from_db(row.get::<_, i64>(8)?),
                created_at: row.get(9)?,
                completed_at: row.get(10)?,
                deleted_at: row.get(11)?,
                due: Self::row_due(row, 12, 13, 14, 15, 16)?,
            })
        })?;

        // `collect` turns the iterator of per-row results into one
        // `Result<Vec<Task>>`, stopping early if any row conversion fails.
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .context("failed to load tasks")
    }

    fn create(
        &self,
        title: &str,
        project_id: ProjectId,
        due: Option<&TaskDue>,
        now: DateTime<Local>,
    ) -> Result<Task> {
        let now_utc = now_utc_rfc3339();
        self.connection
            .execute(
                "INSERT INTO tasks(project_id, section_id, parent_task_id, child_order, title, status, priority, created_at, updated_at, synced_at, todoist_id, completed_at, due_date, due_datetime_utc, due_timezone, due_string, due_is_recurring)
                 VALUES (?1, NULL, NULL, ?2, ?3, ?4, ?5, ?6, ?7, NULL, NULL, NULL, ?8, ?9, ?10, ?11, ?12)",
                params![
                    project_id.0,
                    self.next_child_order(None)?,
                    title,
                    TaskStatus::Todo.as_str(),
                    TaskPriority::P4.to_db(),
                    now,
                    now,
                    due.map(|due| due.date),
                    due.and_then(|due| due.datetime).map(|dt| dt.to_rfc3339()),
                    due.and_then(|due| due.timezone.clone()),
                    due.map(|due| due.string.clone()),
                    due.map(|due| if due.is_recurring { 1_i64 } else { 0_i64 })
                        .unwrap_or(0_i64),
                ],
            )
            .context("failed to create task")?;

        let task_id = TaskId(self.connection.last_insert_rowid());
        enqueue_sync_outbox(
            self.connection,
            "todoist",
            "task",
            task_id.0,
            "create",
            "{}",
            now_utc.as_str(),
        )?;
        self.load_task(task_id)?
            .context("created task could not be reloaded")
    }

    fn update(&self, task_id: TaskId, update: &TaskUpdate) -> Result<Task> {
        let current = self
            .load_task(task_id)?
            .context("task to update does not exist")?;
        self.validate_parent(
            task_id,
            update.project_id,
            update.section_id,
            update.parent_task_id,
        )?;
        let child_order = if current.parent_task_id == update.parent_task_id {
            current.child_order
        } else {
            self.next_child_order(update.parent_task_id)?
        };

        self.connection
            .execute(
                "UPDATE tasks
                 SET title = ?1, description = ?2, project_id = ?3, section_id = ?4, parent_task_id = ?5, child_order = ?6, priority = ?7, due_date = ?8, due_datetime_utc = ?9, due_timezone = ?10, due_string = ?11, due_is_recurring = ?12, updated_at = ?13
                 WHERE id = ?14",
                params![
                    update.title,
                    update.description,
                    update.project_id.0,
                    update.section_id.map(|id| id.0),
                    update.parent_task_id.map(|id| id.0),
                    child_order,
                    update.priority.to_db(),
                    update.due.as_ref().map(|due| due.date),
                    update
                        .due
                        .as_ref()
                        .and_then(|due| due.datetime)
                        .map(|dt| dt.to_rfc3339()),
                    update.due.as_ref().and_then(|due| due.timezone.clone()),
                    update.due.as_ref().map(|due| due.string.clone()),
                    update
                        .due
                        .as_ref()
                        .map(|due| if due.is_recurring { 1_i64 } else { 0_i64 })
                        .unwrap_or(0_i64),
                    Local::now(),
                    task_id.0,
                ],
            )
            .with_context(|| format!("failed to update task {}", task_id.0))?;
        enqueue_sync_outbox(
            self.connection,
            "todoist",
            "task",
            task_id.0,
            "update",
            "{}",
            now_utc_rfc3339().as_str(),
        )?;

        self.normalize_sibling_order(current.parent_task_id)?;
        self.normalize_sibling_order(update.parent_task_id)?;
        self.load_task(task_id)?
            .context("updated task could not be reloaded")
    }

    fn update_status(
        &self,
        task_id: TaskId,
        status: TaskStatus,
        completed_at: Option<DateTime<Local>>,
    ) -> Result<Task> {
        self.connection
            .execute(
                "UPDATE tasks
                 SET status = ?1, completed_at = ?2, updated_at = ?3
                 WHERE id = ?4",
                params![status.as_str(), completed_at, Local::now(), task_id.0],
            )
            .with_context(|| format!("failed to update task status for {}", task_id.0))?;
        enqueue_sync_outbox(
            self.connection,
            "todoist",
            "task",
            task_id.0,
            "update",
            "{}",
            now_utc_rfc3339().as_str(),
        )?;

        self.load_task(task_id)?
            .context("updated task could not be reloaded")
    }

    fn move_within_parent(&self, task_id: TaskId, direction: isize) -> Result<()> {
        if direction == 0 {
            return Ok(());
        }

        let Some(task) = self.load_task(task_id)? else {
            return Ok(());
        };
        if task.deleted_at.is_some() {
            return Ok(());
        }

        let mut sibling_ids = self.active_sibling_ids(task.parent_task_id)?;
        let Some(current_index) = sibling_ids.iter().position(|id| *id == task_id) else {
            return Ok(());
        };
        let target_index = (current_index as isize + direction)
            .clamp(0, sibling_ids.len().saturating_sub(1) as isize)
            as usize;
        if target_index == current_index {
            return Ok(());
        }

        sibling_ids.remove(current_index);
        sibling_ids.insert(target_index, task_id);
        let mut statement = self
            .connection
            .prepare("UPDATE tasks SET child_order = ?1 WHERE id = ?2")?;
        for (index, sibling_id) in sibling_ids.iter().enumerate() {
            statement.execute(params![index as i64 + 1, sibling_id.0])?;
        }
        self.connection.execute(
            "UPDATE tasks SET updated_at = ?1 WHERE id = ?2",
            params![Local::now(), task_id.0],
        )?;
        enqueue_sync_outbox(
            self.connection,
            "todoist",
            "task",
            task_id.0,
            "update",
            "{}",
            now_utc_rfc3339().as_str(),
        )?;
        Ok(())
    }

    fn delete(&self, task_id: TaskId) -> Result<()> {
        let parent_task_id = self
            .load_task(task_id)?
            .and_then(|task| task.parent_task_id);
        self.connection
            .execute(
                "UPDATE tasks SET deleted_at = ?1, updated_at = ?2 WHERE id = ?3",
                params![Local::now(), Local::now(), task_id.0],
            )
            .with_context(|| format!("failed to soft-delete task {}", task_id.0))?;
        enqueue_sync_outbox(
            self.connection,
            "todoist",
            "task",
            task_id.0,
            "delete",
            "{}",
            now_utc_rfc3339().as_str(),
        )?;
        self.normalize_sibling_order(parent_task_id)?;
        Ok(())
    }
}

impl SqliteTaskRepository<'_> {
    fn validate_parent(
        &self,
        task_id: TaskId,
        project_id: ProjectId,
        section_id: Option<SectionId>,
        parent_task_id: Option<TaskId>,
    ) -> Result<()> {
        self.validate_section(project_id, section_id)?;
        let Some(parent_task_id) = parent_task_id else {
            return Ok(());
        };
        if parent_task_id == task_id {
            bail!("task cannot be its own parent");
        }

        let parent = self
            .load_task(parent_task_id)?
            .context("selected parent task does not exist")?;
        if parent.deleted_at.is_some() {
            bail!("selected parent task is deleted");
        }
        if parent.project_id != project_id {
            bail!("parent and child tasks must belong to the same project");
        }
        if parent.section_id != section_id {
            bail!("parent and child tasks must belong to the same section");
        }

        let mut ancestor = parent.parent_task_id;
        while let Some(ancestor_id) = ancestor {
            if ancestor_id == task_id {
                bail!("task hierarchy cannot contain cycles");
            }
            ancestor = self
                .load_task(ancestor_id)?
                .and_then(|candidate| candidate.parent_task_id);
        }
        Ok(())
    }

    fn next_child_order(&self, parent_task_id: Option<TaskId>) -> Result<i64> {
        let next = if let Some(parent_task_id) = parent_task_id {
            self.connection.query_row(
                "SELECT COALESCE(MAX(child_order), 0) + 1
                 FROM tasks
                 WHERE deleted_at IS NULL AND parent_task_id = ?1",
                params![parent_task_id.0],
                |row| row.get(0),
            )?
        } else {
            self.connection.query_row(
                "SELECT COALESCE(MAX(child_order), 0) + 1
                 FROM tasks
                 WHERE deleted_at IS NULL AND parent_task_id IS NULL",
                [],
                |row| row.get(0),
            )?
        };
        Ok(next)
    }

    fn active_sibling_ids(&self, parent_task_id: Option<TaskId>) -> Result<Vec<TaskId>> {
        if let Some(parent_task_id) = parent_task_id {
            let mut statement = self.connection.prepare(
                "SELECT id
                 FROM tasks
                 WHERE deleted_at IS NULL AND parent_task_id = ?1
                 ORDER BY child_order ASC, created_at ASC, id ASC",
            )?;
            let rows =
                statement.query_map(params![parent_task_id.0], |row| Ok(TaskId(row.get(0)?)))?;
            rows.collect::<std::result::Result<Vec<_>, _>>()
                .context("failed to load sibling tasks")
        } else {
            let mut statement = self.connection.prepare(
                "SELECT id
                 FROM tasks
                 WHERE deleted_at IS NULL AND parent_task_id IS NULL
                 ORDER BY child_order ASC, created_at ASC, id ASC",
            )?;
            let rows = statement.query_map([], |row| Ok(TaskId(row.get(0)?)))?;
            rows.collect::<std::result::Result<Vec<_>, _>>()
                .context("failed to load sibling tasks")
        }
    }

    fn normalize_sibling_order(&self, parent_task_id: Option<TaskId>) -> Result<()> {
        let sibling_ids = self.active_sibling_ids(parent_task_id)?;
        let mut statement = self
            .connection
            .prepare("UPDATE tasks SET child_order = ?1 WHERE id = ?2")?;
        for (index, sibling_id) in sibling_ids.iter().enumerate() {
            statement.execute(params![index as i64 + 1, sibling_id.0])?;
        }
        Ok(())
    }

    fn load_task(&self, task_id: TaskId) -> Result<Option<Task>> {
        self.connection
            .query_row(
                "SELECT id, project_id, section_id, parent_task_id, child_order, title, description, status,
                        created_at, completed_at, priority, deleted_at, due_date,
                        due_datetime_utc, due_timezone, due_string, due_is_recurring
                 FROM tasks
                 WHERE id = ?1",
                params![task_id.0],
                |row| {
                    Ok(Task {
                        id: TaskId(row.get(0)?),
                        project_id: ProjectId(row.get(1)?),
                        section_id: row.get::<_, Option<i64>>(2)?.map(SectionId),
                        parent_task_id: row.get::<_, Option<i64>>(3)?.map(TaskId),
                        child_order: row.get(4)?,
                        title: row.get(5)?,
                        description: row.get(6)?,
                        status: TaskStatus::from_db(row.get::<_, String>(7)?.as_str()),
                        created_at: row.get(8)?,
                        completed_at: row.get(9)?,
                        priority: TaskPriority::from_db(row.get::<_, i64>(10)?),
                        deleted_at: row.get(11)?,
                        due: Self::row_due(row, 12, 13, 14, 15, 16)?,
                    })
                },
            )
            .optional()
            .context("failed to load task")
    }

    fn validate_section(&self, project_id: ProjectId, section_id: Option<SectionId>) -> Result<()> {
        let Some(section_id) = section_id else {
            return Ok(());
        };
        let section = self
            .connection
            .query_row(
                "SELECT project_id, deleted_at FROM sections WHERE id = ?1",
                params![section_id.0],
                |row| {
                    Ok((
                        ProjectId(row.get::<_, i64>(0)?),
                        row.get::<_, Option<DateTime<Local>>>(1)?,
                    ))
                },
            )
            .optional()
            .context("failed to load section for task validation")?
            .context("selected section does not exist")?;
        if section.1.is_some() {
            bail!("selected section is deleted");
        }
        if section.0 != project_id {
            bail!("task project and section project must match");
        }
        Ok(())
    }

    fn row_due(
        row: &rusqlite::Row<'_>,
        date_col: usize,
        datetime_col: usize,
        timezone_col: usize,
        string_col: usize,
        recurring_col: usize,
    ) -> rusqlite::Result<Option<TaskDue>> {
        let date = row.get::<_, Option<chrono::NaiveDate>>(date_col)?;
        let datetime_utc = row.get::<_, Option<String>>(datetime_col)?;
        let timezone = row.get::<_, Option<String>>(timezone_col)?;
        let string = row.get::<_, Option<String>>(string_col)?;
        let is_recurring = row.get::<_, i64>(recurring_col)?;

        match (date, string) {
            (Some(date), Some(string)) => {
                let datetime = datetime_utc
                    .as_deref()
                    .and_then(|raw| chrono::DateTime::parse_from_rfc3339(raw).ok())
                    .map(|dt| dt.with_timezone(&Utc));
                Ok(Some(TaskDue {
                    date,
                    datetime,
                    timezone,
                    string,
                    is_recurring: is_recurring != 0,
                }))
            }
            _ => Ok(None),
        }
    }
}

pub struct SqliteProjectRepository<'a> {
    connection: &'a Connection,
}

impl ProjectRepository for SqliteProjectRepository<'_> {
    fn list_all(&self) -> Result<Vec<Project>> {
        let mut statement = self.connection.prepare(
            "SELECT id, name, parent_project_id, color, is_favorite, is_inbox, child_order, created_at, deleted_at
             FROM projects
             ORDER BY is_inbox DESC, child_order ASC, created_at ASC, id ASC",
        )?;

        let rows = statement.query_map([], |row| {
            Ok(Project {
                id: ProjectId(row.get(0)?),
                name: row.get(1)?,
                parent_project_id: row.get::<_, Option<i64>>(2)?.map(ProjectId),
                color: ProjectColor::from_db(row.get::<_, String>(3)?.as_str()),
                is_favorite: row.get::<_, i64>(4)? != 0,
                is_inbox: row.get::<_, i64>(5)? != 0,
                child_order: row.get(6)?,
                created_at: row.get(7)?,
                deleted_at: row.get(8)?,
            })
        })?;

        rows.collect::<std::result::Result<Vec<_>, _>>()
            .context("failed to load projects")
    }

    fn inbox_project_id(&self) -> Result<ProjectId> {
        let project_id = self
            .connection
            .query_row(
                "SELECT id FROM projects WHERE is_inbox = 1 ORDER BY id LIMIT 1",
                [],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .context("failed to load inbox project")?
            .map(ProjectId)
            .context("inbox project is missing")?;
        Ok(project_id)
    }

    fn create(
        &self,
        name: &str,
        parent_project_id: Option<ProjectId>,
        color: ProjectColor,
        is_favorite: bool,
        now: DateTime<Local>,
    ) -> Result<Project> {
        self.validate_parent(None, parent_project_id)?;
        let child_order = self.next_child_order(parent_project_id)?;
        self.connection
            .execute(
                "INSERT INTO projects(name, parent_project_id, color, is_favorite, is_inbox, child_order, created_at, updated_at, synced_at, todoist_id, deleted_at)
                 VALUES (?1, ?2, ?3, ?4, 0, ?5, ?6, ?7, NULL, NULL, NULL)",
                params![
                    name,
                    parent_project_id.map(|id| id.0),
                    color.as_str(),
                    if is_favorite { 1_i64 } else { 0_i64 },
                    child_order,
                    now,
                    now,
                ],
            )
            .context("failed to create project")?;
        let project_id = ProjectId(self.connection.last_insert_rowid());
        enqueue_sync_outbox(
            self.connection,
            "todoist",
            "project",
            project_id.0,
            "create",
            "{}",
            now_utc_rfc3339().as_str(),
        )?;
        self.load_project(project_id)?
            .context("created project could not be reloaded")
    }

    fn update(&self, project_id: ProjectId, update: &ProjectUpdate) -> Result<Project> {
        let current = self
            .load_project(project_id)?
            .context("project to update does not exist")?;
        if current.is_inbox {
            bail!("inbox project cannot be edited");
        }
        self.validate_parent(Some(project_id), update.parent_project_id)?;
        let child_order = if current.parent_project_id == update.parent_project_id {
            current.child_order
        } else {
            self.next_child_order(update.parent_project_id)?
        };

        self.connection
            .execute(
                "UPDATE projects
                 SET name = ?1, parent_project_id = ?2, color = ?3, is_favorite = ?4, child_order = ?5, updated_at = ?6
                 WHERE id = ?7",
                params![
                    update.name,
                    update.parent_project_id.map(|id| id.0),
                    update.color.as_str(),
                    if update.is_favorite { 1_i64 } else { 0_i64 },
                    child_order,
                    Local::now(),
                    project_id.0,
                ],
            )
            .with_context(|| format!("failed to update project {}", project_id.0))?;
        enqueue_sync_outbox(
            self.connection,
            "todoist",
            "project",
            project_id.0,
            "update",
            "{}",
            now_utc_rfc3339().as_str(),
        )?;

        self.load_project(project_id)?
            .context("updated project could not be reloaded")
    }

    fn move_within_parent(&self, project_id: ProjectId, direction: isize) -> Result<()> {
        if direction == 0 {
            return Ok(());
        }

        let Some(project) = self.load_project(project_id)? else {
            return Ok(());
        };
        if project.is_inbox || project.deleted_at.is_some() {
            return Ok(());
        }

        let mut sibling_ids = self.active_sibling_ids(project.parent_project_id)?;
        let Some(current_index) = sibling_ids.iter().position(|id| *id == project_id) else {
            return Ok(());
        };
        let target_index = (current_index as isize + direction)
            .clamp(0, sibling_ids.len().saturating_sub(1) as isize)
            as usize;
        if current_index == target_index {
            return Ok(());
        }

        sibling_ids.remove(current_index);
        sibling_ids.insert(target_index, project_id);

        let mut statement = self
            .connection
            .prepare("UPDATE projects SET child_order = ?1 WHERE id = ?2")?;
        for (index, sibling_id) in sibling_ids.iter().enumerate() {
            statement.execute(params![index as i64 + 1, sibling_id.0])?;
        }
        self.connection.execute(
            "UPDATE projects SET updated_at = ?1 WHERE id = ?2",
            params![Local::now(), project_id.0],
        )?;
        enqueue_sync_outbox(
            self.connection,
            "todoist",
            "project",
            project_id.0,
            "update",
            "{}",
            now_utc_rfc3339().as_str(),
        )?;
        Ok(())
    }

    fn delete(&self, project_id: ProjectId, now: DateTime<Local>) -> Result<()> {
        let project = self
            .load_project(project_id)?
            .context("project to delete does not exist")?;
        if project.is_inbox {
            bail!("inbox project cannot be deleted");
        }

        self.connection
            .execute(
                "WITH RECURSIVE project_tree(id) AS (
                     SELECT id FROM projects WHERE id = ?1
                     UNION ALL
                     SELECT projects.id
                     FROM projects
                     INNER JOIN project_tree ON projects.parent_project_id = project_tree.id
                 )
                 UPDATE projects
                 SET deleted_at = ?2, updated_at = ?2
                 WHERE id IN (SELECT id FROM project_tree)",
                params![project_id.0, now],
            )
            .with_context(|| format!("failed to soft-delete project {}", project_id.0))?;
        self.connection
            .execute(
                "WITH RECURSIVE project_tree(id) AS (
                     SELECT id FROM projects WHERE id = ?1
                     UNION ALL
                     SELECT projects.id
                     FROM projects
                     INNER JOIN project_tree ON projects.parent_project_id = project_tree.id
                 )
                 UPDATE tasks
                 SET deleted_at = COALESCE(deleted_at, ?2),
                     updated_at = ?2
                 WHERE project_id IN (SELECT id FROM project_tree)",
                params![project_id.0, now],
            )
            .with_context(|| format!("failed to soft-delete tasks for project {}", project_id.0))?;
        self.connection
            .execute(
                "WITH RECURSIVE project_tree(id) AS (
                     SELECT id FROM projects WHERE id = ?1
                     UNION ALL
                     SELECT projects.id
                     FROM projects
                     INNER JOIN project_tree ON projects.parent_project_id = project_tree.id
                 )
                 UPDATE sections
                 SET deleted_at = COALESCE(deleted_at, ?2),
                     updated_at = ?2
                 WHERE project_id IN (SELECT id FROM project_tree)",
                params![project_id.0, now],
            )
            .with_context(|| {
                format!(
                    "failed to soft-delete sections for project {}",
                    project_id.0
                )
            })?;
        enqueue_sync_outbox(
            self.connection,
            "todoist",
            "project",
            project_id.0,
            "delete",
            "{}",
            now_utc_rfc3339().as_str(),
        )?;
        Ok(())
    }
}

impl SqliteProjectRepository<'_> {
    fn load_project(&self, project_id: ProjectId) -> Result<Option<Project>> {
        self.connection
            .query_row(
                "SELECT id, name, parent_project_id, color, is_favorite, is_inbox, child_order, created_at, deleted_at
                 FROM projects
                 WHERE id = ?1",
                params![project_id.0],
                |row| {
                    Ok(Project {
                        id: ProjectId(row.get(0)?),
                        name: row.get(1)?,
                        parent_project_id: row.get::<_, Option<i64>>(2)?.map(ProjectId),
                        color: ProjectColor::from_db(row.get::<_, String>(3)?.as_str()),
                        is_favorite: row.get::<_, i64>(4)? != 0,
                        is_inbox: row.get::<_, i64>(5)? != 0,
                        child_order: row.get(6)?,
                        created_at: row.get(7)?,
                        deleted_at: row.get(8)?,
                    })
                },
            )
            .optional()
            .context("failed to load project")
    }

    fn next_child_order(&self, parent_project_id: Option<ProjectId>) -> Result<i64> {
        let next = self
            .connection
            .query_row(
                "SELECT COALESCE(MAX(child_order), -1) + 1
                 FROM projects
                 WHERE parent_project_id IS ?1 AND deleted_at IS NULL",
                params![parent_project_id.map(|id| id.0)],
                |row| row.get::<_, i64>(0),
            )
            .context("failed to calculate next project order")?;
        Ok(next)
    }

    fn active_sibling_ids(&self, parent_project_id: Option<ProjectId>) -> Result<Vec<ProjectId>> {
        let mut statement = self.connection.prepare(
            "SELECT id
             FROM projects
             WHERE parent_project_id IS ?1
               AND deleted_at IS NULL
               AND is_inbox = 0
             ORDER BY child_order ASC, name COLLATE NOCASE ASC, id ASC",
        )?;
        let rows = statement.query_map(params![parent_project_id.map(|id| id.0)], |row| {
            Ok(ProjectId(row.get(0)?))
        })?;

        rows.collect::<std::result::Result<Vec<_>, _>>()
            .context("failed to load project siblings")
    }

    fn validate_parent(
        &self,
        project_id: Option<ProjectId>,
        parent_project_id: Option<ProjectId>,
    ) -> Result<()> {
        let Some(parent_project_id) = parent_project_id else {
            return Ok(());
        };

        let parent = self
            .load_project(parent_project_id)?
            .context("parent project does not exist")?;
        if parent.deleted_at.is_some() {
            bail!("parent project is deleted");
        }
        if parent.is_inbox {
            bail!("inbox project cannot be nested");
        }

        if Some(parent_project_id) == project_id {
            bail!("project cannot be its own parent");
        }

        let Some(project_id) = project_id else {
            return Ok(());
        };

        let cycle = self
            .connection
            .query_row(
                "WITH RECURSIVE ancestors(id) AS (
                     SELECT id FROM projects WHERE id = ?1
                     UNION ALL
                     SELECT projects.parent_project_id
                     FROM projects
                     INNER JOIN ancestors ON projects.id = ancestors.id
                     WHERE projects.parent_project_id IS NOT NULL
                 )
                 SELECT EXISTS(SELECT 1 FROM ancestors WHERE id = ?2)",
                params![parent_project_id.0, project_id.0],
                |row| row.get::<_, i64>(0),
            )
            .context("failed to validate project ancestry")?;
        if cycle != 0 {
            bail!("project parent assignment creates a cycle");
        }

        Ok(())
    }
}

pub struct SqliteSectionRepository<'a> {
    connection: &'a Connection,
}

impl SectionRepository for SqliteSectionRepository<'_> {
    fn list_all(&self) -> Result<Vec<Section>> {
        let mut statement = self.connection.prepare(
            "SELECT id, project_id, name, section_order, created_at, deleted_at
             FROM sections
             ORDER BY section_order ASC, name COLLATE NOCASE ASC, id ASC",
        )?;

        let rows = statement.query_map([], |row| {
            Ok(Section {
                id: SectionId(row.get(0)?),
                project_id: ProjectId(row.get(1)?),
                name: row.get(2)?,
                section_order: row.get(3)?,
                created_at: row.get(4)?,
                deleted_at: row.get(5)?,
            })
        })?;

        rows.collect::<std::result::Result<Vec<_>, _>>()
            .context("failed to load sections")
    }

    fn create(&self, project_id: ProjectId, name: &str, now: DateTime<Local>) -> Result<Section> {
        self.validate_project(project_id)?;
        let section_order = self.next_section_order(project_id)?;
        self.connection
            .execute(
                "INSERT INTO sections(project_id, name, section_order, created_at, updated_at, synced_at, todoist_id, deleted_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, NULL, NULL, NULL)",
                params![project_id.0, name, section_order, now, now],
            )
            .context("failed to create section")?;
        let section_id = SectionId(self.connection.last_insert_rowid());
        enqueue_sync_outbox(
            self.connection,
            "todoist",
            "section",
            section_id.0,
            "create",
            "{}",
            now_utc_rfc3339().as_str(),
        )?;
        self.load_section(section_id)?
            .context("created section could not be reloaded")
    }

    fn update(&self, section_id: SectionId, update: &SectionUpdate) -> Result<Section> {
        let current = self
            .load_section(section_id)?
            .context("section to update does not exist")?;
        if current.deleted_at.is_some() {
            bail!("section is deleted");
        }
        self.connection
            .execute(
                "UPDATE sections SET name = ?1, updated_at = ?2 WHERE id = ?3",
                params![update.name, Local::now(), section_id.0],
            )
            .with_context(|| format!("failed to update section {}", section_id.0))?;
        enqueue_sync_outbox(
            self.connection,
            "todoist",
            "section",
            section_id.0,
            "update",
            "{}",
            now_utc_rfc3339().as_str(),
        )?;
        self.load_section(section_id)?
            .context("updated section could not be reloaded")
    }

    fn move_within_project(&self, section_id: SectionId, direction: isize) -> Result<()> {
        if direction == 0 {
            return Ok(());
        }
        let Some(section) = self.load_section(section_id)? else {
            return Ok(());
        };
        if section.deleted_at.is_some() {
            return Ok(());
        }
        let mut sibling_ids = self.active_sibling_ids(section.project_id)?;
        let Some(current_index) = sibling_ids.iter().position(|id| *id == section_id) else {
            return Ok(());
        };
        let target_index = (current_index as isize + direction)
            .clamp(0, sibling_ids.len().saturating_sub(1) as isize)
            as usize;
        if target_index == current_index {
            return Ok(());
        }
        sibling_ids.remove(current_index);
        sibling_ids.insert(target_index, section_id);
        let mut statement = self
            .connection
            .prepare("UPDATE sections SET section_order = ?1 WHERE id = ?2")?;
        for (index, sibling_id) in sibling_ids.iter().enumerate() {
            statement.execute(params![index as i64 + 1, sibling_id.0])?;
        }
        self.connection.execute(
            "UPDATE sections SET updated_at = ?1 WHERE id = ?2",
            params![Local::now(), section_id.0],
        )?;
        enqueue_sync_outbox(
            self.connection,
            "todoist",
            "section",
            section_id.0,
            "update",
            "{}",
            now_utc_rfc3339().as_str(),
        )?;
        Ok(())
    }

    fn delete(&self, section_id: SectionId, now: DateTime<Local>) -> Result<()> {
        let section = self
            .load_section(section_id)?
            .context("section to delete does not exist")?;
        if section.deleted_at.is_some() {
            return Ok(());
        }
        self.connection
            .execute(
                "UPDATE sections SET deleted_at = ?1, updated_at = ?2 WHERE id = ?3",
                params![now, now, section_id.0],
            )
            .with_context(|| format!("failed to soft-delete section {}", section_id.0))?;
        self.connection
            .execute(
                "UPDATE tasks SET section_id = NULL, updated_at = ?2 WHERE section_id = ?1",
                params![section_id.0, now],
            )
            .with_context(|| format!("failed to detach tasks from section {}", section_id.0))?;
        enqueue_sync_outbox(
            self.connection,
            "todoist",
            "section",
            section_id.0,
            "delete",
            "{}",
            now_utc_rfc3339().as_str(),
        )?;
        Ok(())
    }
}

impl SqliteSectionRepository<'_> {
    fn validate_project(&self, project_id: ProjectId) -> Result<()> {
        let project = self
            .connection
            .query_row(
                "SELECT deleted_at, is_inbox FROM projects WHERE id = ?1",
                params![project_id.0],
                |row| {
                    Ok((
                        row.get::<_, Option<DateTime<Local>>>(0)?,
                        row.get::<_, i64>(1)? != 0,
                    ))
                },
            )
            .optional()
            .context("failed to validate section project")?
            .context("section project does not exist")?;
        if project.0.is_some() {
            bail!("section project is deleted");
        }
        if project.1 {
            bail!("inbox project cannot have sections");
        }
        Ok(())
    }

    fn load_section(&self, section_id: SectionId) -> Result<Option<Section>> {
        self.connection
            .query_row(
                "SELECT id, project_id, name, section_order, created_at, deleted_at
                 FROM sections
                 WHERE id = ?1",
                params![section_id.0],
                |row| {
                    Ok(Section {
                        id: SectionId(row.get(0)?),
                        project_id: ProjectId(row.get(1)?),
                        name: row.get(2)?,
                        section_order: row.get(3)?,
                        created_at: row.get(4)?,
                        deleted_at: row.get(5)?,
                    })
                },
            )
            .optional()
            .context("failed to load section")
    }

    fn next_section_order(&self, project_id: ProjectId) -> Result<i64> {
        self.connection
            .query_row(
                "SELECT COALESCE(MAX(section_order), 0) + 1
                 FROM sections
                 WHERE project_id = ?1 AND deleted_at IS NULL",
                params![project_id.0],
                |row| row.get::<_, i64>(0),
            )
            .context("failed to calculate next section order")
    }

    fn active_sibling_ids(&self, project_id: ProjectId) -> Result<Vec<SectionId>> {
        let mut statement = self.connection.prepare(
            "SELECT id
             FROM sections
             WHERE project_id = ?1 AND deleted_at IS NULL
             ORDER BY section_order ASC, name COLLATE NOCASE ASC, id ASC",
        )?;
        let rows = statement.query_map(params![project_id.0], |row| Ok(SectionId(row.get(0)?)))?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .context("failed to load section siblings")
    }
}

pub struct SqliteTagRepository<'a> {
    connection: &'a Connection,
}

impl TagRepository for SqliteTagRepository<'_> {
    fn list_all(&self) -> Result<Vec<Tag>> {
        let mut statement = self.connection.prepare(
            "SELECT id, name, color, is_favorite, item_order, created_at, deleted_at
             FROM tags
             ORDER BY item_order ASC, name COLLATE NOCASE ASC, id ASC",
        )?;

        let rows = statement.query_map([], |row| {
            Ok(Tag {
                id: TagId(row.get(0)?),
                name: row.get(1)?,
                color: TagColor::from_db(row.get::<_, String>(2)?.as_str()),
                is_favorite: row.get::<_, i64>(3)? != 0,
                item_order: row.get(4)?,
                created_at: row.get(5)?,
                deleted_at: row.get(6)?,
            })
        })?;

        rows.collect::<std::result::Result<Vec<_>, _>>()
            .context("failed to load tags")
    }

    fn create(
        &self,
        name: &str,
        color: TagColor,
        is_favorite: bool,
        now: DateTime<Local>,
    ) -> Result<Tag> {
        let item_order = self.next_item_order()?;
        self.connection
            .execute(
                "INSERT INTO tags(name, color, is_favorite, item_order, created_at, updated_at, synced_at, todoist_id, deleted_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, NULL, NULL)",
                params![
                    name,
                    color.as_str(),
                    if is_favorite { 1_i64 } else { 0_i64 },
                    item_order,
                    now,
                    now
                ],
            )
            .context("failed to create tag")?;
        let tag_id = TagId(self.connection.last_insert_rowid());
        enqueue_sync_outbox(
            self.connection,
            "todoist",
            "tag",
            tag_id.0,
            "create",
            "{}",
            now_utc_rfc3339().as_str(),
        )?;
        self.load_tag(tag_id)?
            .context("created tag could not be reloaded")
    }

    fn update(&self, tag_id: TagId, update: &TagUpdate) -> Result<Tag> {
        let current = self
            .load_tag(tag_id)?
            .context("tag to update does not exist")?;
        if current.deleted_at.is_some() {
            bail!("tag is deleted");
        }

        self.connection
            .execute(
                "UPDATE tags
                 SET name = ?1, color = ?2, is_favorite = ?3, updated_at = ?4
                 WHERE id = ?5",
                params![
                    update.name,
                    update.color.as_str(),
                    if update.is_favorite { 1_i64 } else { 0_i64 },
                    Local::now(),
                    tag_id.0
                ],
            )
            .with_context(|| format!("failed to update tag {}", tag_id.0))?;
        enqueue_sync_outbox(
            self.connection,
            "todoist",
            "tag",
            tag_id.0,
            "update",
            "{}",
            now_utc_rfc3339().as_str(),
        )?;

        self.load_tag(tag_id)?
            .context("updated tag could not be reloaded")
    }

    fn move_within_list(&self, tag_id: TagId, direction: isize) -> Result<()> {
        if direction == 0 {
            return Ok(());
        }
        let Some(tag) = self.load_tag(tag_id)? else {
            return Ok(());
        };
        if tag.deleted_at.is_some() {
            return Ok(());
        }

        let mut ids = self.active_tag_ids()?;
        let Some(current_index) = ids.iter().position(|id| *id == tag_id) else {
            return Ok(());
        };
        let target_index = (current_index as isize + direction)
            .clamp(0, ids.len().saturating_sub(1) as isize) as usize;
        if target_index == current_index {
            return Ok(());
        }

        ids.remove(current_index);
        ids.insert(target_index, tag_id);
        let mut statement = self
            .connection
            .prepare("UPDATE tags SET item_order = ?1 WHERE id = ?2")?;
        for (index, id) in ids.iter().enumerate() {
            statement.execute(params![index as i64 + 1, id.0])?;
        }
        self.connection.execute(
            "UPDATE tags SET updated_at = ?1 WHERE id = ?2",
            params![Local::now(), tag_id.0],
        )?;
        enqueue_sync_outbox(
            self.connection,
            "todoist",
            "tag",
            tag_id.0,
            "update",
            "{}",
            now_utc_rfc3339().as_str(),
        )?;
        Ok(())
    }

    fn delete(&self, tag_id: TagId, now: DateTime<Local>) -> Result<()> {
        let tag = self
            .load_tag(tag_id)?
            .context("tag to delete does not exist")?;
        if tag.deleted_at.is_some() {
            return Ok(());
        }
        self.connection
            .execute(
                "UPDATE tags SET deleted_at = ?1, updated_at = ?2 WHERE id = ?3",
                params![now, now, tag_id.0],
            )
            .with_context(|| format!("failed to soft-delete tag {}", tag_id.0))?;
        self.connection
            .execute("DELETE FROM task_tags WHERE tag_id = ?1", params![tag_id.0])
            .with_context(|| format!("failed to detach task-tag links for {}", tag_id.0))?;
        enqueue_sync_outbox(
            self.connection,
            "todoist",
            "tag",
            tag_id.0,
            "delete",
            "{}",
            now_utc_rfc3339().as_str(),
        )?;
        Ok(())
    }

    fn list_task_tag_links(&self) -> Result<Vec<(TaskId, TagId)>> {
        let mut statement = self.connection.prepare(
            "SELECT task_tags.task_id, task_tags.tag_id
             FROM task_tags
             INNER JOIN tags ON tags.id = task_tags.tag_id
             WHERE tags.deleted_at IS NULL",
        )?;
        let rows = statement.query_map([], |row| Ok((TaskId(row.get(0)?), TagId(row.get(1)?))))?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .context("failed to load task-tag links")
    }

    fn replace_task_tags(&self, task_id: TaskId, tag_ids: &[TagId]) -> Result<()> {
        let transaction = self.connection.unchecked_transaction()?;
        transaction.execute(
            "DELETE FROM task_tags WHERE task_id = ?1",
            params![task_id.0],
        )?;
        {
            let mut statement =
                transaction.prepare("INSERT INTO task_tags(task_id, tag_id) VALUES (?1, ?2)")?;
            for tag_id in tag_ids {
                statement.execute(params![task_id.0, tag_id.0])?;
            }
        }
        transaction.commit()?;
        self.connection.execute(
            "UPDATE tasks SET updated_at = ?1 WHERE id = ?2",
            params![Local::now(), task_id.0],
        )?;
        enqueue_sync_outbox(
            self.connection,
            "todoist",
            "task",
            task_id.0,
            "update",
            "{}",
            now_utc_rfc3339().as_str(),
        )?;
        Ok(())
    }
}

impl SqliteTagRepository<'_> {
    fn load_tag(&self, tag_id: TagId) -> Result<Option<Tag>> {
        self.connection
            .query_row(
                "SELECT id, name, color, is_favorite, item_order, created_at, deleted_at
                 FROM tags
                 WHERE id = ?1",
                params![tag_id.0],
                |row| {
                    Ok(Tag {
                        id: TagId(row.get(0)?),
                        name: row.get(1)?,
                        color: TagColor::from_db(row.get::<_, String>(2)?.as_str()),
                        is_favorite: row.get::<_, i64>(3)? != 0,
                        item_order: row.get(4)?,
                        created_at: row.get(5)?,
                        deleted_at: row.get(6)?,
                    })
                },
            )
            .optional()
            .context("failed to load tag")
    }

    fn next_item_order(&self) -> Result<i64> {
        self.connection
            .query_row(
                "SELECT COALESCE(MAX(item_order), -1) + 1
                 FROM tags
                 WHERE deleted_at IS NULL",
                [],
                |row| row.get::<_, i64>(0),
            )
            .context("failed to calculate next tag order")
    }

    fn active_tag_ids(&self) -> Result<Vec<TagId>> {
        let mut statement = self.connection.prepare(
            "SELECT id
             FROM tags
             WHERE deleted_at IS NULL
             ORDER BY item_order ASC, name COLLATE NOCASE ASC, id ASC",
        )?;
        let rows = statement.query_map([], |row| Ok(TagId(row.get(0)?)))?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .context("failed to load active tag ids")
    }
}

pub struct SqliteFilterRepository<'a> {
    connection: &'a Connection,
}

impl FilterRepository for SqliteFilterRepository<'_> {
    fn list_all(&self) -> Result<Vec<Filter>> {
        let mut statement = self.connection.prepare(
            "SELECT id, name, query, color, is_favorite, item_order, created_at, deleted_at
             FROM filters
             ORDER BY item_order ASC, name COLLATE NOCASE ASC, id ASC",
        )?;

        let rows = statement.query_map([], |row| {
            Ok(Filter {
                id: FilterId(row.get(0)?),
                name: row.get(1)?,
                query: row.get(2)?,
                color: FilterColor::from_db(row.get::<_, String>(3)?.as_str()),
                is_favorite: row.get::<_, i64>(4)? != 0,
                item_order: row.get(5)?,
                created_at: row.get(6)?,
                deleted_at: row.get(7)?,
            })
        })?;

        rows.collect::<std::result::Result<Vec<_>, _>>()
            .context("failed to load filters")
    }

    fn create(
        &self,
        name: &str,
        query: &str,
        color: FilterColor,
        is_favorite: bool,
        now: DateTime<Local>,
    ) -> Result<Filter> {
        let item_order = self.next_item_order()?;
        self.connection
            .execute(
                "INSERT INTO filters(name, query, color, is_favorite, item_order, created_at, updated_at, synced_at, todoist_id, deleted_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL, NULL, NULL)",
                params![
                    name,
                    query,
                    color.as_str(),
                    if is_favorite { 1_i64 } else { 0_i64 },
                    item_order,
                    now,
                    now
                ],
            )
            .context("failed to create filter")?;
        let filter_id = FilterId(self.connection.last_insert_rowid());
        enqueue_sync_outbox(
            self.connection,
            "todoist",
            "filter",
            filter_id.0,
            "create",
            "{}",
            now_utc_rfc3339().as_str(),
        )?;
        self.load_filter(filter_id)?
            .context("created filter could not be reloaded")
    }

    fn update(&self, filter_id: FilterId, update: &FilterUpdate) -> Result<Filter> {
        let current = self
            .load_filter(filter_id)?
            .context("filter to update does not exist")?;
        if current.deleted_at.is_some() {
            bail!("filter is deleted");
        }

        self.connection
            .execute(
                "UPDATE filters
                 SET name = ?1, query = ?2, color = ?3, is_favorite = ?4, updated_at = ?5
                 WHERE id = ?6",
                params![
                    update.name,
                    update.query,
                    update.color.as_str(),
                    if update.is_favorite { 1_i64 } else { 0_i64 },
                    Local::now(),
                    filter_id.0
                ],
            )
            .with_context(|| format!("failed to update filter {}", filter_id.0))?;
        enqueue_sync_outbox(
            self.connection,
            "todoist",
            "filter",
            filter_id.0,
            "update",
            "{}",
            now_utc_rfc3339().as_str(),
        )?;

        self.load_filter(filter_id)?
            .context("updated filter could not be reloaded")
    }

    fn move_within_list(&self, filter_id: FilterId, direction: isize) -> Result<()> {
        if direction == 0 {
            return Ok(());
        }
        let Some(filter) = self.load_filter(filter_id)? else {
            return Ok(());
        };
        if filter.deleted_at.is_some() {
            return Ok(());
        }

        let mut ids = self.active_filter_ids()?;
        let Some(current_index) = ids.iter().position(|id| *id == filter_id) else {
            return Ok(());
        };
        let target_index = (current_index as isize + direction)
            .clamp(0, ids.len().saturating_sub(1) as isize) as usize;
        if target_index == current_index {
            return Ok(());
        }

        ids.remove(current_index);
        ids.insert(target_index, filter_id);
        let mut statement = self
            .connection
            .prepare("UPDATE filters SET item_order = ?1 WHERE id = ?2")?;
        for (index, id) in ids.iter().enumerate() {
            statement.execute(params![index as i64 + 1, id.0])?;
        }
        self.connection.execute(
            "UPDATE filters SET updated_at = ?1 WHERE id = ?2",
            params![Local::now(), filter_id.0],
        )?;
        enqueue_sync_outbox(
            self.connection,
            "todoist",
            "filter",
            filter_id.0,
            "update",
            "{}",
            now_utc_rfc3339().as_str(),
        )?;
        Ok(())
    }

    fn delete(&self, filter_id: FilterId, now: DateTime<Local>) -> Result<()> {
        let filter = self
            .load_filter(filter_id)?
            .context("filter to delete does not exist")?;
        if filter.deleted_at.is_some() {
            return Ok(());
        }
        self.connection
            .execute(
                "UPDATE filters SET deleted_at = ?1, updated_at = ?2 WHERE id = ?3",
                params![now, now, filter_id.0],
            )
            .with_context(|| format!("failed to soft-delete filter {}", filter_id.0))?;
        enqueue_sync_outbox(
            self.connection,
            "todoist",
            "filter",
            filter_id.0,
            "delete",
            "{}",
            now_utc_rfc3339().as_str(),
        )?;
        Ok(())
    }
}

impl SqliteFilterRepository<'_> {
    fn load_filter(&self, filter_id: FilterId) -> Result<Option<Filter>> {
        self.connection
            .query_row(
                "SELECT id, name, query, color, is_favorite, item_order, created_at, deleted_at
                 FROM filters
                 WHERE id = ?1",
                params![filter_id.0],
                |row| {
                    Ok(Filter {
                        id: FilterId(row.get(0)?),
                        name: row.get(1)?,
                        query: row.get(2)?,
                        color: FilterColor::from_db(row.get::<_, String>(3)?.as_str()),
                        is_favorite: row.get::<_, i64>(4)? != 0,
                        item_order: row.get(5)?,
                        created_at: row.get(6)?,
                        deleted_at: row.get(7)?,
                    })
                },
            )
            .optional()
            .context("failed to load filter")
    }

    fn next_item_order(&self) -> Result<i64> {
        self.connection
            .query_row(
                "SELECT COALESCE(MAX(item_order), -1) + 1
                 FROM filters
                 WHERE deleted_at IS NULL",
                [],
                |row| row.get::<_, i64>(0),
            )
            .context("failed to calculate next filter order")
    }

    fn active_filter_ids(&self) -> Result<Vec<FilterId>> {
        let mut statement = self.connection.prepare(
            "SELECT id
             FROM filters
             WHERE deleted_at IS NULL
             ORDER BY item_order ASC, name COLLATE NOCASE ASC, id ASC",
        )?;
        let rows = statement.query_map([], |row| Ok(FilterId(row.get(0)?)))?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .context("failed to load active filter ids")
    }
}

pub struct SqlitePomodoroRepository<'a> {
    connection: &'a Connection,
}

impl PomodoroRepository for SqlitePomodoroRepository<'_> {
    fn list_day(
        &self,
        started_at: DateTime<Local>,
        ended_at: DateTime<Local>,
    ) -> Result<Vec<SessionEntry>> {
        let mut statement = self.connection.prepare(
            "SELECT id, task_id, notes, kind, outcome, next_break_kind, started_at, ended_at, duration_seconds
             FROM session_history
             WHERE started_at >= ?1 AND started_at < ?2
             ORDER BY started_at DESC, id DESC
            ",
        )?;

        let rows = statement.query_map(params![started_at, ended_at], |row| {
            Ok(SessionEntry {
                id: row.get(0)?,
                task_id: row.get::<_, Option<i64>>(1)?.map(TaskId),
                notes: row.get(2)?,
                kind: SessionKind::from_db(row.get::<_, String>(3)?.as_str()),
                outcome: SessionOutcome::from_db(row.get::<_, String>(4)?.as_str()),
                next_break_kind: row
                    .get::<_, Option<String>>(5)?
                    .map(|value| SessionKind::from_db(value.as_str())),
                started_at: row.get(6)?,
                ended_at: row.get(7)?,
                duration_seconds: row.get::<_, i64>(8)? as u32,
            })
        })?;

        rows.collect::<std::result::Result<Vec<_>, _>>()
            .context("failed to load session history")
    }

    fn stats_for_day(
        &self,
        started_at: DateTime<Local>,
        ended_at: DateTime<Local>,
    ) -> Result<HistoryStats> {
        let mut statement = self.connection.prepare(
            "SELECT
                COALESCE(SUM(CASE WHEN kind IN ('focus', 'work') THEN duration_seconds ELSE 0 END), 0) AS total_work_seconds,
                COALESCE(SUM(CASE WHEN kind IN ('short_break', 'long_break') THEN duration_seconds ELSE 0 END), 0) AS total_break_seconds
             FROM session_history
             WHERE started_at >= ?1 AND started_at < ?2",
        )?;

        let (total_work_seconds, total_break_seconds): (i64, i64) = statement
            .query_row(params![started_at, ended_at], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .context("failed to compute session stats")?;

        // `optional()` converts "query returned no row" into `Ok(None)`.
        // For `COUNT(*)` that case should not really occur, but this keeps the
        // code explicit about the database API contract we are handling.
        let completed_tasks = self
            .connection
            .query_row(
                "SELECT COUNT(*) FROM tasks WHERE status = 'done'",
                [],
                |row| row.get(0),
            )
            .optional()?
            .unwrap_or(0_i64);

        Ok(HistoryStats {
            total_sessions: self
                .connection
                .query_row(
                    "SELECT COUNT(*) FROM session_history
                     WHERE started_at >= ?1 AND started_at < ?2
                       AND kind IN ('focus', 'work')",
                    params![started_at, ended_at],
                    |row| row.get::<_, i64>(0),
                )
                .context("failed to compute focus session count")?
                as usize,
            total_minutes: (total_work_seconds as u32).div_ceil(60),
            total_work_seconds: total_work_seconds as u32,
            total_break_seconds: total_break_seconds as u32,
            completed_tasks: completed_tasks as usize,
        })
    }

    fn summarize_days(
        &self,
        started_at: DateTime<Local>,
        ended_at: DateTime<Local>,
    ) -> Result<Vec<DayHistorySummary>> {
        let mut statement = self.connection.prepare(
            "SELECT
                DATE(started_at, 'localtime') AS day,
                COALESCE(SUM(CASE WHEN kind IN ('focus', 'work') AND outcome = 'completed' THEN 1 ELSE 0 END), 0) AS completed_sessions,
                COALESCE(SUM(CASE WHEN kind IN ('focus', 'work') AND outcome = 'voided' THEN 1 ELSE 0 END), 0) AS voided_sessions,
                COALESCE(SUM(CASE WHEN kind IN ('focus', 'work') THEN duration_seconds ELSE 0 END), 0) AS focus_seconds,
                COALESCE(SUM(CASE WHEN kind IN ('short_break', 'long_break') THEN duration_seconds ELSE 0 END), 0) AS break_seconds
             FROM session_history
             WHERE started_at >= ?1 AND started_at < ?2
             GROUP BY DATE(started_at, 'localtime')
             ORDER BY DATE(started_at, 'localtime') DESC",
        )?;

        let rows = statement.query_map(params![started_at, ended_at], |row| {
            Ok(DayHistorySummary {
                day: row.get(0)?,
                completed_sessions: row.get::<_, i64>(1)? as usize,
                voided_sessions: row.get::<_, i64>(2)? as usize,
                focus_seconds: row.get::<_, i64>(3)? as u32,
                break_seconds: row.get::<_, i64>(4)? as u32,
            })
        })?;

        rows.collect::<std::result::Result<Vec<_>, _>>()
            .context("failed to summarize history by day")
    }

    fn summarize_completed_focus_days(
        &self,
        started_at: DateTime<Local>,
        ended_at: DateTime<Local>,
    ) -> Result<Vec<FocusDaySummary>> {
        let mut statement = self.connection.prepare(
            "SELECT
                DATE(started_at, 'localtime') AS day,
                COALESCE(SUM(duration_seconds), 0) AS focus_seconds
             FROM session_history
             WHERE started_at >= ?1 AND started_at < ?2
               AND kind IN ('focus', 'work')
               AND outcome = 'completed'
             GROUP BY DATE(started_at, 'localtime')
             ORDER BY DATE(started_at, 'localtime') ASC",
        )?;

        let rows = statement.query_map(params![started_at, ended_at], |row| {
            Ok(FocusDaySummary {
                day: row.get(0)?,
                focus_seconds: row.get::<_, i64>(1)? as u32,
            })
        })?;

        rows.collect::<std::result::Result<Vec<_>, _>>()
            .context("failed to summarize completed focus by day")
    }

    fn summarize_completed_focus_hours(
        &self,
        started_at: DateTime<Local>,
        ended_at: DateTime<Local>,
    ) -> Result<Vec<FocusHourSummary>> {
        let mut statement = self.connection.prepare(
            "SELECT
                CAST(strftime('%H', started_at, 'localtime') AS INTEGER) AS hour,
                COALESCE(SUM(duration_seconds), 0) AS focus_seconds
             FROM session_history
             WHERE started_at >= ?1 AND started_at < ?2
               AND kind IN ('focus', 'work')
               AND outcome = 'completed'
             GROUP BY CAST(strftime('%H', started_at, 'localtime') AS INTEGER)
             ORDER BY CAST(strftime('%H', started_at, 'localtime') AS INTEGER) ASC",
        )?;

        let rows = statement.query_map(params![started_at, ended_at], |row| {
            Ok(FocusHourSummary {
                hour: row.get::<_, i64>(0)? as u8,
                focus_seconds: row.get::<_, i64>(1)? as u32,
            })
        })?;

        rows.collect::<std::result::Result<Vec<_>, _>>()
            .context("failed to summarize completed focus by hour")
    }

    fn create(
        &self,
        task_id: Option<TaskId>,
        notes: &str,
        next_break_kind: Option<SessionKind>,
        started_at: DateTime<Local>,
        ended_at: DateTime<Local>,
        duration_minutes: u32,
    ) -> Result<SessionEntry> {
        self.connection
            .execute(
                "INSERT INTO pomodoros(task_id, started_at, ended_at, duration_minutes)
                 VALUES (?1, ?2, ?3, ?4)",
                params![
                    task_id.map(|task_id| task_id.0),
                    started_at,
                    ended_at,
                    i64::from(duration_minutes)
                ],
            )
            .context("failed to insert pomodoro session")?;

        self.record_session_entry(
            task_id,
            notes,
            SessionKind::Focus,
            SessionOutcome::Completed,
            next_break_kind,
            started_at,
            ended_at,
            duration_minutes * 60,
        )
    }

    fn record_session_entry(
        &self,
        task_id: Option<TaskId>,
        notes: &str,
        kind: SessionKind,
        outcome: SessionOutcome,
        next_break_kind: Option<SessionKind>,
        started_at: DateTime<Local>,
        ended_at: DateTime<Local>,
        duration_seconds: u32,
    ) -> Result<SessionEntry> {
        self.connection
            .execute(
                "INSERT INTO session_history(task_id, notes, kind, outcome, next_break_kind, started_at, ended_at, duration_seconds)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    task_id.map(|task_id| task_id.0),
                    notes,
                    kind.as_str(),
                    outcome.as_str(),
                    next_break_kind.as_ref().map(SessionKind::as_str),
                    started_at,
                    ended_at,
                    i64::from(duration_seconds)
                ],
            )
            .context("failed to insert session history entry")?;

        Ok(SessionEntry {
            id: self.connection.last_insert_rowid(),
            task_id,
            notes: notes.to_string(),
            kind,
            outcome,
            next_break_kind,
            started_at,
            ended_at,
            duration_seconds,
        })
    }

    fn update_session_task(&self, session_id: i64, task_id: Option<TaskId>) -> Result<()> {
        self.connection
            .execute(
                "UPDATE session_history SET task_id = ?1 WHERE id = ?2",
                params![task_id.map(|task_id| task_id.0), session_id],
            )
            .with_context(|| format!("failed to update task for session {}", session_id))?;
        Ok(())
    }

    fn update_session_notes(&self, session_id: i64, notes: &str) -> Result<()> {
        self.connection
            .execute(
                "UPDATE session_history SET notes = ?1 WHERE id = ?2",
                params![notes, session_id],
            )
            .with_context(|| format!("failed to update notes for session {}", session_id))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use chrono::{Local, NaiveDate, Timelike, Utc};

    use crate::domain::{
        FilterColor, FilterUpdate, ProjectColor, ProjectUpdate, TagColor, TagUpdate, TaskDue,
        TaskPriority, TaskStatus, TaskUpdate,
    };
    use crate::domain::{SessionKind, SessionOutcome};

    use super::{
        Database, FilterRepository, PomodoroRepository, ProjectRepository, SyncRepository,
        SyncStateRecord, TagRepository, TaskRepository,
    };

    fn naive_to_utc(naive: chrono::NaiveDateTime) -> chrono::DateTime<Utc> {
        chrono::DateTime::<Utc>::from_naive_utc_and_offset(naive, Utc)
    }

    #[test]
    fn in_memory_database_bootstraps_empty_state() -> Result<()> {
        // In-memory SQLite is ideal for unit tests: fast, isolated, and no
        // manual cleanup. This is the same testing goal as using a temp DB in C
        // without the boilerplate of creating and deleting files yourself.
        let database = Database::open_in_memory()?;
        let tasks = database.task_repository().list_all()?;
        let projects = database.project_repository().list_all()?;
        let now = Local::now();
        let sessions = database.pomodoro_repository().list_day(
            now - chrono::Duration::hours(1),
            now + chrono::Duration::hours(1),
        )?;
        let stats = database.pomodoro_repository().stats_for_day(
            now - chrono::Duration::hours(1),
            now + chrono::Duration::hours(1),
        )?;

        assert!(tasks.is_empty());
        assert_eq!(projects.len(), 1);
        assert!(projects[0].is_inbox);
        assert!(sessions.is_empty());
        assert_eq!(stats.total_sessions, 0);
        assert_eq!(stats.total_minutes, 0);
        assert_eq!(stats.total_work_seconds, 0);
        assert_eq!(stats.total_break_seconds, 0);
        assert_eq!(stats.completed_tasks, 0);

        Ok(())
    }

    #[test]
    fn sync_repository_persists_state_and_outbox_lifecycle() -> Result<()> {
        let database = Database::open_in_memory()?;
        let sync = database.sync_repository();
        let now = Utc::now().to_rfc3339();
        sync.upsert_state(&SyncStateRecord {
            provider: "todoist".to_string(),
            sync_token: Some("token-1".to_string()),
            last_synced_at: Some(now.clone()),
            last_status: Some("ok".to_string()),
            last_error: None,
            updated_at: now.clone(),
        })?;

        let state = sync
            .get_state("todoist")?
            .expect("todoist state should exist");
        assert_eq!(state.sync_token.as_deref(), Some("token-1"));
        assert_eq!(state.last_status.as_deref(), Some("ok"));

        let entry_id = sync.enqueue_outbox(
            "todoist",
            "task",
            42,
            "update",
            "{}",
            Utc::now().to_rfc3339().as_str(),
        )?;
        let outbox = sync.list_outbox("todoist", 10)?;
        assert_eq!(outbox.len(), 1);
        assert_eq!(outbox[0].id, entry_id);
        assert_eq!(outbox[0].entity_type, "task");

        let retry_at = Utc::now().to_rfc3339();
        let attempted_at = Utc::now().to_rfc3339();
        sync.mark_outbox_failed(
            entry_id,
            "temporary failure",
            Some("temporary"),
            Some(retry_at.as_str()),
            attempted_at.as_str(),
        )?;
        let outbox = sync.list_outbox("todoist", 10)?;
        assert_eq!(outbox[0].attempts, 1);
        assert_eq!(outbox[0].last_error.as_deref(), Some("temporary failure"));
        assert_eq!(outbox[0].error_code.as_deref(), Some("temporary"));
        assert!(outbox[0].next_attempt_at.is_some());
        assert!(outbox[0].last_attempt_at.is_some());

        sync.mark_outbox_delivered(entry_id)?;
        assert!(sync.list_outbox("todoist", 10)?.is_empty());
        Ok(())
    }

    #[test]
    fn pomodoro_repository_summarizes_completed_focus_for_stats_panels() -> Result<()> {
        let database = Database::open_in_memory()?;
        let repository = database.pomodoro_repository();
        let day = Local::now().date_naive();
        let day_start = day
            .and_hms_opt(0, 0, 0)
            .expect("midnight should be valid")
            .and_local_timezone(Local)
            .single()
            .expect("local midnight should be representable");
        let range_start = day_start - chrono::Duration::days(1);
        let range_end = day_start + chrono::Duration::days(1);

        let start_9 = day
            .and_hms_opt(9, 0, 0)
            .expect("time should be valid")
            .and_local_timezone(Local)
            .single()
            .expect("time should be representable");
        let end_9 = start_9 + chrono::Duration::minutes(25);
        repository.record_session_entry(
            None,
            "",
            SessionKind::Focus,
            SessionOutcome::Completed,
            None,
            start_9,
            end_9,
            1500,
        )?;

        let start_11 = day
            .and_hms_opt(11, 0, 0)
            .expect("time should be valid")
            .and_local_timezone(Local)
            .single()
            .expect("time should be representable");
        let end_11 = start_11 + chrono::Duration::minutes(25);
        repository.record_session_entry(
            None,
            "",
            SessionKind::Focus,
            SessionOutcome::Voided,
            None,
            start_11,
            end_11,
            1500,
        )?;

        let start_14 = day
            .and_hms_opt(14, 0, 0)
            .expect("time should be valid")
            .and_local_timezone(Local)
            .single()
            .expect("time should be representable");
        let end_14 = start_14 + chrono::Duration::minutes(20);
        repository.record_session_entry(
            None,
            "",
            SessionKind::Focus,
            SessionOutcome::Completed,
            None,
            start_14,
            end_14,
            1200,
        )?;

        let break_start = day
            .and_hms_opt(15, 0, 0)
            .expect("time should be valid")
            .and_local_timezone(Local)
            .single()
            .expect("time should be representable");
        let break_end = break_start + chrono::Duration::minutes(5);
        repository.record_session_entry(
            None,
            "",
            SessionKind::ShortBreak,
            SessionOutcome::Completed,
            None,
            break_start,
            break_end,
            300,
        )?;

        let day_summary = repository.summarize_completed_focus_days(range_start, range_end)?;
        assert_eq!(day_summary.len(), 1);
        assert_eq!(day_summary[0].day, day);
        assert_eq!(day_summary[0].focus_seconds, 2700);

        let hourly = repository.summarize_completed_focus_hours(range_start, range_end)?;
        assert_eq!(hourly.len(), 2);
        let mut seconds = hourly
            .iter()
            .map(|bucket| bucket.focus_seconds)
            .collect::<Vec<_>>();
        seconds.sort_unstable();
        assert_eq!(seconds, vec![1200, 1500]);

        Ok(())
    }

    #[test]
    fn pomodoro_summary_uses_local_calendar_day_and_hour() -> Result<()> {
        let database = Database::open_in_memory()?;
        let repository = database.pomodoro_repository();
        let day = Local::now().date_naive();
        let session_start = day
            .and_hms_opt(0, 30, 0)
            .expect("time should be valid")
            .and_local_timezone(Local)
            .single()
            .expect("time should be representable");
        let session_end = session_start + chrono::Duration::minutes(25);
        let range_start = (day - chrono::Days::new(1))
            .and_hms_opt(0, 0, 0)
            .expect("midnight should be valid")
            .and_local_timezone(Local)
            .single()
            .expect("local midnight should be representable");
        let range_end = (day + chrono::Days::new(1))
            .and_hms_opt(0, 0, 0)
            .expect("midnight should be valid")
            .and_local_timezone(Local)
            .single()
            .expect("local midnight should be representable");

        repository.record_session_entry(
            None,
            "",
            SessionKind::Focus,
            SessionOutcome::Completed,
            None,
            session_start,
            session_end,
            1500,
        )?;

        let day_summary = repository.summarize_days(range_start, range_end)?;
        assert_eq!(day_summary.len(), 1);
        assert_eq!(day_summary[0].day, session_start.date_naive());

        let hourly = repository.summarize_completed_focus_hours(range_start, range_end)?;
        assert_eq!(hourly.len(), 1);
        assert_eq!(hourly[0].hour, session_start.hour() as u8);

        Ok(())
    }

    #[test]
    fn task_repository_supports_basic_crud() -> Result<()> {
        let database = Database::open_in_memory()?;
        let repository = database.task_repository();
        let inbox_project_id = database.project_repository().inbox_project_id()?;
        let created =
            repository.create("Write release notes", inbox_project_id, None, Local::now())?;

        assert_eq!(created.title, "Write release notes");
        assert_eq!(created.status, TaskStatus::Todo);
        assert_eq!(created.priority, TaskPriority::P4);
        assert_eq!(created.description, "");
        assert_eq!(created.completed_at, None);
        assert_eq!(created.deleted_at, None);
        assert_eq!(created.due, None);

        let updated = repository.update(
            created.id,
            &TaskUpdate {
                title: "Ship release notes".to_string(),
                description: "Ship checklist".to_string(),
                project_id: inbox_project_id,
                section_id: None,
                parent_task_id: None,
                priority: TaskPriority::P4,
                due: None,
            },
        )?;
        assert_eq!(updated.title, "Ship release notes");
        assert_eq!(updated.description, "Ship checklist");
        assert_eq!(updated.status, TaskStatus::Todo);
        assert_eq!(updated.priority, TaskPriority::P4);

        let completed_at = Local::now();
        let completed =
            repository.update_status(created.id, TaskStatus::Done, Some(completed_at))?;
        assert_eq!(completed.status, TaskStatus::Done);
        assert_eq!(completed.completed_at, Some(completed_at));

        let tasks = repository.list_all()?;
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].id, created.id);
        assert_eq!(tasks[0].title, "Ship release notes");
        assert_eq!(tasks[0].description, "Ship checklist");
        assert_eq!(tasks[0].priority, TaskPriority::P4);

        repository.delete(created.id)?;
        let tasks = repository.list_all()?;
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].id, created.id);
        assert!(tasks[0].deleted_at.is_some());

        Ok(())
    }

    #[test]
    fn task_repository_persists_due_fields() -> Result<()> {
        let database = Database::open_in_memory()?;
        let repository = database.task_repository();
        let inbox_project_id = database.project_repository().inbox_project_id()?;
        let created = repository.create(
            "Ship release notes",
            inbox_project_id,
            Some(&TaskDue {
                date: NaiveDate::from_ymd_opt(2026, 4, 10).expect("valid date"),
                datetime: None,
                timezone: None,
                string: "tomorrow".to_string(),
                is_recurring: false,
            }),
            Local::now(),
        )?;

        assert_eq!(
            created.due,
            Some(TaskDue {
                date: NaiveDate::from_ymd_opt(2026, 4, 10).expect("valid date"),
                datetime: None,
                timezone: None,
                string: "tomorrow".to_string(),
                is_recurring: false,
            })
        );

        let tasks = repository.list_all()?;
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].due, created.due);

        Ok(())
    }

    #[test]
    fn task_repository_persists_due_datetime() -> Result<()> {
        let database = Database::open_in_memory()?;
        let repository = database.task_repository();
        let inbox_project_id = database.project_repository().inbox_project_id()?;
        let due_date = NaiveDate::from_ymd_opt(2026, 4, 10).expect("valid date");
        let due_datetime = due_date.and_hms_opt(15, 0, 0).expect("valid time");
        let created = repository.create(
            "Ship release notes",
            inbox_project_id,
            Some(&TaskDue {
                date: due_date,
                datetime: Some(naive_to_utc(due_datetime)),
                timezone: None,
                string: "tomorrow at 3pm".to_string(),
                is_recurring: false,
            }),
            Local::now(),
        )?;

        assert_eq!(
            created.due,
            Some(TaskDue {
                date: due_date,
                datetime: Some(naive_to_utc(due_datetime)),
                timezone: None,
                string: "tomorrow at 3pm".to_string(),
                is_recurring: false,
            })
        );

        Ok(())
    }

    #[test]
    fn pomodoro_repository_persists_and_updates_session_notes() -> Result<()> {
        let database = Database::open_in_memory()?;
        let now = Local::now();
        let started_at = now - chrono::Duration::minutes(25);
        let ended_at = now;
        let created = database.pomodoro_repository().record_session_entry(
            None,
            "Line 1\nLine 2",
            crate::domain::SessionKind::Focus,
            crate::domain::SessionOutcome::Completed,
            None,
            started_at,
            ended_at,
            1500,
        )?;

        let listed = database.pomodoro_repository().list_day(
            now - chrono::Duration::hours(1),
            now + chrono::Duration::hours(1),
        )?;
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].notes, "Line 1\nLine 2");

        database
            .pomodoro_repository()
            .update_session_notes(created.id, "Updated note")?;
        let updated = database.pomodoro_repository().list_day(
            now - chrono::Duration::hours(1),
            now + chrono::Duration::hours(1),
        )?;
        assert_eq!(updated[0].notes, "Updated note");
        Ok(())
    }

    #[test]
    fn task_repository_updates_due_fields_atomically() -> Result<()> {
        let database = Database::open_in_memory()?;
        let repository = database.task_repository();
        let inbox_project_id = database.project_repository().inbox_project_id()?;
        let created = repository.create("Draft roadmap", inbox_project_id, None, Local::now())?;
        let due_date = NaiveDate::from_ymd_opt(2026, 4, 15).expect("valid date");
        let due_datetime = due_date.and_hms_opt(9, 30, 0).expect("valid time");

        let updated = repository.update(
            created.id,
            &TaskUpdate {
                title: "Draft quarterly roadmap".to_string(),
                description: "Weekly planning note".to_string(),
                project_id: inbox_project_id,
                section_id: None,
                parent_task_id: None,
                priority: TaskPriority::P2,
                due: Some(TaskDue {
                    date: due_date,
                    datetime: Some(naive_to_utc(due_datetime)),
                    timezone: None,
                    string: "every week at 9:30am".to_string(),
                    is_recurring: true,
                }),
            },
        )?;

        assert_eq!(updated.title, "Draft quarterly roadmap");
        assert_eq!(updated.description, "Weekly planning note");
        assert_eq!(updated.priority, TaskPriority::P2);
        assert_eq!(
            updated.due,
            Some(TaskDue {
                date: due_date,
                datetime: Some(naive_to_utc(due_datetime)),
                timezone: None,
                string: "every week at 9:30am".to_string(),
                is_recurring: true,
            })
        );

        let cleared = repository.update(
            created.id,
            &TaskUpdate {
                title: "Draft quarterly roadmap".to_string(),
                description: String::new(),
                project_id: inbox_project_id,
                section_id: None,
                parent_task_id: None,
                priority: TaskPriority::P4,
                due: None,
            },
        )?;

        assert_eq!(cleared.due, None);
        assert_eq!(cleared.priority, TaskPriority::P4);
        assert_eq!(cleared.status, TaskStatus::Todo);
        assert_eq!(cleared.completed_at, None);
        Ok(())
    }

    #[test]
    fn project_repository_supports_crud_and_subtree_delete() -> Result<()> {
        let database = Database::open_in_memory()?;
        let projects = database.project_repository();
        let tasks = database.task_repository();

        let parent = projects.create("Work", None, ProjectColor::Blue, true, Local::now())?;
        let child = projects.create(
            "Client A",
            Some(parent.id),
            ProjectColor::Teal,
            false,
            Local::now(),
        )?;
        let task = tasks.create("Review brief", child.id, None, Local::now())?;

        let updated = projects.update(
            child.id,
            &ProjectUpdate {
                name: "Client Alpha".to_string(),
                parent_project_id: Some(parent.id),
                color: ProjectColor::Grape,
                is_favorite: true,
            },
        )?;
        assert_eq!(updated.name, "Client Alpha");
        assert_eq!(updated.color, ProjectColor::Grape);
        assert!(updated.is_favorite);

        projects.delete(parent.id, Local::now())?;

        let all_projects = projects.list_all()?;
        let stored_parent = all_projects
            .iter()
            .find(|project| project.id == parent.id)
            .expect("parent project should remain stored");
        let stored_child = all_projects
            .iter()
            .find(|project| project.id == child.id)
            .expect("child project should remain stored");
        assert!(stored_parent.deleted_at.is_some());
        assert!(stored_child.deleted_at.is_some());

        let all_tasks = tasks.list_all()?;
        let stored_task = all_tasks
            .iter()
            .find(|candidate| candidate.id == task.id)
            .expect("task should remain stored");
        assert!(stored_task.deleted_at.is_some());
        Ok(())
    }

    #[test]
    fn project_repository_moves_project_within_parent() -> Result<()> {
        let database = Database::open_in_memory()?;
        let projects = database.project_repository();

        let alpha = projects.create("Alpha", None, ProjectColor::Blue, false, Local::now())?;
        let bravo = projects.create("Bravo", None, ProjectColor::Blue, false, Local::now())?;
        let charlie = projects.create("Charlie", None, ProjectColor::Blue, false, Local::now())?;

        projects.move_within_parent(charlie.id, -1)?;

        let ordered_ids = projects
            .list_all()?
            .into_iter()
            .filter(|project| !project.is_inbox && project.parent_project_id.is_none())
            .map(|project| project.id)
            .collect::<Vec<_>>();

        assert_eq!(ordered_ids, vec![alpha.id, charlie.id, bravo.id]);
        Ok(())
    }

    #[test]
    fn project_repository_rejects_inbox_updates() -> Result<()> {
        let database = Database::open_in_memory()?;
        let projects = database.project_repository();
        let inbox_id = projects.inbox_project_id()?;

        let error = projects
            .update(
                inbox_id,
                &ProjectUpdate {
                    name: "Renamed Inbox".to_string(),
                    parent_project_id: None,
                    color: ProjectColor::Blue,
                    is_favorite: true,
                },
            )
            .expect_err("inbox update should fail");

        assert!(error.to_string().contains("inbox project cannot be edited"));
        Ok(())
    }

    #[test]
    fn tag_repository_supports_crud_reorder_and_detach() -> Result<()> {
        let database = Database::open_in_memory()?;
        let tags = database.tag_repository();
        let tasks = database.task_repository();
        let inbox = database.project_repository().inbox_project_id()?;

        let alpha = tags.create("alpha", TagColor::Blue, false, Local::now())?;
        let beta = tags.create("beta", TagColor::Red, true, Local::now())?;
        let task = tasks.create("Write docs", inbox, None, Local::now())?;

        tags.replace_task_tags(task.id, &[alpha.id, beta.id])?;
        let links = tags.list_task_tag_links()?;
        assert_eq!(links.len(), 2);

        let updated = tags.update(
            beta.id,
            &TagUpdate {
                name: "beta-updated".to_string(),
                color: TagColor::Green,
                is_favorite: false,
            },
        )?;
        assert_eq!(updated.name, "beta-updated");
        assert_eq!(updated.color, TagColor::Green);
        assert!(!updated.is_favorite);

        tags.move_within_list(beta.id, -1)?;
        let ordered = tags
            .list_all()?
            .into_iter()
            .filter(|tag| tag.deleted_at.is_none())
            .map(|tag| tag.id)
            .collect::<Vec<_>>();
        assert_eq!(ordered, vec![beta.id, alpha.id]);

        tags.delete(beta.id, Local::now())?;
        let links_after_delete = tags.list_task_tag_links()?;
        assert_eq!(links_after_delete, vec![(task.id, alpha.id)]);
        Ok(())
    }

    #[test]
    fn filter_repository_supports_crud_and_reorder() -> Result<()> {
        let database = Database::open_in_memory()?;
        let filters = database.filter_repository();

        let alpha = filters.create("alpha", "@work", FilterColor::Blue, false, Local::now())?;
        let beta = filters.create(
            "beta",
            "today & !@personal",
            FilterColor::Red,
            true,
            Local::now(),
        )?;

        let updated = filters.update(
            beta.id,
            &FilterUpdate {
                name: "beta-updated".to_string(),
                query: "p1 | overdue".to_string(),
                color: FilterColor::Green,
                is_favorite: false,
            },
        )?;
        assert_eq!(updated.name, "beta-updated");
        assert_eq!(updated.query, "p1 | overdue");
        assert_eq!(updated.color, FilterColor::Green);
        assert!(!updated.is_favorite);

        filters.move_within_list(beta.id, -1)?;
        let ordered = filters
            .list_all()?
            .into_iter()
            .filter(|filter| filter.deleted_at.is_none())
            .map(|filter| filter.id)
            .collect::<Vec<_>>();
        assert_eq!(ordered, vec![beta.id, alpha.id]);

        filters.delete(beta.id, Local::now())?;
        let active = filters
            .list_all()?
            .into_iter()
            .filter(|filter| filter.deleted_at.is_none())
            .map(|filter| filter.id)
            .collect::<Vec<_>>();
        assert_eq!(active, vec![alpha.id]);
        Ok(())
    }
}
