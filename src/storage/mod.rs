use std::path::Path;

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Local};
use rusqlite::{Connection, OptionalExtension, params};

use crate::domain::{
    DayHistorySummary, HistoryStats, Project, ProjectColor, ProjectId, ProjectUpdate, SessionEntry,
    SessionKind, SessionOutcome, Task, TaskDue, TaskId, TaskStatus, TaskUpdate,
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
    deleted_at TEXT,
    FOREIGN KEY(parent_project_id) REFERENCES projects(id)
);

CREATE TABLE IF NOT EXISTS tasks (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id INTEGER,
    title TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'todo',
    created_at TEXT NOT NULL,
    completed_at TEXT,
    deleted_at TEXT,
    due_date TEXT,
    due_datetime TEXT,
    due_string TEXT,
    due_is_recurring INTEGER NOT NULL DEFAULT 0,
    FOREIGN KEY(project_id) REFERENCES projects(id)
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
        self.ensure_tasks_column("deleted_at", "ALTER TABLE tasks ADD COLUMN deleted_at TEXT")?;
        self.ensure_tasks_column(
            "project_id",
            "ALTER TABLE tasks ADD COLUMN project_id INTEGER",
        )?;
        self.ensure_tasks_column("due_date", "ALTER TABLE tasks ADD COLUMN due_date TEXT")?;
        self.ensure_tasks_column(
            "due_datetime",
            "ALTER TABLE tasks ADD COLUMN due_datetime TEXT",
        )?;
        self.ensure_tasks_column("due_string", "ALTER TABLE tasks ADD COLUMN due_string TEXT")?;
        self.ensure_tasks_column(
            "due_is_recurring",
            "ALTER TABLE tasks ADD COLUMN due_is_recurring INTEGER NOT NULL DEFAULT 0",
        )?;
        self.connection
            .execute(
                "INSERT OR IGNORE INTO app_metadata(key, value) VALUES (?1, ?2)",
                params!["schema_version", "1"],
            )
            .context("failed to initialize app metadata")?;
        let inbox_project_id = self.ensure_inbox_project()?;
        self.assign_tasks_to_inbox(inbox_project_id)?;
        Ok(())
    }

    fn ensure_tasks_column(&self, column_name: &str, alter_sql: &str) -> Result<()> {
        let mut statement = self
            .connection
            .prepare("PRAGMA table_info(tasks)")
            .context("failed to inspect task schema")?;
        let columns = statement
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<std::result::Result<Vec<_>, _>>()
            .context("failed to read task schema")?;

        if columns.iter().any(|column| column == column_name) {
            return Ok(());
        }

        self.connection
            .execute(alter_sql, [])
            .with_context(|| format!("failed to add {} column to tasks", column_name))?;
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
                "INSERT INTO projects(name, parent_project_id, color, is_favorite, is_inbox, child_order, created_at, deleted_at)
                 VALUES (?1, NULL, ?2, 0, 1, 0, ?3, NULL)",
                params!["Inbox", ProjectColor::Charcoal.as_str(), Local::now()],
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

pub struct SqliteTaskRepository<'a> {
    connection: &'a Connection,
}

