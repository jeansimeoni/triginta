// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jean Simeoni

//! Event hooks: user-configured commands that run after app events.
//!
//! The app builds a [`HookEventData`] and hands it to a [`HookDispatcher`].
//! The dispatcher turns each configured command into a [`HookInvocation`] and
//! passes it to a [`HookRunner`]. The production runner spawns processes in
//! the background; tests swap in a recording runner so app state transitions
//! can be checked without spawning anything.

use std::{
    fmt,
    io::Write,
    process::{Child, Command, Stdio},
    sync::mpsc::{self, Receiver, Sender},
    thread,
    time::{Duration, Instant},
};

use chrono::{DateTime, Local};
use serde::Serialize;

use crate::config::{HookCommandConfig, HookEventDataFormat, HooksConfig};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HookEvent {
    PomodoroStarted,
    PomodoroPaused,
    PomodoroResumed,
    PomodoroCompleted,
    PomodoroVoided,
    BreakStarted,
    BreakCompleted,
    BreakSkipped,
    TaskCreated,
    TaskUpdated,
    TaskDeleted,
    TaskCompleted,
    SyncFailed,
}

impl HookEvent {
    pub const ALL: [HookEvent; 13] = [
        Self::PomodoroStarted,
        Self::PomodoroPaused,
        Self::PomodoroResumed,
        Self::PomodoroCompleted,
        Self::PomodoroVoided,
        Self::BreakStarted,
        Self::BreakCompleted,
        Self::BreakSkipped,
        Self::TaskCreated,
        Self::TaskUpdated,
        Self::TaskDeleted,
        Self::TaskCompleted,
        Self::SyncFailed,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::PomodoroStarted => "pomodoro_started",
            Self::PomodoroPaused => "pomodoro_paused",
            Self::PomodoroResumed => "pomodoro_resumed",
            Self::PomodoroCompleted => "pomodoro_completed",
            Self::PomodoroVoided => "pomodoro_voided",
            Self::BreakStarted => "break_started",
            Self::BreakCompleted => "break_completed",
            Self::BreakSkipped => "break_skipped",
            Self::TaskCreated => "task_created",
            Self::TaskUpdated => "task_updated",
            Self::TaskDeleted => "task_deleted",
            Self::TaskCompleted => "task_completed",
            Self::SyncFailed => "sync_failed",
        }
    }

    fn commands(self, config: &HooksConfig) -> &[HookCommandConfig] {
        match self {
            Self::PomodoroStarted => &config.pomodoro_started,
            Self::PomodoroPaused => &config.pomodoro_paused,
            Self::PomodoroResumed => &config.pomodoro_resumed,
            Self::PomodoroCompleted => &config.pomodoro_completed,
            Self::PomodoroVoided => &config.pomodoro_voided,
            Self::BreakStarted => &config.break_started,
            Self::BreakCompleted => &config.break_completed,
            Self::BreakSkipped => &config.break_skipped,
            Self::TaskCreated => &config.task_created,
            Self::TaskUpdated => &config.task_updated,
            Self::TaskDeleted => &config.task_deleted,
            Self::TaskCompleted => &config.task_completed,
            Self::SyncFailed => &config.sync_failed,
        }
    }
}

/// Where the change behind an event came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HookEventSource {
    Local,
    Todoist,
}

impl HookEventSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Todoist => "todoist",
        }
    }
}

/// Everything a hook command can learn about an event.
///
/// Optional fields are only filled when they apply: task fields for task
/// events and pomodoros with an assigned task, phase fields for timer
/// events, and `error` for sync failures.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HookEventData {
    pub event: HookEvent,
    pub source: HookEventSource,
    pub timestamp: DateTime<Local>,
    pub task_id: Option<i64>,
    pub task_title: Option<String>,
    pub project: Option<String>,
    pub phase: Option<&'static str>,
    pub duration_seconds: Option<u64>,
    pub elapsed_seconds: Option<u64>,
    pub error: Option<String>,
}

impl HookEventData {
    pub fn new(event: HookEvent, timestamp: DateTime<Local>) -> Self {
        Self {
            event,
            source: HookEventSource::Local,
            timestamp,
            task_id: None,
            task_title: None,
            project: None,
            phase: None,
            duration_seconds: None,
            elapsed_seconds: None,
            error: None,
        }
    }

