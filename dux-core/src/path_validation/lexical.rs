use std::path::{Component, Path, PathBuf};

use thiserror::Error;

const MAX_PATH_ENCODED_UNITS: usize = 32 * 1024;

/// An absolute scan root with unambiguous host-native lexical syntax.
///
/// This is syntax evidence only. It does not prove that the path exists or is
/// permitted as a cleanup scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LexicalScanRoot {
    path: PathBuf,
}

impl LexicalScanRoot {
    pub(crate) fn as_path(&self) -> &Path {
        &self.path
    }
}

/// A strict lexical descendant of a [`LexicalScanRoot`].
///
/// This is not canonical filesystem evidence or cleanup authority.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LexicalCleanupPath {
    scan_root: PathBuf,
    path: PathBuf,
    relative_to_scan_root: PathBuf,
}

impl LexicalCleanupPath {
    pub(crate) fn scan_root(&self) -> &Path {
        &self.scan_root
    }

    pub(crate) fn as_path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn relative_to_scan_root(&self) -> &Path {
        &self.relative_to_scan_root
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub(crate) enum LexicalPathError {
    #[error("path must not be empty")]
    Empty,
    #[error("path contains {actual_units} encoded units; maximum is {maximum_units}")]
    TooLong {
        actual_units: usize,
        maximum_units: usize,
    },
    #[error("path must use a lossless host-native Unicode representation")]
    InvalidEncoding,
    #[error("path contains a control character at encoded unit {unit_index}")]
    ControlCharacter { unit_index: usize },
    #[error("path contains a repeated separator at encoded unit {unit_index}")]
    RepeatedSeparator { unit_index: usize },
    #[error("path must not have a trailing separator")]
    TrailingSeparator,
    #[error("path contains a current-directory component at index {component_index}")]
    CurrentDirectory { component_index: usize },
    #[error("path contains a parent-traversal component at index {component_index}")]
    ParentTraversal { component_index: usize },
    #[error("path must be absolute")]
    NotAbsolute,
    #[cfg(windows)]
    #[error("path uses an unsupported Windows prefix")]
    UnsupportedWindowsPrefix,
    #[cfg(windows)]
    #[error("path component {component_index} is invalid on Windows: {reason}")]
    InvalidWindowsComponent {
        component_index: usize,
        reason: WindowsComponentError,
    },
    #[error("a cleanup path must not be a bare filesystem root")]
    FilesystemRoot,
    #[error("a cleanup path must be a strict descendant, not the scan root itself")]
    EqualToScanRoot,
    #[error("cleanup path is outside the scan root")]
    OutsideScanRoot,
}

#[cfg(windows)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub(crate) enum WindowsComponentError {
    #[error("alternate data streams are not supported")]
    AlternateDataStream,
    #[error("component contains a reserved Windows character")]
    ReservedCharacter,
    #[error("component has a trailing dot or space")]
    TrailingDotOrSpace,
    #[error("component is a reserved Windows device name")]
    ReservedDeviceName,
}

pub(crate) fn validate_scan_root(path: &Path) -> Result<LexicalScanRoot, LexicalPathError> {
    validate_absolute_path(path)?;
    Ok(LexicalScanRoot {
        path: path.to_path_buf(),
    })
}

pub(crate) fn validate_cleanup_path(
    scan_root: &LexicalScanRoot,
    path: &Path,
) -> Result<LexicalCleanupPath, LexicalPathError> {
    validate_absolute_path(path)?;
    if is_filesystem_root(path) {
        return Err(LexicalPathError::FilesystemRoot);
    }
    if path == scan_root.as_path() {
        return Err(LexicalPathError::EqualToScanRoot);
    }
    let relative_to_scan_root = path
        .strip_prefix(scan_root.as_path())
        .map_err(|_| LexicalPathError::OutsideScanRoot)?;
    if relative_to_scan_root.as_os_str().is_empty() {
        return Err(LexicalPathError::EqualToScanRoot);
    }
    if relative_to_scan_root
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(LexicalPathError::OutsideScanRoot);
    }

