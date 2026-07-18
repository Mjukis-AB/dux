//! Bounded positive Cargo 1.96 configuration-file closure.
//!
//! This independently reproduces only Cargo's file/include discovery graph.
//! All other TOML values remain Cargo-owned. The exact enrolled Cargo must also
//! report the same ordered read intent through its pinned tracing callsite.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::path::{Component, Path, PathBuf};

use sha2::{Digest, Sha256};

use super::cargo_config::CargoConfigurationError;
use crate::path_validation::{
    CanonicalFileContentsSnapshot, CanonicalFileDigestSnapshot, CanonicalScanRoot,
    FilesystemEntryKind, capture_regular_file_contents, capture_scan_root, validate_cleanup_path,
    validate_scan_root,
};

pub(super) const MAX_CONFIG_FILES: usize = 64;
const MAX_INCLUDE_EDGES: usize = 128;
const MAX_INCLUDE_DEPTH: usize = 16;
const MAX_CONFIG_FILE_BYTES: usize = 1024 * 1024;
const MAX_CONFIG_CLOSURE_BYTES: usize = 16 * 1024 * 1024;
const MAX_CONFIG_PATH_BYTES: usize = 128 * 1024;
const MAX_TRACE_LINE_BYTES: usize = 256 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ObservedCargoConfigurationFile {
    path: PathBuf,
    parent: CanonicalScanRoot,
    file: CanonicalFileDigestSnapshot,
}

impl ObservedCargoConfigurationFile {
    pub(super) fn path(&self) -> &Path {
        &self.path
    }

    pub(super) fn parent(&self) -> &CanonicalScanRoot {
        &self.parent
    }

