// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jean Simeoni

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
const TODOIST_COMPLETED_TASKS_BOOTSTRAP_LOOKBACK_DAYS: i64 = 90;
const TODOIST_COMPLETED_TASKS_OVERLAP_SECONDS: i64 = 90;
const TODOIST_COMPLETED_TASKS_PAGE_LIMIT: usize = 200;

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
    fn todoist_sync_command<'a>(
        command_type: &'a str,
        temp_id: Option<&'a str>,
        uuid: &'a str,
        args: Value,
    ) -> TodoistSyncCommand<'a> {
        TodoistSyncCommand {
            command_type,
            temp_id,
            uuid,
            args,
        }
    }

    fn ensure_sync_command_succeeded(
        response: &TodoistSyncCommandResponse,
        uuid: &str,
    ) -> Result<()> {
        match response.sync_status.get(uuid) {
            Some(Value::String(status)) if status == "ok" => Ok(()),
            Some(status) => bail!("todoist sync command failed: {}", status),
            None => bail!("todoist sync response is missing sync status for command {uuid}"),
        }
    }

    fn created_id_from_sync_response(
        response: &TodoistSyncCommandResponse,
        temp_id: &str,
    ) -> Result<String> {
        scalar_value_as_string(
            response
                .temp_id_mapping
                .get(temp_id)
                .context("todoist sync response is missing temp id mapping")?,
        )
        .context("todoist sync response temp id mapping is missing a valid id")
    }

    fn task_outbox_payload(payload: &str) -> Result<TodoistTaskOutboxPayload> {
        serde_json::from_str(payload).context("failed to parse Todoist task outbox payload")
    }

    fn project_outbox_payload(payload: &str) -> Result<TodoistProjectOutboxPayload> {
        serde_json::from_str(payload).context("failed to parse Todoist project outbox payload")
    }

    fn sync_task_location(
        task: &SyncTaskSnapshot,
        client: &TodoistRestClient,
        remote_id: &str,
        payload: &TodoistTaskOutboxPayload,
    ) -> Result<()> {
        if !payload.location_changed {
            return Ok(());
        }

        let target = payload.location_target.as_deref().unwrap_or("project");
        let body = match target {
            "parent" => json!({
                "parent_id": task.parent_todoist_id.as_deref().context(
                    "task parent mapping is not synced yet; retrying after parent task sync"
                )?,
            }),
            "section" => json!({
                "section_id": task.section_todoist_id.as_deref().context(
                    "task section mapping is not synced yet; retrying after section sync"
                )?,
            }),
            "project" => json!({
                "project_id": task.project_todoist_id.as_deref().context(
                    "task project mapping is not synced yet; retrying after project sync"
                )?,
            }),
            other => bail!("unsupported Todoist task location target {other}"),
        };

        let _ = client.post_json::<Value>(&format!("/tasks/{remote_id}/move"), &body)?;
        Ok(())
    }

    fn sync_project_parent(
        project: &SyncProjectSnapshot,
        client: &TodoistRestClient,
        remote_id: &str,
        payload: &TodoistProjectOutboxPayload,
    ) -> Result<()> {
        if !payload.parent_changed {
            return Ok(());
        }

        let command_uuid = Uuid::new_v4().to_string();
        let response = client.run_sync_command(&Self::todoist_sync_command(
            "project_move",
            None,
            command_uuid.as_str(),
            json!({
                "id": remote_id,
                "parent_id": if project.has_parent_project {
                    Value::String(
                        project.parent_todoist_id.as_deref().context(
                            "project parent mapping is not synced yet; retrying after parent project sync"
                        )?.to_string()
                    )
                } else {
                    Value::Null
                },
            }),
        ))?;
        Self::ensure_sync_command_succeeded(&response, command_uuid.as_str())
    }

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
                let created_id =
                    self.sync_task(entry.op_kind.as_str(), entry.payload.as_str(), &task, client)?;
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
                let created_id = self.sync_project(
                    entry.op_kind.as_str(),
                    entry.payload.as_str(),
                    &project,
                    client,
                )?;
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
        payload: &str,
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

        if task.todoist_id.is_none() {
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
            Self::sync_task_location(
                task,
                client,
                remote_id,
                &Self::task_outbox_payload(payload)?,
            )?;
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
        payload: &str,
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
            Self::sync_project_parent(
                project,
                client,
                remote_id,
                &Self::project_outbox_payload(payload)?,
            )?;
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
                let command_uuid = Uuid::new_v4().to_string();
                let response = client.run_sync_command(&Self::todoist_sync_command(
                    "filter_delete",
                    None,
                    command_uuid.as_str(),
                    json!({ "id": remote_id }),
                ))?;
                Self::ensure_sync_command_succeeded(&response, command_uuid.as_str())?;
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
            let command_uuid = Uuid::new_v4().to_string();
            let response = client.run_sync_command(&Self::todoist_sync_command(
                "filter_update",
                None,
                command_uuid.as_str(),
                json!({
                    "id": remote_id,
                    "name": filter.name,
                    "query": filter.query,
                    "color": filter.color,
                    "is_favorite": filter.is_favorite,
                }),
            ))?;
            Self::ensure_sync_command_succeeded(&response, command_uuid.as_str())?;
            Ok(None)
        } else {
            let command_uuid = Uuid::new_v4().to_string();
            let temp_id = Uuid::new_v4().to_string();
            let response = client.run_sync_command(&Self::todoist_sync_command(
                "filter_add",
                Some(temp_id.as_str()),
                command_uuid.as_str(),
                body,
            ))?;
            Self::ensure_sync_command_succeeded(&response, command_uuid.as_str())?;
            Ok(Some(Self::created_id_from_sync_response(
                &response,
                temp_id.as_str(),
            )?))
        }
    }

    fn pull_remote_downstream(
        &self,
        sync_repository: &dyn SyncRepository,
        client: &TodoistRestClient,
        synced_at_utc: &str,
        sync_token: &str,
        previous_completed_tasks_synced_until: Option<&str>,
        now: DateTime<Utc>,
    ) -> Result<DownstreamApplyStats> {
        let mut stats = self.pull_remote_completed_tasks(
            sync_repository,
            client,
            synced_at_utc,
            previous_completed_tasks_synced_until,
            now,
        )?;
        let active_stats =
            self.pull_remote_active_resources(sync_repository, client, synced_at_utc, sync_token)?;
        stats.merge(active_stats);
        Ok(stats)
    }

    fn pull_remote_completed_tasks(
        &self,
        sync_repository: &dyn SyncRepository,
        client: &TodoistRestClient,
        synced_at_utc: &str,
        previous_completed_tasks_synced_until: Option<&str>,
        now: DateTime<Utc>,
    ) -> Result<DownstreamApplyStats> {
        let completed_since = completed_tasks_since(previous_completed_tasks_synced_until, now);
        let completed_until = now.to_rfc3339();
        let mut cursor = None;
        let mut stats = DownstreamApplyStats::default();

        loop {
            let response = client.get_completed_tasks_by_completion_date(
                completed_since.as_str(),
                completed_until.as_str(),
                cursor.as_deref(),
                TODOIST_COMPLETED_TASKS_PAGE_LIMIT,
            )?;

            for task in response.items {
                let remote = RemoteTaskRecord {
                    todoist_id: task.id,
                    todoist_sync_id: task.sync_id,
                    project_todoist_id: task.project_id,
                    section_todoist_id: task.section_id,
                    parent_todoist_id: task.parent_id,
                    content: task.content,
                    description: task.description.unwrap_or_default(),
                    priority: Self::todoist_priority_to_local(task.priority.unwrap_or(1)),
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
            }

            match response.next_cursor.filter(|cursor| !cursor.is_empty()) {
                Some(next_cursor) => cursor = Some(next_cursor),
                None => break,
            }
        }

        stats.completed_tasks_synced_until = Some(completed_until);
        Ok(stats)
    }

    fn pull_remote_active_resources(
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
                is_inbox: project.inbox_project.unwrap_or(false),
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
                todoist_sync_id: task.sync_id,
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
    completed_tasks_synced_until: Option<String>,
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

    fn merge(&mut self, other: DownstreamApplyStats) {
        self.created += other.created;
        self.updated += other.updated;
        self.skipped += other.skipped;
        if other.next_sync_token.is_some() {
            self.next_sync_token = other.next_sync_token;
        }
        if other.completed_tasks_synced_until.is_some() {
            self.completed_tasks_synced_until = other.completed_tasks_synced_until;
        }
        if other.preferred_language.is_some() {
            self.preferred_language = other.preferred_language;
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
    #[serde(alias = "is_inbox_project")]
    inbox_project: Option<bool>,
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
    sync_id: Option<String>,
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
        let previous_completed_tasks_synced_until = previous
            .as_ref()
            .and_then(|state| state.completed_tasks_synced_until.clone());
        let mut downstream_error = None;
        let should_pull_downstream =
            !self.dry_run || !matches!(trigger, SyncTrigger::MutationDebounced);
        let downstream_stats = if should_pull_downstream {
            match self.pull_remote_downstream(
                sync_repository,
                &client,
                now_rfc3339.as_str(),
                next_sync_token_seed.as_str(),
                previous_completed_tasks_synced_until.as_deref(),
                now,
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
        let next_completed_tasks_synced_until = downstream_stats
            .as_ref()
            .and_then(|stats| stats.completed_tasks_synced_until.clone())
            .or(previous_completed_tasks_synced_until.clone());
        let persisted_sync_token = if self.dry_run {
            previous_sync_token
        } else {
            next_sync_token
        };
        let persisted_completed_tasks_synced_until = if self.dry_run {
            previous_completed_tasks_synced_until
        } else {
            next_completed_tasks_synced_until
        };
        sync_repository.upsert_state(&SyncStateRecord {
            provider: self.provider_name().to_string(),
            sync_token: persisted_sync_token,
            completed_tasks_synced_until: persisted_completed_tasks_synced_until,
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

#[derive(Debug, Clone, serde::Serialize)]
struct TodoistSyncCommand<'a> {
    #[serde(rename = "type")]
    command_type: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    temp_id: Option<&'a str>,
    uuid: &'a str,
    args: Value,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct TodoistSyncCommandResponse {
    #[serde(default)]
    sync_status: HashMap<String, Value>,
    #[serde(default)]
    temp_id_mapping: HashMap<String, Value>,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
struct TodoistTaskOutboxPayload {
    #[serde(default)]
    location_changed: bool,
    location_target: Option<String>,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
struct TodoistProjectOutboxPayload {
    #[serde(default)]
    parent_changed: bool,
}

impl TodoistRestClient {
    fn new(token: String) -> Self {
        Self {
            token,
            correlation_id: Uuid::new_v4().to_string(),
        }
    }

    fn post_json<T: DeserializeOwned>(&self, path: &str, body: &Value) -> Result<T> {
        let mut response = self
            .post_request(path)
            .send_json(body)
            .map_err(map_ureq_error)?;
        ensure_success_status(&mut response)?;
        response
            .body_mut()
            .read_json::<T>()
            .map_err(|error| anyhow!(error))
    }

    fn post_no_content(&self, path: &str, body: &Value) -> Result<()> {
        let mut response = self
            .post_request(path)
            .send_json(body)
            .map_err(map_ureq_error)?;
        ensure_success_status(&mut response)?;
        Ok(())
    }

    fn post_empty(&self, path: &str) -> Result<()> {
        let mut response = self
            .post_request(path)
            .send_empty()
            .map_err(map_ureq_error)?;
        ensure_success_status(&mut response)?;
        Ok(())
    }

    fn delete(&self, path: &str) -> Result<()> {
        let mut response = self.delete_request(path).call().map_err(map_ureq_error)?;
        ensure_success_status(&mut response)?;
        Ok(())
    }

    fn get_completed_tasks_by_completion_date(
        &self,
        since: &str,
        until: &str,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<TodoistCompletedTasksResponse> {
        let mut request = self
            .get_request("/tasks/completed/by_completion_date")
            .query("since", since)
            .query("until", until)
            .query("limit", limit.to_string());
        if let Some(cursor) = cursor {
            request = request.query("cursor", cursor);
        }

        let mut response = request.call().map_err(map_ureq_error)?;
        ensure_success_status(&mut response)?;
        response
            .body_mut()
            .read_json::<TodoistCompletedTasksResponse>()
            .map_err(|error| anyhow!(error))
    }

    fn post_request(&self, path: &str) -> ureq::RequestBuilder<ureq::typestate::WithBody> {
        let url = format!("{TODOIST_API_BASE}{path}");
        self.configure_request(ureq::post(url.as_str()))
    }

    fn get_request(&self, path: &str) -> ureq::RequestBuilder<ureq::typestate::WithoutBody> {
        let url = format!("{TODOIST_API_BASE}{path}");
        self.configure_request(ureq::get(url.as_str()))
    }

    fn delete_request(&self, path: &str) -> ureq::RequestBuilder<ureq::typestate::WithoutBody> {
        let url = format!("{TODOIST_API_BASE}{path}");
        self.configure_request(ureq::delete(url.as_str()))
    }

    fn configure_request<B>(&self, request: ureq::RequestBuilder<B>) -> ureq::RequestBuilder<B> {
        request
            .config()
            .http_status_as_error(false)
            .build()
            .header("Authorization", format!("Bearer {}", self.token).as_str())
            .header("X-Request-Id", self.correlation_id.as_str())
    }

    fn sync_resources(&self, sync_token: &str) -> Result<TodoistSyncResponse> {
        let mut response = self
            .post_request("/sync")
            .send_form([
                ("sync_token", sync_token),
                (
                    "resource_types",
                    r#"["user","projects","sections","labels","filters","items"]"#,
                ),
            ])
            .map_err(map_ureq_error)?;
        ensure_success_status(&mut response)?;
        response
            .body_mut()
            .read_json::<TodoistSyncResponse>()
            .map_err(|error| anyhow!(error))
    }

    fn run_sync_command(&self, command: &TodoistSyncCommand<'_>) -> Result<TodoistSyncCommandResponse> {
        let commands = serde_json::to_string(&[command]).context("failed to encode Todoist sync command")?;
        let mut response = self
            .post_request("/sync")
            .send_form([("commands", commands.as_str())])
            .map_err(map_ureq_error)?;
        ensure_success_status(&mut response)?;
        response
            .body_mut()
            .read_json::<TodoistSyncCommandResponse>()
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

#[derive(Debug, Clone, serde::Deserialize)]
struct TodoistCompletedTasksResponse {
    #[serde(default)]
    items: Vec<TodoistTask>,
    next_cursor: Option<String>,
}

fn value_id_as_string(value: &Value) -> Option<String> {
    value.get("id").and_then(|id| match id {
        Value::String(raw) => Some(raw.clone()),
        Value::Number(raw) => Some(raw.to_string()),
        _ => None,
    })
}

fn scalar_value_as_string(value: &Value) -> Option<String> {
    match value {
        Value::String(raw) => Some(raw.clone()),
        Value::Number(raw) => Some(raw.to_string()),
        _ => None,
    }
}

fn ensure_success_status(response: &mut ureq::http::Response<ureq::Body>) -> Result<()> {
    let status = response.status();
    if status.is_success() {
        return Ok(());
    }

    let body = response
        .body_mut()
        .read_to_string()
        .unwrap_or_else(|_| "<body unavailable>".to_string());
    bail!("todoist API status {}: {}", status.as_u16(), body);
}

fn map_ureq_error(error: ureq::Error) -> anyhow::Error {
    anyhow!("todoist transport error: {}", error)
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

fn completed_tasks_since(
    previous_completed_tasks_synced_until: Option<&str>,
    now: DateTime<Utc>,
) -> String {
    let fallback = now - chrono::Duration::days(TODOIST_COMPLETED_TASKS_BOOTSTRAP_LOOKBACK_DAYS);
    let overlap = chrono::Duration::seconds(TODOIST_COMPLETED_TASKS_OVERLAP_SECONDS);
    previous_completed_tasks_synced_until
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .map(|value| value.with_timezone(&Utc) - overlap)
        .unwrap_or(fallback)
        .to_rfc3339()
}

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use chrono::Utc;
    use serde_json::json;

    use crate::config::TodoistIntegrationConfig;
    use crate::storage::{Database, SyncRepository};

    use super::{
        SyncTrigger, TaskSyncProvider, TodoistSyncCommandResponse, TodoistSyncProvider,
    };

    #[test]
    fn todoist_sync_dry_run_marks_outbox_as_pending_without_failures() -> Result<()> {
        unsafe {
            std::env::set_var("TRIGINTA_TODOIST_TOKEN", "token");
        }

        let database = Database::open_in_memory()?;
        let sync = database.sync_repository();
        let config = TodoistIntegrationConfig {
            enabled: true,
            ..TodoistIntegrationConfig::default()
        };
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

    #[test]
    fn todoist_filter_add_command_uses_sync_api_payload_shape() -> Result<()> {
        let command = TodoistSyncProvider::todoist_sync_command(
            "filter_add",
            Some("temp-123"),
            "uuid-123",
            json!({
                "name": "Today",
                "query": "today",
                "color": "charcoal",
                "is_favorite": true,
            }),
        );
        let serialized = serde_json::to_value(&command)?;

        assert_eq!(serialized["type"], "filter_add");
        assert_eq!(serialized["temp_id"], "temp-123");
        assert_eq!(serialized["uuid"], "uuid-123");
        assert_eq!(serialized["args"]["name"], "Today");
        assert_eq!(serialized["args"]["query"], "today");
        assert_eq!(serialized["args"]["color"], "charcoal");
        assert_eq!(serialized["args"]["is_favorite"], true);
        Ok(())
    }

    #[test]
    fn todoist_created_id_from_sync_response_reads_temp_id_mapping() -> Result<()> {
        let response = TodoistSyncCommandResponse {
            sync_status: std::iter::once(("uuid-123".to_string(), json!("ok"))).collect(),
            temp_id_mapping: std::iter::once(("temp-123".to_string(), json!("4638878"))).collect(),
        };

        TodoistSyncProvider::ensure_sync_command_succeeded(&response, "uuid-123")?;
        assert_eq!(
            TodoistSyncProvider::created_id_from_sync_response(&response, "temp-123")?,
            "4638878"
        );
        Ok(())
    }
}