    Ok(LexicalCleanupPath {
        scan_root: scan_root.as_path().to_path_buf(),
        path: path.to_path_buf(),
        relative_to_scan_root: relative_to_scan_root.to_path_buf(),
    })
}

fn validate_absolute_path(path: &Path) -> Result<(), LexicalPathError> {
    validate_raw_syntax(path)?;
    if !path.is_absolute() {
        return Err(LexicalPathError::NotAbsolute);
    }
    validate_components(path)
}

fn validate_components(path: &Path) -> Result<(), LexicalPathError> {
    #[cfg(windows)]
    validate_windows_prefix(path)?;

    let mut normal_component_index = 0;
    for component in path.components() {
        match component {
            Component::CurDir => {
                return Err(LexicalPathError::CurrentDirectory {
                    component_index: normal_component_index,
                });
            }
            Component::ParentDir => {
                return Err(LexicalPathError::ParentTraversal {
                    component_index: normal_component_index,
                });
            }
            Component::Normal(value) => {
                #[cfg(windows)]
                validate_windows_component(value, normal_component_index)?;
                #[cfg(not(windows))]
                let _ = value;
                normal_component_index += 1;
            }
            Component::Prefix(_) | Component::RootDir => {}
        }
    }
    Ok(())
}

fn is_filesystem_root(path: &Path) -> bool {
    path.components()
        .all(|component| matches!(component, Component::Prefix(_) | Component::RootDir))
}

#[cfg(unix)]
fn validate_raw_syntax(path: &Path) -> Result<(), LexicalPathError> {
    use std::os::unix::ffi::OsStrExt;

    let bytes = path.as_os_str().as_bytes();
    if bytes.is_empty() {
        return Err(LexicalPathError::Empty);
    }
    if bytes.len() > MAX_PATH_ENCODED_UNITS {
        return Err(LexicalPathError::TooLong {
            actual_units: bytes.len(),
            maximum_units: MAX_PATH_ENCODED_UNITS,
        });
    }
    let value = std::str::from_utf8(bytes).map_err(|_| LexicalPathError::InvalidEncoding)?;
    if let Some((unit_index, _)) = value
        .char_indices()
        .find(|(_, character)| character.is_control())
    {
        return Err(LexicalPathError::ControlCharacter { unit_index });
    }
    validate_separators_and_dot_segments(value, |character| character == '/', char::len_utf8)
}

#[cfg(windows)]
fn validate_raw_syntax(path: &Path) -> Result<(), LexicalPathError> {
    use std::os::windows::ffi::OsStrExt;

    let units = path.as_os_str().encode_wide().collect::<Vec<_>>();
    if units.is_empty() {
        return Err(LexicalPathError::Empty);
    }
    if units.len() > MAX_PATH_ENCODED_UNITS {
        return Err(LexicalPathError::TooLong {
            actual_units: units.len(),
            maximum_units: MAX_PATH_ENCODED_UNITS,
        });
    }
    let value = String::from_utf16(&units).map_err(|_| LexicalPathError::InvalidEncoding)?;
    let mut unit_index = 0;
    for character in value.chars() {
        if character.is_control() {
            return Err(LexicalPathError::ControlCharacter { unit_index });
        }
        unit_index += character.len_utf16();
    }
    validate_separators_and_dot_segments(
        &value,
        |character| matches!(character, '/' | '\\'),
        char::len_utf16,
    )
}

