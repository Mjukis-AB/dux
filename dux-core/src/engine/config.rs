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
    UnsupportedPlatformSyntax,
    OverlappingStorage,
    UnexpectedLayout,
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
/// Opening an engine securely provisions and migrates the database parent.
/// Snapshot and cache roots remain reserved for their later storage owners.
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
        let expected_snapshots = database_path
            .parent()
            .ok_or(EngineConfigError {
                field: EngineConfigField::Database,
                reason: EngineConfigReason::FilesystemRoot,
            })?
            .join("snapshots");
        if !paths_equal(&snapshots_directory, &expected_snapshots) {
            return Err(EngineConfigError {
                field: EngineConfigField::Snapshots,
                reason: EngineConfigReason::UnexpectedLayout,
            });
        }
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

#[cfg(not(windows))]
pub(super) fn paths_overlap(left: &Path, right: &Path) -> bool {
    left == right || left.starts_with(right) || right.starts_with(left)
}

#[cfg(not(windows))]
fn paths_equal(left: &Path, right: &Path) -> bool {
    left == right
}

#[cfg(windows)]
pub(super) fn paths_overlap(left: &Path, right: &Path) -> bool {
    windows_path_is_prefix(left, right) || windows_path_is_prefix(right, left)
}

#[cfg(windows)]
fn paths_equal(left: &Path, right: &Path) -> bool {
    windows_path_is_prefix(left, right) && windows_path_is_prefix(right, left)
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
    #[cfg(windows)]
    validate_windows_path(path)
        .map_err(|()| reject(EngineConfigReason::UnsupportedPlatformSyntax))?;
    Ok(())
}

#[cfg(windows)]
fn validate_windows_path(path: &Path) -> Result<(), ()> {
    use std::path::Prefix;

    let Some(Component::Prefix(prefix)) = path.components().next() else {
        return Err(());
    };
    if !matches!(prefix.kind(), Prefix::Disk(_)) {
        return Err(());
    }

    for component in path.components() {
        let Component::Normal(name) = component else {
            continue;
        };
        let text = name.to_str().ok_or(())?;
        if text.ends_with('.')
            || text.ends_with(' ')
            || text
                .chars()
                .any(|character| matches!(character, '<' | '>' | ':' | '"' | '|' | '?' | '*' | '~'))
            || is_windows_reserved_name(text)
        {
            return Err(());
        }
    }
    Ok(())
}

