use anyhow::Result;
use tempfile::tempdir;
use triginta::{config::AppPaths, storage::Database};

#[test]
fn database_file_is_created_for_fresh_install() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = AppPaths::from_data_dir(tempdir.path().to_path_buf())?;
    paths.ensure_dirs()?;

    let _database = Database::open(&paths.db_path)?;

    assert!(paths.db_path.exists());
    Ok(())
}