#[cfg(not(any(unix, windows)))]
fn validate_raw_syntax(path: &Path) -> Result<(), LexicalPathError> {
    let Some(value) = path.to_str() else {
        return Err(LexicalPathError::InvalidEncoding);
    };
    if value.is_empty() {
        return Err(LexicalPathError::Empty);
    }
    if value.len() > MAX_PATH_ENCODED_UNITS {
        return Err(LexicalPathError::TooLong {
            actual_units: value.len(),
            maximum_units: MAX_PATH_ENCODED_UNITS,
        });
    }
    if let Some((unit_index, _)) = value
        .char_indices()
        .find(|(_, character)| character.is_control())
    {
        return Err(LexicalPathError::ControlCharacter { unit_index });
    }
    validate_separators_and_dot_segments(value, |character| character == '/', char::len_utf8)
}

fn validate_separators_and_dot_segments(
    value: &str,
    is_separator: impl Fn(char) -> bool,
    encoded_units: impl Fn(char) -> usize,
) -> Result<(), LexicalPathError> {
    let mut next_unit_index = 0;
    let characters = value
        .chars()
        .map(|character| {
            let unit_index = next_unit_index;
            next_unit_index += encoded_units(character);
            (unit_index, character)
        })
        .collect::<Vec<_>>();
    for pair in characters.windows(2) {
        if is_separator(pair[0].1) && is_separator(pair[1].1) {
            return Err(LexicalPathError::RepeatedSeparator {
                unit_index: pair[1].0,
            });
        }
    }

    let root_terminator = is_host_filesystem_root_text(value);
    if !root_terminator
        && characters
            .last()
            .is_some_and(|(_, character)| is_separator(*character))
    {
        return Err(LexicalPathError::TrailingSeparator);
    }

    let mut component_index = 0;
    for component in value.split(is_separator) {
        if component.is_empty() {
            continue;
        }
        if component == "." {
            return Err(LexicalPathError::CurrentDirectory { component_index });
        }
        if component == ".." {
            return Err(LexicalPathError::ParentTraversal { component_index });
        }
        #[cfg(windows)]
        if component_index == 0 && component.ends_with(':') {
            continue;
        }
        component_index += 1;
    }
    Ok(())
}

#[cfg(unix)]
fn is_host_filesystem_root_text(value: &str) -> bool {
    value == "/"
}

#[cfg(windows)]
fn is_host_filesystem_root_text(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'/' | b'\\')
}

#[cfg(not(any(unix, windows)))]
fn is_host_filesystem_root_text(value: &str) -> bool {
    value == "/"
}

#[cfg(windows)]
fn validate_windows_prefix(path: &Path) -> Result<(), LexicalPathError> {
    use std::path::Prefix;

    let mut components = path.components();
    match components.next() {
        Some(Component::Prefix(prefix)) if matches!(prefix.kind(), Prefix::Disk(_)) => {}
        _ => return Err(LexicalPathError::UnsupportedWindowsPrefix),
    }
    if !matches!(components.next(), Some(Component::RootDir)) {
        return Err(LexicalPathError::NotAbsolute);
    }
    Ok(())
}

#[cfg(windows)]
fn validate_windows_component(
    value: &std::ffi::OsStr,
    component_index: usize,
) -> Result<(), LexicalPathError> {
    let value = value.to_str().ok_or(LexicalPathError::InvalidEncoding)?;
    let invalid = |reason| LexicalPathError::InvalidWindowsComponent {
        component_index,
        reason,
    };
    if value.contains(':') {
        return Err(invalid(WindowsComponentError::AlternateDataStream));
    }
    if value.contains(['<', '>', '"', '|', '?', '*']) {
        return Err(invalid(WindowsComponentError::ReservedCharacter));
    }
    if value.ends_with(['.', ' ']) {
        return Err(invalid(WindowsComponentError::TrailingDotOrSpace));
    }
    let base = value.split('.').next().unwrap_or(value);
    if is_reserved_windows_device_name(base) {
        return Err(invalid(WindowsComponentError::ReservedDeviceName));
    }
    Ok(())
}

