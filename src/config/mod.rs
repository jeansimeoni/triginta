use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use directories::ProjectDirs;
use serde::Deserialize;
use tracing_appender::non_blocking::WorkerGuard;

// This struct centralizes every filesystem location the app cares about.
// It plays the same role that a "resolved paths" config struct would in C,
// but `PathBuf` gives an owned, growable path type instead of raw strings.
#[derive(Debug, Clone)]
pub struct AppPaths {
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
    pub db_path: PathBuf,
    pub log_path: PathBuf,
    pub ui_config_path: PathBuf,
}

impl AppPaths {
    pub fn resolve() -> Result<Self> {
        // Environment-variable override first, then platform-default lookup.
        // The early return is common Rust style when one branch can finish the
        // function immediately.
        if let Ok(override_dir) = std::env::var("TRIGINTA_DATA_DIR") {
            return Self::from_data_dir(PathBuf::from(override_dir));
        }

        let project_dirs = ProjectDirs::from("dev", "jeansimeoni", "Triginta")
            .ok_or_else(|| anyhow!("failed to resolve application directories"))?;

        Self::from_project_dirs(project_dirs.config_dir(), project_dirs.data_dir())
    }

    pub fn from_data_dir(data_dir: PathBuf) -> Result<Self> {
        let config_dir = data_dir.join("config");
        Self::from_project_dirs(&config_dir, &data_dir)
    }

    fn from_project_dirs(config_dir: &Path, data_dir: &Path) -> Result<Self> {
        // `&Path` means we borrow the input paths rather than taking ownership.
        // `to_path_buf()` performs the owned copy when we want to store them.
        Ok(Self {
            config_dir: config_dir.to_path_buf(),
            data_dir: data_dir.to_path_buf(),
            db_path: data_dir.join("triginta.sqlite3"),
            log_path: data_dir.join("logs").join("triginta.log"),
            ui_config_path: config_dir.join("ui.toml"),
        })
    }

    pub fn ensure_dirs(&self) -> Result<()> {
        // `with_context` enriches low-level I/O errors with app-specific
        // details. In C you might log the path near each failing syscall;
        // here we attach that context to the error value itself.
        fs::create_dir_all(&self.config_dir).with_context(|| {
            format!(
                "failed to create config dir at {}",
                self.config_dir.display()
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum GlyphMode {
    Ascii,
    #[default]
    NerdFonts,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Default)]
pub struct UiConfig {
    #[serde(default)]
    pub glyph_mode: GlyphMode,
}

pub fn load_ui_config(paths: &AppPaths) -> Result<UiConfig> {
    let config_text = match fs::read_to_string(&paths.ui_config_path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(UiConfig::default());
        }
        Err(error) => {
            return Err(error).with_context(|| {
                format!(
                    "failed to read UI config at {}",
                    paths.ui_config_path.display()
                )
            });
        }
    };

    toml::from_str(&config_text).with_context(|| {
        format!(
            "failed to parse UI config at {}",
            paths.ui_config_path.display()
        )
    })
}

pub fn init_tracing(paths: &AppPaths) -> Result<WorkerGuard> {
    // The guard value keeps the background logging worker alive.
    // This is an RAII pattern: when the guard is dropped, cleanup happens
    // automatically. In C this would usually be a manual init/shutdown pair.
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

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{AppPaths, GlyphMode, UiConfig, load_ui_config};

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
            paths.ui_config_path,
            PathBuf::from("/tmp/triginta-test/config/ui.toml")
        );
    }

    #[test]
    fn load_ui_config_defaults_to_nerd_fonts_when_missing() {
        let base = tempfile::tempdir().expect("tempdir should be created");
        let paths =
            AppPaths::from_data_dir(base.path().to_path_buf()).expect("paths should resolve");

        let config = load_ui_config(&paths).expect("missing config should use defaults");
        assert_eq!(config, UiConfig::default());
        assert_eq!(config.glyph_mode, GlyphMode::NerdFonts);
    }
}
