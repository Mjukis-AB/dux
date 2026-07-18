//! Bounded Cargo 1.96 workspace glob namespace observation.
//!
//! Cargo metadata is discovery, not authority. This guard parses the exact
//! root manifest, reproduces the finite workspace-pattern traversal, records
//! every directory namespace consulted by that traversal, and fences it until
//! the caller has validated an identical metadata result. Paths exposed here
//! are immutable lexical evidence only and grant no cleanup authority.

use std::collections::BTreeMap;
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::path::{Component, Path, PathBuf};

use glob::Pattern;
use sha2::{Digest, Sha256};

use crate::path_validation::{
    CanonicalScanRoot, FilesystemEntryKind, FilesystemIdentity, capture_path_snapshot,
    capture_regular_file_contents, capture_scan_root, validate_cleanup_path, validate_scan_root,
};

const WORKSPACE_GLOB_POLICY_REVISION: u32 = 1;
const MAX_ROOT_MANIFEST_BYTES: usize = 4 * 1024 * 1024;
const MAX_PATTERNS: usize = 768;
const MAX_PATTERN_BYTES: usize = 4 * 1024;
const MAX_PATTERN_TEXT_BYTES: usize = 256 * 1024;
const MAX_PATTERNS_PER_ARRAY: usize = 256;
const MAX_PATTERN_COMPONENTS: usize = 64;
const MAX_DIRECTORIES: usize = 4_096;
const MAX_NAMESPACE_ENTRIES: usize = 65_536;
const MAX_RAW_MATCHES: usize = 65_536;
const MAX_NATIVE_PATH_BYTES: usize = 8 * 1024 * 1024;
const MAX_TRAVERSAL_STEPS: usize = 262_144;
const MAX_ENTRY_COMPARISONS: usize = 2_097_152;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CargoWorkspaceGlobError {
    Invalid,
    Changed,
    Unavailable,
}

/// One Cargo pattern and the exact paths it produced before and after Cargo's
/// directory filter. `used_literal_fallback` is true only when the glob
/// produced zero raw paths; a raw file match therefore never falls back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct CargoWorkspaceGlobPatternExpansion {
    pub(super) pattern: String,
    pub(super) raw_matches: Vec<PathBuf>,
    pub(super) directory_matches: Vec<PathBuf>,
    pub(super) used_literal_fallback: bool,
}

