use std::path::Path;

use anyhow::{Context, Result};
use chrono::{DateTime, Local};
use rusqlite::{Connection, OptionalExtension, params};

use crate::domain::{
    DayHistorySummary, HistoryStats, SessionEntry, SessionKind, SessionOutcome, Task, TaskId,
    TaskStatus,
};

// Keeping the schema as a string literal makes bootstrap simple for this early
// vertical slice. `execute_batch` sends the whole script to SQLite at once,
// which is similar to feeding a schema file to sqlite3 in a C program.
const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS tasks (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    title TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'todo',
    created_at TEXT NOT NULL,
    completed_at TEXT
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
"#;

// These traits define the storage-facing API the rest of the app relies on.
// This separation matters because it keeps the higher layers talking in domain
// terms (`Task`, `PomodoroSession`) instead of raw SQL concepts.
pub trait TaskRepository {
    fn list_all(&self) -> Result<Vec<Task>>;
    fn create(&self, title: &str, now: DateTime<Local>) -> Result<Task>;
    fn update_title(&self, task_id: TaskId, title: &str) -> Result<Task>;
    fn update_status(
        &self,
        task_id: TaskId,
        status: TaskStatus,
        completed_at: Option<DateTime<Local>>,
    ) -> Result<Task>;
    fn delete(&self, task_id: TaskId) -> Result<()>;
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
    fn create(
        &self,
        task_id: Option<TaskId>,
        next_break_kind: Option<SessionKind>,
        started_at: DateTime<Local>,
        ended_at: DateTime<Local>,
        duration_minutes: u32,
    ) -> Result<SessionEntry>;
    fn record_session_entry(
        &self,
        task_id: Option<TaskId>,
        kind: SessionKind,
        outcome: SessionOutcome,
        next_break_kind: Option<SessionKind>,
        started_at: DateTime<Local>,
        ended_at: DateTime<Local>,
        duration_seconds: u32,
    ) -> Result<SessionEntry>;
    fn update_session_task(&self, session_id: i64, task_id: Option<TaskId>) -> Result<()>;
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
        self.connection
            .execute(
                "INSERT OR IGNORE INTO app_metadata(key, value) VALUES (?1, ?2)",
                params!["schema_version", "1"],
            )
            .context("failed to initialize app metadata")?;
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
            "SELECT id, title, status, created_at, completed_at
             FROM tasks
             ORDER BY created_at DESC, id DESC",
        )?;

        let rows = statement.query_map([], |row| {
            Ok(Task {
                id: TaskId(row.get(0)?),
                title: row.get(1)?,
                status: TaskStatus::from_db(row.get::<_, String>(2)?.as_str()),
                created_at: row.get(3)?,
                completed_at: row.get(4)?,
            })
        })?;

        // `collect` turns the iterator of per-row results into one
        // `Result<Vec<Task>>`, stopping early if any row conversion fails.
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .context("failed to load tasks")
    }

    fn create(&self, title: &str, now: DateTime<Local>) -> Result<Task> {
        self.connection
            .execute(
                "INSERT INTO tasks(title, status, created_at, completed_at)
                 VALUES (?1, ?2, ?3, NULL)",
                params![title, TaskStatus::Todo.as_str(), now],
            )
            .context("failed to create task")?;

        let task_id = TaskId(self.connection.last_insert_rowid());
        self.load_task(task_id)?
            .context("created task could not be reloaded")
    }

    fn update_title(&self, task_id: TaskId, title: &str) -> Result<Task> {
        self.connection
            .execute(
                "UPDATE tasks SET title = ?1 WHERE id = ?2",
                params![title, task_id.0],
            )
            .with_context(|| format!("failed to rename task {}", task_id.0))?;

        self.load_task(task_id)?
            .context("renamed task could not be reloaded")
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
                 SET status = ?1, completed_at = ?2
                 WHERE id = ?3",
                params![status.as_str(), completed_at, task_id.0],
            )
            .with_context(|| format!("failed to update task status for {}", task_id.0))?;

        self.load_task(task_id)?
            .context("updated task could not be reloaded")
    }

    fn delete(&self, task_id: TaskId) -> Result<()> {
        self.connection
            .execute("DELETE FROM tasks WHERE id = ?1", params![task_id.0])
            .with_context(|| format!("failed to delete task {}", task_id.0))?;
        Ok(())
    }
}