    pub fn with_source(mut self, source: HookEventSource) -> Self {
        self.source = source;
        self
    }

    pub fn with_task(mut self, task_id: i64, title: &str, project: Option<&str>) -> Self {
        self.task_id = Some(task_id);
        self.task_title = Some(title.to_string());
        self.project = project.map(str::to_string);
        self
    }

    pub fn with_phase(
        mut self,
        phase: &'static str,
        duration_seconds: u64,
        elapsed_seconds: u64,
    ) -> Self {
        self.phase = Some(phase);
        self.duration_seconds = Some(duration_seconds);
        self.elapsed_seconds = Some(elapsed_seconds);
        self
    }

    pub fn with_error(mut self, error: &str) -> Self {
        self.error = Some(error.to_string());
        self
    }

    /// Value for a `{placeholder}` name, or `None` for unknown names.
    /// Known names without a value for this event resolve to an empty string.
    fn placeholder_value(&self, name: &str) -> Option<String> {
        let value = match name {
            "event" => Some(self.event.as_str().to_string()),
            "source" => Some(self.source.as_str().to_string()),
            "timestamp" => Some(self.timestamp.to_rfc3339()),
            "task_id" => self.task_id.map(|id| id.to_string()),
            "task_title" => self.task_title.clone(),
            "project" => self.project.clone(),
            "phase" => self.phase.map(str::to_string),
            "duration_seconds" => self.duration_seconds.map(|value| value.to_string()),
            "elapsed_seconds" => self.elapsed_seconds.map(|value| value.to_string()),
            "error" => self.error.clone(),
            _ => return None,
        };
        Some(value.unwrap_or_default())
    }

    fn env_vars(&self) -> Vec<(String, String)> {
        let mut vars = vec![
            ("TRIGINTA_EVENT", Some(self.event.as_str().to_string())),
            (
                "TRIGINTA_EVENT_SOURCE",
                Some(self.source.as_str().to_string()),
            ),
            ("TRIGINTA_TIMESTAMP", Some(self.timestamp.to_rfc3339())),
            ("TRIGINTA_TASK_ID", self.task_id.map(|id| id.to_string())),
            ("TRIGINTA_TASK_TITLE", self.task_title.clone()),
            ("TRIGINTA_PROJECT", self.project.clone()),
            ("TRIGINTA_PHASE", self.phase.map(str::to_string)),
            (
                "TRIGINTA_DURATION_SECONDS",
                self.duration_seconds.map(|value| value.to_string()),
            ),
            (
                "TRIGINTA_ELAPSED_SECONDS",
                self.elapsed_seconds.map(|value| value.to_string()),
            ),
            ("TRIGINTA_SYNC_ERROR", self.error.clone()),
        ];
        vars.retain(|(_, value)| value.is_some());
        vars.into_iter()
            .map(|(name, value)| (name.to_string(), value.unwrap_or_default()))
            .collect()
    }
}

/// A fully resolved command, ready to run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookInvocation {
    pub event: HookEvent,
    pub program: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub input: Option<String>,
    pub timeout: Duration,
}

impl HookInvocation {
    fn build(command: &HookCommandConfig, data: &HookEventData) -> Self {
        let args = if command.expand_placeholders {
            command
                .args
                .iter()
                .map(|arg| expand_placeholders(arg, data))
                .collect()
        } else {
            command.args.clone()
        };
        let env = if command.env {
            data.env_vars()
        } else {
            Vec::new()
        };
        let input = match command.event_data {
            HookEventDataFormat::None => None,
            HookEventDataFormat::Json => serde_json::to_string(data).ok(),
        };
        Self {
            event: data.event,
            program: command.program.clone(),
            args,
            env,
            input,
            timeout: Duration::from_millis(command.timeout_ms),
        }
    }
}