/// Caller-usable immutable workspace declaration and expansion summary.
/// Paths are normalized and relative to `project_root` (the empty path is the
/// root). Keeping the three arrays separate preserves Cargo's ordered lexical
/// member/default-member/exclude semantics for later metadata validation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct CargoWorkspaceGlobExpansion {
    pub(super) root_manifest: PathBuf,
    pub(super) root_manifest_identity: FilesystemIdentity,
    pub(super) root_manifest_sha256: [u8; 32],
    pub(super) workspace_present: bool,
    pub(super) root_package_present: bool,
    pub(super) members_declared: bool,
    pub(super) default_members_declared: bool,
    pub(super) members: Vec<CargoWorkspaceGlobPatternExpansion>,
    pub(super) default_members: Vec<CargoWorkspaceGlobPatternExpansion>,
    pub(super) excludes: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct CargoWorkspaceGlobEvidence {
    pub(super) policy_revision: u32,
    pub(super) pattern_count: u32,
    pub(super) observed_directory_count: u32,
    pub(super) namespace_entry_count: u32,
    pub(super) raw_match_count: u32,
    pub(super) directory_match_count: u32,
    pub(super) root_manifest_sha256: [u8; 32],
    pub(super) closure_sha256: [u8; 32],
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct WorkspaceGlobObservation {
    expansion: CargoWorkspaceGlobExpansion,
    directories: Vec<DirectoryNamespace>,
    literal_probes: Vec<LiteralProbe>,
    closure_sha256: [u8; 32],
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct DirectoryNamespace {
    relative_path: PathBuf,
    identity: FilesystemIdentity,
    entries: Option<Vec<NamespaceEntry>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct NamespaceEntry {
    relative_path: PathBuf,
    kind: FilesystemEntryKind,
    identity: FilesystemIdentity,
    hard_link_count: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct LiteralProbe {
    relative_path: PathBuf,
    state: LiteralProbeState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LiteralProbeState {
    Missing,
    Present {
        kind: FilesystemEntryKind,
        identity: FilesystemIdentity,
        hard_link_count: u64,
    },
}

#[derive(Clone)]
enum PatternComponent {
    Recursive,
    Literal(String),
    Ordinary(Pattern),
}

#[derive(Clone)]
struct ParsedPattern {
    text: String,
    components: Vec<PatternComponent>,
}

struct WorkspacePatternDeclarations {
    workspace_present: bool,
    members_declared: bool,
    default_members_declared: bool,
    members: Vec<String>,
    default_members: Vec<String>,
    excludes: Vec<String>,
}

/// Non-cloneable exact root-manifest and workspace-glob namespace fence.
pub(super) struct CargoWorkspaceGlobGuard {
    project_root: CanonicalScanRoot,
    root_manifest: PathBuf,
    observation: WorkspaceGlobObservation,
    fence: platform::WorkspaceGlobFence,
}

impl CargoWorkspaceGlobGuard {
    pub(super) fn capture(
        project_root: &CanonicalScanRoot,
        root_manifest: &Path,
    ) -> Result<Self, CargoWorkspaceGlobError> {
        let observation = capture_observation(project_root, root_manifest)?;
        let fence = platform::WorkspaceGlobFence::new(project_root, &observation)?;
        let guard = Self {
            project_root: project_root.clone(),
            root_manifest: root_manifest.to_path_buf(),
            observation,
            fence,
        };
        // The first observation discovers the watch set. Arm it, then replay
        // the complete observation before any Cargo result can be accepted.
        guard.revalidate()?;
        Ok(guard)
    }

    #[cfg(test)]
    pub(super) fn capture_unfenced_for_test(
        project_root: &CanonicalScanRoot,
        root_manifest: &Path,
    ) -> Result<Self, CargoWorkspaceGlobError> {
        let observation = capture_observation(project_root, root_manifest)?;
        Ok(Self {
            project_root: project_root.clone(),
            root_manifest: root_manifest.to_path_buf(),
            observation,
            fence: platform::WorkspaceGlobFence::disabled_for_test(),
        })
    }

    pub(super) fn poll(&self) -> Result<(), CargoWorkspaceGlobError> {
        self.fence.poll()
    }

    pub(super) fn revalidate(&self) -> Result<(), CargoWorkspaceGlobError> {
        self.poll()?;
        let current = capture_observation(&self.project_root, &self.root_manifest)
            .map_err(|_| CargoWorkspaceGlobError::Changed)?;
        if current != self.observation {
            return Err(CargoWorkspaceGlobError::Changed);
        }
        self.poll()
    }

    pub(super) fn evidence(&self) -> Result<CargoWorkspaceGlobEvidence, CargoWorkspaceGlobError> {
        self.revalidate()?;
        let expansion = &self.observation.expansion;
        let patterns = expansion.members.iter().chain(&expansion.default_members);
        let (expanded_pattern_count, raw_match_count, directory_match_count) = patterns.fold(
            (0_usize, 0_usize, 0_usize),
            |(patterns, raw, directories), expansion| {
                (
                    patterns.saturating_add(1),
                    raw.saturating_add(expansion.raw_matches.len()),
                    directories.saturating_add(expansion.directory_matches.len()),
                )
            },
        );
        let pattern_count = expanded_pattern_count
            .checked_add(expansion.excludes.len())
            .ok_or(CargoWorkspaceGlobError::Unavailable)?;
        let namespace_entry_count = self
            .observation
            .directories
            .iter()
            .try_fold(0_usize, |count, directory| {
                count.checked_add(directory.entries.as_ref().map_or(0, Vec::len))
            })
            .ok_or(CargoWorkspaceGlobError::Unavailable)?;
        Ok(CargoWorkspaceGlobEvidence {
            policy_revision: WORKSPACE_GLOB_POLICY_REVISION,
            pattern_count: u32::try_from(pattern_count)
                .map_err(|_| CargoWorkspaceGlobError::Unavailable)?,
            observed_directory_count: u32::try_from(self.observation.directories.len())
                .map_err(|_| CargoWorkspaceGlobError::Unavailable)?,
            namespace_entry_count: u32::try_from(namespace_entry_count)
                .map_err(|_| CargoWorkspaceGlobError::Unavailable)?,
            raw_match_count: u32::try_from(raw_match_count)
                .map_err(|_| CargoWorkspaceGlobError::Unavailable)?,
            directory_match_count: u32::try_from(directory_match_count)
                .map_err(|_| CargoWorkspaceGlobError::Unavailable)?,
            root_manifest_sha256: expansion.root_manifest_sha256,
            closure_sha256: self.observation.closure_sha256,
        })
    }

    pub(super) fn expansion(&self) -> Result<CargoWorkspaceGlobExpansion, CargoWorkspaceGlobError> {
        self.revalidate()?;
        Ok(self.observation.expansion.clone())
    }
}

fn capture_observation(
    project_root: &CanonicalScanRoot,
    root_manifest: &Path,
) -> Result<WorkspaceGlobObservation, CargoWorkspaceGlobError> {
    let current_root = capture_exact_directory(project_root.canonical_path())?;
    if &current_root != project_root
        || root_manifest != project_root.canonical_path().join("Cargo.toml")
        || !is_normalized_absolute(root_manifest)
    {
        return Err(CargoWorkspaceGlobError::Invalid);
    }
    let lexical_root = validate_scan_root(project_root.canonical_path())
        .map_err(|_| CargoWorkspaceGlobError::Invalid)?;
    let lexical_manifest = validate_cleanup_path(&lexical_root, root_manifest)
        .map_err(|_| CargoWorkspaceGlobError::Invalid)?;
    let manifest =
        capture_regular_file_contents(project_root, lexical_manifest, MAX_ROOT_MANIFEST_BYTES)
            .map_err(|_| CargoWorkspaceGlobError::Unavailable)?;
    if manifest.file().path().hard_link_count() != 1 {
        return Err(CargoWorkspaceGlobError::Invalid);
    }
    let text =
        std::str::from_utf8(manifest.contents()).map_err(|_| CargoWorkspaceGlobError::Invalid)?;
    let document = text
        .parse::<toml::Table>()
        .map_err(|_| CargoWorkspaceGlobError::Invalid)?;
    let root_package_present = match document.get("package") {
        None => false,
        Some(package) if package.is_table() => true,
        Some(_) => return Err(CargoWorkspaceGlobError::Invalid),
    };
    let WorkspacePatternDeclarations {
        workspace_present,
        members_declared,
        default_members_declared,
        members: member_text,
        default_members: default_text,
        excludes,
    } = parse_workspace_patterns(&document)?;
    let pattern_count = member_text
        .len()
        .checked_add(default_text.len())
        .and_then(|count| count.checked_add(excludes.len()))
        .ok_or(CargoWorkspaceGlobError::Unavailable)?;
    let pattern_text_bytes = member_text
        .iter()
        .chain(&default_text)
        .chain(&excludes)
        .try_fold(0_usize, |total, pattern| total.checked_add(pattern.len()))
        .ok_or(CargoWorkspaceGlobError::Unavailable)?;
    if member_text.len() > MAX_PATTERNS_PER_ARRAY
        || default_text.len() > MAX_PATTERNS_PER_ARRAY
        || excludes.len() > MAX_PATTERNS_PER_ARRAY
        || pattern_count > MAX_PATTERNS
        || pattern_text_bytes > MAX_PATTERN_TEXT_BYTES
    {
        return Err(CargoWorkspaceGlobError::Invalid);
    }
    let parsed_members = parse_patterns(member_text)?;
    let parsed_defaults = parse_patterns(default_text)?;
    validate_literal_excludes(&excludes)?;
    if !parsed_members.is_empty() || !parsed_defaults.is_empty() {
        validate_workspace_root_profile(project_root.canonical_path())?;
    }

    let mut capture = NamespaceCapture::new(project_root, root_manifest)?;
    let members = expand_patterns(&mut capture, &parsed_members)?;
    let default_members = expand_patterns(&mut capture, &parsed_defaults)?;
    let (directories, literal_probes) = capture.finish();
    let expansion = CargoWorkspaceGlobExpansion {
        root_manifest: root_manifest.to_path_buf(),
        root_manifest_identity: manifest.file().path().target_identity(),
        root_manifest_sha256: manifest.file().sha256(),
        workspace_present,
        root_package_present,
        members_declared,
        default_members_declared,
        members,
        default_members,
        excludes,
    };
    let closure_sha256 = digest_observation(&expansion, &directories, &literal_probes);
    Ok(WorkspaceGlobObservation {
        expansion,
        directories,
        literal_probes,
        closure_sha256,
    })
}

fn parse_workspace_patterns(
    document: &toml::Table,
) -> Result<WorkspacePatternDeclarations, CargoWorkspaceGlobError> {
    let Some(workspace) = document.get("workspace") else {
        return Ok(WorkspacePatternDeclarations {
            workspace_present: false,
            members_declared: false,
            default_members_declared: false,
            members: Vec::new(),
            default_members: Vec::new(),
            excludes: Vec::new(),
        });
    };
    let table = workspace
        .as_table()
        .ok_or(CargoWorkspaceGlobError::Invalid)?;
    Ok(WorkspacePatternDeclarations {
        workspace_present: true,
        members_declared: table.contains_key("members"),
        default_members_declared: table.contains_key("default-members"),
        members: parse_string_array(table, "members")?,
        default_members: parse_string_array(table, "default-members")?,
        excludes: parse_string_array(table, "exclude")?,
    })
}

fn parse_string_array(
    table: &toml::Table,
    key: &str,
) -> Result<Vec<String>, CargoWorkspaceGlobError> {
    let Some(value) = table.get(key) else {
        return Ok(Vec::new());
    };
    let array = value.as_array().ok_or(CargoWorkspaceGlobError::Invalid)?;
    array
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::to_owned)
                .ok_or(CargoWorkspaceGlobError::Invalid)
        })
        .collect()
}

fn parse_patterns(patterns: Vec<String>) -> Result<Vec<ParsedPattern>, CargoWorkspaceGlobError> {
    if patterns.len() > MAX_PATTERNS_PER_ARRAY {
        return Err(CargoWorkspaceGlobError::Invalid);
    }
    let mut total_text = 0_usize;
    patterns
        .into_iter()
        .map(|text| {
            total_text = total_text
                .checked_add(text.len())
                .ok_or(CargoWorkspaceGlobError::Unavailable)?;
            if text.is_empty()
                || text.len() > MAX_PATTERN_BYTES
                || total_text > MAX_PATTERN_TEXT_BYTES
                || text.chars().any(char::is_control)
            {
                return Err(CargoWorkspaceGlobError::Invalid);
            }
            if text == "." {
                return Ok(ParsedPattern {
                    text,
                    components: Vec::new(),
                });
            }
            let path = Path::new(&text);
            if path.is_absolute()
                || path.components().any(|component| {
                    matches!(
                        component,
                        Component::RootDir
                            | Component::Prefix(_)
                            | Component::CurDir
                            | Component::ParentDir
                    )
                })
                || text.starts_with('/')
                || text.ends_with('/')
                || text.split('/').any(str::is_empty)
            {
                return Err(CargoWorkspaceGlobError::Invalid);
            }
            let components = text
                .split('/')
                .map(|component| {
                    if component == "**" {
                        Ok(PatternComponent::Recursive)
                    } else if component.contains("**") {
                        Err(CargoWorkspaceGlobError::Invalid)
                    } else if !component.contains(['*', '?', '[']) {
                        Ok(PatternComponent::Literal(component.to_owned()))
                    } else {
                        Pattern::new(component)
                            .map(PatternComponent::Ordinary)
                            .map_err(|_| CargoWorkspaceGlobError::Invalid)
                    }
                })
                .collect::<Result<Vec<_>, _>>()?;
            // glob 0.3.3 collapses consecutive recursive components before
            // traversal, but distinct recursive components separated by an
            // ordinary component retain their independent derivations.
            let components = components.into_iter().fold(
                Vec::<PatternComponent>::new(),
                |mut collapsed, component| {
                    if !matches!(component, PatternComponent::Recursive)
                        || !matches!(collapsed.last(), Some(PatternComponent::Recursive))
                    {
                        collapsed.push(component);
                    }
                    collapsed
                },
            );
            if components.len() > MAX_PATTERN_COMPONENTS {
                return Err(CargoWorkspaceGlobError::Invalid);
            }
            Ok(ParsedPattern { text, components })
        })
        .collect()
}

fn validate_literal_excludes(excludes: &[String]) -> Result<(), CargoWorkspaceGlobError> {
    for exclude in excludes {
        if exclude.is_empty()
            || exclude.len() > MAX_PATTERN_BYTES
            || exclude.chars().any(char::is_control)
        {
            return Err(CargoWorkspaceGlobError::Invalid);
        }
        if exclude == "." {
            continue;
        }
        let path = Path::new(exclude);
        if path.is_absolute()
            || path.components().any(|component| {
                matches!(
                    component,
                    Component::RootDir
                        | Component::Prefix(_)
                        | Component::CurDir
                        | Component::ParentDir
                )
            })
            || exclude.starts_with('/')
            || exclude.ends_with('/')
            || exclude.split('/').any(str::is_empty)
        {
            return Err(CargoWorkspaceGlobError::Invalid);
        }
    }
    Ok(())
}

fn expand_patterns(
    capture: &mut NamespaceCapture<'_>,
    patterns: &[ParsedPattern],
) -> Result<Vec<CargoWorkspaceGlobPatternExpansion>, CargoWorkspaceGlobError> {
    patterns
        .iter()
        .map(|pattern| expand_pattern(capture, pattern))
        .collect()
}

fn expand_pattern(
    capture: &mut NamespaceCapture<'_>,
    pattern: &ParsedPattern,
) -> Result<CargoWorkspaceGlobPatternExpansion, CargoWorkspaceGlobError> {
    let mut raw = Vec::new();
    if pattern.components.is_empty() {
        capture.push_raw_match(&mut raw, PathBuf::new())?;
    } else {
        walk_pattern(capture, Path::new(""), &pattern.components, 0, &mut raw)?;
    }
    if raw.len() > MAX_RAW_MATCHES {
        return Err(CargoWorkspaceGlobError::Unavailable);
    }
    let used_literal_fallback = raw.is_empty();
    let raw_matches = raw;
    capture.charge_raw_matches(raw_matches.len())?;
    let directory_matches = if used_literal_fallback {
        let fallback = if pattern.text == "." {
            PathBuf::new()
        } else {
            PathBuf::from(&pattern.text)
        };
        if capture.kind(&fallback) == Some(FilesystemEntryKind::Directory) {
            capture.retain_path_copy(&fallback)?;
            vec![fallback]
        } else {
            Vec::new()
        }
    } else {
        let mut directories = Vec::new();
        for path in &raw_matches {
            if capture.kind(path) == Some(FilesystemEntryKind::Directory) {
                capture.retain_path_copy(path)?;
                directories.push(path.clone());
            }
        }
        directories
    };
    Ok(CargoWorkspaceGlobPatternExpansion {
        pattern: pattern.text.clone(),
        raw_matches,
        directory_matches,
        used_literal_fallback,
    })
}

fn walk_pattern(
    capture: &mut NamespaceCapture<'_>,
    directory: &Path,
    components: &[PatternComponent],
    index: usize,
    raw: &mut Vec<PathBuf>,
) -> Result<(), CargoWorkspaceGlobError> {
    if directory.components().count() > MAX_PATTERN_COMPONENTS {
        return Err(CargoWorkspaceGlobError::Unavailable);
    }
    // Charge each Cargo-equivalent derivation separately. Distinct recursive
    // paths may reach the same `(directory, index)` and yield duplicate rows.
    capture.retain_path_copy(directory)?;
    capture.charge_traversal_step()?;
    if raw.len() > MAX_RAW_MATCHES {
        return Err(CargoWorkspaceGlobError::Unavailable);
    }
    match &components[index] {
        PatternComponent::Recursive => {
            let entries = capture.directory(directory)?;
            for entry in entries {
                capture.charge_entry_comparison()?;
                capture.charge_traversal_step()?;
                if index + 1 == components.len() {
                    if entry.kind == FilesystemEntryKind::Directory {
                        capture.push_raw_match(raw, entry.relative_path.clone())?;
                    }
                } else {
                    match_recursive_entry(capture, &entry, components, index + 1, raw)?;
                }
                if entry.kind == FilesystemEntryKind::Directory {
                    walk_pattern(capture, &entry.relative_path, components, index, raw)?;
                }
            }
        }
        PatternComponent::Literal(component) => {
            let relative_path = directory.join(component);
            capture.retain_path_copy(&relative_path)?;
            match capture.literal_probe(&relative_path)? {
                LiteralProbeState::Missing => {}
                LiteralProbeState::Present { .. } if index + 1 == components.len() => {
                    capture.push_raw_match(raw, relative_path)?;
                }
                LiteralProbeState::Present {
                    kind: FilesystemEntryKind::Directory,
                    ..
                } => {
                    walk_pattern(capture, &relative_path, components, index + 1, raw)?;
                }
                LiteralProbeState::Present { .. } => {}
            }
        }
        PatternComponent::Ordinary(component) => {
            let entries = capture.directory(directory)?;
            for entry in entries {
                capture.charge_entry_comparison()?;
                let Some(name) = entry.relative_path.file_name() else {
                    return Err(CargoWorkspaceGlobError::Unavailable);
                };
                if !component.matches_path(Path::new(name)) {
                    continue;
                }
                if index + 1 == components.len() {
                    capture.push_raw_match(raw, entry.relative_path)?;
                } else if entry.kind == FilesystemEntryKind::Directory {
                    walk_pattern(capture, &entry.relative_path, components, index + 1, raw)?;
                }
            }
        }
    }
    Ok(())
}

/// Match the component after `**` against the current path, mirroring glob
/// 0.3.3's iterator before it descends into that path's children.
fn match_recursive_entry(
    capture: &mut NamespaceCapture<'_>,
    entry: &NamespaceEntry,
    components: &[PatternComponent],
    index: usize,
    raw: &mut Vec<PathBuf>,
) -> Result<(), CargoWorkspaceGlobError> {
    let name = entry
        .relative_path
        .file_name()
        .and_then(|name| name.to_str());
    let matched = match &components[index] {
        PatternComponent::Literal(component) => name == Some(component.as_str()),
        PatternComponent::Ordinary(component) => name
            .map(|name| component.matches_path(Path::new(name)))
            .unwrap_or(false),
        PatternComponent::Recursive => return Err(CargoWorkspaceGlobError::Invalid),
    };
    if !matched {
        return Ok(());
    }
    if index + 1 == components.len() {
        capture.push_raw_match(raw, entry.relative_path.clone())?;
    } else if entry.kind == FilesystemEntryKind::Directory {
        walk_pattern(capture, &entry.relative_path, components, index + 1, raw)?;
    }
    Ok(())
}

struct NamespaceCapture<'a> {
    project_root: &'a CanonicalScanRoot,
    directories: BTreeMap<PathBuf, DirectoryNamespace>,
    literal_probes: BTreeMap<PathBuf, LiteralProbe>,
    entry_count: usize,
    native_path_bytes: usize,
    traversal_steps: usize,
    entry_comparisons: usize,
    raw_match_count: usize,
}

impl<'a> NamespaceCapture<'a> {
    fn new(
        project_root: &'a CanonicalScanRoot,
        root_manifest: &Path,
    ) -> Result<Self, CargoWorkspaceGlobError> {
        let mut capture = Self {
            project_root,
            directories: BTreeMap::new(),
            literal_probes: BTreeMap::new(),
            entry_count: 0,
            native_path_bytes: 0,
            traversal_steps: 0,
            entry_comparisons: 0,
            raw_match_count: 0,
        };
        // These are the owned path values retained directly by the guard and
        // expansion outside the namespace maps.
        capture.retain_path_copy(project_root.requested_path())?;
        capture.retain_path_copy(project_root.canonical_path())?;
        capture.retain_path_copy(root_manifest)?;
        capture.retain_path_copy(root_manifest)?;
        Ok(capture)
    }

    fn watch_directory(&mut self, relative: &Path) -> Result<(), CargoWorkspaceGlobError> {
        if self.directories.contains_key(relative) {
            return Ok(());
        }
        if self.directories.len() >= MAX_DIRECTORIES || !is_normalized_relative(relative) {
            return Err(CargoWorkspaceGlobError::Unavailable);
        }
        let absolute = if relative.as_os_str().is_empty() {
            self.project_root.canonical_path().to_path_buf()
        } else {
            self.project_root.canonical_path().join(relative)
        };
        let identity = if relative.as_os_str().is_empty() {
            self.project_root.identity()
        } else {
            let snapshot = capture_native_named_entry(self.project_root, &absolute)?;
            if snapshot.kind != FilesystemEntryKind::Directory {
                return Err(CargoWorkspaceGlobError::Changed);
            }
            snapshot.identity
        };
        // The map key and record each retain an owned relative path.
        self.retain_path_copy(relative)?;
        self.retain_path_copy(relative)?;
        self.directories.insert(
            relative.to_path_buf(),
            DirectoryNamespace {
                relative_path: relative.to_path_buf(),
                identity,
                entries: None,
            },
        );
        Ok(())
    }

    fn directory(
        &mut self,
        relative: &Path,
    ) -> Result<Vec<NamespaceEntry>, CargoWorkspaceGlobError> {
        self.watch_directory(relative)?;
        if let Some(entries) = self
            .directories
            .get(relative)
            .and_then(|directory| directory.entries.as_ref())
            .cloned()
        {
            for entry in &entries {
                self.retain_path_copy(&entry.relative_path)?;
            }
            return Ok(entries);
        }
        let absolute = if relative.as_os_str().is_empty() {
            self.project_root.canonical_path().to_path_buf()
        } else {
            self.project_root.canonical_path().join(relative)
        };
        let identity = self
            .directories
            .get(relative)
            .map(|directory| directory.identity)
            .ok_or(CargoWorkspaceGlobError::Unavailable)?;
        let mut relative_paths = Vec::new();
        for entry in fs::read_dir(&absolute).map_err(|_| CargoWorkspaceGlobError::Unavailable)? {
            let name = entry
                .map_err(|_| CargoWorkspaceGlobError::Unavailable)?
                .file_name();
            if name.as_bytes().is_empty() || name.as_bytes().contains(&0) {
                return Err(CargoWorkspaceGlobError::Invalid);
            }
            charge_namespace_entry(&mut self.entry_count)?;
            let relative_path = relative.join(name);
            self.retain_path_copy(&relative_path)?;
            relative_paths.push(relative_path);
        }
        relative_paths.sort_by(|left, right| {
            left.file_name()
                .map(|name| name.as_bytes())
                .cmp(&right.file_name().map(|name| name.as_bytes()))
        });
        let mut entries = Vec::new();
        for relative_path in relative_paths {
            let path = self.project_root.canonical_path().join(&relative_path);
            let metadata =
                fs::symlink_metadata(&path).map_err(|_| CargoWorkspaceGlobError::Unavailable)?;
            if metadata.file_type().is_symlink() {
                return Err(CargoWorkspaceGlobError::Invalid);
            }
            let entry = capture_present(self.project_root, &path)?;
            entries.push(NamespaceEntry {
                relative_path,
                kind: entry.kind,
                identity: entry.identity,
                hard_link_count: entry.hard_link_count,
            });
        }
        let after = capture_exact_directory(&absolute)?;
        if after.identity() != identity {
            return Err(CargoWorkspaceGlobError::Changed);
        }
        let returned_paths = entries
            .iter()
            .map(|entry| entry.relative_path.as_path())
            .collect::<Vec<_>>();
        for path in returned_paths {
            self.retain_path_copy(path)?;
        }
        self.directories
            .get_mut(relative)
            .ok_or(CargoWorkspaceGlobError::Unavailable)?
            .entries = Some(entries.clone());
        Ok(entries)
    }

    fn kind(&self, relative: &Path) -> Option<FilesystemEntryKind> {
        if relative.as_os_str().is_empty() {
            return Some(FilesystemEntryKind::Directory);
        }
        if let Some(state) = self.literal_probes.get(relative).map(|probe| probe.state) {
            return match state {
                LiteralProbeState::Missing => None,
                LiteralProbeState::Present { kind, .. } => Some(kind),
            };
        }
        let parent = relative.parent().unwrap_or_else(|| Path::new(""));
        self.directories
            .get(parent)
            .and_then(|directory| directory.entries.as_ref())
            .and_then(|entries| {
                entries
                    .iter()
                    .find(|entry| entry.relative_path == relative)
                    .map(|entry| entry.kind)
            })
    }

    fn finish(self) -> (Vec<DirectoryNamespace>, Vec<LiteralProbe>) {
        (
            self.directories.into_values().collect(),
            self.literal_probes.into_values().collect(),
        )
    }

    fn literal_probe(
        &mut self,
        relative: &Path,
    ) -> Result<LiteralProbeState, CargoWorkspaceGlobError> {
        if let Some(probe) = self.literal_probes.get(relative) {
            return Ok(probe.state);
        }
        if relative.as_os_str().is_empty() || !is_normalized_relative(relative) {
            return Err(CargoWorkspaceGlobError::Invalid);
        }
        let parent = relative.parent().unwrap_or_else(|| Path::new(""));
        self.watch_directory(parent)?;
        let absolute = self.project_root.canonical_path().join(relative);
        let state = match fs::symlink_metadata(&absolute) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                LiteralProbeState::Missing
            }
            Err(_) => return Err(CargoWorkspaceGlobError::Unavailable),
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(CargoWorkspaceGlobError::Invalid);
            }
            Ok(_) => {
                let entry = capture_native_named_entry(self.project_root, &absolute)?;
                LiteralProbeState::Present {
                    kind: entry.kind,
                    identity: entry.identity,
                    hard_link_count: entry.hard_link_count,
                }
            }
        };
        // The map key and immutable probe each retain the lexical path Cargo
        // used for its native targeted lookup.
        self.retain_path_copy(relative)?;
        self.retain_path_copy(relative)?;
        self.literal_probes.insert(
            relative.to_path_buf(),
            LiteralProbe {
                relative_path: relative.to_path_buf(),
                state,
            },
        );
        Ok(state)
    }

    fn push_raw_match(
        &mut self,
        matches: &mut Vec<PathBuf>,
        path: PathBuf,
    ) -> Result<(), CargoWorkspaceGlobError> {
        if matches.len() >= MAX_RAW_MATCHES {
            return Err(CargoWorkspaceGlobError::Unavailable);
        }
        self.retain_path_copy(&path)?;
        matches.push(path);
        Ok(())
    }

    fn retain_path_copy(&mut self, path: &Path) -> Result<(), CargoWorkspaceGlobError> {
        charge_path(&mut self.native_path_bytes, path)
    }

    fn charge_traversal_step(&mut self) -> Result<(), CargoWorkspaceGlobError> {
        self.traversal_steps = self
            .traversal_steps
            .checked_add(1)
            .ok_or(CargoWorkspaceGlobError::Unavailable)?;
        if self.traversal_steps > MAX_TRAVERSAL_STEPS {
            Err(CargoWorkspaceGlobError::Unavailable)
        } else {
            Ok(())
        }
    }

    fn charge_entry_comparison(&mut self) -> Result<(), CargoWorkspaceGlobError> {
        self.entry_comparisons = self
            .entry_comparisons
            .checked_add(1)
            .ok_or(CargoWorkspaceGlobError::Unavailable)?;
        if self.entry_comparisons > MAX_ENTRY_COMPARISONS {
            Err(CargoWorkspaceGlobError::Unavailable)
        } else {
            Ok(())
        }
    }

    fn charge_raw_matches(&mut self, count: usize) -> Result<(), CargoWorkspaceGlobError> {
        self.raw_match_count = self
            .raw_match_count
            .checked_add(count)
            .ok_or(CargoWorkspaceGlobError::Unavailable)?;
        if self.raw_match_count > MAX_RAW_MATCHES {
            Err(CargoWorkspaceGlobError::Unavailable)
        } else {
            Ok(())
        }
    }
}

