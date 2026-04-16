use std::{
    collections::HashMap,
    io::Read,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, anyhow, bail};
use chrono::{DateTime, Utc};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tracing::info;
use uuid::Uuid;

use crate::{
    config::{TodoistIntegrationConfig, TodoistTokenSource},
    storage::{
        RemoteFilterRecord, RemoteProjectRecord, RemoteSectionRecord, RemoteTagRecord,
        RemoteTaskRecord, SyncApplyOutcome, SyncEntitySnapshot, SyncFilterSnapshot,
        SyncOutboxEntry, SyncProjectSnapshot, SyncRepository, SyncSectionSnapshot, SyncStateRecord,
        SyncTagSnapshot, SyncTaskSnapshot,
    },
};

const TODOIST_REST_BASE: &str = "https://api.todoist.com/rest/v2";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncTrigger {
    Startup,
    MutationDebounced,
    Poll,
}

impl SyncTrigger {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Startup => "startup",
            Self::MutationDebounced => "mutation_debounced",
            Self::Poll => "poll",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncReport {
    pub provider: &'static str,
    pub configured: bool,
    pub trigger: SyncTrigger,
    pub status: String,
    pub pending_outbox: usize,
    pub delivered_outbox: usize,
    pub failed_outbox: usize,
    pub last_error: Option<String>,
}

pub trait TaskSyncProvider {
    fn provider_name(&self) -> &'static str;
    fn is_configured(&self) -> bool;
    fn sync(
        &self,
        sync_repository: &dyn SyncRepository,
        trigger: SyncTrigger,
    ) -> Result<SyncReport>;
}

#[derive(Debug, Clone)]
pub struct TodoistSyncProvider {
    config: TodoistIntegrationConfig,
    dry_run: bool,
}

impl TodoistSyncProvider {
    pub fn new(config: TodoistIntegrationConfig, dry_run: bool) -> Self {
        Self { config, dry_run }
    }

    fn resolve_token(&self) -> Result<String> {
        match self.config.token_source {
            TodoistTokenSource::Env => std::env::var(self.config.token_env_var.as_str())
                .with_context(|| {
                    format!(
                        "missing Todoist token in env var {}",
                        self.config.token_env_var
                    )
                })
                .and_then(|value| {
                    if value.trim().is_empty() {
                        bail!(
                            "Todoist token env var {} is empty",
                            self.config.token_env_var
                        );
                    }
                    Ok(value)
                }),
            TodoistTokenSource::Command => {
                let command = self
                    .config
                    .token_command
                    .as_ref()
                    .context("token command config is missing")?;
                read_token_from_command(
                    command.program.as_str(),
                    &command.args,
                    Duration::from_millis(command.timeout_ms),
                )
            }
        }
    }

    fn coalesce_outbox(entries: Vec<SyncOutboxEntry>) -> Vec<SyncOutboxEntry> {
        let mut latest_by_entity: HashMap<(String, i64), SyncOutboxEntry> = HashMap::new();
        for entry in entries {
            let key = (entry.entity_type.clone(), entry.entity_local_id);
            let should_replace = match latest_by_entity.get(&key) {
                None => true,
                Some(previous) => {
                    if entry.op_kind == "delete" {
                        true
                    } else if previous.op_kind == "delete" {
                        false
                    } else {
                        entry.id > previous.id
                    }
                }
            };
            if should_replace {
                latest_by_entity.insert(key, entry);
            }
        }

        let mut values = latest_by_entity.into_values().collect::<Vec<_>>();
        values.sort_by_key(|entry| entry.id);
        values
    }

    fn next_retry_at(&self, attempts_after_failure: i64, now: DateTime<Utc>) -> String {
        let base_seconds: i64 = 30;
        let exp = attempts_after_failure.saturating_sub(1).min(10) as u32;
        let multiplier = 2_i64.pow(exp);
        let max_seconds = self
            .config
            .sync_runtime
            .poll_max_interval_seconds
            .saturating_mul(3) as i64;
        let seconds = (base_seconds.saturating_mul(multiplier)).min(max_seconds.max(base_seconds));
        (now + chrono::Duration::seconds(seconds)).to_rfc3339()
    }

