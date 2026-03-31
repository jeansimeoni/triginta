use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, params};

use crate::domain::{HistoryStats, PomodoroId, PomodoroSession, Task, TaskId, TaskStatus};

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

pub trait TaskRepository {
    fn list_all(&self) -> Result<Vec<Task>>;
}

pub trait PomodoroRepository {
    fn list_recent(&self, limit: usize) -> Result<Vec<PomodoroSession>>;
    fn stats(&self) -> Result<HistoryStats>;
}

#[derive(Debug)]
pub struct Database {
    connection: Connection,
}

impl Database {
    pub fn open(path: &Path) -> Result<Self> {
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
}

#[cfg(test)]
mod tests {
    use anyhow::Result;

    use super::{Database, PomodoroRepository, TaskRepository};

    #[test]
    fn in_memory_database_bootstraps_empty_state() -> Result<()> {
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