#[derive(Clone, Copy)]
struct PresentEntry {
    kind: FilesystemEntryKind,
    identity: FilesystemIdentity,
    hard_link_count: u64,
}

fn capture_present(
    project_root: &CanonicalScanRoot,
    path: &Path,
) -> Result<PresentEntry, CargoWorkspaceGlobError> {
    let lexical_root = validate_scan_root(project_root.canonical_path())
        .map_err(|_| CargoWorkspaceGlobError::Invalid)?;
    let lexical =
        validate_cleanup_path(&lexical_root, path).map_err(|_| CargoWorkspaceGlobError::Invalid)?;
    let snapshot = capture_path_snapshot(project_root, lexical)
        .map_err(|_| CargoWorkspaceGlobError::Unavailable)?;
    Ok(PresentEntry {
        kind: snapshot.target_kind(),
        identity: snapshot.target_identity(),
        hard_link_count: snapshot.hard_link_count(),
    })
}

fn capture_native_named_entry(
    project_root: &CanonicalScanRoot,
    requested: &Path,
) -> Result<PresentEntry, CargoWorkspaceGlobError> {
    match capture_present(project_root, requested) {
        Ok(entry) => Ok(entry),
        #[cfg(target_os = "macos")]
        Err(CargoWorkspaceGlobError::Unavailable) => {
            // glob 0.3.3 asks the filesystem about a literal path directly.
            // On a case-insensitive APFS volume, canonicalization can return
            // the stored spelling even though Cargo retains the declaration's
            // spelling. Prove that every native requested component is not a
            // symlink before and after resolving that spelling, then capture
            // the ordinary bounded snapshot at the canonical in-root path.
            validate_native_named_components(project_root, requested)?;
            let canonical =
                fs::canonicalize(requested).map_err(|_| CargoWorkspaceGlobError::Unavailable)?;
            validate_native_named_components(project_root, requested)?;
            canonical
                .strip_prefix(project_root.canonical_path())
                .map_err(|_| CargoWorkspaceGlobError::Invalid)?;
            capture_present(project_root, &canonical)
        }
        Err(error) => Err(error),
    }
}