/// Replaces `{name}` tokens in one pass, so values that themselves contain
/// braces (a task titled "{error}", say) are never expanded again. Unknown
/// names are kept as written.
fn expand_placeholders(arg: &str, data: &HookEventData) -> String {
    let mut output = String::with_capacity(arg.len());
    let mut rest = arg;
    while let Some(open) = rest.find('{') {
        output.push_str(&rest[..open]);
        let after_open = &rest[open + 1..];
        let Some(close) = after_open.find('}') else {
            output.push_str(&rest[open..]);
            return output;
        };
        let name = &after_open[..close];
        match data.placeholder_value(name) {
            Some(value) => output.push_str(&value),
            None => {
                output.push('{');
                output.push_str(name);
                output.push('}');
            }
        }
        rest = &after_open[close + 1..];
    }
    output.push_str(rest);
    output
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookFailure {
    pub event: HookEvent,
    pub program: String,
    pub reason: String,
}

impl fmt::Display for HookFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} hook '{}' {}",
            self.event.as_str(),
            self.program,
            self.reason
        )
    }
}

/// Runs hook invocations without blocking the caller.
pub trait HookRunner: fmt::Debug {
    fn spawn(&self, invocation: HookInvocation);
    /// Failures reported since the last call.
    fn drain_failures(&self) -> Vec<HookFailure>;
}

#[derive(Debug)]
pub struct HookDispatcher {
    config: HooksConfig,
    runner: Box<dyn HookRunner>,
}

impl HookDispatcher {
    pub fn new(config: HooksConfig, runner: Box<dyn HookRunner>) -> Self {
        Self { config, runner }
    }

    pub fn has_commands(&self, event: HookEvent) -> bool {
        !event.commands(&self.config).is_empty()
    }

    pub fn emit(&self, data: &HookEventData) {
        for command in data.event.commands(&self.config) {
            self.runner.spawn(HookInvocation::build(command, data));
        }
    }

    pub fn drain_failures(&self) -> Vec<HookFailure> {
        self.runner.drain_failures()
    }
}

/// Spawns each hook as a child process and watches it on a background thread.
///
/// The process is started on the calling thread so a hook fired right before
/// quitting still launches; only the wait (and timeout kill) happens in the
/// background. Hooks never inherit the terminal: stdout and stderr go to
/// null, and stdin is either null or the JSON event data.
#[derive(Debug)]
pub struct ProcessHookRunner {
    failure_tx: Sender<HookFailure>,
    failure_rx: Receiver<HookFailure>,
}

impl ProcessHookRunner {
    pub fn new() -> Self {
        let (failure_tx, failure_rx) = mpsc::channel();
        Self {
            failure_tx,
            failure_rx,
        }
    }

    fn start(invocation: &HookInvocation) -> std::io::Result<Child> {
        let mut command = Command::new(&invocation.program);
        command
            .args(&invocation.args)
            .envs(invocation.env.iter().map(|(name, value)| (name, value)))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .stdin(if invocation.input.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            });
        let mut child = command.spawn()?;
        if let (Some(input), Some(mut stdin)) = (invocation.input.as_ref(), child.stdin.take()) {
            // The payload is small, so this fits in the pipe buffer. A command
            // that exits without reading it is not an error.
            let _ = stdin.write_all(input.as_bytes());
        }
        Ok(child)
    }

    fn watch(mut child: Child, timeout: Duration) -> Result<(), String> {
        let started = Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(status)) if status.success() => return Ok(()),
                Ok(Some(status)) => return Err(format!("exited with {status}")),
                Ok(None) => {}
                Err(error) => return Err(format!("could not be awaited: {error}")),
            }
            if started.elapsed() >= timeout {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("timed out after {}ms", timeout.as_millis()));
            }
            thread::sleep(Duration::from_millis(25));
        }
    }
}

impl Default for ProcessHookRunner {
    fn default() -> Self {
        Self::new()
    }
}

impl HookRunner for ProcessHookRunner {
    fn spawn(&self, invocation: HookInvocation) {
        let failure = |reason: String| HookFailure {
            event: invocation.event,
            program: invocation.program.clone(),
            reason,
        };
        let child = match Self::start(&invocation) {
            Ok(child) => child,
            Err(error) => {
                let _ = self
                    .failure_tx
                    .send(failure(format!("failed to start: {error}")));
                return;
            }
        };

        let failure_tx = self.failure_tx.clone();
        let timeout = invocation.timeout;
        let event = invocation.event;
        let program = invocation.program.clone();
        thread::spawn(move || {
            if let Err(reason) = Self::watch(child, timeout) {
                let _ = failure_tx.send(HookFailure {
                    event,
                    program,
                    reason,
                });
            }
        });
    }