    fn process_ready_outbox(
        &self,
        sync_repository: &dyn SyncRepository,
        now: DateTime<Utc>,
        client: &TodoistRestClient,
    ) -> Result<(usize, usize, Option<String>)> {
        let ready = sync_repository.list_ready_outbox(
            self.provider_name(),
            i64::from(self.config.sync_runtime.max_batch_size),
            now.to_rfc3339().as_str(),
        )?;
        let ready = Self::coalesce_outbox(ready);

        if ready.is_empty() {
            return Ok((0, 0, None));
        }

        let mut delivered = 0usize;
        let mut failed = 0usize;
        let mut last_error = None;

        for entry in ready {
            let snapshot = sync_repository
                .load_entity_snapshot(entry.entity_type.as_str(), entry.entity_local_id)?;

            let step_result =
                self.apply_outbox_entry(sync_repository, client, now, &entry, snapshot);

            match step_result {
                Ok(()) => delivered += 1,
                Err(error) => {
                    failed += 1;
                    let now_rfc3339 = now.to_rfc3339();
                    let attempts_after_failure = entry.attempts + 1;
                    let retryable = attempts_after_failure
                        < i64::from(self.config.sync_runtime.max_retry_attempts);
                    let next_attempt_at = if retryable {
                        Some(self.next_retry_at(attempts_after_failure, now))
                    } else {
                        None
                    };
                    let error_string = error.to_string();
                    sync_repository.mark_outbox_failed(
                        entry.id,
                        error_string.as_str(),
                        Some(if retryable {
                            "transport_error"
                        } else {
                            "retry_exhausted"
                        }),
                        next_attempt_at.as_deref(),
                        now_rfc3339.as_str(),
                    )?;
                    last_error = Some(error_string);
                }
            }
        }

        Ok((delivered, failed, last_error))
    }

    fn apply_outbox_entry(
        &self,
        sync_repository: &dyn SyncRepository,
        client: &TodoistRestClient,
        now: DateTime<Utc>,
        entry: &SyncOutboxEntry,
        snapshot: Option<SyncEntitySnapshot>,
    ) -> Result<()> {
        let now_rfc3339 = now.to_rfc3339();
        let summary = format!(
            "{}#{} {}",
            entry.entity_type, entry.entity_local_id, entry.op_kind
        );

        if self.dry_run {
            info!(target: "integrations.todoist", entry_id = entry.id, summary = %summary, "dry-run sync action");
            return Ok(());
        }

        let Some(snapshot) = snapshot else {
            // Entity vanished locally; the outbox entry can be drained.
            sync_repository.mark_outbox_delivered(entry.id)?;
            return Ok(());
        };

        match snapshot {
            SyncEntitySnapshot::Task(task) => {
                let created_id = self.sync_task(entry.op_kind.as_str(), &task, client)?;
                if let Some(created_id) = created_id {
                    sync_repository.set_entity_todoist_id(
                        "task",
                        task.local_id,
                        created_id.as_str(),
                        now_rfc3339.as_str(),
                    )?;
                } else if task.todoist_id.is_some() {
                    sync_repository.mark_entity_synced(
                        "task",
                        task.local_id,
                        now_rfc3339.as_str(),
                    )?;
                }
            }
            SyncEntitySnapshot::Project(project) => {
                let created_id = self.sync_project(entry.op_kind.as_str(), &project, client)?;
                if let Some(created_id) = created_id {
                    sync_repository.set_entity_todoist_id(
                        "project",
                        project.local_id,
                        created_id.as_str(),
                        now_rfc3339.as_str(),
                    )?;
                } else if project.todoist_id.is_some() {
                    sync_repository.mark_entity_synced(
                        "project",
                        project.local_id,
                        now_rfc3339.as_str(),
                    )?;
                }
            }
            SyncEntitySnapshot::Section(section) => {
                let created_id = self.sync_section(entry.op_kind.as_str(), &section, client)?;
                if let Some(created_id) = created_id {
                    sync_repository.set_entity_todoist_id(
                        "section",
                        section.local_id,
                        created_id.as_str(),
                        now_rfc3339.as_str(),
                    )?;
                } else if section.todoist_id.is_some() {
                    sync_repository.mark_entity_synced(
                        "section",
                        section.local_id,
                        now_rfc3339.as_str(),
                    )?;
                }
            }
            SyncEntitySnapshot::Tag(tag) => {
                let created_id = self.sync_tag(entry.op_kind.as_str(), &tag, client)?;
                if let Some(created_id) = created_id {
                    sync_repository.set_entity_todoist_id(
                        "tag",
                        tag.local_id,
                        created_id.as_str(),
                        now_rfc3339.as_str(),
                    )?;
                } else if tag.todoist_id.is_some() {
                    sync_repository.mark_entity_synced(
                        "tag",
                        tag.local_id,
                        now_rfc3339.as_str(),
                    )?;
                }
            }
            SyncEntitySnapshot::Filter(filter) => {
                let created_id = self.sync_filter(entry.op_kind.as_str(), &filter, client)?;
                if let Some(created_id) = created_id {
                    sync_repository.set_entity_todoist_id(
                        "filter",
                        filter.local_id,
                        created_id.as_str(),
                        now_rfc3339.as_str(),
                    )?;
                } else if filter.todoist_id.is_some() {
                    sync_repository.mark_entity_synced(
                        "filter",
                        filter.local_id,
                        now_rfc3339.as_str(),
                    )?;
                }
            }
        }

        sync_repository.mark_outbox_delivered(entry.id)?;
        Ok(())
    }

