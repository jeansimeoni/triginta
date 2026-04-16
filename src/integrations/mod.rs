use std::{
    io::Read,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use chrono::Utc;

use crate::{
    config::{TodoistIntegrationConfig, TodoistTokenSource},
    storage::{SyncRepository, SyncStateRecord},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncReport {
    pub provider: &'static str,
    pub configured: bool,
    pub status: String,
    pub pending_outbox: usize,
}

// Traits are Rust's interface mechanism.
// Compared with C, this is closer to defining a vtable contract up front and
// letting multiple concrete implementations satisfy it.
pub trait TaskSyncProvider {
    fn provider_name(&self) -> &'static str;
    fn is_configured(&self) -> bool;
    fn sync(&self, sync_repository: &dyn SyncRepository) -> Result<SyncReport>;
}

#[derive(Debug, Clone)]
pub struct TodoistSyncProvider {
    config: TodoistIntegrationConfig,
}

impl TodoistSyncProvider {
    pub fn new(config: TodoistIntegrationConfig) -> Self {
        Self { config }
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
}

impl TaskSyncProvider for TodoistSyncProvider {
    fn provider_name(&self) -> &'static str {
        "todoist"
    }

    fn is_configured(&self) -> bool {
        self.config.enabled
    }

    fn sync(&self, sync_repository: &dyn SyncRepository) -> Result<SyncReport> {
        if !self.config.enabled {
            return Ok(SyncReport {
                provider: self.provider_name(),
                configured: false,
                status: "disabled".to_string(),
                pending_outbox: 0,
            });
        }

        let token = self.resolve_token()?;
        if token.trim().is_empty() {
            bail!("resolved Todoist token is empty");
        }

        // We persist sync diagnostics here even before the actual API client is
        // implemented, so repeated startup attempts provide useful state.
        let now = Utc::now().to_rfc3339();
        let pending = sync_repository.list_outbox(self.provider_name(), i64::MAX)?;
        sync_repository.upsert_state(&SyncStateRecord {
            provider: self.provider_name().to_string(),
            sync_token: None,
            last_synced_at: Some(now.clone()),
            last_status: Some("noop".to_string()),
            last_error: None,
            updated_at: now,
        })?;

        Ok(SyncReport {
            provider: self.provider_name(),
            configured: true,
            status: "noop".to_string(),
            pending_outbox: pending.len(),
        })
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