#[cfg(windows)]
fn is_windows_reserved_name(component: &str) -> bool {
    let stem = component.split('.').next().unwrap_or(component);
    if stem.eq_ignore_ascii_case("CON")
        || stem.eq_ignore_ascii_case("PRN")
        || stem.eq_ignore_ascii_case("AUX")
        || stem.eq_ignore_ascii_case("NUL")
        || stem.eq_ignore_ascii_case("CONIN$")
        || stem.eq_ignore_ascii_case("CONOUT$")
    {
        return true;
    }
    let upper = stem.to_ascii_uppercase();
    matches!(
        upper.as_str(),
        "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    ) || ["COM¹", "COM²", "COM³", "LPT¹", "LPT²", "LPT³"]
        .iter()
        .any(|reserved| stem.eq_ignore_ascii_case(reserved))
}

#[cfg(windows)]
fn windows_path_is_prefix(prefix: &Path, path: &Path) -> bool {
    let prefix_components: Vec<_> = prefix.components().collect();
    let path_components: Vec<_> = path.components().collect();
    prefix_components.len() <= path_components.len()
        && prefix_components
            .iter()
            .zip(path_components.iter())
            .all(|(left, right)| windows_components_equal(*left, *right))
}

#[cfg(windows)]
fn windows_components_equal(left: Component<'_>, right: Component<'_>) -> bool {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Globalization::{CSTR_EQUAL, CompareStringOrdinal};

    let (left, right) = match (left, right) {
        (Component::Prefix(left), Component::Prefix(right)) => {
            (left.as_os_str(), right.as_os_str())
        }
        (Component::RootDir, Component::RootDir) => return true,
        (Component::Normal(left), Component::Normal(right)) => (left, right),
        _ => return false,
    };
    let left: Vec<u16> = left.encode_wide().collect();
    let right: Vec<u16> = right.encode_wide().collect();
    let Ok(left_len) = i32::try_from(left.len()) else {
        return false;
    };
    let Ok(right_len) = i32::try_from(right.len()) else {
        return false;
    };
    // SAFETY: both pointers remain live for the call and their explicit UTF-16
    // lengths fit the Win32 API. Ordinal ignore-case matches Windows path
    // comparison without locale-sensitive transformations.
    unsafe {
        CompareStringOrdinal(left.as_ptr(), left_len, right.as_ptr(), right_len, 1) == CSTR_EQUAL
    }
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
            temp.path().join("snapshots"),
            temp.path().join("snapshots"),
        )
        .unwrap_err();
        assert_eq!(duplicate.field, EngineConfigField::Cache);
        assert_eq!(duplicate.reason, EngineConfigReason::OverlappingStorage);
    }

    #[test]
    fn snapshots_must_use_the_reserved_database_sibling() {
        let temp = TempDir::new().unwrap();
        let database = temp.path().join("data/dux.sqlite3");
        let error = EngineConfig::new(
            database,
            temp.path().join("elsewhere/snapshots"),
            temp.path().join("cache"),
        )
        .unwrap_err();
        assert_eq!(error.field, EngineConfigField::Snapshots);
        assert_eq!(error.reason, EngineConfigReason::UnexpectedLayout);
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
                EngineConfigReason::UnexpectedLayout,
            ),
            (
                snapshots.join("dux.sqlite3"),
                snapshots.clone(),
                cache.clone(),
                EngineConfigField::Snapshots,
                EngineConfigReason::UnexpectedLayout,
            ),
            (
                database.clone(),
                database.join("snapshots"),
                cache.clone(),
                EngineConfigField::Snapshots,
                EngineConfigReason::UnexpectedLayout,
            ),
            (
                cache.join("dux.sqlite3"),
                cache.join("snapshots"),
                cache.clone(),
                EngineConfigField::Cache,
                EngineConfigReason::OverlappingStorage,
            ),
            (
                database.clone(),
                snapshots.clone(),
                database.parent().unwrap().to_path_buf(),
                EngineConfigField::Cache,
                EngineConfigReason::OverlappingStorage,
            ),
            (
                database.clone(),
                snapshots.clone(),
                snapshots.join("cache"),
                EngineConfigField::Cache,
                EngineConfigReason::OverlappingStorage,
            ),
            (
                database,
                snapshots.clone(),
                snapshots,
                EngineConfigField::Cache,
                EngineConfigReason::OverlappingStorage,
            ),
        ];

        for (database, snapshots, cache, field, reason) in cases {
            let error = EngineConfig::new(database, snapshots, cache).unwrap_err();
            assert_eq!(error.field, field);
            assert_eq!(error.reason, reason);
        }
    }

    #[cfg(windows)]
    #[test]
    fn windows_storage_paths_reject_ambiguous_namespace_forms() {
        let invalid = [
            r"C:\data\dux.sqlite3:stream",
            r"C:\data\dux.sqlite3.",
            r"C:\data\NUL.sqlite3",
            r"C:\PROGRA~1\dux.sqlite3",
            r"\\?\C:\data\dux.sqlite3",
            r"\\server\share\dux.sqlite3",
        ];
        for path in invalid {
            let error = EngineConfig::new(
                PathBuf::from(path),
                PathBuf::from(r"C:\data\snapshots"),
                PathBuf::from(r"C:\cache"),
            )
            .unwrap_err();
            assert_eq!(error.field, EngineConfigField::Database);
            assert_eq!(error.reason, EngineConfigReason::UnsupportedPlatformSyntax);
        }
    }

    #[cfg(windows)]
    #[test]
    fn windows_storage_roles_compare_with_ordinal_case_folding() {
        let error = EngineConfig::new(
            PathBuf::from(r"C:\Data\dux.sqlite3"),
            PathBuf::from(r"c:\data"),
            PathBuf::from(r"C:\cache"),
        )
        .unwrap_err();
        assert_eq!(error.field, EngineConfigField::Snapshots);
        assert_eq!(error.reason, EngineConfigReason::UnexpectedLayout);
    }

    #[cfg(windows)]
    #[test]
    fn windows_canonical_scan_scopes_compare_verbatim_prefixes_case_insensitively() {
        let root = Path::new(r"\\?\C:\Users\Alice\Projects");
        let descendant = Path::new(r"\\?\c:\users\ALICE\projects\dux");
        let sibling = Path::new(r"\\?\C:\Users\Alice\Other");

        assert!(paths_overlap(root, descendant));
        assert!(paths_overlap(descendant, root));
        assert!(!paths_overlap(root, sibling));
    }
}