#[cfg(target_os = "macos")]
fn validate_native_named_components(
    project_root: &CanonicalScanRoot,
    requested: &Path,
) -> Result<(), CargoWorkspaceGlobError> {
    let relative = requested
        .strip_prefix(project_root.canonical_path())
        .map_err(|_| CargoWorkspaceGlobError::Invalid)?;
    if relative.as_os_str().is_empty() || !is_normalized_relative(relative) {
        return Err(CargoWorkspaceGlobError::Invalid);
    }
    let mut prefix = project_root.canonical_path().to_path_buf();
    for component in relative.components() {
        let Component::Normal(component) = component else {
            return Err(CargoWorkspaceGlobError::Invalid);
        };
        prefix.push(component);
        let metadata =
            fs::symlink_metadata(&prefix).map_err(|_| CargoWorkspaceGlobError::Unavailable)?;
        if metadata.file_type().is_symlink() {
            return Err(CargoWorkspaceGlobError::Invalid);
        }
    }
    Ok(())
}

fn capture_exact_directory(path: &Path) -> Result<CanonicalScanRoot, CargoWorkspaceGlobError> {
    let lexical = validate_scan_root(path).map_err(|_| CargoWorkspaceGlobError::Invalid)?;
    capture_scan_root(lexical).map_err(|_| CargoWorkspaceGlobError::Unavailable)
}

