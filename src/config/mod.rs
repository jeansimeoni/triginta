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

        Self::from_project_dirs(project_dirs.config_dir(), project_dirs.data_dir())
    }

    pub fn from_data_dir(data_dir: PathBuf) -> Result<Self> {
        let config_dir = data_dir.join("config");
        Self::from_project_dirs(&config_dir, &data_dir)
    }

    fn from_project_dirs(config_dir: &Path, data_dir: &Path) -> Result<Self> {
        Ok(Self {
            config_dir: config_dir.to_path_buf(),
            data_dir: data_dir.to_path_buf(),
            db_path: data_dir.join("triginta.sqlite3"),
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

    fn config_candidates(&self) -> [PathBuf; 3] {
        [
            self.config_toml_path.clone(),
            self.config_yaml_path.clone(),
            self.config_yml_path.clone(),
        ]
    }
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
}

impl TaskSortOrder {
    const ALL: [Self; 6] = [
        Self::DueAsc,
        Self::DueDesc,
        Self::TitleAsc,
        Self::TitleDesc,
        Self::CreatedNewest,
        Self::CreatedOldest,
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
pub struct AppConfig {
    pub ui: UiConfig,
    pub timer: TimerSettings,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            ui: UiConfig::default(),
            timer: TimerSettings::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct UiConfig {
    pub glyph_mode: GlyphMode,
    pub theme: String,
    pub task_list_sort: TaskSortOrder,
    pub hide_completed_tasks: bool,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            glyph_mode: GlyphMode::default(),
            theme: "catppuccin-mocha".to_string(),
            task_list_sort: TaskSortOrder::default(),
            hide_completed_tasks: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Default)]
#[serde(default)]
struct AppConfigFile {
    ui: UiConfig,
    timer: TimerConfigFile,
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
    Ok(config)
}

pub fn save_app_config(paths: &AppPaths, config: &AppConfig) -> Result<()> {
    config.timer.validate()?;
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
        AppConfig, AppPaths, GlyphMode, TaskSortOrder, TimerSettings, load_app_config,
        parse_duration_string, save_app_config,
    };

    #[test]
    fn from_data_dir_builds_expected_paths() {
        let base = PathBuf::from("/tmp/triginta-test");
        let paths = AppPaths::from_data_dir(base.clone()).expect("paths should resolve");

        assert_eq!(paths.data_dir, base);
        assert_eq!(paths.config_dir, PathBuf::from("/tmp/triginta-test/config"));
        assert_eq!(
            paths.db_path,
            PathBuf::from("/tmp/triginta-test/triginta.sqlite3")
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
        assert!(config.ui.hide_completed_tasks);
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
hide_completed_tasks = false

[timer]
pomodoro_length = "30m"
short_break_length = "7m"
long_break_length = "20m"
long_break_interval = 5
"#,
        )
        .expect("config should be written");

        let config = load_app_config(&paths).expect("config should load");
        assert_eq!(config.ui.glyph_mode, GlyphMode::Ascii);
        assert_eq!(config.ui.theme, "catppuccin-frappe");
        assert_eq!(config.ui.task_list_sort, TaskSortOrder::TitleDesc);
        assert!(!config.ui.hide_completed_tasks);
        assert_eq!(config.timer.long_break_interval, 5);
        assert_eq!(config.timer.pomodoro_length, Duration::from_secs(30 * 60));
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
  hide_completed_tasks: false
timer:
  pomodoro_length: 1800
  short_break_length: 300
  long_break_length: 900
  long_break_interval: 4
"#,
        )
        .expect("config should be written");

        let config = load_app_config(&paths).expect("config should load");
        assert_eq!(config.ui.glyph_mode, GlyphMode::Ascii);
        assert_eq!(config.ui.theme, "forest");
        assert_eq!(config.ui.task_list_sort, TaskSortOrder::CreatedOldest);
        assert!(!config.ui.hide_completed_tasks);
        assert_eq!(config.timer.pomodoro_length, Duration::from_secs(1800));
        assert_eq!(config.timer.short_break_length, Duration::from_secs(300));
        assert_eq!(config.timer.long_break_length, Duration::from_secs(900));
        assert_eq!(config.timer.long_break_interval, 4);
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
    fn save_app_config_writes_toml_when_missing() {
        let base = tempfile::tempdir().expect("tempdir should be created");
        let paths =
            AppPaths::from_data_dir(base.path().to_path_buf()).expect("paths should resolve");
        let mut config = AppConfig::default();
        config.ui.task_list_sort = TaskSortOrder::CreatedNewest;
        config.ui.hide_completed_tasks = false;

        save_app_config(&paths, &config).expect("config should save");

        let saved = fs::read_to_string(&paths.config_toml_path).expect("config should exist");
        assert!(saved.contains("task_list_sort = \"created-newest\""));
        assert!(saved.contains("hide_completed_tasks = false"));
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
        config.ui.hide_completed_tasks = false;

        save_app_config(&paths, &config).expect("config should save");

        let saved = fs::read_to_string(&paths.config_yaml_path).expect("yaml should exist");
        assert!(saved.contains("task_list_sort: title-asc"));
        assert!(saved.contains("hide_completed_tasks: false"));
        assert!(!paths.config_toml_path.exists());
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