impl TaskRepository for SqliteTaskRepository<'_> {
    fn list_all(&self) -> Result<Vec<Task>> {
        // `prepare` compiles SQL once and `query_map` walks each row through a
        // closure. The closure is conceptually similar to a row-to-struct
        // callback in C, but its return type is checked by the compiler.
        let mut statement = self.connection.prepare(
            "SELECT id, project_id, title, status, created_at, completed_at, deleted_at, due_date, due_datetime, due_string, due_is_recurring
             FROM tasks
             ORDER BY created_at DESC, id DESC",
        )?;

        let rows = statement.query_map([], |row| {
            Ok(Task {
                id: TaskId(row.get(0)?),
                project_id: ProjectId(row.get(1)?),
                title: row.get(2)?,
                status: TaskStatus::from_db(row.get::<_, String>(3)?.as_str()),
                created_at: row.get(4)?,
                completed_at: row.get(5)?,
                deleted_at: row.get(6)?,
                due: match (
                    row.get::<_, Option<chrono::NaiveDate>>(7)?,
                    row.get::<_, Option<chrono::NaiveDateTime>>(8)?,
                    row.get::<_, Option<String>>(9)?,
                    row.get::<_, i64>(10)?,
                ) {
                    (Some(date), datetime, Some(string), is_recurring) => Some(TaskDue {
                        date,
                        datetime,
                        string,
                        is_recurring: is_recurring != 0,
                    }),
                    _ => None,
                },
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
        self.connection
            .execute(
                "INSERT INTO tasks(project_id, title, status, created_at, completed_at, due_date, due_datetime, due_string, due_is_recurring)
                 VALUES (?1, ?2, ?3, ?4, NULL, ?5, ?6, ?7, ?8)",
                params![
                    project_id.0,
                    title,
                    TaskStatus::Todo.as_str(),
                    now,
                    due.map(|due| due.date),
                    due.and_then(|due| due.datetime),
                    due.map(|due| due.string.clone()),
                    due.map(|due| if due.is_recurring { 1_i64 } else { 0_i64 })
                        .unwrap_or(0_i64),
                ],
            )
            .context("failed to create task")?;

        let task_id = TaskId(self.connection.last_insert_rowid());
        self.load_task(task_id)?
            .context("created task could not be reloaded")
    }

    fn update(&self, task_id: TaskId, update: &TaskUpdate) -> Result<Task> {
        self.connection
            .execute(
                "UPDATE tasks
                 SET title = ?1, project_id = ?2, due_date = ?3, due_datetime = ?4, due_string = ?5, due_is_recurring = ?6
                 WHERE id = ?7",
                params![
                    update.title,
                    update.project_id.0,
                    update.due.as_ref().map(|due| due.date),
                    update.due.as_ref().and_then(|due| due.datetime),
                    update.due.as_ref().map(|due| due.string.clone()),
                    update
                        .due
                        .as_ref()
                        .map(|due| if due.is_recurring { 1_i64 } else { 0_i64 })
                        .unwrap_or(0_i64),
                    task_id.0,
                ],
            )
            .with_context(|| format!("failed to update task {}", task_id.0))?;

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
            .execute(
                "UPDATE tasks SET deleted_at = ?1 WHERE id = ?2",
                params![Local::now(), task_id.0],
            )
            .with_context(|| format!("failed to soft-delete task {}", task_id.0))?;
        Ok(())
    }
}

impl SqliteTaskRepository<'_> {
    fn load_task(&self, task_id: TaskId) -> Result<Option<Task>> {
        self.connection
            .query_row(
                "SELECT id, project_id, title, status, created_at, completed_at
                 , deleted_at, due_date, due_datetime, due_string, due_is_recurring
                 FROM tasks
                 WHERE id = ?1",
                params![task_id.0],
                |row| {
                    Ok(Task {
                        id: TaskId(row.get(0)?),
                        project_id: ProjectId(row.get(1)?),
                        title: row.get(2)?,
                        status: TaskStatus::from_db(row.get::<_, String>(3)?.as_str()),
                        created_at: row.get(4)?,
                        completed_at: row.get(5)?,
                        deleted_at: row.get(6)?,
                        due: match (
                            row.get::<_, Option<chrono::NaiveDate>>(7)?,
                            row.get::<_, Option<chrono::NaiveDateTime>>(8)?,
                            row.get::<_, Option<String>>(9)?,
                            row.get::<_, i64>(10)?,
                        ) {
                            (Some(date), datetime, Some(string), is_recurring) => Some(TaskDue {
                                date,
                                datetime,
                                string,
                                is_recurring: is_recurring != 0,
                            }),
                            _ => None,
                        },
                    })
                },
            )
            .optional()
            .context("failed to load task")
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
                "INSERT INTO projects(name, parent_project_id, color, is_favorite, is_inbox, child_order, created_at, deleted_at)
                 VALUES (?1, ?2, ?3, ?4, 0, ?5, ?6, NULL)",
                params![
                    name,
                    parent_project_id.map(|id| id.0),
                    color.as_str(),
                    if is_favorite { 1_i64 } else { 0_i64 },
                    child_order,
                    now,
                ],
            )
            .context("failed to create project")?;
        self.load_project(ProjectId(self.connection.last_insert_rowid()))?
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
                 SET name = ?1, parent_project_id = ?2, color = ?3, is_favorite = ?4, child_order = ?5
                 WHERE id = ?6",
                params![
                    update.name,
                    update.parent_project_id.map(|id| id.0),
                    update.color.as_str(),
                    if update.is_favorite { 1_i64 } else { 0_i64 },
                    child_order,
                    project_id.0,
                ],
            )
            .with_context(|| format!("failed to update project {}", project_id.0))?;

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
                 SET deleted_at = ?2
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
                 SET deleted_at = COALESCE(deleted_at, ?2)
                 WHERE project_id IN (SELECT id FROM project_tree)",
                params![project_id.0, now],
            )
            .with_context(|| format!("failed to soft-delete tasks for project {}", project_id.0))?;
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
    use chrono::{Local, NaiveDate};

    use crate::domain::{ProjectColor, ProjectUpdate, TaskDue, TaskStatus, TaskUpdate};

    use super::{Database, PomodoroRepository, ProjectRepository, TaskRepository};

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
    fn task_repository_supports_basic_crud() -> Result<()> {
        let database = Database::open_in_memory()?;
        let repository = database.task_repository();
        let inbox_project_id = database.project_repository().inbox_project_id()?;
        let created =
            repository.create("Write release notes", inbox_project_id, None, Local::now())?;

        assert_eq!(created.title, "Write release notes");
        assert_eq!(created.status, TaskStatus::Todo);
        assert_eq!(created.completed_at, None);
        assert_eq!(created.deleted_at, None);
        assert_eq!(created.due, None);

        let updated = repository.update(
            created.id,
            &TaskUpdate {
                title: "Ship release notes".to_string(),
                project_id: inbox_project_id,
                due: None,
            },
        )?;
        assert_eq!(updated.title, "Ship release notes");
        assert_eq!(updated.status, TaskStatus::Todo);

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
                datetime: Some(due_datetime),
                string: "tomorrow at 3pm".to_string(),
                is_recurring: false,
            }),
            Local::now(),
        )?;

        assert_eq!(
            created.due,
            Some(TaskDue {
                date: due_date,
                datetime: Some(due_datetime),
                string: "tomorrow at 3pm".to_string(),
                is_recurring: false,
            })
        );

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
                project_id: inbox_project_id,
                due: Some(TaskDue {
                    date: due_date,
                    datetime: Some(due_datetime),
                    string: "every week at 9:30am".to_string(),
                    is_recurring: true,
                }),
            },
        )?;

        assert_eq!(updated.title, "Draft quarterly roadmap");
        assert_eq!(
            updated.due,
            Some(TaskDue {
                date: due_date,
                datetime: Some(due_datetime),
                string: "every week at 9:30am".to_string(),
                is_recurring: true,
            })
        );

        let cleared = repository.update(
            created.id,
            &TaskUpdate {
                title: "Draft quarterly roadmap".to_string(),
                project_id: inbox_project_id,
                due: None,
            },
        )?;

        assert_eq!(cleared.due, None);
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
}