    pub(super) fn file(&self) -> &CanonicalFileDigestSnapshot {
        &self.file
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct IncludeEdge {
    parent_file: usize,
    child_file: usize,
    declaration_order: usize,
    declared_path: String,
    optional: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct CargoConfigurationFileClosure {
    files: Vec<ObservedCargoConfigurationFile>,
    read_sequence: Vec<usize>,
    root_count: usize,
    include_edges: Vec<IncludeEdge>,
    total_bytes: usize,
    digest_sha256: [u8; 32],
    read_intent_sha256: [u8; 32],
}

impl CargoConfigurationFileClosure {
    pub(super) fn empty() -> Self {
        ClosureBuilder::default()
            .finish()
            .expect("an empty configuration closure is within every bound")
    }

    pub(super) fn capture(root_paths: &[PathBuf]) -> Result<Self, CargoConfigurationError> {
        if root_paths.len() > MAX_CONFIG_FILES {
            return Err(CargoConfigurationError::Unavailable);
        }
        let mut builder = ClosureBuilder::default();
        for root in root_paths {
            let mut seen = BTreeSet::new();
            builder.capture_recursive(root, 0, &mut seen)?;
            builder.root_count = builder
                .root_count
                .checked_add(1)
                .ok_or(CargoConfigurationError::Unavailable)?;
        }
        builder.finish()
    }

    pub(super) fn files(&self) -> &[ObservedCargoConfigurationFile] {
        &self.files
    }

    pub(super) fn root_count(&self) -> usize {
        self.root_count
    }

    pub(super) fn file_count(&self) -> usize {
        self.files.len()
    }

    pub(super) fn include_edge_count(&self) -> usize {
        self.include_edges.len()
    }

    pub(super) fn total_bytes(&self) -> usize {
        self.total_bytes
    }

    pub(super) fn digest_sha256(&self) -> [u8; 32] {
        self.digest_sha256
    }

    pub(super) fn read_intent_sha256(&self) -> [u8; 32] {
        self.read_intent_sha256
    }

    pub(super) fn verify_read_intent(&self, stderr: &[u8]) -> Result<(), CargoConfigurationError> {
        let reported = parse_read_intent(stderr)?;
        if reported.len() != self.read_sequence.len() {
            return Err(CargoConfigurationError::Changed);
        }
        for (reported, expected_index) in reported.iter().zip(&self.read_sequence) {
            if reported != self.files[*expected_index].path() {
                return Err(CargoConfigurationError::Changed);
            }
        }
        if digest_read_intent(reported.iter().map(PathBuf::as_path)) != self.read_intent_sha256 {
            return Err(CargoConfigurationError::Changed);
        }
        Ok(())
    }
}

#[derive(Default)]
struct ClosureBuilder {
    files: Vec<ObservedCargoConfigurationFile>,
    file_indexes: BTreeMap<PathBuf, usize>,
    identities: BTreeMap<(u64, u128), PathBuf>,
    read_sequence: Vec<usize>,
    root_count: usize,
    include_edges: Vec<IncludeEdge>,
    reserved_include_edges: usize,
    total_bytes: usize,
    native_path_bytes: usize,
}

impl ClosureBuilder {
    fn capture_recursive(
        &mut self,
        path: &Path,
        depth: usize,
        seen: &mut BTreeSet<PathBuf>,
    ) -> Result<usize, CargoConfigurationError> {
        if depth > MAX_INCLUDE_DEPTH || !seen.insert(path.to_path_buf()) {
            return Err(CargoConfigurationError::Unsupported);
        }
        let captured = capture_config_file(path)?;
        self.charge_path(captured.contents.file().path().canonical_path())?;
        self.total_bytes = self
            .total_bytes
            .checked_add(captured.contents.file().byte_length() as usize)
            .ok_or(CargoConfigurationError::Unavailable)?;
        if self.total_bytes > MAX_CONFIG_CLOSURE_BYTES {
            return Err(CargoConfigurationError::Unavailable);
        }
        let identity = captured.contents.file().path().target_identity();
        let identity_key = (identity.volume(), identity.object());
        if let Some(existing) = self.identities.get(&identity_key)
            && existing != path
        {
            return Err(CargoConfigurationError::Unsupported);
        }

        let file_index = if let Some(index) = self.file_indexes.get(path) {
            let expected = &self.files[*index];
            if expected.parent() != &captured.parent || expected.file() != captured.contents.file()
            {
                return Err(CargoConfigurationError::Changed);
            }
            *index
        } else {
            if self.files.len() == MAX_CONFIG_FILES {
                return Err(CargoConfigurationError::Unavailable);
            }
            let index = self.files.len();
            self.file_indexes.insert(path.to_path_buf(), index);
            self.identities.insert(identity_key, path.to_path_buf());
            self.files.push(ObservedCargoConfigurationFile {
                path: path.to_path_buf(),
                parent: captured.parent,
                file: captured.contents.file().clone(),
            });
            index
        };
        self.read_sequence.push(file_index);

        let includes = parse_includes(captured.contents.contents())?;
        for (declaration_order, include) in includes.into_iter().enumerate() {
            if self.reserved_include_edges == MAX_INCLUDE_EDGES {
                return Err(CargoConfigurationError::Unavailable);
            }
            self.reserved_include_edges += 1;
            self.charge_declared_path(&include.path)?;
            let child_path = resolve_include(path, &include)?;
            let child_file = self.capture_recursive(&child_path, depth + 1, seen)?;
            self.include_edges.push(IncludeEdge {
                parent_file: file_index,
                child_file,
                declaration_order,
                declared_path: include.path,
                optional: include.optional,
            });
        }
        Ok(file_index)
    }

    fn charge_path(&mut self, path: &Path) -> Result<(), CargoConfigurationError> {
        self.native_path_bytes = self
            .native_path_bytes
            .checked_add(path.as_os_str().as_bytes().len())
            .ok_or(CargoConfigurationError::Unavailable)?;
        if self.native_path_bytes > MAX_CONFIG_PATH_BYTES {
            return Err(CargoConfigurationError::Unavailable);
        }
        Ok(())
    }

    fn charge_declared_path(&mut self, path: &str) -> Result<(), CargoConfigurationError> {
        self.native_path_bytes = self
            .native_path_bytes
            .checked_add(path.len())
            .ok_or(CargoConfigurationError::Unavailable)?;
        if self.native_path_bytes > MAX_CONFIG_PATH_BYTES {
            return Err(CargoConfigurationError::Unavailable);
        }
        Ok(())
    }

    fn finish(self) -> Result<CargoConfigurationFileClosure, CargoConfigurationError> {
        if self.reserved_include_edges != self.include_edges.len() {
            return Err(CargoConfigurationError::Unavailable);
        }
        let mut digest = Sha256::new();
        digest.update(b"dux-cargo-config-files-v1\0");
        digest.update((self.root_count as u64).to_le_bytes());
        digest.update((self.files.len() as u64).to_le_bytes());
        for (index, file) in self.files.iter().enumerate() {
            digest.update((index as u64).to_le_bytes());
            digest_path(&mut digest, file.path());
            let identity = file.file().path().target_identity();
            digest.update(identity.volume().to_le_bytes());
            digest.update(identity.object().to_le_bytes());
            digest.update(file.file().byte_length().to_le_bytes());
            digest.update(file.file().sha256());
        }
        digest.update((self.include_edges.len() as u64).to_le_bytes());
        for edge in &self.include_edges {
            digest.update((edge.parent_file as u64).to_le_bytes());
            digest.update((edge.child_file as u64).to_le_bytes());
            digest.update((edge.declaration_order as u64).to_le_bytes());
            digest.update([u8::from(edge.optional)]);
            digest.update((edge.declared_path.len() as u64).to_le_bytes());
            digest.update(edge.declared_path.as_bytes());
        }
        digest.update((self.read_sequence.len() as u64).to_le_bytes());
        for index in &self.read_sequence {
            digest.update((*index as u64).to_le_bytes());
        }
        let read_intent_sha256 = digest_read_intent(
            self.read_sequence
                .iter()
                .map(|index| self.files[*index].path()),
        );
        Ok(CargoConfigurationFileClosure {
            files: self.files,
            read_sequence: self.read_sequence,
            root_count: self.root_count,
            include_edges: self.include_edges,
            total_bytes: self.total_bytes,
            digest_sha256: digest.finalize().into(),
            read_intent_sha256,
        })
    }
}

struct CapturedConfigFile {
    parent: CanonicalScanRoot,
    contents: CanonicalFileContentsSnapshot,
}

fn capture_config_file(path: &Path) -> Result<CapturedConfigFile, CargoConfigurationError> {
    if !path.is_absolute() || path.to_str().is_none() || path_has_control_bytes(path) {
        return Err(CargoConfigurationError::Unsupported);
    }
    let canonical = fs::canonicalize(path).map_err(map_config_io)?;
    if canonical != path {
        return Err(CargoConfigurationError::Unsupported);
    }
    let parent_path = path.parent().ok_or(CargoConfigurationError::Unsupported)?;
    let canonical_parent = fs::canonicalize(parent_path).map_err(map_config_io)?;
    if canonical_parent != parent_path {
        return Err(CargoConfigurationError::Unsupported);
    }
    let lexical_parent =
        validate_scan_root(parent_path).map_err(|_| CargoConfigurationError::Unsupported)?;
    let lexical_file = validate_cleanup_path(&lexical_parent, path)
        .map_err(|_| CargoConfigurationError::Unsupported)?;
    let parent =
        capture_scan_root(lexical_parent).map_err(|_| CargoConfigurationError::Unavailable)?;
    let contents = capture_regular_file_contents(&parent, lexical_file, MAX_CONFIG_FILE_BYTES)
        .map_err(|_| CargoConfigurationError::Unsupported)?;
    if contents.file().path().target_kind() != FilesystemEntryKind::RegularFile
        || contents.file().path().hard_link_count() != 1
    {
        return Err(CargoConfigurationError::Unsupported);
    }
    Ok(CapturedConfigFile { parent, contents })
}

fn map_config_io(error: std::io::Error) -> CargoConfigurationError {
    match error.kind() {
        std::io::ErrorKind::NotFound => CargoConfigurationError::Unsupported,
        _ => CargoConfigurationError::Unavailable,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ConfigInclude {
    path: String,
    optional: bool,
}

fn parse_includes(bytes: &[u8]) -> Result<Vec<ConfigInclude>, CargoConfigurationError> {
    let text = std::str::from_utf8(bytes).map_err(|_| CargoConfigurationError::Unsupported)?;
    let table = text
        .parse::<toml::Table>()
        .map_err(|_| CargoConfigurationError::Unsupported)?;
    let Some(include) = table.get("include") else {
        return Ok(Vec::new());
    };
    let values = include
        .as_array()
        .ok_or(CargoConfigurationError::Unsupported)?;
    let mut includes = Vec::with_capacity(values.len());
    for value in values {
        let include = if let Some(path) = value.as_str() {
            ConfigInclude {
                path: path.to_owned(),
                optional: false,
            }
        } else if let Some(table) = value.as_table() {
            let path = table
                .get("path")
                .and_then(toml::Value::as_str)
                .ok_or(CargoConfigurationError::Unsupported)?;
            let optional = match table.get("optional") {
                Some(value) => value
                    .as_bool()
                    .ok_or(CargoConfigurationError::Unsupported)?,
                None => false,
            };
            ConfigInclude {
                path: path.to_owned(),
                optional,
            }
        } else {
            return Err(CargoConfigurationError::Unsupported);
        };
        validate_include_declaration(&include)?;
        includes.push(include);
    }
    Ok(includes)
}

fn validate_include_declaration(include: &ConfigInclude) -> Result<(), CargoConfigurationError> {
    if include.path.is_empty()
        || include.path.chars().any(char::is_control)
        || Path::new(&include.path).extension() != Some(OsStr::new("toml"))
        || include
            .path
            .chars()
            .any(|character| matches!(character, '*' | '?' | '[' | ']' | '{' | '}'))
    {
        return Err(CargoConfigurationError::Unsupported);
    }
    Ok(())
}

fn resolve_include(
    including_file: &Path,
    include: &ConfigInclude,
) -> Result<PathBuf, CargoConfigurationError> {
    let include_path = Path::new(&include.path);
    if include_path
        .components()
        .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
    {
        return Err(CargoConfigurationError::Unsupported);
    }
    let resolved = including_file
        .parent()
        .ok_or(CargoConfigurationError::Unsupported)?
        .join(include_path);
    match fs::symlink_metadata(&resolved) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            Err(CargoConfigurationError::Unsupported)
        }
        Ok(_) => Ok(resolved),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound && include.optional => {
            // Missing optional includes add an absence namespace that this
            // positive-file slice deliberately does not claim to fence.
            Err(CargoConfigurationError::Unsupported)
        }
        Err(error) => Err(map_config_io(error)),
    }
}

fn parse_read_intent(stderr: &[u8]) -> Result<Vec<PathBuf>, CargoConfigurationError> {
    const MARKER: &str = " DEBUG cargo::util::context: load config from file path=";
    const SUFFIX: &str = " why_load=FileDiscovery includes=true";

    let stderr = std::str::from_utf8(stderr).map_err(|_| CargoConfigurationError::Changed)?;
    let mut paths = Vec::new();
    for line in stderr.lines() {
        if line.len() > MAX_TRACE_LINE_BYTES {
            return Err(CargoConfigurationError::Changed);
        }
        let Some(marker_index) = line.find(MARKER) else {
            continue;
        };
        if line[marker_index + MARKER.len()..].contains(MARKER) {
            return Err(CargoConfigurationError::Changed);
        }
        validate_uptime_prefix(&line[..marker_index])?;
        let literal = line[marker_index + MARKER.len()..]
            .strip_suffix(SUFFIX)
            .ok_or(CargoConfigurationError::Changed)?;
        let path: String =
            serde_json::from_str(literal).map_err(|_| CargoConfigurationError::Changed)?;
        if path.is_empty() || path.chars().any(char::is_control) {
            return Err(CargoConfigurationError::Changed);
        }
        paths.push(PathBuf::from(path));
    }
    Ok(paths)
}

fn validate_uptime_prefix(prefix: &str) -> Result<(), CargoConfigurationError> {
    let Some(number) = prefix.trim_start().strip_suffix('s') else {
        return Err(CargoConfigurationError::Changed);
    };
    let Some((seconds, nanos)) = number.split_once('.') else {
        return Err(CargoConfigurationError::Changed);
    };
    if seconds.is_empty()
        || !seconds.bytes().all(|byte| byte.is_ascii_digit())
        || nanos.len() != 9
        || !nanos.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(CargoConfigurationError::Changed);
    }
    Ok(())
}

fn digest_read_intent<'a>(paths: impl Iterator<Item = &'a Path>) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(b"dux-cargo-config-read-intent-v1\0");
    for path in paths {
        digest_path(&mut digest, path);
    }
    digest.finalize().into()
}