#[cfg(windows)]
fn is_reserved_windows_device_name(value: &str) -> bool {
    let upper = value.to_ascii_uppercase();
    matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || matches!(
            upper.as_str(),
            "COM¹" | "COM²" | "COM³" | "LPT¹" | "LPT²" | "LPT³"
        )
        || upper
            .strip_prefix("COM")
            .or_else(|| upper.strip_prefix("LPT"))
            .is_some_and(|suffix| suffix.len() == 1 && matches!(suffix.as_bytes()[0], b'1'..=b'9'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_roots_must_be_absolute_unambiguous_and_bounded() {
        assert_eq!(
            validate_scan_root(Path::new("")),
            Err(LexicalPathError::Empty)
        );
        assert_eq!(
            validate_scan_root(Path::new("relative/path")),
            Err(LexicalPathError::NotAbsolute)
        );
        assert!(matches!(
            validate_scan_root(Path::new(&format!(
                "/{}",
                "a".repeat(MAX_PATH_ENCODED_UNITS)
            ))),
            Err(LexicalPathError::TooLong { .. })
        ));

        #[cfg(unix)]
        {
            let maximum = format!("/{}", "a".repeat(MAX_PATH_ENCODED_UNITS - 1));
            assert!(validate_scan_root(Path::new(&maximum)).is_ok());
        }
    }

    #[test]
    fn cleanup_paths_are_strict_descendants_and_never_filesystem_roots() {
        let filesystem_root = validate_scan_root(Path::new(host_root())).unwrap();
        assert_eq!(
            validate_cleanup_path(&filesystem_root, Path::new(host_root())),
            Err(LexicalPathError::FilesystemRoot)
        );

        let scan_root = validate_scan_root(Path::new(host_scan_root())).unwrap();
        assert_eq!(
            validate_cleanup_path(&scan_root, Path::new(host_scan_root())),
            Err(LexicalPathError::EqualToScanRoot)
        );
        assert_eq!(
            validate_cleanup_path(&scan_root, Path::new(host_outside_path())),
            Err(LexicalPathError::OutsideScanRoot)
        );

        let cleanup = validate_cleanup_path(&scan_root, Path::new(host_descendant_path())).unwrap();
        assert_eq!(cleanup.scan_root(), Path::new(host_scan_root()));
        assert_eq!(cleanup.as_path(), Path::new(host_descendant_path()));
        assert_eq!(
            cleanup.relative_to_scan_root(),
            Path::new("project").join("target")
        );
    }

    #[test]
    fn dot_controls_repeated_and_trailing_syntax_fail_before_normalization() {
        let scan_root = validate_scan_root(Path::new(host_scan_root())).unwrap();
        for (path, expected) in host_unsafe_paths() {
            assert_eq!(
                validate_cleanup_path(&scan_root, Path::new(path)),
                Err(expected)
            );
        }
        assert!(matches!(
            validate_scan_root(Path::new(host_repeated_leading_separator())),
            Err(LexicalPathError::RepeatedSeparator { .. })
        ));
    }

    #[test]
    fn printable_unicode_is_preserved_losslessly() {
        let scan_root = validate_scan_root(Path::new(host_scan_root())).unwrap();
        let path = host_unicode_path();
        let cleanup = validate_cleanup_path(&scan_root, Path::new(&path)).unwrap();
        assert_eq!(cleanup.as_path().as_os_str(), Path::new(&path).as_os_str());
    }

    #[cfg(unix)]
    #[test]
    fn invalid_utf8_is_rejected_instead_of_entering_lossy_cache_paths() {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;

        let invalid = PathBuf::from(OsString::from_vec(vec![b'/', b't', b'm', b'p', b'/', 0xff]));
        assert_eq!(
            validate_scan_root(&invalid),
            Err(LexicalPathError::InvalidEncoding)
        );
    }

    #[cfg(unix)]
    fn host_root() -> &'static str {
        "/"
    }

    #[cfg(windows)]
    fn host_root() -> &'static str {
        r"C:\"
    }

    #[cfg(unix)]
    fn host_scan_root() -> &'static str {
        "/fixture/root"
    }

    #[cfg(windows)]
    fn host_scan_root() -> &'static str {
        r"C:\fixture\root"
    }

    #[cfg(unix)]
    fn host_descendant_path() -> &'static str {
        "/fixture/root/project/target"
    }

    #[cfg(windows)]
    fn host_descendant_path() -> &'static str {
        r"C:\fixture\root\project\target"
    }

    #[cfg(unix)]
    fn host_outside_path() -> &'static str {
        "/fixture/root-other"
    }

    #[cfg(windows)]
    fn host_outside_path() -> &'static str {
        r"C:\fixture\root-other"
    }

    #[cfg(unix)]
    fn host_unicode_path() -> String {
        "/fixture/root/日本語/🙂".to_owned()
    }

    #[cfg(windows)]
    fn host_unicode_path() -> String {
        r"C:\fixture\root\日本語\🙂".to_owned()
    }

    #[cfg(unix)]
    fn host_repeated_leading_separator() -> &'static str {
        "//fixture/root"
    }

    #[cfg(windows)]
    fn host_repeated_leading_separator() -> &'static str {
        r"C:\\fixture\root"
    }

    #[cfg(unix)]
    fn host_unsafe_paths() -> Vec<(&'static str, LexicalPathError)> {
        vec![
            (
                "/fixture/root//target",
                LexicalPathError::RepeatedSeparator { unit_index: 14 },
            ),
            ("/fixture/root/target/", LexicalPathError::TrailingSeparator),
            (
                "/fixture/root/./target",
                LexicalPathError::CurrentDirectory { component_index: 2 },
            ),
            (
                "/fixture/root/../target",
                LexicalPathError::ParentTraversal { component_index: 2 },
            ),
            (
                "/fixture/root/bad\nname",
                LexicalPathError::ControlCharacter { unit_index: 17 },
            ),
            (
                "/fixture/root/bad\0name",
                LexicalPathError::ControlCharacter { unit_index: 17 },
            ),
            (
                "/fixture/root/bad\u{7f}name",
                LexicalPathError::ControlCharacter { unit_index: 17 },
            ),
        ]
    }

    #[cfg(windows)]
    fn host_unsafe_paths() -> Vec<(&'static str, LexicalPathError)> {
        vec![
            (
                r"C:\fixture\root\\target",
                LexicalPathError::RepeatedSeparator { unit_index: 16 },
            ),
            (
                r"C:\fixture\root\target\",
                LexicalPathError::TrailingSeparator,
            ),
            (
                r"C:\fixture\root\.\target",
                LexicalPathError::CurrentDirectory { component_index: 2 },
            ),
            (
                r"C:\fixture\root\..\target",
                LexicalPathError::ParentTraversal { component_index: 2 },
            ),
            (
                "C:\\fixture\\root\\bad\nname",
                LexicalPathError::ControlCharacter { unit_index: 19 },
            ),
            (
                "C:\\fixture\\root\\bad\0name",
                LexicalPathError::ControlCharacter { unit_index: 19 },
            ),
            (
                "C:\\fixture\\root\\bad\u{7f}name",
                LexicalPathError::ControlCharacter { unit_index: 19 },
            ),
        ]
    }

    #[cfg(windows)]
    #[test]
    fn windows_rejects_ambiguous_namespaces_and_components() {
        for path in [
            r"C:relative",
            r"\\server\share\target",
            r"\\?\C:\target",
            r"\\.\C:\target",
        ] {
            assert!(
                validate_scan_root(Path::new(path)).is_err(),
                "accepted {path}"
            );
        }
        for path in [
            r"C:\fixture\root\NUL.txt",
            r"C:\fixture\root\COM¹.log",
            r"C:\fixture\root\LPT³",
            r"C:\fixture\root\stream:secret",
            r"C:\fixture\root\trailing.",
            r"C:\fixture\root\bad*name",
        ] {
            assert!(
                validate_scan_root(Path::new(path)).is_err(),
                "accepted {path}"
            );
        }
    }
}