impl SqliteTaskRepository<'_> {
    fn load_task(&self, task_id: TaskId) -> Result<Option<Task>> {
        self.connection
            .query_row(
                "SELECT id, title, status, created_at, completed_at
                 FROM tasks
                 WHERE id = ?1",
                params![task_id.0],
                |row| {
                    Ok(Task {
                        id: TaskId(row.get(0)?),
                        title: row.get(1)?,
                        status: TaskStatus::from_db(row.get::<_, String>(2)?.as_str()),
                        created_at: row.get(3)?,
                        completed_at: row.get(4)?,
                    })
                },
            )
            .optional()
            .context("failed to load task")
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
            "SELECT id, task_id, kind, outcome, next_break_kind, started_at, ended_at, duration_seconds
             FROM session_history
             WHERE started_at >= ?1 AND started_at < ?2
             ORDER BY started_at DESC, id DESC
            ",
        )?;

        let rows = statement.query_map(params![started_at, ended_at], |row| {
            Ok(SessionEntry {
                id: row.get(0)?,
                task_id: row.get::<_, Option<i64>>(1)?.map(TaskId),
                kind: SessionKind::from_db(row.get::<_, String>(2)?.as_str()),
                outcome: SessionOutcome::from_db(row.get::<_, String>(3)?.as_str()),
                next_break_kind: row
                    .get::<_, Option<String>>(4)?
                    .map(|value| SessionKind::from_db(value.as_str())),
                started_at: row.get(5)?,
                ended_at: row.get(6)?,
                duration_seconds: row.get::<_, i64>(7)? as u32,
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
                DATE(started_at) AS day,
                COALESCE(SUM(CASE WHEN kind IN ('focus', 'work') AND outcome = 'completed' THEN 1 ELSE 0 END), 0) AS completed_sessions,
                COALESCE(SUM(CASE WHEN kind IN ('focus', 'work') AND outcome = 'voided' THEN 1 ELSE 0 END), 0) AS voided_sessions,
                COALESCE(SUM(CASE WHEN kind IN ('focus', 'work') THEN duration_seconds ELSE 0 END), 0) AS focus_seconds,
                COALESCE(SUM(CASE WHEN kind IN ('short_break', 'long_break') THEN duration_seconds ELSE 0 END), 0) AS break_seconds
             FROM session_history
             WHERE started_at >= ?1 AND started_at < ?2
             GROUP BY DATE(started_at)
             ORDER BY DATE(started_at) DESC",
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

    fn create(
        &self,
        task_id: Option<TaskId>,
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
        kind: SessionKind,
        outcome: SessionOutcome,
        next_break_kind: Option<SessionKind>,
        started_at: DateTime<Local>,
        ended_at: DateTime<Local>,
        duration_seconds: u32,
    ) -> Result<SessionEntry> {
        self.connection
            .execute(
                "INSERT INTO session_history(task_id, kind, outcome, next_break_kind, started_at, ended_at, duration_seconds)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    task_id.map(|task_id| task_id.0),
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
}

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use chrono::Local;

    use crate::domain::TaskStatus;

    use super::{Database, PomodoroRepository, TaskRepository};

    #[test]
    fn in_memory_database_bootstraps_empty_state() -> Result<()> {
        // In-memory SQLite is ideal for unit tests: fast, isolated, and no
        // manual cleanup. This is the same testing goal as using a temp DB in C
        // without the boilerplate of creating and deleting files yourself.
        let database = Database::open_in_memory()?;
        let tasks = database.task_repository().list_all()?;
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
        assert!(sessions.is_empty());
        assert_eq!(stats.total_sessions, 0);
        assert_eq!(stats.total_minutes, 0);
        assert_eq!(stats.total_work_seconds, 0);
        assert_eq!(stats.total_break_seconds, 0);
        assert_eq!(stats.completed_tasks, 0);

        Ok(())
    }

    #[test]
    fn task_repository_supports_basic_crud() -> Result<()> {
        let database = Database::open_in_memory()?;
        let repository = database.task_repository();
        let created = repository.create("Write release notes", Local::now())?;

        assert_eq!(created.title, "Write release notes");
        assert_eq!(created.status, TaskStatus::Todo);
        assert_eq!(created.completed_at, None);

        let renamed = repository.update_title(created.id, "Ship release notes")?;
        assert_eq!(renamed.title, "Ship release notes");
        assert_eq!(renamed.status, TaskStatus::Todo);

        let completed_at = Local::now();
        let completed =
            repository.update_status(created.id, TaskStatus::Done, Some(completed_at))?;
        assert_eq!(completed.status, TaskStatus::Done);
        assert_eq!(completed.completed_at, Some(completed_at));

        let tasks = repository.list_all()?;
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].id, created.id);
        assert_eq!(tasks[0].title, "Ship release notes");

        repository.delete(created.id)?;
        assert!(repository.list_all()?.is_empty());

        Ok(())
    }
}
