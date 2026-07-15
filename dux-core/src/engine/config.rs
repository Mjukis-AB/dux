use std::path::{Component, Path, PathBuf};

const MAX_STORAGE_PATH_BYTES: usize = 4_096;

/// Storage location whose configuration failed validation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EngineConfigField {
    Database,
    Snapshots,
    Cache,
}

/// Path-independent reason an engine storage location was rejected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EngineConfigReason {
    Relative,
    AmbiguousComponent,
    InvalidText,
    ControlCharacter,
    TooLong,
    FilesystemRoot,
    MissingFileName,
    OverlappingStorage,
}

/// Invalid explicit engine storage configuration.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("invalid {field:?} engine storage path: {reason:?}")]
pub struct EngineConfigError {
    pub field: EngineConfigField,
    pub reason: EngineConfigReason,
}

/// Explicit process-independent locations owned by one engine session.
///
/// This checkpoint validates and retains these paths but performs no storage
/// I/O. Directory creation, symlink inspection, private permissions, and
/// migrations belong to the persistence milestone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EngineConfig {
    database_path: PathBuf,
    snapshots_directory: PathBuf,
    cache_directory: PathBuf,
}

impl EngineConfig {
    pub fn new(
        database_path: PathBuf,
        snapshots_directory: PathBuf,
        cache_directory: PathBuf,
    ) -> Result<Self, EngineConfigError> {
        validate_path(&database_path, EngineConfigField::Database, true)?;
        validate_path(&snapshots_directory, EngineConfigField::Snapshots, false)?;
        validate_path(&cache_directory, EngineConfigField::Cache, false)?;
        if paths_overlap(&database_path, &snapshots_directory) {
            return Err(EngineConfigError {
                field: EngineConfigField::Snapshots,
                reason: EngineConfigReason::OverlappingStorage,
            });
        }
        if paths_overlap(&database_path, &cache_directory)
            || paths_overlap(&snapshots_directory, &cache_directory)
        {
            return Err(EngineConfigError {
                field: EngineConfigField::Cache,
                reason: EngineConfigReason::OverlappingStorage,
            });
        }

        Ok(Self {
            database_path,
            snapshots_directory,
            cache_directory,
        })
    }

    pub fn database_path(&self) -> &Path {
        &self.database_path
    }

    pub fn snapshots_directory(&self) -> &Path {
        &self.snapshots_directory
    }

    pub fn cache_directory(&self) -> &Path {
        &self.cache_directory
    }
}

fn paths_overlap(left: &Path, right: &Path) -> bool {
    left == right || left.starts_with(right) || right.starts_with(left)
}

fn validate_path(
    path: &Path,
    field: EngineConfigField,
    require_file_name: bool,
) -> Result<(), EngineConfigError> {
    let reject = |reason| EngineConfigError { field, reason };
    if !path.is_absolute() {
        return Err(reject(EngineConfigReason::Relative));
    }
    let text = path
        .to_str()
        .ok_or_else(|| reject(EngineConfigReason::InvalidText))?;
    if text.len() > MAX_STORAGE_PATH_BYTES {
        return Err(reject(EngineConfigReason::TooLong));
    }
    if text.chars().any(char::is_control) {
        return Err(reject(EngineConfigReason::ControlCharacter));
    }
    if path
        .components()
        .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
    {
        return Err(reject(EngineConfigReason::AmbiguousComponent));
    }
    if path.parent().is_none() {
        return Err(reject(EngineConfigReason::FilesystemRoot));
    }
    if require_file_name && path.file_name().is_none() {
        return Err(reject(EngineConfigReason::MissingFileName));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn explicit_absolute_paths_are_retained_without_creating_storage() {
        let temp = TempDir::new().unwrap();
        let database = temp.path().join("data/dux.sqlite3");
        let snapshots = temp.path().join("data/snapshots");
        let cache = temp.path().join("cache");

        let config = EngineConfig::new(database.clone(), snapshots.clone(), cache.clone()).unwrap();

        assert_eq!(config.database_path(), database);
        assert_eq!(config.snapshots_directory(), snapshots);
        assert_eq!(config.cache_directory(), cache);
        assert!(!database.exists());
        assert!(!snapshots.exists());
        assert!(!cache.exists());
    }

    #[test]
    fn invalid_paths_fail_without_echoing_user_text() {
        let temp = TempDir::new().unwrap();
        let invalid = EngineConfig::new(
            PathBuf::from("relative.sqlite3"),
            temp.path().join("snapshots"),
            temp.path().join("cache"),
        )
        .unwrap_err();
        assert_eq!(invalid.field, EngineConfigField::Database);
        assert_eq!(invalid.reason, EngineConfigReason::Relative);
        assert!(!invalid.to_string().contains("relative.sqlite3"));

        let duplicate = EngineConfig::new(
            temp.path().join("dux.sqlite3"),
            temp.path().join("owned"),
            temp.path().join("owned"),
        )
        .unwrap_err();
        assert_eq!(duplicate.field, EngineConfigField::Cache);
        assert_eq!(duplicate.reason, EngineConfigReason::OverlappingStorage);
    }

    #[test]
    fn ambiguous_control_root_and_oversized_paths_fail_closed() {
        let temp = TempDir::new().unwrap();
        let root = temp.path().ancestors().last().unwrap().to_path_buf();
        let cases = [
            (
                temp.path().join("..").join("dux.sqlite3"),
                EngineConfigReason::AmbiguousComponent,
            ),
            (
                temp.path().join("line\nbreak.sqlite3"),
                EngineConfigReason::ControlCharacter,
            ),
            (root, EngineConfigReason::FilesystemRoot),
            (
                temp.path().join(format!("{}.sqlite3", "x".repeat(4_096))),
                EngineConfigReason::TooLong,
            ),
        ];

        for (database, expected) in cases {
            let error = EngineConfig::new(
                database,
                temp.path().join("snapshots"),
                temp.path().join("cache"),
            )
            .unwrap_err();
            assert_eq!(error.reason, expected);
        }
    }

    #[test]
    fn every_storage_role_must_be_lexically_disjoint() {
        let temp = TempDir::new().unwrap();
        let database = temp.path().join("durable/dux.sqlite3");
        let snapshots = temp.path().join("durable/snapshots");
        let cache = temp.path().join("cache");

        let cases = [
            (
                database.clone(),
                database.clone(),
                cache.clone(),
                EngineConfigField::Snapshots,
            ),
            (
                snapshots.join("dux.sqlite3"),
                snapshots.clone(),
                cache.clone(),
                EngineConfigField::Snapshots,
            ),
            (
                database.clone(),
                database.join("snapshots"),
                cache.clone(),
                EngineConfigField::Snapshots,
            ),
            (
                cache.join("dux.sqlite3"),
                snapshots.clone(),
                cache.clone(),
                EngineConfigField::Cache,
            ),
            (
                database.clone(),
                cache.join("snapshots"),
                cache.clone(),
                EngineConfigField::Cache,
            ),
            (
                database.clone(),
                snapshots.clone(),
                snapshots.join("cache"),
                EngineConfigField::Cache,
            ),
            (
                database,
                snapshots.clone(),
                snapshots,
                EngineConfigField::Cache,
            ),
        ];

        for (database, snapshots, cache, field) in cases {
            let error = EngineConfig::new(database, snapshots, cache).unwrap_err();
            assert_eq!(error.field, field);
            assert_eq!(error.reason, EngineConfigReason::OverlappingStorage);
        }
    }
}