    fn sync_task(
        &self,
        op_kind: &str,
        task: &SyncTaskSnapshot,
        client: &TodoistRestClient,
    ) -> Result<Option<String>> {
        if op_kind == "delete" || task.deleted_at.is_some() {
            if let Some(remote_id) = task.todoist_id.as_deref() {
                client.delete(&format!("/tasks/{remote_id}"))?;
            }
            return Ok(None);
        }

        let mut body = serde_json::Map::new();
        body.insert("content".to_string(), Value::String(task.title.clone()));
        body.insert(
            "description".to_string(),
            Value::String(task.description.clone()),
        );
        body.insert("priority".to_string(), Value::Number(task.priority.into()));
        if let Some(project_id) = task.project_todoist_id.as_deref() {
            body.insert(
                "project_id".to_string(),
                Value::String(project_id.to_string()),
            );
        }
        if let Some(section_id) = task.section_todoist_id.as_deref() {
            body.insert(
                "section_id".to_string(),
                Value::String(section_id.to_string()),
            );
        }
        if let Some(parent_id) = task.parent_todoist_id.as_deref() {
            body.insert(
                "parent_id".to_string(),
                Value::String(parent_id.to_string()),
            );
        }
        if let Some(due_string) = task.due_string.as_ref() {
            body.insert("due_string".to_string(), Value::String(due_string.clone()));
        }
        if !task.labels.is_empty() {
            body.insert(
                "labels".to_string(),
                Value::Array(
                    task.labels
                        .iter()
                        .map(|label| Value::String(label.clone()))
                        .collect(),
                ),
            );
        }

        if let Some(remote_id) = task.todoist_id.as_deref() {
            client.post_no_content(&format!("/tasks/{remote_id}"), &Value::Object(body))?;
            Ok(None)
        } else {
            let created = client.post_json::<Value>("/tasks", &Value::Object(body))?;
            let created_id = value_id_as_string(&created)
                .context("todoist task create response is missing id")?;
            Ok(Some(created_id))
        }
    }