/// Conservative supported profile: glob metacharacters in the canonical
/// workspace-root ancestry would be interpreted by glob 0.3.3 when Cargo
/// concatenates the absolute root and a relative declaration. Refuse that
/// ambiguous scope before capturing any namespace or manifest observation.
fn validate_workspace_root_profile(root: &Path) -> Result<(), CargoWorkspaceGlobError> {
    if !root.is_absolute() {
        return Err(CargoWorkspaceGlobError::Invalid);
    }
    for component in root.components() {
        let Component::Normal(component) = component else {
            continue;
        };
        let text = component
            .to_str()
            .ok_or(CargoWorkspaceGlobError::Unavailable)?;
        if text.contains(['*', '?', '[', ']']) {
            return Err(CargoWorkspaceGlobError::Invalid);
        }
    }
    Ok(())
}

fn is_normalized_absolute(path: &Path) -> bool {
    path.is_absolute()
        && path.components().collect::<PathBuf>() == path
        && !path
            .components()
            .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
}

fn is_normalized_relative(path: &Path) -> bool {
    !path.is_absolute()
        && path.components().collect::<PathBuf>() == path
        && !path.components().any(|component| {
            matches!(
                component,
                Component::RootDir
                    | Component::Prefix(_)
                    | Component::CurDir
                    | Component::ParentDir
            )
        })
}

fn charge_path(total: &mut usize, path: &Path) -> Result<(), CargoWorkspaceGlobError> {
    *total = total
        .checked_add(path.as_os_str().as_bytes().len())
        .ok_or(CargoWorkspaceGlobError::Unavailable)?;
    if *total > MAX_NATIVE_PATH_BYTES {
        Err(CargoWorkspaceGlobError::Unavailable)
    } else {
        Ok(())
    }
}

fn charge_namespace_entry(count: &mut usize) -> Result<(), CargoWorkspaceGlobError> {
    *count = count
        .checked_add(1)
        .ok_or(CargoWorkspaceGlobError::Unavailable)?;
    if *count > MAX_NAMESPACE_ENTRIES {
        Err(CargoWorkspaceGlobError::Unavailable)
    } else {
        Ok(())
    }
}

fn digest_observation(
    expansion: &CargoWorkspaceGlobExpansion,
    directories: &[DirectoryNamespace],
    literal_probes: &[LiteralProbe],
) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(b"dux-cargo-workspace-glob-v1\0");
    digest_path(&mut digest, &expansion.root_manifest);
    digest.update(expansion.root_manifest_identity.volume().to_le_bytes());
    digest.update(expansion.root_manifest_identity.object().to_le_bytes());
    digest.update(expansion.root_manifest_sha256);
    digest.update([u8::from(expansion.workspace_present)]);
    digest.update([u8::from(expansion.root_package_present)]);
    digest.update([u8::from(expansion.members_declared)]);
    digest.update([u8::from(expansion.default_members_declared)]);
    for patterns in [&expansion.members, &expansion.default_members] {
        digest.update((patterns.len() as u64).to_le_bytes());
        for pattern in patterns {
            digest_bytes(&mut digest, pattern.pattern.as_bytes());
            digest.update([u8::from(pattern.used_literal_fallback)]);
            digest_paths(&mut digest, &pattern.raw_matches);
            digest_paths(&mut digest, &pattern.directory_matches);
        }
    }
    digest.update((expansion.excludes.len() as u64).to_le_bytes());
    for exclude in &expansion.excludes {
        digest_bytes(&mut digest, exclude.as_bytes());
    }
    digest.update((directories.len() as u64).to_le_bytes());
    for directory in directories {
        digest_path(&mut digest, &directory.relative_path);
        digest.update(directory.identity.volume().to_le_bytes());
        digest.update(directory.identity.object().to_le_bytes());
        match &directory.entries {
            None => digest.update([0]),
            Some(entries) => {
                digest.update([1]);
                digest.update((entries.len() as u64).to_le_bytes());
                for entry in entries {
                    digest_path(&mut digest, &entry.relative_path);
                    digest.update([match entry.kind {
                        FilesystemEntryKind::Directory => 1,
                        FilesystemEntryKind::RegularFile => 2,
                    }]);
                    digest.update(entry.identity.volume().to_le_bytes());
                    digest.update(entry.identity.object().to_le_bytes());
                    digest.update(entry.hard_link_count.to_le_bytes());
                }
            }
        }
    }
    digest.update((literal_probes.len() as u64).to_le_bytes());
    for probe in literal_probes {
        digest_path(&mut digest, &probe.relative_path);
        match probe.state {
            LiteralProbeState::Missing => digest.update([0]),
            LiteralProbeState::Present {
                kind,
                identity,
                hard_link_count,
            } => {
                digest.update([match kind {
                    FilesystemEntryKind::Directory => 1,
                    FilesystemEntryKind::RegularFile => 2,
                }]);
                digest.update(identity.volume().to_le_bytes());
                digest.update(identity.object().to_le_bytes());
                digest.update(hard_link_count.to_le_bytes());
            }
        }
    }
    digest.finalize().into()
}

fn digest_paths(digest: &mut Sha256, paths: &[PathBuf]) {
    digest.update((paths.len() as u64).to_le_bytes());
    for path in paths {
        digest_path(digest, path);
    }
}

