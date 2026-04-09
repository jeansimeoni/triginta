use std::path::Path;

use anyhow::{Context, Result};
use chrono::{DateTime, Local};
use rusqlite::{Connection, OptionalExtension, params};

use crate::domain::{HistoryStats, PomodoroId, PomodoroSession, Task, TaskId, TaskStatus};

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
}

pub trait PomodoroRepository {
    fn list_recent(&self, limit: usize) -> Result<Vec<PomodoroSession>>;
    fn stats(&self) -> Result<HistoryStats>;
    fn create(
        &self,
        task_id: Option<TaskId>,
        started_at: DateTime<Local>,
        ended_at: DateTime<Local>,
        duration_minutes: u32,
    ) -> Result<PomodoroSession>;
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
}

pub struct SqlitePomodoroRepository<'a> {
    connection: &'a Connection,
}

impl PomodoroRepository for SqlitePomodoroRepository<'_> {
    fn list_recent(&self, limit: usize) -> Result<Vec<PomodoroSession>> {
        let mut statement = self.connection.prepare(
            "SELECT id, task_id, started_at, ended_at, duration_minutes
             FROM pomodoros
             ORDER BY started_at DESC, id DESC
             LIMIT ?1",
        )?;

        let rows = statement.query_map([limit as i64], |row| {
            Ok(PomodoroSession {
                id: PomodoroId(row.get(0)?),
                // `Option<T>` replaces the common C pattern of sentinel values
                // or NULL checks. `map(TaskId)` converts `Some(i64)` into
                // `Some(TaskId)` and leaves `None` unchanged.
                task_id: row.get::<_, Option<i64>>(1)?.map(TaskId),
                started_at: row.get(2)?,
                ended_at: row.get(3)?,
                duration_minutes: row.get::<_, i64>(4)? as u32,
            })
        })?;

        rows.collect::<std::result::Result<Vec<_>, _>>()
            .context("failed to load pomodoro history")
    }

    fn stats(&self) -> Result<HistoryStats> {
        let mut statement = self.connection.prepare(
            "SELECT
                COUNT(*) AS total_sessions,
                COALESCE(SUM(duration_minutes), 0) AS total_minutes
             FROM pomodoros",
        )?;

        let (total_sessions, total_minutes): (i64, i64) = statement
            .query_row([], |row| Ok((row.get(0)?, row.get(1)?)))
            .context("failed to compute pomodoro stats")?;

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
            total_sessions: total_sessions as usize,
            total_minutes: total_minutes as u32,
            completed_tasks: completed_tasks as usize,
        })
    }

    fn create(
        &self,
        task_id: Option<TaskId>,
        started_at: DateTime<Local>,
        ended_at: DateTime<Local>,
        duration_minutes: u32,
    ) -> Result<PomodoroSession> {
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

        Ok(PomodoroSession {
            id: PomodoroId(self.connection.last_insert_rowid()),
            task_id,
            started_at,
            ended_at: Some(ended_at),
            duration_minutes,
        })
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Result;

    use super::{Database, PomodoroRepository, TaskRepository};

    #[test]
    fn in_memory_database_bootstraps_empty_state() -> Result<()> {
        // In-memory SQLite is ideal for unit tests: fast, isolated, and no
        // manual cleanup. This is the same testing goal as using a temp DB in C
        // without the boilerplate of creating and deleting files yourself.
        let database = Database::open_in_memory()?;
        let tasks = database.task_repository().list_all()?;
        let sessions = database.pomodoro_repository().list_recent(25)?;
        let stats = database.pomodoro_repository().stats()?;

        assert!(tasks.is_empty());
        assert!(sessions.is_empty());
        assert_eq!(stats.total_sessions, 0);
        assert_eq!(stats.total_minutes, 0);
        assert_eq!(stats.completed_tasks, 0);

        Ok(())
    }
}
