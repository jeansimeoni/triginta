use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use directories::ProjectDirs;
use tracing_appender::non_blocking::WorkerGuard;

#[derive(Debug, Clone)]
pub struct AppPaths {
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
    pub db_path: PathBuf,
    pub log_path: PathBuf,
}

impl AppPaths {
    pub fn resolve() -> Result<Self> {
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
        Ok(Self {
            config_dir: config_dir.to_path_buf(),
            data_dir: data_dir.to_path_buf(),
            db_path: data_dir.join("triginta.sqlite3"),
            log_path: data_dir.join("logs").join("triginta.log"),
        })
    }

    pub fn ensure_dirs(&self) -> Result<()> {
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

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::AppPaths;

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
    }
}
