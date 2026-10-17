//! Local persistence only: settings, calibration weights and game summaries.
//! No camera frames, landmarks, or per-frame features are accepted by these APIs.

#![forbid(unsafe_code)]

mod database;
mod models;
mod settings;

pub use database::{DATABASE_NAME, Store};
pub use models::*;
pub use settings::*;

use std::path::PathBuf;

pub type Result<T> = std::result::Result<T, StoreError>;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("local data I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("local database: {0}")]
    Sql(#[from] rusqlite::Error),
    #[error("invalid stored JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("invalid settings TOML: {0}")]
    TomlRead(#[from] toml::de::Error),
    #[error("cannot encode settings: {0}")]
    TomlWrite(#[from] toml::ser::Error),
    #[error("unsupported {kind} schema version {version}")]
    Schema { kind: &'static str, version: u32 },
    #[error("invalid data: {0}")]
    Invalid(&'static str),
    #[error("LOCALAPPDATA is unavailable; supply an explicit data directory")]
    NoDataDirectory,
}

/// Windows production location. Tests and developer tools use `Store::open_in`.
/// Never silently falls back to the current directory.
pub fn default_data_dir() -> Result<PathBuf> {
    let root = std::env::var_os("LOCALAPPDATA")
        .filter(|value| !value.is_empty())
        .ok_or(StoreError::NoDataDirectory)?;
    Ok(PathBuf::from(root).join("lucas"))
}

pub(crate) fn ensure_version(kind: &'static str, version: u32) -> Result<()> {
    if version != 1 {
        return Err(StoreError::Schema { kind, version });
    }
    Ok(())
}

pub(crate) fn atomic_write(path: &std::path::Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    let parent = path
        .parent()
        .ok_or(StoreError::Invalid("missing data directory"))?;
    let mut file = tempfile::Builder::new()
        .prefix(".lucas-write-")
        .tempfile_in(parent)?;
    file.write_all(bytes)?;
    file.as_file().sync_all()?;
    file.persist(path)
        .map_err(|error| StoreError::Io(error.error))?;
    Ok(())
}
