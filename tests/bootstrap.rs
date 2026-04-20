// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jean Simeoni

use anyhow::Result;
use tempfile::tempdir;
use triginta::{config::AppPaths, storage::Database};

#[test]
fn database_file_is_created_for_fresh_install() -> Result<()> {
    // `tempdir()` creates a directory whose cleanup is tied to Rust scope
    // rather than manual teardown code. This keeps the test focused on the
    // behavior under test, not cleanup mechanics.
    let tempdir = tempdir()?;
    let paths = AppPaths::from_data_dir(tempdir.path().to_path_buf())?;
    paths.ensure_dirs()?;

    let _database = Database::open(&paths.db_path)?;

    assert!(paths.db_path.exists());
    Ok(())
}