    fn sync_project(
        &self,
        op_kind: &str,
        project: &SyncProjectSnapshot,
        client: &TodoistRestClient,
    ) -> Result<Option<String>> {
        if op_kind == "delete" || project.deleted_at.is_some() {
            if let Some(remote_id) = project.todoist_id.as_deref() {
                client.delete(&format!("/projects/{remote_id}"))?;
            }
            return Ok(None);
        }

        let mut body = serde_json::Map::new();
        body.insert("name".to_string(), Value::String(project.name.clone()));
        body.insert("color".to_string(), Value::String(project.color.clone()));
        body.insert("is_favorite".to_string(), Value::Bool(project.is_favorite));
        if let Some(parent_id) = project.parent_todoist_id.as_deref() {
            body.insert(
                "parent_id".to_string(),
                Value::String(parent_id.to_string()),
            );
        }

        if let Some(remote_id) = project.todoist_id.as_deref() {
            client.post_no_content(&format!("/projects/{remote_id}"), &Value::Object(body))?;
            Ok(None)
        } else {
            let created = client.post_json::<Value>("/projects", &Value::Object(body))?;
            Ok(Some(value_id_as_string(&created).context(
                "todoist project create response is missing id",
            )?))
        }
    }

    fn sync_section(
        &self,
        op_kind: &str,
        section: &SyncSectionSnapshot,
        client: &TodoistRestClient,
    ) -> Result<Option<String>> {
        if op_kind == "delete" || section.deleted_at.is_some() {
            if let Some(remote_id) = section.todoist_id.as_deref() {
                client.delete(&format!("/sections/{remote_id}"))?;
            }
            return Ok(None);
        }

        let mut body = serde_json::Map::new();
        body.insert("name".to_string(), Value::String(section.name.clone()));
        if let Some(project_id) = section.project_todoist_id.as_deref() {
            body.insert(
                "project_id".to_string(),
                Value::String(project_id.to_string()),
            );
        }

        if let Some(remote_id) = section.todoist_id.as_deref() {
            client.post_no_content(&format!("/sections/{remote_id}"), &Value::Object(body))?;
            Ok(None)
        } else {
            let created = client.post_json::<Value>("/sections", &Value::Object(body))?;
            Ok(Some(value_id_as_string(&created).context(
                "todoist section create response is missing id",
            )?))
        }
    }

    fn sync_tag(
        &self,
        op_kind: &str,
        tag: &SyncTagSnapshot,
        client: &TodoistRestClient,
    ) -> Result<Option<String>> {
        if op_kind == "delete" || tag.deleted_at.is_some() {
            if let Some(remote_id) = tag.todoist_id.as_deref() {
                client.delete(&format!("/labels/{remote_id}"))?;
            }
            return Ok(None);
        }

        let body = json!({
            "name": tag.name,
            "color": tag.color,
            "is_favorite": tag.is_favorite,
        });
        if let Some(remote_id) = tag.todoist_id.as_deref() {
            client.post_no_content(&format!("/labels/{remote_id}"), &body)?;
            Ok(None)
        } else {
            let created = client.post_json::<Value>("/labels", &body)?;
            Ok(Some(
                value_id_as_string(&created)
                    .context("todoist label create response is missing id")?,
            ))
        }
    }

    fn sync_filter(
        &self,
        op_kind: &str,
        filter: &SyncFilterSnapshot,
        client: &TodoistRestClient,
    ) -> Result<Option<String>> {
        if op_kind == "delete" || filter.deleted_at.is_some() {
            if let Some(remote_id) = filter.todoist_id.as_deref() {
                client.delete(&format!("/filters/{remote_id}"))?;
            }
            return Ok(None);
        }

        let body = json!({
            "name": filter.name,
            "query": filter.query,
            "color": filter.color,
            "is_favorite": filter.is_favorite,
        });
        if let Some(remote_id) = filter.todoist_id.as_deref() {
            client.post_no_content(&format!("/filters/{remote_id}"), &body)?;
            Ok(None)
        } else {
            let created = client.post_json::<Value>("/filters", &body)?;
            Ok(Some(
                value_id_as_string(&created)
                    .context("todoist filter create response is missing id")?,
            ))
        }
    }