fn digest_path(digest: &mut Sha256, path: &Path) {
    let bytes = path.as_os_str().as_bytes();
    digest.update((bytes.len() as u64).to_le_bytes());
    digest.update(bytes);
}

fn path_has_control_bytes(path: &Path) -> bool {
    path.as_os_str()
        .as_bytes()
        .iter()
        .any(|byte| byte.is_ascii_control())
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::symlink;

    use tempfile::TempDir;

    use super::*;

    #[test]
    fn recursive_required_includes_bind_exact_bytes_and_read_intent() {
        let temp = TempDir::new().unwrap();
        let root = fs::canonicalize(temp.path()).unwrap();
        let config = root.join("config.toml");
        let first = root.join("first.toml");
        let second = root.join("second.toml");
        fs::write(&config, "include = [\"first.toml\"]\n").unwrap();
        fs::write(&first, "include = [{ path = \"second.toml\" }]\n").unwrap();
        fs::write(&second, "[build]\ntarget-dir = \"target\"\n").unwrap();

        let closure =
            CargoConfigurationFileClosure::capture(std::slice::from_ref(&config)).unwrap();
        assert_eq!(closure.root_count(), 1);
        assert_eq!(closure.file_count(), 3);
        assert_eq!(closure.include_edge_count(), 2);
        assert_ne!(closure.digest_sha256(), [0; 32]);
        let stderr = [config, first, second]
            .iter()
            .enumerate()
            .map(|(index, path)| {
                format!(
                    "   0.{index:09}s DEBUG cargo::util::context: load config from file path={} why_load=FileDiscovery includes=true\n",
                    serde_json::to_string(path.to_str().unwrap()).unwrap()
                )
            })
            .collect::<String>();
        closure.verify_read_intent(stderr.as_bytes()).unwrap();

        let records = stderr.lines().collect::<Vec<_>>();
        let missing = format!("{}\n{}\n", records[0], records[1]);
        assert_eq!(
            closure.verify_read_intent(missing.as_bytes()),
            Err(CargoConfigurationError::Changed)
        );
        let reordered = format!("{}\n{}\n{}\n", records[0], records[2], records[1]);
        assert_eq!(
            closure.verify_read_intent(reordered.as_bytes()),
            Err(CargoConfigurationError::Changed)
        );
    }

    #[test]
    fn missing_optional_cycle_alias_and_spoofed_trace_fail_closed() {
        let temp = TempDir::new().unwrap();
        let root = fs::canonicalize(temp.path()).unwrap();
        let config = root.join("config.toml");
        fs::write(
            &config,
            "include = [{ path = \"missing.toml\", optional = true }]\n",
        )
        .unwrap();
        assert_eq!(
            CargoConfigurationFileClosure::capture(std::slice::from_ref(&config)),
            Err(CargoConfigurationError::Unsupported)
        );

        fs::write(&config, "include = [\"cycle.toml\"]\n").unwrap();
        fs::write(root.join("cycle.toml"), "include = [\"config.toml\"]\n").unwrap();
        assert_eq!(
            CargoConfigurationFileClosure::capture(std::slice::from_ref(&config)),
            Err(CargoConfigurationError::Unsupported)
        );

        fs::write(&config, "").unwrap();
        let closure =
            CargoConfigurationFileClosure::capture(std::slice::from_ref(&config)).unwrap();
        let spoof = format!(
            "   0.000000001s DEBUG cargo::util::context: load config from file path={} why_load=FileDiscovery includes=true\nwarning\n   0.123456789s DEBUG cargo::util::context: load config from file path={} why_load=FileDiscovery includes=true\n",
            serde_json::to_string(config.to_str().unwrap()).unwrap(),
            serde_json::to_string(config.to_str().unwrap()).unwrap()
        );
        assert_eq!(
            closure.verify_read_intent(spoof.as_bytes()),
            Err(CargoConfigurationError::Changed)
        );
    }

    #[test]
    fn symlink_hard_link_file_count_and_depth_bounds_fail_closed() {
        let symlink_temp = TempDir::new().unwrap();
        let symlink_root = fs::canonicalize(symlink_temp.path()).unwrap();
        let symlink_config = symlink_root.join("config.toml");
        let real = symlink_root.join("real.toml");
        fs::write(&real, "").unwrap();
        symlink(&real, symlink_root.join("linked.toml")).unwrap();
        fs::write(&symlink_config, "include = [\"linked.toml\"]\n").unwrap();
        assert_eq!(
            CargoConfigurationFileClosure::capture(std::slice::from_ref(&symlink_config)),
            Err(CargoConfigurationError::Unsupported)
        );

        let hard_temp = TempDir::new().unwrap();
        let hard_root = fs::canonicalize(hard_temp.path()).unwrap();
        let hard_config = hard_root.join("config.toml");
        let hard_real = hard_root.join("real.toml");
        let hard_alias = hard_root.join("alias.toml");
        fs::write(&hard_real, "").unwrap();
        fs::hard_link(&hard_real, &hard_alias).unwrap();
        fs::write(&hard_config, "include = [\"alias.toml\"]\n").unwrap();
        assert_eq!(
            CargoConfigurationFileClosure::capture(std::slice::from_ref(&hard_config)),
            Err(CargoConfigurationError::Unsupported)
        );

        let count_temp = TempDir::new().unwrap();
        let count_root = fs::canonicalize(count_temp.path()).unwrap();
        let count_config = count_root.join("config.toml");
        let mut declarations = Vec::new();
        for index in 0..MAX_CONFIG_FILES {
            let name = format!("include-{index}.toml");
            fs::write(count_root.join(&name), "").unwrap();
            declarations.push(format!("\"{name}\""));
        }
        fs::write(
            &count_config,
            format!("include = [{}]\n", declarations.join(", ")),
        )
        .unwrap();
        assert_eq!(
            CargoConfigurationFileClosure::capture(std::slice::from_ref(&count_config)),
            Err(CargoConfigurationError::Unavailable)
        );

        let depth_temp = TempDir::new().unwrap();
        let depth_root = fs::canonicalize(depth_temp.path()).unwrap();
        let depth_config = depth_root.join("depth-0.toml");
        for depth in 0..=MAX_INCLUDE_DEPTH {
            fs::write(
                depth_root.join(format!("depth-{depth}.toml")),
                format!("include = [\"depth-{}.toml\"]\n", depth + 1),
            )
            .unwrap();
        }
        fs::write(
            depth_root.join(format!("depth-{}.toml", MAX_INCLUDE_DEPTH + 1)),
            "",
        )
        .unwrap();
        assert_eq!(
            CargoConfigurationFileClosure::capture(std::slice::from_ref(&depth_config)),
            Err(CargoConfigurationError::Unsupported)
        );
    }

    #[test]
    fn shared_include_and_aggregate_edge_and_byte_bounds_fail_closed() {
        let shared_temp = TempDir::new().unwrap();
        let shared_root = fs::canonicalize(shared_temp.path()).unwrap();
        let shared_config = shared_root.join("config.toml");
        fs::write(
            &shared_config,
            "include = [\"first.toml\", \"second.toml\"]\n",
        )
        .unwrap();
        fs::write(
            shared_root.join("first.toml"),
            "include = [\"shared.toml\"]\n",
        )
        .unwrap();
        fs::write(
            shared_root.join("second.toml"),
            "include = [\"shared.toml\"]\n",
        )
        .unwrap();
        fs::write(shared_root.join("shared.toml"), "").unwrap();
        assert_eq!(
            CargoConfigurationFileClosure::capture(std::slice::from_ref(&shared_config)),
            Err(CargoConfigurationError::Unsupported)
        );

        let edge_temp = TempDir::new().unwrap();
        let edge_root = fs::canonicalize(edge_temp.path()).unwrap();
        for depth in 0..MAX_INCLUDE_DEPTH {
            let contents = if depth + 1 == MAX_INCLUDE_DEPTH {
                String::new()
            } else {
                format!("include = [\"chain-{}.toml\"]\n", depth + 1)
            };
            fs::write(edge_root.join(format!("chain-{depth}.toml")), contents).unwrap();
        }
        let mut edge_roots = Vec::new();
        for index in 0..9 {
            let root = edge_root.join(format!("root-{index}.toml"));
            fs::write(&root, "include = [\"chain-0.toml\"]\n").unwrap();
            edge_roots.push(root);
        }
        assert_eq!(
            CargoConfigurationFileClosure::capture(&edge_roots),
            Err(CargoConfigurationError::Unavailable)
        );

        let byte_temp = TempDir::new().unwrap();
        let byte_root = fs::canonicalize(byte_temp.path()).unwrap();
        let large = format!("#{}\n", "a".repeat(MAX_CONFIG_FILE_BYTES - 2));
        fs::write(byte_root.join("large.toml"), large).unwrap();
        let mut byte_roots = Vec::new();
        for index in 0..17 {
            let root = byte_root.join(format!("root-{index}.toml"));
            fs::write(&root, "include = [\"large.toml\"]\n").unwrap();
            byte_roots.push(root);
        }
        assert_eq!(
            CargoConfigurationFileClosure::capture(&byte_roots),
            Err(CargoConfigurationError::Unavailable)
        );
    }
}
