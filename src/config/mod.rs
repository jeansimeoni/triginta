use std::{
    fmt, fs,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result, anyhow, bail};
use directories::ProjectDirs;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use tracing_appender::non_blocking::WorkerGuard;

#[derive(Debug, Clone)]
pub struct AppPaths {
    pub config_dir: PathBuf,
    pub config_search_dirs: Vec<PathBuf>,
    pub data_dir: PathBuf,
    pub db_path: PathBuf,
    pub log_path: PathBuf,
    pub config_toml_path: PathBuf,
    pub config_yaml_path: PathBuf,
    pub config_yml_path: PathBuf,
    pub themes_dir: PathBuf,
}

impl AppPaths {
    pub fn resolve() -> Result<Self> {
        if let Ok(override_dir) = std::env::var("TRIGINTA_DATA_DIR") {
            return Self::from_data_dir(PathBuf::from(override_dir));
        }

        let project_dirs = ProjectDirs::from("", "", "triginta")
            .ok_or_else(|| anyhow!("failed to resolve application directories"))?;

        Self::from_project_dirs_with_extra_config_dirs(
            project_dirs.config_dir(),
            project_dirs.data_dir(),
            macos_xdg_config_dir().into_iter().collect(),
        )
    }

    pub fn from_data_dir(data_dir: PathBuf) -> Result<Self> {
        let config_dir = data_dir.join("config");
        Self::from_project_dirs(&config_dir, &data_dir)
    }

    fn from_project_dirs(config_dir: &Path, data_dir: &Path) -> Result<Self> {
        Self::from_project_dirs_with_extra_config_dirs(config_dir, data_dir, Vec::new())
    }

    fn from_project_dirs_with_extra_config_dirs(
        config_dir: &Path,
        data_dir: &Path,
        extra_config_dirs: Vec<PathBuf>,
    ) -> Result<Self> {
        let mut config_search_dirs = vec![config_dir.to_path_buf()];
        for extra_config_dir in extra_config_dirs {
            if !config_search_dirs.contains(&extra_config_dir) {
                config_search_dirs.push(extra_config_dir);
            }
        }

        Ok(Self {
            config_dir: config_dir.to_path_buf(),
            config_search_dirs,
            data_dir: data_dir.to_path_buf(),
            db_path: data_dir.join(db_filename()),
            log_path: data_dir.join("logs").join("triginta.log"),
            config_toml_path: config_dir.join("config.toml"),
            config_yaml_path: config_dir.join("config.yaml"),
            config_yml_path: config_dir.join("config.yml"),
            themes_dir: config_dir.join("themes"),
        })
    }

    pub fn ensure_dirs(&self) -> Result<()> {
        fs::create_dir_all(&self.config_dir).with_context(|| {
            format!(
                "failed to create config dir at {}",
                self.config_dir.display()
            )
        })?;
        fs::create_dir_all(&self.themes_dir).with_context(|| {
            format!(
                "failed to create themes dir at {}",
                self.themes_dir.display()
            )
        })?;
        fs::create_dir_all(&self.data_dir)
            .with_context(|| format!("failed to create data dir at {}", self.data_dir.display()))?;

        if let Some(parent) = self.log_path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create log dir at {}", parent.display()))?;
        }

        Ok(())
    }

    fn config_candidates(&self) -> Vec<PathBuf> {
        self.config_search_dirs
            .iter()
            .flat_map(|config_dir| {
                [
                    config_dir.join("config.toml"),
                    config_dir.join("config.yaml"),
                    config_dir.join("config.yml"),
                ]
            })
            .collect()
    }
}

#[cfg(target_os = "macos")]
fn macos_xdg_config_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
        .map(|home| home.join(".config").join("triginta"))
}

#[cfg(not(target_os = "macos"))]
fn macos_xdg_config_dir() -> Option<PathBuf> {
    None
}

#[cfg(debug_assertions)]
const fn db_filename() -> &'static str {
    "triginta-dbg.sqlite3"
}