fn digest_path(digest: &mut Sha256, path: &Path) {
    digest_bytes(digest, path.as_os_str().as_bytes());
}

fn digest_bytes(digest: &mut Sha256, bytes: &[u8]) {
    digest.update((bytes.len() as u64).to_le_bytes());
    digest.update(bytes);
}

#[cfg(target_os = "macos")]
mod platform {
    use std::fs;
    use std::mem::MaybeUninit;
    use std::os::fd::{AsFd, AsRawFd, OwnedFd};

    use nix::fcntl::{FcntlArg, FdFlag, OFlag, fcntl, open};
    use nix::libc;
    use nix::mount::MntFlags;
    use nix::sys::event::{EvFlags, EventFilter, FilterFlag, KEvent, Kqueue};
    use nix::sys::stat::{Mode, fstat};
    use nix::sys::statfs::fstatfs;

    use super::{
        CanonicalScanRoot, CargoWorkspaceGlobError, DirectoryNamespace, FilesystemIdentity,
        WorkspaceGlobObservation,
    };

    pub(super) struct WorkspaceGlobFence {
        queue: Kqueue,
        _manifest: OwnedFd,
        _directories: Vec<OwnedFd>,
    }

    impl WorkspaceGlobFence {
        #[cfg(test)]
        pub(super) fn disabled_for_test() -> Self {
            let queue = Kqueue::new().expect("test kqueue must be available");
            fcntl(&queue, FcntlArg::F_SETFD(FdFlag::FD_CLOEXEC))
                .expect("test kqueue must support close-on-exec");
            let manifest = open(
                "/dev/null",
                OFlag::O_RDONLY | OFlag::O_CLOEXEC,
                Mode::empty(),
            )
            .expect("test null descriptor must be available");
            Self {
                queue,
                _manifest: manifest,
                _directories: Vec::new(),
            }
        }

        pub(super) fn new(
            project_root: &CanonicalScanRoot,
            observation: &WorkspaceGlobObservation,
        ) -> Result<Self, CargoWorkspaceGlobError> {
            require_descriptor_budget(observation.directories.len().saturating_add(2))?;
            let queue = Kqueue::new().map_err(|_| CargoWorkspaceGlobError::Unavailable)?;
            fcntl(&queue, FcntlArg::F_SETFD(FdFlag::FD_CLOEXEC))
                .map_err(|_| CargoWorkspaceGlobError::Unavailable)?;
            let manifest = open_watch(
                &observation.expansion.root_manifest,
                observation.expansion.root_manifest_identity,
                false,
            )?;
            let directories = observation
                .directories
                .iter()
                .map(|directory| open_directory(project_root, directory))
                .collect::<Result<Vec<_>, _>>()?;
            let all_mutations = FilterFlag::NOTE_DELETE
                | FilterFlag::NOTE_WRITE
                | FilterFlag::NOTE_EXTEND
                | FilterFlag::NOTE_ATTRIB
                | FilterFlag::NOTE_LINK
                | FilterFlag::NOTE_RENAME
                | FilterFlag::NOTE_REVOKE;
            let changes = std::iter::once(vnode_event(manifest.as_raw_fd(), all_mutations))
                .chain(
                    directories
                        .iter()
                        .map(|directory| vnode_event(directory.as_raw_fd(), all_mutations)),
                )
                .collect::<Vec<_>>();
            let mut events = vec![empty_event(); changes.len().max(1)];
            let count = queue
                .kevent(&changes, &mut events, Some(zero_timeout()))
                .map_err(|_| CargoWorkspaceGlobError::Unavailable)?;
            if events[..count]
                .iter()
                .any(|event| event.flags().contains(EvFlags::EV_ERROR))
            {
                return Err(CargoWorkspaceGlobError::Unavailable);
            }
            if count != 0 {
                return Err(CargoWorkspaceGlobError::Changed);
            }
            Ok(Self {
                queue,
                _manifest: manifest,
                _directories: directories,
            })
        }

        pub(super) fn poll(&self) -> Result<(), CargoWorkspaceGlobError> {
            let mut event = [empty_event()];
            let count = self
                .queue
                .kevent(&[], &mut event, Some(zero_timeout()))
                .map_err(|_| CargoWorkspaceGlobError::Unavailable)?;
            if count == 0 {
                Ok(())
            } else {
                Err(CargoWorkspaceGlobError::Changed)
            }
        }
    }

    fn open_directory(
        project_root: &CanonicalScanRoot,
        directory: &DirectoryNamespace,
    ) -> Result<OwnedFd, CargoWorkspaceGlobError> {
        let path = if directory.relative_path.as_os_str().is_empty() {
            project_root.canonical_path().to_path_buf()
        } else {
            project_root.canonical_path().join(&directory.relative_path)
        };
        open_watch(&path, directory.identity, true)
    }

    fn open_watch(
        path: &std::path::Path,
        identity: FilesystemIdentity,
        directory: bool,
    ) -> Result<OwnedFd, CargoWorkspaceGlobError> {
        let descriptor = open(
            path,
            OFlag::from_bits_retain(libc::O_EVTONLY) | OFlag::O_CLOEXEC | OFlag::O_NOFOLLOW,
            Mode::empty(),
        )
        .map_err(|_| CargoWorkspaceGlobError::Unavailable)?;
        require_reviewed_filesystem(&descriptor)?;
        let status = fstat(&descriptor).map_err(|_| CargoWorkspaceGlobError::Unavailable)?;
        let expected_kind = if directory {
            libc::S_IFDIR
        } else {
            libc::S_IFREG
        };
        if (status.st_mode & libc::S_IFMT) != expected_kind
            || status.st_dev as u64 != identity.volume()
            || u128::from(status.st_ino) != identity.object()
            || (!directory && status.st_nlink != 1)
        {
            return Err(CargoWorkspaceGlobError::Changed);
        }
        Ok(descriptor)
    }

    fn require_reviewed_filesystem(descriptor: &impl AsFd) -> Result<(), CargoWorkspaceGlobError> {
        let status = fstatfs(descriptor).map_err(|_| CargoWorkspaceGlobError::Unavailable)?;
        if status.filesystem_type_name().eq_ignore_ascii_case("apfs")
            && status.flags().contains(MntFlags::MNT_LOCAL)
        {
            Ok(())
        } else {
            Err(CargoWorkspaceGlobError::Unavailable)
        }
    }