    fn pull_remote_downstream(
        &self,
        sync_repository: &dyn SyncRepository,
        client: &TodoistRestClient,
        synced_at_utc: &str,
    ) -> Result<DownstreamApplyStats> {
        let projects = client.get_json::<Vec<TodoistProject>>("/projects")?;
        let sections = client.get_json::<Vec<TodoistSection>>("/sections")?;
        let labels = client.get_json::<Vec<TodoistLabel>>("/labels")?;
        let filters = client.get_json::<Vec<TodoistFilter>>("/filters")?;
        let tasks = client.get_json::<Vec<TodoistTask>>("/tasks")?;

        let mut stats = DownstreamApplyStats::default();

        for project in projects {
            let remote = RemoteProjectRecord {
                todoist_id: project.id,
                parent_todoist_id: project.parent_id,
                name: project.name,
                color: project.color.unwrap_or_else(|| "charcoal".to_string()),
                is_favorite: project.is_favorite.unwrap_or(false),
                is_inbox: project.is_inbox_project.unwrap_or(false),
            };
            let outcome =
                sync_repository.apply_remote_project(&remote, synced_at_utc, self.dry_run)?;
            stats.record("project", remote.todoist_id.as_str(), outcome, self.dry_run);
        }

        for section in sections {
            let Some(project_todoist_id) = section.project_id else {
                continue;
            };
            let remote = RemoteSectionRecord {
                todoist_id: section.id,
                project_todoist_id,
                name: section.name,
            };
            let outcome =
                sync_repository.apply_remote_section(&remote, synced_at_utc, self.dry_run)?;
            stats.record("section", remote.todoist_id.as_str(), outcome, self.dry_run);
        }

        for label in labels {
            let remote = RemoteTagRecord {
                todoist_id: label.id,
                name: label.name,
                color: label.color.unwrap_or_else(|| "charcoal".to_string()),
                is_favorite: label.is_favorite.unwrap_or(false),
            };
            let outcome = sync_repository.apply_remote_tag(&remote, synced_at_utc, self.dry_run)?;
            stats.record("tag", remote.todoist_id.as_str(), outcome, self.dry_run);
        }

        for filter in filters {
            let remote = RemoteFilterRecord {
                todoist_id: filter.id,
                name: filter.name,
                query: filter.query,
                color: filter.color.unwrap_or_else(|| "charcoal".to_string()),
                is_favorite: filter.is_favorite.unwrap_or(false),
            };
            let outcome =
                sync_repository.apply_remote_filter(&remote, synced_at_utc, self.dry_run)?;
            stats.record("filter", remote.todoist_id.as_str(), outcome, self.dry_run);
        }

        for task in tasks {
            let remote = RemoteTaskRecord {
                todoist_id: task.id,
                project_todoist_id: task.project_id,
                section_todoist_id: task.section_id,
                parent_todoist_id: task.parent_id,
                content: task.content,
                description: task.description.unwrap_or_default(),
                priority: i64::from(task.priority.unwrap_or(4)),
                labels: task.labels.unwrap_or_default(),
                due_date: task
                    .due
                    .as_ref()
                    .and_then(|due| due.date.as_deref())
                    .and_then(|date| chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").ok()),
                due_datetime_utc: task
                    .due
                    .as_ref()
                    .and_then(|due| due.datetime.as_ref().cloned()),
                due_timezone: task.due.as_ref().and_then(|due| due.timezone.clone()),
                due_string: task.due.as_ref().and_then(|due| due.string.clone()),
                due_is_recurring: task
                    .due
                    .as_ref()
                    .and_then(|due| due.is_recurring)
                    .unwrap_or(false),
                completed_at: task.completed_at,
            };
            let outcome =
                sync_repository.apply_remote_task(&remote, synced_at_utc, self.dry_run)?;
            stats.record("task", remote.todoist_id.as_str(), outcome, self.dry_run);
        }

        Ok(stats)
    }
}

#[derive(Debug, Clone, Default)]
struct DownstreamApplyStats {
    created: usize,
    updated: usize,
    skipped: usize,
}

impl DownstreamApplyStats {
    fn record(
        &mut self,
        entity_type: &str,
        remote_id: &str,
        outcome: SyncApplyOutcome,
        dry_run: bool,
    ) {
        match outcome {
            SyncApplyOutcome::Created => self.created += 1,
            SyncApplyOutcome::Updated => self.updated += 1,
            SyncApplyOutcome::Skipped => self.skipped += 1,
        }
        if dry_run {
            info!(
                target: "integrations.todoist",
                entity_type,
                remote_id,
                outcome = ?outcome,
                "dry-run pull action"
            );
        }
    }
}

#[derive(Debug, Clone, serde::Deserialize)]
struct TodoistProject {
    id: String,
    name: String,
    parent_id: Option<String>,
    color: Option<String>,
    is_favorite: Option<bool>,
    is_inbox_project: Option<bool>,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct TodoistSection {
    id: String,
    project_id: Option<String>,
    name: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct TodoistLabel {
    id: String,
    name: String,
    color: Option<String>,
    is_favorite: Option<bool>,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct TodoistFilter {
    id: String,
    name: String,
    query: String,
    color: Option<String>,
    is_favorite: Option<bool>,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct TodoistTask {
    id: String,
    project_id: Option<String>,
    section_id: Option<String>,
    parent_id: Option<String>,
    content: String,
    description: Option<String>,
    priority: Option<u8>,
    labels: Option<Vec<String>>,
    due: Option<TodoistDue>,
    completed_at: Option<String>,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct TodoistDue {
    date: Option<String>,
    datetime: Option<String>,
    timezone: Option<String>,
    string: Option<String>,
    is_recurring: Option<bool>,
}

impl TaskSyncProvider for TodoistSyncProvider {
    fn provider_name(&self) -> &'static str {
        "todoist"
    }

    fn is_configured(&self) -> bool {
        self.config.enabled
    }

    fn sync(
        &self,
        sync_repository: &dyn SyncRepository,
        trigger: SyncTrigger,
    ) -> Result<SyncReport> {
        if !self.config.enabled {
            return Ok(SyncReport {
                provider: self.provider_name(),
                configured: false,
                trigger,
                status: "disabled".to_string(),
                pending_outbox: 0,
                delivered_outbox: 0,
                failed_outbox: 0,
                last_error: None,
            });
        }

        let token = self.resolve_token()?;
        if token.trim().is_empty() {
            bail!("resolved Todoist token is empty");
        }

        let now = Utc::now();
        let now_rfc3339 = now.to_rfc3339();
        let client = TodoistRestClient::new(token);
        let (delivered, failed, cycle_error) =
            self.process_ready_outbox(sync_repository, now, &client)?;
        let mut downstream_error = None;
        let downstream_stats = if matches!(trigger, SyncTrigger::Startup | SyncTrigger::Poll) {
            match self.pull_remote_downstream(sync_repository, &client, now_rfc3339.as_str()) {
                Ok(stats) => Some(stats),
                Err(error) => {
                    downstream_error = Some(error.to_string());
                    None
                }
            }
        } else {
            None
        };

        let pending = sync_repository.list_outbox(self.provider_name(), i64::MAX)?;
        let combined_error = cycle_error.clone().or(downstream_error.clone());

        let previous = sync_repository.get_state(self.provider_name())?;
        sync_repository.upsert_state(&SyncStateRecord {
            provider: self.provider_name().to_string(),
            sync_token: previous.and_then(|state| state.sync_token),
            last_synced_at: Some(now_rfc3339.clone()),
            last_status: Some(if combined_error.is_some() {
                "degraded".to_string()
            } else if self.dry_run {
                "dry_run".to_string()
            } else {
                "ok".to_string()
            }),
            last_error: combined_error.clone(),
            updated_at: now_rfc3339,
        })?;

        if let Some(stats) = downstream_stats {
            info!(
                target: "integrations.todoist",
                created = stats.created,
                updated = stats.updated,
                skipped = stats.skipped,
                dry_run = self.dry_run,
                "downstream pull merge summary"
            );
        }

        Ok(SyncReport {
            provider: self.provider_name(),
            configured: true,
            trigger,
            status: if combined_error.is_some() {
                "degraded".to_string()
            } else if self.dry_run {
                "dry_run".to_string()
            } else {
                "ok".to_string()
            },
            pending_outbox: pending.len(),
            delivered_outbox: delivered,
            failed_outbox: failed,
            last_error: combined_error,
        })
    }
}

#[derive(Debug, Clone)]
struct TodoistRestClient {
    token: String,
    correlation_id: String,
}

impl TodoistRestClient {
    fn new(token: String) -> Self {
        Self {
            token,
            correlation_id: Uuid::new_v4().to_string(),
        }
    }

    fn get_json<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        let response = self
            .request_builder("GET", path)
            .call()
            .map_err(map_ureq_error)?;
        response.into_json::<T>().map_err(|error| anyhow!(error))
    }

    fn post_json<T: DeserializeOwned>(&self, path: &str, body: &Value) -> Result<T> {
        let response = self
            .request_builder("POST", path)
            .send_json(body)
            .map_err(map_ureq_error)?;
        response.into_json::<T>().map_err(|error| anyhow!(error))
    }

    fn post_no_content(&self, path: &str, body: &Value) -> Result<()> {
        self.request_builder("POST", path)
            .send_json(body)
            .map_err(map_ureq_error)?;
        Ok(())
    }

    fn delete(&self, path: &str) -> Result<()> {
        self.request_builder("DELETE", path)
            .call()
            .map_err(map_ureq_error)?;
        Ok(())
    }

    fn request_builder(&self, method: &str, path: &str) -> ureq::Request {
        let url = format!("{TODOIST_REST_BASE}{path}");
        ureq::request(method, url.as_str())
            .set("Authorization", format!("Bearer {}", self.token).as_str())
            .set("X-Request-Id", self.correlation_id.as_str())
    }
}

fn value_id_as_string(value: &Value) -> Option<String> {
    value.get("id").and_then(|id| match id {
        Value::String(raw) => Some(raw.clone()),
        Value::Number(raw) => Some(raw.to_string()),
        _ => None,
    })
}

fn map_ureq_error(error: ureq::Error) -> anyhow::Error {
    match error {
        ureq::Error::Status(status, response) => {
            let body = response
                .into_string()
                .unwrap_or_else(|_| "<body unavailable>".to_string());
            anyhow!("todoist API status {}: {}", status, body)
        }
        ureq::Error::Transport(transport) => anyhow!("todoist transport error: {}", transport),
    }
}

fn read_token_from_command(program: &str, args: &[String], timeout: Duration) -> Result<String> {
    if timeout.is_zero() {
        bail!("token command timeout must be greater than zero");
    }

    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("failed to start token command: {program}"))?;

    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            if !status.success() {
                bail!("token command exited with status {status}");
            }
            let mut stdout = String::new();
            let mut handle = child
                .stdout
                .take()
                .context("token command stdout was not captured")?;
            handle.read_to_string(&mut stdout)?;
            let token = stdout.trim().to_string();
            if token.is_empty() {
                bail!("token command returned empty output");
            }
            return Ok(token);
        }

        if start.elapsed() > timeout {
            child.kill()?;
            let _ = child.wait();
            bail!("token command timed out after {}ms", timeout.as_millis());
        }

        thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use chrono::Utc;

    use crate::config::TodoistIntegrationConfig;
    use crate::storage::{Database, SyncRepository};

    use super::{SyncTrigger, TaskSyncProvider, TodoistSyncProvider};

    #[test]
    fn todoist_sync_dry_run_marks_outbox_as_pending_without_failures() -> Result<()> {
        unsafe {
            std::env::set_var("TRIGINTA_TODOIST_TOKEN", "token");
        }

        let database = Database::open_in_memory()?;
        let sync = database.sync_repository();
        let mut config = TodoistIntegrationConfig::default();
        config.enabled = true;
        let provider = TodoistSyncProvider::new(config, true);

        sync.enqueue_outbox(
            "todoist",
            "task",
            1,
            "update",
            "{}",
            Utc::now().to_rfc3339().as_str(),
        )?;

        let report = provider.sync(&sync, SyncTrigger::MutationDebounced)?;
        assert_eq!(report.status, "dry_run");
        assert_eq!(report.failed_outbox, 0);
        assert_eq!(report.pending_outbox, 1);

        let entries = sync.list_outbox("todoist", 10)?;
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].attempts, 0);
        Ok(())
    }
}