#[cfg(not(debug_assertions))]
const fn db_filename() -> &'static str {
    "triginta.sqlite3"
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum GlyphMode {
    Ascii,
    #[default]
    NerdFonts,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum TaskSortOrder {
    #[default]
    DueAsc,
    DueDesc,
    TitleAsc,
    TitleDesc,
    CreatedNewest,
    CreatedOldest,
    PriorityHigh,
    PriorityLow,
}

impl TaskSortOrder {
    const ALL: [Self; 8] = [
        Self::DueAsc,
        Self::DueDesc,
        Self::TitleAsc,
        Self::TitleDesc,
        Self::CreatedNewest,
        Self::CreatedOldest,
        Self::PriorityHigh,
        Self::PriorityLow,
    ];

    pub fn all() -> &'static [Self] {
        &Self::ALL
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::DueAsc => "Due Date ↑",
            Self::DueDesc => "Due Date ↓",
            Self::TitleAsc => "Title A-Z",
            Self::TitleDesc => "Title Z-A",
            Self::CreatedNewest => "Created Newest",
            Self::CreatedOldest => "Created Oldest",
            Self::PriorityHigh => "Priority High-Low",
            Self::PriorityLow => "Priority Low-High",
        }
    }

    pub fn short_label(self) -> &'static str {
        match self {
            Self::DueAsc => "due ↑",
            Self::DueDesc => "due ↓",
            Self::TitleAsc => "title ↑",
            Self::TitleDesc => "title ↓",
            Self::CreatedNewest => "newest",
            Self::CreatedOldest => "oldest",
            Self::PriorityHigh => "prio ↑",
            Self::PriorityLow => "prio ↓",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum ProjectSortOrder {
    NameAsc,
    NameDesc,
    TaskCountAsc,
    TaskCountDesc,
    #[default]
    Manual,
}

impl ProjectSortOrder {
    const ALL: [Self; 5] = [
        Self::NameAsc,
        Self::NameDesc,
        Self::TaskCountAsc,
        Self::TaskCountDesc,
        Self::Manual,
    ];

    pub fn all() -> &'static [Self] {
        &Self::ALL
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::NameAsc => "Name A-Z",
            Self::NameDesc => "Name Z-A",
            Self::TaskCountAsc => "Task Count ↑",
            Self::TaskCountDesc => "Task Count ↓",
            Self::Manual => "Manual",
        }
    }

    pub fn short_label(self) -> &'static str {
        match self {
            Self::NameAsc => "name ↑",
            Self::NameDesc => "name ↓",
            Self::TaskCountAsc => "tasks ↑",
            Self::TaskCountDesc => "tasks ↓",
            Self::Manual => "manual",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum TagSortOrder {
    NameAsc,
    NameDesc,
    TaskCountAsc,
    TaskCountDesc,
    #[default]
    Manual,
}

impl TagSortOrder {
    const ALL: [Self; 5] = [
        Self::NameAsc,
        Self::NameDesc,
        Self::TaskCountAsc,
        Self::TaskCountDesc,
        Self::Manual,
    ];

    pub fn all() -> &'static [Self] {
        &Self::ALL
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::NameAsc => "Name A-Z",
            Self::NameDesc => "Name Z-A",
            Self::TaskCountAsc => "Task Count ↑",
            Self::TaskCountDesc => "Task Count ↓",
            Self::Manual => "Manual",
        }
    }

    pub fn short_label(self) -> &'static str {
        match self {
            Self::NameAsc => "name ↑",
            Self::NameDesc => "name ↓",
            Self::TaskCountAsc => "tasks ↑",
            Self::TaskCountDesc => "tasks ↓",
            Self::Manual => "manual",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum FilterSortOrder {
    NameAsc,
    NameDesc,
    TaskCountAsc,
    TaskCountDesc,
    #[default]
    Manual,
}

impl FilterSortOrder {
    const ALL: [Self; 5] = [
        Self::NameAsc,
        Self::NameDesc,
        Self::TaskCountAsc,
        Self::TaskCountDesc,
        Self::Manual,
    ];

    pub fn all() -> &'static [Self] {
        &Self::ALL
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::NameAsc => "Name A-Z",
            Self::NameDesc => "Name Z-A",
            Self::TaskCountAsc => "Task Count ↑",
            Self::TaskCountDesc => "Task Count ↓",
            Self::Manual => "Manual",
        }
    }

    pub fn short_label(self) -> &'static str {
        match self {
            Self::NameAsc => "name ↑",
            Self::NameDesc => "name ↓",
            Self::TaskCountAsc => "tasks ↑",
            Self::TaskCountDesc => "tasks ↓",
            Self::Manual => "manual",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimerSettings {
    pub pomodoro_length: Duration,
    pub short_break_length: Duration,
    pub long_break_length: Duration,
    pub long_break_interval: u32,
}

impl Default for TimerSettings {
    fn default() -> Self {
        Self {
            pomodoro_length: Duration::from_secs(25 * 60),
            short_break_length: Duration::from_secs(5 * 60),
            long_break_length: Duration::from_secs(15 * 60),
            long_break_interval: 4,
        }
    }
}

impl TimerSettings {
    pub fn short_timer_preset() -> Self {
        Self {
            pomodoro_length: Duration::from_secs(30),
            short_break_length: Duration::from_secs(10),
            long_break_length: Duration::from_secs(20),
            long_break_interval: 4,
        }
    }

    fn validate(&self) -> Result<()> {
        if self.pomodoro_length.is_zero() {
            bail!("timer.pomodoro-length must be greater than zero");
        }
        if self.short_break_length.is_zero() {
            bail!("timer.short-break-length must be greater than zero");
        }
        if self.long_break_length.is_zero() {
            bail!("timer.long-break-length must be greater than zero");
        }
        if self.long_break_interval == 0 {
            bail!("timer.long-break-interval must be greater than zero");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatsSettings {
    pub daily_target: Duration,
}

impl Default for StatsSettings {
    fn default() -> Self {
        Self {
            daily_target: Duration::from_secs(150 * 60),
        }
    }
}

impl StatsSettings {
    fn validate(&self) -> Result<()> {
        if self.daily_target.is_zero() {
            bail!("stats.daily-target must be greater than zero");
        }
        Ok(())
    }
}

impl IntegrationConfig {
    fn validate(&self) -> Result<()> {
        self.todoist.validate()
    }
}

impl TodoistIntegrationConfig {
    fn validate(&self) -> Result<()> {
        if self.token_env_var.trim().is_empty() {
            bail!("integrations.todoist.token-env-var cannot be empty");
        }

        if self.token_source == TodoistTokenSource::Command {
            let command = self.token_command.as_ref().context(
                "integrations.todoist.token-command is required when token-source is command",
            )?;
            if command.program.trim().is_empty() {
                bail!("integrations.todoist.token-command.program cannot be empty");
            }
            if command.timeout_ms == 0 {
                bail!("integrations.todoist.token-command.timeout-ms must be greater than zero");
            }
        }

        self.sync_runtime.validate()?;

        Ok(())
    }
}

impl TodoistSyncRuntimeConfig {
    fn validate(&self) -> Result<()> {
        if self.push_debounce_ms == 0 {
            bail!("integrations.todoist.sync-runtime.push-debounce-ms must be greater than zero");
        }
        if self.poll_min_interval_seconds == 0 {
            bail!(
                "integrations.todoist.sync-runtime.poll-min-interval-seconds must be greater than zero"
            );
        }
        if self.poll_max_interval_seconds < self.poll_min_interval_seconds {
            bail!(
                "integrations.todoist.sync-runtime.poll-max-interval-seconds must be greater than or equal to poll-min-interval-seconds"
            );
        }
        if self.max_batch_size == 0 {
            bail!("integrations.todoist.sync-runtime.max-batch-size must be greater than zero");
        }
        if self.max_retry_attempts == 0 {
            bail!("integrations.todoist.sync-runtime.max-retry-attempts must be greater than zero");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppConfig {
    pub ui: UiConfig,
    pub timer: TimerSettings,
    pub stats: StatsSettings,
    pub integrations: IntegrationConfig,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            ui: UiConfig::default(),
            timer: TimerSettings::default(),
            stats: StatsSettings::default(),
            integrations: IntegrationConfig::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntegrationConfig {
    pub todoist: TodoistIntegrationConfig,
}

impl Default for IntegrationConfig {
    fn default() -> Self {
        Self {
            todoist: TodoistIntegrationConfig::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TodoistIntegrationConfig {
    pub enabled: bool,
    pub sync_on_startup: bool,
    pub token_source: TodoistTokenSource,
    pub token_env_var: String,
    pub token_command: Option<TokenCommandConfig>,
    pub sync_runtime: TodoistSyncRuntimeConfig,
}

impl Default for TodoistIntegrationConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            sync_on_startup: false,
            token_source: TodoistTokenSource::Env,
            token_env_var: "TRIGINTA_TODOIST_TOKEN".to_string(),
            token_command: None,
            sync_runtime: TodoistSyncRuntimeConfig::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TodoistSyncRuntimeConfig {
    pub push_debounce_ms: u64,
    pub poll_min_interval_seconds: u64,
    pub poll_max_interval_seconds: u64,
    pub max_batch_size: u32,
    pub max_retry_attempts: u32,
}

impl Default for TodoistSyncRuntimeConfig {
    fn default() -> Self {
        Self {
            push_debounce_ms: 1_200,
            poll_min_interval_seconds: 30,
            poll_max_interval_seconds: 300,
            max_batch_size: 50,
            max_retry_attempts: 8,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum TodoistTokenSource {
    #[default]
    Env,
    Command,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenCommandConfig {
    pub program: String,
    pub args: Vec<String>,
    pub timeout_ms: u64,
}

impl Default for TokenCommandConfig {
    fn default() -> Self {
        Self {
            program: String::new(),
            args: Vec::new(),
            timeout_ms: 5_000,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct UiConfig {
    pub glyph_mode: GlyphMode,
    pub theme: String,
    pub task_list_sort: TaskSortOrder,
    pub project_list_sort: ProjectSortOrder,
    pub persist_project_list_sort: bool,
    pub tag_list_sort: TagSortOrder,
    pub persist_tag_list_sort: bool,
    pub filter_list_sort: FilterSortOrder,
    pub persist_filter_list_sort: bool,
    pub hide_completed_tasks: bool,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            glyph_mode: GlyphMode::default(),
            theme: "catppuccin-mocha".to_string(),
            task_list_sort: TaskSortOrder::default(),
            project_list_sort: ProjectSortOrder::default(),
            persist_project_list_sort: false,
            tag_list_sort: TagSortOrder::default(),
            persist_tag_list_sort: false,
            filter_list_sort: FilterSortOrder::default(),
            persist_filter_list_sort: false,
            hide_completed_tasks: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Default)]
#[serde(default)]
struct AppConfigFile {
    ui: UiConfig,
    timer: TimerConfigFile,
    stats: StatsConfigFile,
    integrations: IntegrationConfigFile,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
struct TimerConfigFile {
    #[serde(
        deserialize_with = "deserialize_duration",
        serialize_with = "serialize_duration"
    )]
    pomodoro_length: Duration,
    #[serde(
        deserialize_with = "deserialize_duration",
        serialize_with = "serialize_duration"
    )]
    short_break_length: Duration,
    #[serde(
        deserialize_with = "deserialize_duration",
        serialize_with = "serialize_duration"
    )]
    long_break_length: Duration,
    long_break_interval: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
struct StatsConfigFile {
    #[serde(
        deserialize_with = "deserialize_duration",
        serialize_with = "serialize_duration"
    )]
    daily_target: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Default)]
#[serde(default)]
struct IntegrationConfigFile {
    todoist: TodoistIntegrationConfigFile,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
struct TodoistIntegrationConfigFile {
    enabled: bool,
    sync_on_startup: bool,
    token_source: TodoistTokenSource,
    token_env_var: String,
    token_command: Option<TokenCommandConfigFile>,
    sync_runtime: TodoistSyncRuntimeConfigFile,
}

impl Default for TodoistIntegrationConfigFile {
    fn default() -> Self {
        let defaults = TodoistIntegrationConfig::default();
        Self {
            enabled: defaults.enabled,
            sync_on_startup: defaults.sync_on_startup,
            token_source: defaults.token_source,
            token_env_var: defaults.token_env_var,
            token_command: None,
            sync_runtime: TodoistSyncRuntimeConfigFile::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
struct TodoistSyncRuntimeConfigFile {
    push_debounce_ms: u64,
    poll_min_interval_seconds: u64,
    poll_max_interval_seconds: u64,
    max_batch_size: u32,
    max_retry_attempts: u32,
}

impl Default for TodoistSyncRuntimeConfigFile {
    fn default() -> Self {
        let defaults = TodoistSyncRuntimeConfig::default();
        Self {
            push_debounce_ms: defaults.push_debounce_ms,
            poll_min_interval_seconds: defaults.poll_min_interval_seconds,
            poll_max_interval_seconds: defaults.poll_max_interval_seconds,
            max_batch_size: defaults.max_batch_size,
            max_retry_attempts: defaults.max_retry_attempts,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
struct TokenCommandConfigFile {
    program: String,
    args: Vec<String>,
    timeout_ms: u64,
}

impl Default for TokenCommandConfigFile {
    fn default() -> Self {
        let defaults = TokenCommandConfig::default();
        Self {
            program: defaults.program,
            args: defaults.args,
            timeout_ms: defaults.timeout_ms,
        }
    }
}

impl Default for TimerConfigFile {
    fn default() -> Self {
        let defaults = TimerSettings::default();
        Self {
            pomodoro_length: defaults.pomodoro_length,
            short_break_length: defaults.short_break_length,
            long_break_length: defaults.long_break_length,
            long_break_interval: defaults.long_break_interval,
        }
    }
}

impl Default for StatsConfigFile {
    fn default() -> Self {
        let defaults = StatsSettings::default();
        Self {
            daily_target: defaults.daily_target,
        }
    }
}

impl From<AppConfigFile> for AppConfig {
    fn from(value: AppConfigFile) -> Self {
        Self {
            ui: value.ui,
            timer: TimerSettings {
                pomodoro_length: value.timer.pomodoro_length,
                short_break_length: value.timer.short_break_length,
                long_break_length: value.timer.long_break_length,
                long_break_interval: value.timer.long_break_interval,
            },
            stats: StatsSettings {
                daily_target: value.stats.daily_target,
            },
            integrations: IntegrationConfig {
                todoist: TodoistIntegrationConfig {
                    enabled: value.integrations.todoist.enabled,
                    sync_on_startup: value.integrations.todoist.sync_on_startup,
                    token_source: value.integrations.todoist.token_source,
                    token_env_var: value.integrations.todoist.token_env_var,
                    token_command: value.integrations.todoist.token_command.map(|command| {
                        TokenCommandConfig {
                            program: command.program,
                            args: command.args,
                            timeout_ms: command.timeout_ms,
                        }
                    }),
                    sync_runtime: TodoistSyncRuntimeConfig {
                        push_debounce_ms: value.integrations.todoist.sync_runtime.push_debounce_ms,
                        poll_min_interval_seconds: value
                            .integrations
                            .todoist
                            .sync_runtime
                            .poll_min_interval_seconds,
                        poll_max_interval_seconds: value
                            .integrations
                            .todoist
                            .sync_runtime
                            .poll_max_interval_seconds,
                        max_batch_size: value.integrations.todoist.sync_runtime.max_batch_size,
                        max_retry_attempts: value
                            .integrations
                            .todoist
                            .sync_runtime
                            .max_retry_attempts,
                    },
                },
            },
        }
    }
}

impl From<&AppConfig> for AppConfigFile {
    fn from(value: &AppConfig) -> Self {
        Self {
            ui: value.ui.clone(),
            timer: TimerConfigFile {
                pomodoro_length: value.timer.pomodoro_length,
                short_break_length: value.timer.short_break_length,
                long_break_length: value.timer.long_break_length,
                long_break_interval: value.timer.long_break_interval,
            },
            stats: StatsConfigFile {
                daily_target: value.stats.daily_target,
            },
            integrations: IntegrationConfigFile {
                todoist: TodoistIntegrationConfigFile {
                    enabled: value.integrations.todoist.enabled,
                    sync_on_startup: value.integrations.todoist.sync_on_startup,
                    token_source: value.integrations.todoist.token_source,
                    token_env_var: value.integrations.todoist.token_env_var.clone(),
                    token_command: value.integrations.todoist.token_command.as_ref().map(
                        |command| TokenCommandConfigFile {
                            program: command.program.clone(),
                            args: command.args.clone(),
                            timeout_ms: command.timeout_ms,
                        },
                    ),
                    sync_runtime: TodoistSyncRuntimeConfigFile {
                        push_debounce_ms: value.integrations.todoist.sync_runtime.push_debounce_ms,
                        poll_min_interval_seconds: value
                            .integrations
                            .todoist
                            .sync_runtime
                            .poll_min_interval_seconds,
                        poll_max_interval_seconds: value
                            .integrations
                            .todoist
                            .sync_runtime
                            .poll_max_interval_seconds,
                        max_batch_size: value.integrations.todoist.sync_runtime.max_batch_size,
                        max_retry_attempts: value
                            .integrations
                            .todoist
                            .sync_runtime
                            .max_retry_attempts,
                    },
                },
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConfigFormat {
    Toml,
    Yaml,
}

fn active_config_path(paths: &AppPaths) -> Result<Option<(PathBuf, ConfigFormat)>> {
    let candidates = paths
        .config_candidates()
        .into_iter()
        .filter(|path| path.exists())
        .collect::<Vec<_>>();

    let Some(config_path) = candidates.first() else {
        return Ok(None);
    };

    if candidates.len() > 1 {
        let names = candidates
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join(", ");
        bail!("multiple config files found; keep only one of: {names}");
    }

    let format = match config_path.extension().and_then(|ext| ext.to_str()) {
        Some("toml") => ConfigFormat::Toml,
        Some("yaml" | "yml") => ConfigFormat::Yaml,
        Some(other) => bail!("unsupported config file extension: {other}"),
        None => bail!("config file {} has no extension", config_path.display()),
    };

    Ok(Some((config_path.clone(), format)))
}

pub fn load_app_config(paths: &AppPaths) -> Result<AppConfig> {
    let Some((config_path, format)) = active_config_path(paths)? else {
        return Ok(AppConfig::default());
    };

    let config_text = fs::read_to_string(&config_path)
        .with_context(|| format!("failed to read config at {}", config_path.display()))?;

    let file_config = match format {
        ConfigFormat::Toml => toml::from_str::<AppConfigFile>(&config_text)
            .with_context(|| format!("failed to parse TOML config at {}", config_path.display()))?,
        ConfigFormat::Yaml => serde_yaml::from_str::<AppConfigFile>(&config_text)
            .with_context(|| format!("failed to parse YAML config at {}", config_path.display()))?,
    };

    let config = AppConfig::from(file_config);
    config.timer.validate()?;
    config.stats.validate()?;
    config.integrations.validate()?;
    Ok(config)
}

pub fn save_app_config(paths: &AppPaths, config: &AppConfig) -> Result<()> {
    config.timer.validate()?;
    config.stats.validate()?;
    config.integrations.validate()?;
    paths.ensure_dirs()?;

    let (path, format) = active_config_path(paths)?
        .unwrap_or_else(|| (paths.config_toml_path.clone(), ConfigFormat::Toml));
    let file_config = AppConfigFile::from(config);
    let content = match format {
        ConfigFormat::Toml => {
            toml::to_string_pretty(&file_config).context("failed to serialize TOML config")?
        }
        ConfigFormat::Yaml => {
            serde_yaml::to_string(&file_config).context("failed to serialize YAML config")?
        }
    };

    fs::write(&path, content)
        .with_context(|| format!("failed to write config at {}", path.display()))?;
    Ok(())
}

pub fn init_tracing(paths: &AppPaths) -> Result<WorkerGuard> {
    let file_appender = tracing_appender::rolling::never(
        paths.log_path.parent().expect("log path has parent"),
        paths
            .log_path
            .file_name()
            .expect("log path has file name")
            .to_string_lossy()
            .as_ref(),
    );
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

    tracing_subscriber::fmt()
        .with_ansi(false)
        .with_target(false)
        .with_writer(non_blocking)
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init()
        .map_err(|error| anyhow!("failed to initialize tracing subscriber: {error}"))?;

    Ok(guard)
}

fn deserialize_duration<'de, D>(deserializer: D) -> std::result::Result<Duration, D::Error>
where
    D: Deserializer<'de>,
{
    struct DurationVisitor;

    impl<'de> serde::de::Visitor<'de> for DurationVisitor {
        type Value = Duration;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter
                .write_str("a positive duration string like '25m' or an integer number of seconds")
        }

        fn visit_u64<E>(self, value: u64) -> std::result::Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            Ok(Duration::from_secs(value))
        }

        fn visit_i64<E>(self, value: i64) -> std::result::Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            if value < 0 {
                return Err(E::custom("duration cannot be negative"));
            }
            Ok(Duration::from_secs(value as u64))
        }

        fn visit_str<E>(self, value: &str) -> std::result::Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            parse_duration_string(value).map_err(E::custom)
        }

        fn visit_string<E>(self, value: String) -> std::result::Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            self.visit_str(&value)
        }
    }

    deserializer.deserialize_any(DurationVisitor)
}

fn serialize_duration<S>(duration: &Duration, serializer: S) -> std::result::Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.serialize_str(format_duration_string(*duration).as_str())
}

fn parse_duration_string(value: &str) -> Result<Duration> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        bail!("duration cannot be empty");
    }

    let split_index = trimmed
        .find(|char: char| !char.is_ascii_digit())
        .unwrap_or(trimmed.len());
    let (number, unit) = trimmed.split_at(split_index);

    if number.is_empty() {
        bail!("duration '{trimmed}' is missing a numeric value");
    }

    let amount = number
        .parse::<u64>()
        .with_context(|| format!("invalid duration value '{trimmed}'"))?;

    let duration = match unit.trim() {
        "" | "s" | "sec" | "secs" | "second" | "seconds" => Duration::from_secs(amount),
        "m" | "min" | "mins" | "minute" | "minutes" => Duration::from_secs(amount * 60),
        "h" | "hr" | "hrs" | "hour" | "hours" => Duration::from_secs(amount * 60 * 60),
        other => bail!("unsupported duration unit '{other}' in '{trimmed}'"),
    };

    if duration.is_zero() {
        bail!("duration must be greater than zero");
    }

    Ok(duration)
}

fn format_duration_string(duration: Duration) -> String {
    let seconds = duration.as_secs();
    if seconds % 3600 == 0 {
        format!("{}h", seconds / 3600)
    } else if seconds % 60 == 0 {
        format!("{}m", seconds / 60)
    } else {
        format!("{seconds}s")
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf, time::Duration};

    use super::{
        AppConfig, AppPaths, GlyphMode, ProjectSortOrder, TaskSortOrder, TimerSettings,
        TodoistTokenSource, db_filename, load_app_config, parse_duration_string, save_app_config,
    };

    #[test]
    fn from_data_dir_builds_expected_paths() {
        let base = PathBuf::from("/tmp/triginta-test");
        let paths = AppPaths::from_data_dir(base.clone()).expect("paths should resolve");

        assert_eq!(paths.data_dir, base);
        assert_eq!(paths.config_dir, PathBuf::from("/tmp/triginta-test/config"));
        assert_eq!(
            paths.config_search_dirs,
            vec![PathBuf::from("/tmp/triginta-test/config")]
        );
        assert_eq!(
            paths.db_path,
            PathBuf::from(format!("/tmp/triginta-test/{}", db_filename()))
        );
        assert_eq!(
            paths.log_path,
            PathBuf::from("/tmp/triginta-test/logs/triginta.log")
        );
        assert_eq!(
            paths.config_toml_path,
            PathBuf::from("/tmp/triginta-test/config/config.toml")
        );
        assert_eq!(
            paths.themes_dir,
            PathBuf::from("/tmp/triginta-test/config/themes")
        );
    }

    #[test]
    fn load_app_config_defaults_when_missing() {
        let base = tempfile::tempdir().expect("tempdir should be created");
        let paths =
            AppPaths::from_data_dir(base.path().to_path_buf()).expect("paths should resolve");

        let config = load_app_config(&paths).expect("missing config should use defaults");
        assert_eq!(config, AppConfig::default());
        assert_eq!(config.ui.glyph_mode, GlyphMode::NerdFonts);
        assert_eq!(config.ui.theme, "catppuccin-mocha");
        assert_eq!(config.ui.task_list_sort, TaskSortOrder::DueAsc);
        assert_eq!(config.ui.project_list_sort, ProjectSortOrder::Manual);
        assert!(!config.ui.persist_project_list_sort);
        assert!(config.ui.hide_completed_tasks);
    }

    #[test]
    fn load_app_config_from_extra_config_dir() {
        let base = tempfile::tempdir().expect("tempdir should be created");
        let data_dir = base.path().join("data");
        let primary_config_dir = base.path().join("primary-config");
        let extra_config_dir = base.path().join("xdg-config");
        fs::create_dir_all(&extra_config_dir).expect("extra config dir should exist");
        let paths = AppPaths::from_project_dirs_with_extra_config_dirs(
            &primary_config_dir,
            &data_dir,
            vec![extra_config_dir.clone()],
        )
        .expect("paths should resolve");
        fs::write(
            extra_config_dir.join("config.toml"),
            r#"[ui]
glyph_mode = "ascii"
"#,
        )
        .expect("fallback config should be written");

        let config = load_app_config(&paths).expect("fallback config should load");
        assert_eq!(config.ui.glyph_mode, GlyphMode::Ascii);
    }

    #[test]
    fn load_app_config_from_toml() {
        let base = tempfile::tempdir().expect("tempdir should be created");
        let paths =
            AppPaths::from_data_dir(base.path().to_path_buf()).expect("paths should resolve");
        paths.ensure_dirs().expect("dirs should exist");
        fs::write(
            &paths.config_toml_path,
            r#"[ui]
glyph_mode = "ascii"
theme = "catppuccin-frappe"
task_list_sort = "title-desc"
project_list_sort = "name-desc"
persist_project_list_sort = true
hide_completed_tasks = false

[timer]
pomodoro_length = "30m"
short_break_length = "7m"
long_break_length = "20m"
long_break_interval = 5

[stats]
daily_target = "2h"
"#,
        )
        .expect("config should be written");

        let config = load_app_config(&paths).expect("config should load");
        assert_eq!(config.ui.glyph_mode, GlyphMode::Ascii);
        assert_eq!(config.ui.theme, "catppuccin-frappe");
        assert_eq!(config.ui.task_list_sort, TaskSortOrder::TitleDesc);
        assert_eq!(config.ui.project_list_sort, ProjectSortOrder::NameDesc);
        assert!(config.ui.persist_project_list_sort);
        assert!(!config.ui.hide_completed_tasks);
        assert_eq!(config.timer.long_break_interval, 5);
        assert_eq!(config.timer.pomodoro_length, Duration::from_secs(30 * 60));
        assert_eq!(config.stats.daily_target, Duration::from_secs(2 * 60 * 60));
        assert!(!config.integrations.todoist.enabled);
    }

    #[test]
    fn load_app_config_from_yaml() {
        let base = tempfile::tempdir().expect("tempdir should be created");
        let paths =
            AppPaths::from_data_dir(base.path().to_path_buf()).expect("paths should resolve");
        paths.ensure_dirs().expect("dirs should exist");
        fs::write(
            &paths.config_yaml_path,
            r#"ui:
  glyph_mode: ascii
  theme: forest
  task_list_sort: created-oldest
  project_list_sort: task-count-desc
  persist_project_list_sort: true
  hide_completed_tasks: false
timer:
  pomodoro_length: 1800
  short_break_length: 300
  long_break_length: 900
  long_break_interval: 4
stats:
  daily_target: 5400
"#,
        )
        .expect("config should be written");

        let config = load_app_config(&paths).expect("config should load");
        assert_eq!(config.ui.glyph_mode, GlyphMode::Ascii);
        assert_eq!(config.ui.theme, "forest");
        assert_eq!(config.ui.task_list_sort, TaskSortOrder::CreatedOldest);
        assert_eq!(config.ui.project_list_sort, ProjectSortOrder::TaskCountDesc);
        assert!(config.ui.persist_project_list_sort);
        assert!(!config.ui.hide_completed_tasks);
        assert_eq!(config.timer.pomodoro_length, Duration::from_secs(1800));
        assert_eq!(config.timer.short_break_length, Duration::from_secs(300));
        assert_eq!(config.timer.long_break_length, Duration::from_secs(900));
        assert_eq!(config.timer.long_break_interval, 4);
        assert_eq!(config.stats.daily_target, Duration::from_secs(5400));
    }

    #[test]
    fn short_timer_preset_uses_expected_testing_durations() {
        let settings = TimerSettings::short_timer_preset();
        assert_eq!(settings.pomodoro_length, Duration::from_secs(30));
        assert_eq!(settings.short_break_length, Duration::from_secs(10));
        assert_eq!(settings.long_break_length, Duration::from_secs(20));
        assert_eq!(settings.long_break_interval, 4);
    }

    #[test]
    fn load_app_config_rejects_multiple_formats() {
        let base = tempfile::tempdir().expect("tempdir should be created");
        let paths =
            AppPaths::from_data_dir(base.path().to_path_buf()).expect("paths should resolve");
        paths.ensure_dirs().expect("dirs should exist");
        fs::write(&paths.config_toml_path, "").expect("toml should be written");
        fs::write(&paths.config_yaml_path, "").expect("yaml should be written");

        let error = load_app_config(&paths).expect_err("multiple config files should fail");
        assert!(error.to_string().contains("multiple config files found"));
    }

    #[test]
    fn load_app_config_rejects_multiple_files_across_config_dirs() {
        let base = tempfile::tempdir().expect("tempdir should be created");
        let data_dir = base.path().join("data");
        let primary_config_dir = base.path().join("primary-config");
        let extra_config_dir = base.path().join("xdg-config");
        let paths = AppPaths::from_project_dirs_with_extra_config_dirs(
            &primary_config_dir,
            &data_dir,
            vec![extra_config_dir.clone()],
        )
        .expect("paths should resolve");
        fs::create_dir_all(&primary_config_dir).expect("primary config dir should exist");
        fs::create_dir_all(&extra_config_dir).expect("extra config dir should exist");
        fs::write(&paths.config_toml_path, "").expect("primary config should be written");
        fs::write(extra_config_dir.join("config.yaml"), "")
            .expect("extra config should be written");

        let error = load_app_config(&paths).expect_err("multiple config files should fail");
        assert!(error.to_string().contains("multiple config files found"));
    }

    #[test]
    fn save_app_config_writes_toml_when_missing() {
        let base = tempfile::tempdir().expect("tempdir should be created");
        let paths =
            AppPaths::from_data_dir(base.path().to_path_buf()).expect("paths should resolve");
        let mut config = AppConfig::default();
        config.ui.task_list_sort = TaskSortOrder::CreatedNewest;
        config.ui.project_list_sort = ProjectSortOrder::TaskCountAsc;
        config.ui.persist_project_list_sort = true;
        config.ui.hide_completed_tasks = false;

        save_app_config(&paths, &config).expect("config should save");

        let saved = fs::read_to_string(&paths.config_toml_path).expect("config should exist");
        assert!(saved.contains("task_list_sort = \"created-newest\""));
        assert!(saved.contains("project_list_sort = \"task-count-asc\""));
        assert!(saved.contains("persist_project_list_sort = true"));
        assert!(saved.contains("hide_completed_tasks = false"));
        assert!(saved.contains("daily_target = \"150m\""));
    }

    #[test]
    fn save_app_config_preserves_existing_yaml_format() {
        let base = tempfile::tempdir().expect("tempdir should be created");
        let paths =
            AppPaths::from_data_dir(base.path().to_path_buf()).expect("paths should resolve");
        paths.ensure_dirs().expect("dirs should exist");
        fs::write(
            &paths.config_yaml_path,
            r#"ui:
  glyph_mode: nerd-fonts
  theme: catppuccin-mocha
timer:
  pomodoro_length: 25m
  short_break_length: 5m
  long_break_length: 15m
  long_break_interval: 4
"#,
        )
        .expect("config should be written");
        let mut config = load_app_config(&paths).expect("config should load");
        config.ui.task_list_sort = TaskSortOrder::TitleAsc;
        config.ui.project_list_sort = ProjectSortOrder::NameAsc;
        config.ui.persist_project_list_sort = true;
        config.ui.hide_completed_tasks = false;

        save_app_config(&paths, &config).expect("config should save");

        let saved = fs::read_to_string(&paths.config_yaml_path).expect("yaml should exist");
        assert!(saved.contains("task_list_sort: title-asc"));
        assert!(saved.contains("project_list_sort: name-asc"));
        assert!(saved.contains("persist_project_list_sort: true"));
        assert!(saved.contains("hide_completed_tasks: false"));
        assert!(saved.contains("daily_target: 150m"));
        assert!(!paths.config_toml_path.exists());
    }

    #[test]
    fn save_app_config_preserves_existing_yaml_format_in_extra_config_dir() {
        let base = tempfile::tempdir().expect("tempdir should be created");
        let data_dir = base.path().join("data");
        let primary_config_dir = base.path().join("primary-config");
        let extra_config_dir = base.path().join("xdg-config");
        let paths = AppPaths::from_project_dirs_with_extra_config_dirs(
            &primary_config_dir,
            &data_dir,
            vec![extra_config_dir.clone()],
        )
        .expect("paths should resolve");
        fs::create_dir_all(&extra_config_dir).expect("extra config dir should exist");
        let extra_yaml_path = extra_config_dir.join("config.yaml");
        fs::write(
            &extra_yaml_path,
            r#"ui:
  glyph_mode: nerd-fonts
  theme: catppuccin-mocha
timer:
  pomodoro_length: 25m
  short_break_length: 5m
  long_break_length: 15m
  long_break_interval: 4
"#,
        )
        .expect("fallback yaml should be written");
        let mut config = load_app_config(&paths).expect("fallback config should load");
        config.ui.task_list_sort = TaskSortOrder::TitleAsc;

        save_app_config(&paths, &config).expect("config should save");

        let saved = fs::read_to_string(&extra_yaml_path).expect("fallback yaml should exist");
        assert!(saved.contains("task_list_sort: title-asc"));
        assert!(!paths.config_toml_path.exists());
    }

    #[test]
    fn load_app_config_rejects_zero_stats_daily_target() {
        let base = tempfile::tempdir().expect("tempdir should be created");
        let paths =
            AppPaths::from_data_dir(base.path().to_path_buf()).expect("paths should resolve");
        paths.ensure_dirs().expect("dirs should exist");
        fs::write(
            &paths.config_toml_path,
            r#"[stats]
daily_target = "0m"
"#,
        )
        .expect("config should be written");

        let error = load_app_config(&paths).expect_err("zero target should fail validation");
        assert!(!error.to_string().is_empty());
    }

    #[test]
    fn load_app_config_supports_todoist_command_token_source() {
        let base = tempfile::tempdir().expect("tempdir should be created");
        let paths =
            AppPaths::from_data_dir(base.path().to_path_buf()).expect("paths should resolve");
        paths.ensure_dirs().expect("dirs should exist");
        fs::write(
            &paths.config_toml_path,
            r#"[integrations.todoist]
enabled = true
sync_on_startup = true
token_source = "command"
token_env_var = "TRIGINTA_TODOIST_TOKEN"

[integrations.todoist.token_command]
program = "/usr/bin/sops"
args = ["-d", "/tmp/token.enc"]
timeout_ms = 1500
"#,
        )
        .expect("config should be written");

        let config = load_app_config(&paths).expect("config should load");
        assert!(config.integrations.todoist.enabled);
        assert!(config.integrations.todoist.sync_on_startup);
        assert_eq!(
            config.integrations.todoist.token_source,
            TodoistTokenSource::Command
        );
        let command = config
            .integrations
            .todoist
            .token_command
            .expect("token command should be present");
        assert_eq!(command.program, "/usr/bin/sops");
        assert_eq!(
            command.args,
            vec!["-d".to_string(), "/tmp/token.enc".to_string()]
        );
        assert_eq!(command.timeout_ms, 1500);
    }

    #[test]
    fn load_app_config_rejects_command_source_without_command_settings() {
        let base = tempfile::tempdir().expect("tempdir should be created");
        let paths =
            AppPaths::from_data_dir(base.path().to_path_buf()).expect("paths should resolve");
        paths.ensure_dirs().expect("dirs should exist");
        fs::write(
            &paths.config_toml_path,
            r#"[integrations.todoist]
token_source = "command"
"#,
        )
        .expect("config should be written");

        let error = load_app_config(&paths).expect_err("command source should require command");
        assert!(error.to_string().contains("token-command"));
    }

    #[test]
    fn parse_duration_supports_seconds_and_minutes() {
        assert_eq!(
            parse_duration_string("30s").expect("seconds should parse"),
            Duration::from_secs(30)
        );
        assert_eq!(
            parse_duration_string("25m").expect("minutes should parse"),
            Duration::from_secs(25 * 60)
        );
    }
}