    fn require_descriptor_budget(retained: usize) -> Result<(), CargoWorkspaceGlobError> {
        const RESERVE: u64 = 128;
        let mut limit = MaybeUninit::<libc::rlimit>::zeroed();
        if unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, limit.as_mut_ptr()) } == -1 {
            return Err(CargoWorkspaceGlobError::Unavailable);
        }
        let soft_limit = unsafe { limit.assume_init() }.rlim_cur;
        if soft_limit == libc::RLIM_INFINITY {
            return Ok(());
        }
        let open_descriptors = fs::read_dir("/dev/fd")
            .map_err(|_| CargoWorkspaceGlobError::Unavailable)?
            .try_fold(0_u64, |count, entry| {
                entry
                    .map(|_| count.saturating_add(1))
                    .map_err(|_| CargoWorkspaceGlobError::Unavailable)
            })?;
        let retained = u64::try_from(retained).map_err(|_| CargoWorkspaceGlobError::Unavailable)?;
        if open_descriptors
            .checked_add(retained)
            .and_then(|count| count.checked_add(RESERVE))
            .is_some_and(|required| required <= soft_limit)
        {
            Ok(())
        } else {
            Err(CargoWorkspaceGlobError::Unavailable)
        }
    }

    fn vnode_event(descriptor: libc::c_int, flags: FilterFlag) -> KEvent {
        KEvent::new(
            descriptor as usize,
            EventFilter::EVFILT_VNODE,
            EvFlags::EV_ADD | EvFlags::EV_ENABLE | EvFlags::EV_CLEAR,
            flags,
            0,
            0,
        )
    }

    fn empty_event() -> KEvent {
        KEvent::new(
            0,
            EventFilter::EVFILT_VNODE,
            EvFlags::empty(),
            FilterFlag::empty(),
            0,
            0,
        )
    }

    fn zero_timeout() -> libc::timespec {
        libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod platform {
    use super::{CanonicalScanRoot, CargoWorkspaceGlobError, WorkspaceGlobObservation};

    pub(super) struct WorkspaceGlobFence;

    impl WorkspaceGlobFence {
        #[cfg(test)]
        pub(super) fn disabled_for_test() -> Self {
            Self
        }

        pub(super) fn new(
            _project_root: &CanonicalScanRoot,
            _observation: &WorkspaceGlobObservation,
        ) -> Result<Self, CargoWorkspaceGlobError> {
            Err(CargoWorkspaceGlobError::Unavailable)
        }

        pub(super) fn poll(&self) -> Result<(), CargoWorkspaceGlobError> {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::symlink;

    use tempfile::TempDir;

    use super::*;

    fn fixture(manifest: &str) -> (TempDir, CanonicalScanRoot, PathBuf) {
        let temp = TempDir::new().unwrap();
        let root_path = fs::canonicalize(temp.path()).unwrap().join("workspace");
        fs::create_dir(&root_path).unwrap();
        let root_manifest = root_path.join("Cargo.toml");
        fs::write(&root_manifest, manifest).unwrap();
        let root = capture_exact_directory(&root_path).unwrap();
        (temp, root, root_manifest)
    }

    fn unfenced(
        root: &CanonicalScanRoot,
        manifest: &Path,
    ) -> Result<CargoWorkspaceGlobGuard, CargoWorkspaceGlobError> {
        CargoWorkspaceGlobGuard::capture_unfenced_for_test(root, manifest)
    }

    #[test]
    fn exact_patterns_preserve_order_and_expand_directories() {
        let (_temp, root, manifest) = fixture(
            "[workspace]\nmembers = [\"crates/a\", \"tools/b\"]\ndefault-members = [\"crates/a\"]\nexclude = [\"tools/b\"]\n",
        );
        fs::create_dir_all(root.canonical_path().join("crates/a")).unwrap();
        fs::create_dir_all(root.canonical_path().join("tools/b")).unwrap();
        let guard = unfenced(&root, &manifest).unwrap();
        let expansion = guard.expansion().unwrap();
        assert_eq!(
            expansion
                .members
                .iter()
                .map(|pattern| pattern.pattern.as_str())
                .collect::<Vec<_>>(),
            ["crates/a", "tools/b"]
        );
        assert_eq!(
            expansion.members[0].raw_matches,
            [PathBuf::from("crates/a")]
        );
        assert_eq!(
            expansion.members[0].directory_matches,
            [PathBuf::from("crates/a")]
        );
        assert!(!expansion.members[0].used_literal_fallback);
        assert_eq!(expansion.default_members[0].pattern, "crates/a");
        assert_eq!(expansion.excludes[0], "tools/b");
        assert!(expansion.members_declared);
        assert!(expansion.default_members_declared);
        assert!(!expansion.root_package_present);
        assert_ne!(guard.evidence().unwrap().closure_sha256, [0; 32]);
    }

    #[test]
    fn literal_lookup_does_not_enumerate_or_reject_unrelated_siblings() {
        let (_temp, root, manifest) = fixture("[workspace]\nmembers = [\"crates/named\"]\n");
        fs::create_dir_all(root.canonical_path().join("crates/named")).unwrap();
        let outside = root
            .canonical_path()
            .parent()
            .unwrap()
            .join("outside-literal");
        fs::create_dir(&outside).unwrap();
        symlink(
            &outside,
            root.canonical_path().join("crates/unrelated-link"),
        )
        .unwrap();

        let guard = unfenced(&root, &manifest).unwrap();
        let expansion = guard.expansion().unwrap();
        assert_eq!(
            expansion.members[0].directory_matches,
            [PathBuf::from("crates/named")]
        );
        assert_eq!(guard.evidence().unwrap().namespace_entry_count, 0);

        // A literal probe is independent from the namespace-enumeration cap:
        // no sibling row is visited or retained by this targeted lookup.
        let mut capture = NamespaceCapture::new(&root, &manifest).unwrap();
        capture.entry_count = MAX_NAMESPACE_ENTRIES;
        assert!(matches!(
            capture.literal_probe(Path::new("crates/named")),
            Ok(LiteralProbeState::Present {
                kind: FilesystemEntryKind::Directory,
                ..
            })
        ));
        assert_eq!(capture.entry_count, MAX_NAMESPACE_ENTRIES);
    }

    #[test]
    fn selected_literal_symlink_still_fails_closed() {
        let (_temp, root, manifest) = fixture("[workspace]\nmembers = [\"linked\"]\n");
        let outside = root
            .canonical_path()
            .parent()
            .unwrap()
            .join("outside-selected");
        fs::create_dir(&outside).unwrap();
        symlink(&outside, root.canonical_path().join("linked")).unwrap();
        assert!(matches!(
            unfenced(&root, &manifest),
            Err(CargoWorkspaceGlobError::Invalid)
        ));
    }

    #[test]
    fn recursive_double_star_matches_zero_and_many_directory_components() {
        let (_temp, root, manifest) = fixture("[workspace]\nmembers = [\"crates/**/member?\"]\n");
        for relative in [
            "crates/member1",
            "crates/group/member2",
            "crates/group/deep/member3",
            "crates/group/deep/not-a-member",
        ] {
            fs::create_dir_all(root.canonical_path().join(relative)).unwrap();
        }
        let expansion = unfenced(&root, &manifest).unwrap().expansion().unwrap();
        assert_eq!(
            expansion.members[0].directory_matches,
            [
                PathBuf::from("crates/group/deep/member3"),
                PathBuf::from("crates/group/member2"),
                PathBuf::from("crates/member1"),
            ]
        );
    }

    #[test]
    fn terminal_double_star_matches_descendant_directories_only() {
        let (_temp, root, manifest) = fixture("[workspace]\nmembers = [\"crates/**\"]\n");
        fs::create_dir_all(root.canonical_path().join("crates/a/b")).unwrap();
        fs::write(
            root.canonical_path().join("crates/file"),
            b"not a directory",
        )
        .unwrap();
        let expansion = unfenced(&root, &manifest).unwrap().expansion().unwrap();
        assert_eq!(
            expansion.members[0].raw_matches,
            [PathBuf::from("crates/a"), PathBuf::from("crates/a/b")]
        );
        assert_eq!(
            expansion.members[0].directory_matches,
            expansion.members[0].raw_matches
        );
    }

    #[test]
    fn consecutive_recursive_components_collapse_and_component_count_is_bounded() {
        let (_temp, root, manifest) = fixture("[workspace]\nmembers = [\"**/**/member\"]\n");
        fs::create_dir_all(root.canonical_path().join("a/b/member")).unwrap();
        let expansion = unfenced(&root, &manifest).unwrap().expansion().unwrap();
        assert_eq!(
            expansion.members[0].directory_matches,
            [PathBuf::from("a/b/member")]
        );

        let too_deep = std::iter::repeat_n("component", MAX_PATTERN_COMPONENTS + 1)
            .collect::<Vec<_>>()
            .join("/");
        let (_temp, root, manifest) = fixture(&format!("[workspace]\nmembers = [{too_deep:?}]\n"));
        assert!(matches!(
            unfenced(&root, &manifest),
            Err(CargoWorkspaceGlobError::Invalid)
        ));
    }

    #[test]
    fn separated_recursive_components_preserve_duplicate_derivations() {
        let (_temp, root, manifest) =
            fixture("[workspace]\ndefault-members = [\"**/foo/**/bar\"]\n");
        fs::create_dir_all(root.canonical_path().join("foo/foo/bar")).unwrap();
        let expansion = unfenced(&root, &manifest).unwrap().expansion().unwrap();
        let matches = &expansion.default_members[0].directory_matches;
        assert_eq!(
            matches
                .iter()
                .filter(|path| path.as_path() == Path::new("foo/foo/bar"))
                .count(),
            2
        );
    }

    #[test]
    fn excludes_are_normalized_literals_and_do_not_expand_namespaces() {
        let (_temp, root, manifest) = fixture(
            "[package]\nname = \"root\"\nversion = \"0.1.0\"\n[workspace]\nmembers = [\".\"]\nexclude = [\"crates/*\"]\n",
        );
        let crates = root.canonical_path().join("crates");
        let outside = root
            .canonical_path()
            .parent()
            .unwrap()
            .join("outside-exclude");
        fs::create_dir(&crates).unwrap();
        fs::create_dir(&outside).unwrap();
        symlink(&outside, crates.join("not-enumerated")).unwrap();
        let expansion = unfenced(&root, &manifest).unwrap().expansion().unwrap();
        assert_eq!(expansion.excludes, ["crates/*"]);
        assert!(expansion.root_package_present);
        let mut forged = expansion.clone();
        forged.root_package_present = false;
        assert_ne!(
            digest_observation(&expansion, &[], &[]),
            digest_observation(&forged, &[], &[])
        );
    }

    #[test]
    fn absent_and_explicit_empty_default_members_remain_distinct() {
        let (_temp, absent_root, absent_manifest) = fixture("[workspace]\nmembers = []\n");
        let absent = unfenced(&absent_root, &absent_manifest)
            .unwrap()
            .expansion()
            .unwrap();
        assert!(absent.members_declared);
        assert!(!absent.default_members_declared);

        let (_temp, empty_root, empty_manifest) =
            fixture("[workspace]\nmembers = []\ndefault-members = []\n");
        let explicit = unfenced(&empty_root, &empty_manifest)
            .unwrap()
            .expansion()
            .unwrap();
        assert!(explicit.default_members_declared);
        assert_ne!(
            digest_observation(&absent, &[], &[]),
            digest_observation(&explicit, &[], &[])
        );
    }

    #[test]
    #[expect(
        clippy::disallowed_methods,
        reason = "test removes only a TempDir-owned entry to prove namespace revalidation"
    )]
    fn namespace_revalidation_detects_creation_and_removal() {
        let (_temp, root, manifest) = fixture("[workspace]\nmembers = [\"crates/*\"]\n");
        fs::create_dir(root.canonical_path().join("crates")).unwrap();
        let before_create = unfenced(&root, &manifest).unwrap();
        fs::create_dir(root.canonical_path().join("crates/new")).unwrap();
        assert_eq!(
            before_create.revalidate(),
            Err(CargoWorkspaceGlobError::Changed)
        );

        let before_remove = unfenced(&root, &manifest).unwrap();
        fs::remove_dir(root.canonical_path().join("crates/new")).unwrap();
        assert_eq!(
            before_remove.revalidate(),
            Err(CargoWorkspaceGlobError::Changed)
        );
    }

    #[test]
    fn literal_fallback_depends_on_raw_glob_paths_before_directory_filter() {
        let (_temp, root, manifest) =
            fixture("[workspace]\nmembers = [\"missing\", \"only-file\"]\n");
        fs::write(
            root.canonical_path().join("only-file"),
            b"not a member directory",
        )
        .unwrap();
        let expansion = unfenced(&root, &manifest).unwrap().expansion().unwrap();
        assert!(expansion.members[0].raw_matches.is_empty());
        assert!(expansion.members[0].used_literal_fallback);
        assert!(expansion.members[0].directory_matches.is_empty());
        assert_eq!(
            expansion.members[1].raw_matches,
            [PathBuf::from("only-file")]
        );
        assert!(!expansion.members[1].used_literal_fallback);
        assert!(expansion.members[1].directory_matches.is_empty());
    }

    #[test]
    fn malformed_outside_symlink_and_pattern_bounds_fail_closed() {
        for pattern in ["../outside", "/absolute", "bad\nname", "[", "a/**b"] {
            let (_temp, root, manifest) =
                fixture(&format!("[workspace]\nmembers = [{pattern:?}]\n"));
            assert!(matches!(
                unfenced(&root, &manifest),
                Err(CargoWorkspaceGlobError::Invalid)
            ));
        }

        let (_temp, root, manifest) = fixture("[workspace]\nmembers = [\"*\"]\n");
        let outside = root.canonical_path().parent().unwrap().join("outside");
        fs::create_dir(&outside).unwrap();
        symlink(&outside, root.canonical_path().join("linked")).unwrap();
        assert!(matches!(
            unfenced(&root, &manifest),
            Err(CargoWorkspaceGlobError::Invalid)
        ));

        let patterns = std::iter::repeat_n("\"missing\"", MAX_PATTERNS + 1)
            .collect::<Vec<_>>()
            .join(",");
        let (_temp, root, manifest) = fixture(&format!("[workspace]\nmembers = [{patterns}]\n"));
        assert!(matches!(
            unfenced(&root, &manifest),
            Err(CargoWorkspaceGlobError::Invalid)
        ));
    }

    #[test]
    fn namespace_entry_bound_accepts_n_and_rejects_n_plus_one_before_storage() {
        let mut count = MAX_NAMESPACE_ENTRIES - 1;
        assert_eq!(charge_namespace_entry(&mut count), Ok(()));
        assert_eq!(count, MAX_NAMESPACE_ENTRIES);
        assert_eq!(
            charge_namespace_entry(&mut count),
            Err(CargoWorkspaceGlobError::Unavailable)
        );
    }

    #[test]
    fn retained_traversal_path_copies_are_aggregate_bounded() {
        let (_temp, root, manifest) = fixture("[workspace]\nmembers = []\n");
        let mut capture = NamespaceCapture::new(&root, &manifest).unwrap();
        let adversarial = PathBuf::from("x".repeat(64 * 1024));
        let mut accepted = 0_usize;
        while capture.retain_path_copy(&adversarial).is_ok() {
            accepted += 1;
            assert!(accepted <= MAX_NATIVE_PATH_BYTES / adversarial.as_os_str().as_bytes().len());
        }
        assert!(accepted > 0);
        assert!(capture.native_path_bytes > MAX_NATIVE_PATH_BYTES);
    }

    #[test]
    fn canonical_workspace_root_metacharacters_are_rejected_only_when_glob_is_active() {
        let temp = TempDir::new().unwrap();
        let root_path = fs::canonicalize(temp.path()).unwrap().join("work[space]");
        fs::create_dir(&root_path).unwrap();
        let manifest = root_path.join("Cargo.toml");
        fs::write(&manifest, "[workspace]\nmembers = []\n").unwrap();
        let root = capture_exact_directory(&root_path).unwrap();

        assert!(unfenced(&root, &manifest).is_ok());

        fs::write(&manifest, "[workspace]\nmembers = [\"*\"]\n").unwrap();
        assert!(matches!(
            unfenced(&root, &manifest),
            Err(CargoWorkspaceGlobError::Invalid)
        ));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn literal_lookup_preserves_native_case_insensitive_resolution() {
        let (_temp, root, manifest) = fixture("[workspace]\nmembers = [\"named\"]\n");
        fs::create_dir(root.canonical_path().join("Named")).unwrap();
        if !root.canonical_path().join("named").is_dir() {
            // A case-sensitive APFS test volume cannot exercise this native
            // lookup behavior.
            return;
        }
        let expansion = unfenced(&root, &manifest).unwrap().expansion().unwrap();
        assert_eq!(
            expansion.members[0].directory_matches,
            [PathBuf::from("named")]
        );
    }

    #[test]
    fn root_manifest_must_be_the_exact_bounded_parseable_file() {
        let (temp, root, manifest) = fixture("[workspace]\nmembers = [\".\"]\n");
        let outside = fs::canonicalize(temp.path()).unwrap().join("Cargo.toml");
        fs::write(&outside, "[workspace]\n").unwrap();
        assert!(matches!(
            unfenced(&root, &outside),
            Err(CargoWorkspaceGlobError::Invalid)
        ));
        fs::write(&manifest, "[workspace\n").unwrap();
        assert!(matches!(
            unfenced(&root, &manifest),
            Err(CargoWorkspaceGlobError::Invalid)
        ));
    }

    #[cfg(target_os = "macos")]
    #[test]
    #[expect(
        clippy::disallowed_methods,
        reason = "test removes only a TempDir-owned transient name to prove NOTE_WRITE persistence"
    )]
    fn apfs_directory_note_write_is_terminal_even_when_name_is_removed() {
        let (_temp, root, manifest) = fixture("[workspace]\nmembers = [\"crates/*\"]\n");
        let crates = root.canonical_path().join("crates");
        fs::create_dir(&crates).unwrap();
        let guard = CargoWorkspaceGlobGuard::capture(&root, &manifest).unwrap();
        let transient = crates.join("transient");
        fs::create_dir(&transient).unwrap();
        fs::remove_dir(&transient).unwrap();
        assert_eq!(guard.poll(), Err(CargoWorkspaceGlobError::Changed));
    }
}