    fn drain_failures(&self) -> Vec<HookFailure> {
        self.failure_rx.try_iter().collect()
    }
}

#[cfg(test)]
pub mod testing {
    use std::sync::{Arc, Mutex};

    use super::{HookFailure, HookInvocation, HookRunner};

    /// Records invocations instead of spawning processes.
    #[derive(Debug, Clone, Default)]
    pub struct RecordingHookRunner {
        invocations: Arc<Mutex<Vec<HookInvocation>>>,
        failures: Arc<Mutex<Vec<HookFailure>>>,
    }

    impl RecordingHookRunner {
        pub fn invocations(&self) -> Vec<HookInvocation> {
            self.invocations.lock().expect("lock").clone()
        }

        pub fn events(&self) -> Vec<&'static str> {
            self.invocations()
                .iter()
                .map(|invocation| invocation.event.as_str())
                .collect()
        }

        pub fn clear(&self) {
            self.invocations.lock().expect("lock").clear();
        }

        pub fn push_failure(&self, failure: HookFailure) {
            self.failures.lock().expect("lock").push(failure);
        }
    }

    impl HookRunner for RecordingHookRunner {
        fn spawn(&self, invocation: HookInvocation) {
            self.invocations.lock().expect("lock").push(invocation);
        }

        fn drain_failures(&self) -> Vec<HookFailure> {
            std::mem::take(&mut *self.failures.lock().expect("lock"))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use chrono::{Local, TimeZone};

    use super::{
        HookDispatcher, HookEvent, HookEventData, HookEventSource, HookInvocation, HookRunner,
        ProcessHookRunner, expand_placeholders, testing::RecordingHookRunner,
    };
    use crate::config::{HookCommandConfig, HookEventDataFormat, HooksConfig};

    fn sample_data() -> HookEventData {
        let timestamp = Local
            .with_ymd_and_hms(2026, 4, 17, 9, 25, 0)
            .single()
            .expect("fixed local datetime should resolve");
        HookEventData::new(HookEvent::PomodoroCompleted, timestamp)
            .with_task(7, "Write report", Some("Work"))
            .with_phase("focus", 1500, 1500)
    }

    fn command(program: &str, args: &[&str]) -> HookCommandConfig {
        HookCommandConfig {
            program: program.to_string(),
            args: args.iter().map(|arg| arg.to_string()).collect(),
            ..HookCommandConfig::default()
        }
    }

    #[test]
    fn every_event_name_matches_a_config_entry() {
        let config = HooksConfig::default();
        let names = config
            .entries()
            .iter()
            .map(|(name, _)| *name)
            .collect::<Vec<_>>();
        let events = HookEvent::ALL
            .iter()
            .map(|event| event.as_str())
            .collect::<Vec<_>>();
        assert_eq!(names, events);
    }

    #[test]
    fn placeholders_expand_known_names_and_keep_unknown_ones() {
        let data = sample_data();
        assert_eq!(
            expand_placeholders("Done: {task_title} ({project}) {phase} {unknown}", &data),
            "Done: Write report (Work) focus {unknown}"
        );
        assert_eq!(expand_placeholders("{error}|{task_id}", &data), "|7");
        assert_eq!(expand_placeholders("open { brace", &data), "open { brace");
    }

    #[test]
    fn placeholder_values_are_not_expanded_twice() {
        let data = sample_data()
            .with_task(1, "{error}", None)
            .with_error("boom");
        assert_eq!(expand_placeholders("{task_title}", &data), "{error}");
    }

    #[test]
    fn invocation_includes_env_args_and_no_event_data_by_default() {
        let data = sample_data();
        let invocation = HookInvocation::build(
            &command("notify-send", &["Triginta", "{task_title}"]),
            &data,
        );

        assert_eq!(invocation.args, vec!["Triginta", "Write report"]);
        assert!(invocation.input.is_none());
        assert!(invocation.env.contains(&(
            "TRIGINTA_EVENT".to_string(),
            "pomodoro_completed".to_string()
        )));
        assert!(invocation.env.contains(&(
            "TRIGINTA_TASK_TITLE".to_string(),
            "Write report".to_string()
        )));
        assert!(
            invocation
                .env
                .contains(&("TRIGINTA_DURATION_SECONDS".to_string(), "1500".to_string()))
        );
        assert!(
            !invocation
                .env
                .iter()
                .any(|(name, _)| name == "TRIGINTA_SYNC_ERROR")
        );
    }

    #[test]
    fn invocation_respects_disabled_env_and_placeholders_and_json_event_data() {
        let data = sample_data().with_source(HookEventSource::Todoist);
        let invocation = HookInvocation::build(
            &HookCommandConfig {
                env: false,
                expand_placeholders: false,
                event_data: HookEventDataFormat::Json,
                timeout_ms: 250,
                ..command("/usr/local/bin/log", &["{task_title}"])
            },
            &data,
        );

        assert_eq!(invocation.args, vec!["{task_title}"]);
        assert!(invocation.env.is_empty());
        assert_eq!(invocation.timeout, Duration::from_millis(250));
        let json: serde_json::Value =
            serde_json::from_str(invocation.input.as_deref().expect("json event data"))
                .expect("event data should be valid JSON");
        assert_eq!(json["event"], "pomodoro_completed");
        assert_eq!(json["source"], "todoist");
        assert_eq!(json["task_id"], 7);
        assert_eq!(json["task_title"], "Write report");
        assert_eq!(json["phase"], "focus");
        assert_eq!(json["error"], serde_json::Value::Null);
    }

    #[test]
    fn dispatcher_runs_every_command_for_the_event_only() {
        let runner = RecordingHookRunner::default();
        let config = HooksConfig {
            pomodoro_completed: vec![command("first", &[]), command("second", &[])],
            task_created: vec![command("other", &[])],
            ..HooksConfig::default()
        };
        let dispatcher = HookDispatcher::new(config, Box::new(runner.clone()));

        dispatcher.emit(&sample_data());

        let programs = runner
            .invocations()
            .into_iter()
            .map(|invocation| invocation.program)
            .collect::<Vec<_>>();
        assert_eq!(programs, vec!["first", "second"]);
        assert!(dispatcher.has_commands(HookEvent::TaskCreated));
        assert!(!dispatcher.has_commands(HookEvent::SyncFailed));
    }

    fn wait_for_failures(runner: &ProcessHookRunner) -> Vec<super::HookFailure> {
        for _ in 0..200 {
            let failures = runner.drain_failures();
            if !failures.is_empty() {
                return failures;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        Vec::new()
    }

    #[test]
    fn process_runner_reports_programs_that_fail_to_start() {
        let runner = ProcessHookRunner::new();
        runner.spawn(HookInvocation::build(
            &command("/nonexistent/triginta-hook", &[]),
            &sample_data(),
        ));

        let failures = runner.drain_failures();
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].event, HookEvent::PomodoroCompleted);
        assert!(failures[0].reason.contains("failed to start"));
    }

    #[cfg(unix)]
    #[test]
    fn process_runner_reports_non_zero_exit_status() {
        let runner = ProcessHookRunner::new();
        runner.spawn(HookInvocation::build(
            &command("false", &[]),
            &sample_data(),
        ));

        let failures = wait_for_failures(&runner);
        assert_eq!(failures.len(), 1);
        assert!(failures[0].reason.contains("exited with"));
    }

    #[cfg(unix)]
    #[test]
    fn process_runner_kills_commands_that_exceed_the_timeout() {
        let runner = ProcessHookRunner::new();
        runner.spawn(HookInvocation::build(
            &HookCommandConfig {
                timeout_ms: 50,
                ..command("sleep", &["5"])
            },
            &sample_data(),
        ));

        let failures = wait_for_failures(&runner);
        assert_eq!(failures.len(), 1);
        assert!(failures[0].reason.contains("timed out"));
    }

    #[cfg(unix)]
    #[test]
    fn process_runner_succeeds_silently_and_delivers_json_event_data() {
        let runner = ProcessHookRunner::new();
        // `grep -q` exits non-zero unless the JSON event data reached stdin.
        runner.spawn(HookInvocation::build(
            &HookCommandConfig {
                event_data: HookEventDataFormat::Json,
                ..command("grep", &["-q", "\"event\":\"pomodoro_completed\""])
            },
            &sample_data(),
        ));

        std::thread::sleep(Duration::from_millis(300));
        assert!(runner.drain_failures().is_empty());
    }
}
