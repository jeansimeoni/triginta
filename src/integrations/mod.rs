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
    task_nlp::{NlpLocale, locale_priority_with_hint, parse_due_input_with_locales},
};

const TODOIST_API_BASE: &str = "https://api.todoist.com/api/v1";

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
    pub preferred_language: Option<String>,
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
    fn todoist_priority_to_local(priority: u8) -> i64 {
        match priority {
            4 => 1,
            3 => 2,
            2 => 3,
            _ => 4,
        }
    }

    fn local_priority_to_todoist(priority: i64) -> i64 {
        match priority {
            1 => 4,
            2 => 3,
            3 => 2,
            _ => 1,
        }
    }

    fn outbox_entity_priority(entity_type: &str) -> u8 {
        match entity_type {
            "project" => 0,
            "section" => 1,
            "tag" => 2,
            "filter" => 3,
            "task" => 4,
            _ => 5,
        }
    }

    fn detect_due_lang(due_string: &str) -> Option<&'static str> {
        let reference_date = Utc::now().date_naive();
        for locale in [NlpLocale::En, NlpLocale::PtBr, NlpLocale::Es] {
            if parse_due_input_with_locales(due_string, reference_date, &[locale]).is_some() {
                return Some(match locale {
                    NlpLocale::En => "en",
                    NlpLocale::PtBr => "pt",
                    NlpLocale::Es => "es",
                });
            }
        }
        None
    }

    fn normalize_due_lang_for_todoist(raw: &str) -> Option<&'static str> {
        let value = raw.trim().to_ascii_lowercase();
        if value.is_empty() {
            return None;
        }
        if value == "en" || value.starts_with("en-") || value == "english" {
            return Some("en");
        }
        if value == "pt"
            || value.starts_with("pt-")
            || value.starts_with("pt_")
            || value.contains("portuguese")
            || value.contains("portugues")
        {
            return Some("pt");
        }
        if value == "es"
            || value.starts_with("es-")
            || value.starts_with("es_")
            || value.contains("spanish")
            || value.contains("espanol")
            || value.contains("espanhol")
        {
            return Some("es");
        }
        None
    }

    fn rewrite_due_string_for_todoist(
        due_string: &str,
        due_lang: Option<&'static str>,
    ) -> Option<(String, &'static str)> {
        if due_lang != Some("pt") {
            return None;
        }

        let lowered = due_string.trim().to_lowercase();
        let folded = lowered
            .chars()
            .map(|character| match character {
                'á' | 'à' | 'â' | 'ä' | 'ã' => 'a',
                'é' | 'è' | 'ê' | 'ë' => 'e',
                'í' | 'ì' | 'î' | 'ï' => 'i',
                'ó' | 'ò' | 'ô' | 'ö' | 'õ' => 'o',
                'ú' | 'ù' | 'û' | 'ü' => 'u',
                'ç' => 'c',
                _ => character,
            })
            .collect::<String>();
        let normalized = folded.split_whitespace().collect::<Vec<_>>().join(" ");

        let rewritten = match normalized.as_str() {
            "todo mes no quinto dia util"
            | "no quinto dia util todo mes"
            | "quinto dia util todo mes"
            | "todo quinto dia util" => "every 5th workday",
            _ => return None,
        };
        Some((rewritten.to_string(), "en"))
    }

    fn due_string_is_recurring_for_todoist(due_string: &str, due_lang: Option<&str>) -> bool {
        let reference_date = Utc::now().date_naive();
        let locales = locale_priority_with_hint(due_lang);
        parse_due_input_with_locales(due_string, reference_date, locales.as_slice())
            .is_some_and(|due| due.is_recurring)
    }

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
        values.sort_by_key(|entry| {
            (
                Self::outbox_entity_priority(entry.entity_type.as_str()),
                entry.id,
            )
        });
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
        body.insert(
            "priority".to_string(),
            Value::Number(Self::local_priority_to_todoist(task.priority).into()),
        );

        if task.todoist_id.is_none() && task.project_todoist_id.is_none() && !task.project_is_inbox
        {
            bail!("task project mapping is not synced yet; retrying after project sync");
        }

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
            if Self::due_string_is_recurring_for_todoist(
                due_string.as_str(),
                task.due_lang.as_deref(),
            ) {
                let mut outgoing_due_string = due_string.clone();
                let mut outgoing_due_lang = task
                    .due_lang
                    .as_deref()
                    .and_then(Self::normalize_due_lang_for_todoist)
                    .or_else(|| Self::detect_due_lang(due_string.as_str()));

                if let Some((rewritten_due_string, rewritten_due_lang)) =
                    Self::rewrite_due_string_for_todoist(due_string.as_str(), outgoing_due_lang)
                {
                    outgoing_due_string = rewritten_due_string;
                    outgoing_due_lang = Some(rewritten_due_lang);
                }

                body.insert("due_string".to_string(), Value::String(outgoing_due_string));
                if let Some(due_lang) = outgoing_due_lang {
                    body.insert("due_lang".to_string(), Value::String(due_lang.to_string()));
                }
            } else if let Some(due_datetime_utc) = task.due_datetime_utc.as_ref() {
                body.insert(
                    "due_datetime".to_string(),
                    Value::String(due_datetime_utc.clone()),
                );
            } else if let Some(due_date) = task.due_date {
                body.insert(
                    "due_date".to_string(),
                    Value::String(due_date.format("%Y-%m-%d").to_string()),
                );
            }
        } else if let Some(due_datetime_utc) = task.due_datetime_utc.as_ref() {
            body.insert(
                "due_datetime".to_string(),
                Value::String(due_datetime_utc.clone()),
            );
        } else if let Some(due_date) = task.due_date {
            body.insert(
                "due_date".to_string(),
                Value::String(due_date.format("%Y-%m-%d").to_string()),
            );
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

        let should_be_completed = task.completed_at.is_some();
        if let Some(remote_id) = task.todoist_id.as_deref() {
            client.post_no_content(&format!("/tasks/{remote_id}"), &Value::Object(body))?;
            if should_be_completed {
                client.post_empty(&format!("/tasks/{remote_id}/close"))?;
            } else {
                client.post_empty(&format!("/tasks/{remote_id}/reopen"))?;
            }
            Ok(None)
        } else {
            let created = client.post_json::<Value>("/tasks", &Value::Object(body))?;
            let created_id = value_id_as_string(&created)
                .context("todoist task create response is missing id")?;
            if should_be_completed {
                client.post_empty(&format!("/tasks/{created_id}/close"))?;
            }
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
        if project.todoist_id.is_none()
            && project.has_parent_project
            && project.parent_todoist_id.is_none()
        {
            bail!("project parent mapping is not synced yet; retrying after parent project sync");
        }
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
        if section.todoist_id.is_none() && section.project_todoist_id.is_none() {
            bail!("section project mapping is not synced yet; retrying after project sync");
        }
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
        sync_token: &str,
    ) -> Result<DownstreamApplyStats> {
        let sync_response = client.sync_resources(sync_token)?;
        let mut preferred_language = sync_response
            .user
            .as_ref()
            .and_then(|user| user.lang.as_ref().cloned());
        let label_name_by_id = sync_response
            .labels
            .iter()
            .map(|label| (label.id.clone(), label.name.clone()))
            .collect::<HashMap<_, _>>();

        let mut stats = DownstreamApplyStats::default();

        for project in sync_response.projects {
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

        for section in sync_response.sections {
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

        for label in sync_response.labels {
            let remote = RemoteTagRecord {
                todoist_id: label.id,
                name: label.name,
                color: label.color.unwrap_or_else(|| "charcoal".to_string()),
                is_favorite: label.is_favorite.unwrap_or(false),
            };
            let outcome = sync_repository.apply_remote_tag(&remote, synced_at_utc, self.dry_run)?;
            stats.record("tag", remote.todoist_id.as_str(), outcome, self.dry_run);
        }

        for filter in sync_response.filters {
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

        for task in sync_response.items {
            let labels = task.labels.unwrap_or_else(|| {
                task.label_ids
                    .unwrap_or_default()
                    .into_iter()
                    .filter_map(|label_id| {
                        let id = label_id.as_string();
                        label_name_by_id.get(id.as_str()).cloned()
                    })
                    .collect::<Vec<_>>()
            });
            let remote = RemoteTaskRecord {
                todoist_id: task.id,
                project_todoist_id: task.project_id,
                section_todoist_id: task.section_id,
                parent_todoist_id: task.parent_id,
                content: task.content,
                description: task.description.unwrap_or_default(),
                priority: Self::todoist_priority_to_local(task.priority.unwrap_or(1)),
                labels,
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
                due_lang: task.due.as_ref().and_then(|due| due.lang.clone()),
                due_is_recurring: task
                    .due
                    .as_ref()
                    .and_then(|due| due.is_recurring)
                    .unwrap_or(false),
                completed_at: task.completed_at.or_else(|| {
                    if task.checked.unwrap_or(false) {
                        Some(synced_at_utc.to_string())
                    } else {
                        None
                    }
                }),
            };
            let outcome =
                sync_repository.apply_remote_task(&remote, synced_at_utc, self.dry_run)?;
            stats.record("task", remote.todoist_id.as_str(), outcome, self.dry_run);
            if preferred_language.is_none() {
                preferred_language = task.due.as_ref().and_then(|due| due.lang.clone());
            }
        }

        stats.next_sync_token = Some(sync_response.sync_token);
        stats.preferred_language = preferred_language;
        Ok(stats)
    }
}

#[derive(Debug, Clone, Default)]
struct DownstreamApplyStats {
    created: usize,
    updated: usize,
    skipped: usize,
    next_sync_token: Option<String>,
    preferred_language: Option<String>,
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
    label_ids: Option<Vec<TodoistIdValue>>,
    due: Option<TodoistDue>,
    completed_at: Option<String>,
    checked: Option<bool>,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(untagged)]
enum TodoistIdValue {
    String(String),
    Number(i64),
}

impl TodoistIdValue {
    fn as_string(&self) -> String {
        match self {
            Self::String(value) => value.clone(),
            Self::Number(value) => value.to_string(),
        }
    }
}

#[derive(Debug, Clone, serde::Deserialize)]
struct TodoistDue {
    date: Option<String>,
    datetime: Option<String>,
    timezone: Option<String>,
    string: Option<String>,
    is_recurring: Option<bool>,
    lang: Option<String>,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct TodoistUser {
    lang: Option<String>,
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
                preferred_language: None,
            });
        }

        let token = self.resolve_token()?;
        if token.trim().is_empty() {
            bail!("resolved Todoist token is empty");
        }

        let now = Utc::now();
        let now_rfc3339 = now.to_rfc3339();
        let client = TodoistRestClient::new(token);
        let previous = sync_repository.get_state(self.provider_name())?;
        let next_sync_token_seed = previous
            .as_ref()
            .and_then(|state| state.sync_token.as_deref())
            .unwrap_or("*")
            .to_string();
        let mut downstream_error = None;
        let should_pull_downstream =
            !self.dry_run || !matches!(trigger, SyncTrigger::MutationDebounced);
        let downstream_stats = if should_pull_downstream {
            match self.pull_remote_downstream(
                sync_repository,
                &client,
                now_rfc3339.as_str(),
                next_sync_token_seed.as_str(),
            ) {
                Ok(stats) => Some(stats),
                Err(error) => {
                    downstream_error = Some(error.to_string());
                    None
                }
            }
        } else {
            None
        };
        let _bootstrap_enqueued =
            sync_repository.enqueue_bootstrap_outbox(self.provider_name(), now_rfc3339.as_str())?;
        let (delivered, failed, cycle_error) =
            self.process_ready_outbox(sync_repository, now, &client)?;

        let pending = sync_repository.list_outbox(self.provider_name(), i64::MAX)?;
        let combined_error = cycle_error.clone().or(downstream_error.clone());
        let previous_sync_token = previous.as_ref().and_then(|state| state.sync_token.clone());
        let next_sync_token = downstream_stats
            .as_ref()
            .and_then(|stats| stats.next_sync_token.clone())
            .or(previous_sync_token.clone());
        let persisted_sync_token = if self.dry_run {
            previous_sync_token
        } else {
            next_sync_token
        };
        sync_repository.upsert_state(&SyncStateRecord {
            provider: self.provider_name().to_string(),
            sync_token: persisted_sync_token,
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

        if let Some(ref stats) = downstream_stats {
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
            preferred_language: downstream_stats
                .as_ref()
                .and_then(|stats| stats.preferred_language.clone()),
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

    fn post_empty(&self, path: &str) -> Result<()> {
        self.request_builder("POST", path)
            .call()
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
        let url = format!("{TODOIST_API_BASE}{path}");
        ureq::request(method, url.as_str())
            .set("Authorization", format!("Bearer {}", self.token).as_str())
            .set("X-Request-Id", self.correlation_id.as_str())
    }

    fn sync_resources(&self, sync_token: &str) -> Result<TodoistSyncResponse> {
        let response = self
            .request_builder("POST", "/sync")
            .send_form(&[
                ("sync_token", sync_token),
                (
                    "resource_types",
                    r#"["user","projects","sections","labels","filters","items"]"#,
                ),
            ])
            .map_err(map_ureq_error)?;
        response
            .into_json::<TodoistSyncResponse>()
            .map_err(|error| anyhow!(error))
    }
}

#[derive(Debug, Clone, serde::Deserialize)]
struct TodoistSyncResponse {
    sync_token: String,
    #[serde(default)]
    user: Option<TodoistUser>,
    #[serde(default)]
    projects: Vec<TodoistProject>,
    #[serde(default)]
    sections: Vec<TodoistSection>,
    #[serde(default)]
    labels: Vec<TodoistLabel>,
    #[serde(default)]
    filters: Vec<TodoistFilter>,
    #[serde(default)]
    items: Vec<TodoistTask>,
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

    #[test]
    fn todoist_priority_maps_to_local_levels() {
        assert_eq!(TodoistSyncProvider::todoist_priority_to_local(1), 4);
        assert_eq!(TodoistSyncProvider::todoist_priority_to_local(2), 3);
        assert_eq!(TodoistSyncProvider::todoist_priority_to_local(3), 2);
        assert_eq!(TodoistSyncProvider::todoist_priority_to_local(4), 1);
    }

    #[test]
    fn local_priority_maps_to_todoist_levels() {
        assert_eq!(TodoistSyncProvider::local_priority_to_todoist(4), 1);
        assert_eq!(TodoistSyncProvider::local_priority_to_todoist(3), 2);
        assert_eq!(TodoistSyncProvider::local_priority_to_todoist(2), 3);
        assert_eq!(TodoistSyncProvider::local_priority_to_todoist(1), 4);
    }

    #[test]
    fn detect_due_lang_prefers_english_for_local_mode_defaults() {
        assert_eq!(
            TodoistSyncProvider::detect_due_lang("every monday at 9am"),
            Some("en")
        );
    }

    #[test]
    fn detect_due_lang_supports_pt_br_and_es_recurrence_strings() {
        assert_eq!(
            TodoistSyncProvider::detect_due_lang("todo dia 25"),
            Some("pt")
        );
        assert_eq!(
            TodoistSyncProvider::detect_due_lang("todos los dias"),
            Some("es")
        );
    }

    #[test]
    fn rewrite_due_string_for_todoist_maps_pt_br_fifth_business_day_variants() {
        let rewritten = TodoistSyncProvider::rewrite_due_string_for_todoist(
            "todo mes no quinto dia util",
            Some("pt"),
        )
        .expect("variant should be rewritten");
        assert_eq!(rewritten, ("every 5th workday".to_string(), "en"));

        let rewritten = TodoistSyncProvider::rewrite_due_string_for_todoist(
            "quinto dia útil todo mês",
            Some("pt"),
        )
        .expect("accented variant should be rewritten");
        assert_eq!(rewritten, ("every 5th workday".to_string(), "en"));

        let rewritten =
            TodoistSyncProvider::rewrite_due_string_for_todoist("todo quinto dia útil", Some("pt"))
                .expect("compact variant should be rewritten");
        assert_eq!(rewritten, ("every 5th workday".to_string(), "en"));
    }

    #[test]
    fn rewrite_due_string_for_todoist_keeps_other_due_strings_unchanged() {
        assert!(
            TodoistSyncProvider::rewrite_due_string_for_todoist("todo dia 25", Some("pt"))
                .is_none()
        );
        assert!(
            TodoistSyncProvider::rewrite_due_string_for_todoist(
                "todo mes no quinto dia util",
                Some("en")
            )
            .is_none()
        );
    }

    #[test]
    fn due_string_is_recurring_for_todoist_supports_pt_br_weekly_variation() {
        assert!(TodoistSyncProvider::due_string_is_recurring_for_todoist(
            "segunda toda a semana",
            Some("pt-BR")
        ));
        assert!(!TodoistSyncProvider::due_string_is_recurring_for_todoist(
            "amanha",
            Some("pt-BR")
        ));
    }
}
