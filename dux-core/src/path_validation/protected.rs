use std::path::{Component, Path};

use thiserror::Error;

use super::{
    CanonicalPathSnapshot, CanonicalScanRoot, FilesystemBoundarySnapshot, LexicalCleanupPath,
    TrustedVolumeLocationError, TrustedVolumeLocationWitness, capture_filesystem_boundary,
    capture_scan_root, validate_scan_root,
};

#[cfg(unix)]
use nix::unistd::{User, geteuid, getuid};

/// Bump whenever a protected-root entry or its coverage changes.
pub(crate) const PROTECTED_ROOT_POLICY_REVISION: u32 = 2;

/// Revision of the current-account-home mount/location observation.
pub(crate) const TRUSTED_HOME_MOUNT_PROOF_REVISION: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum ProtectedPathKind {
    FilesystemRoot,
    OperatingSystem,
    ApplicationInstallations,
    SharedSystemLibrary,
    UserHomesContainer,
    VolumeMountContainer,
    UserHome,
    UserLibrary,
    ManagedSoftware,
    ServiceData,
    PackageStore,
    ProgramFiles,
    ProgramData,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum ProtectedPathForm {
    ScanRootRequested,
    ScanRootCanonical,
    TargetRequested,
    TargetCanonical,
}

/// Textual protected-root assessment only.
///
/// `NoTextualMatch` is deliberately not named "allowed" or "safe": complete
/// root-to-scan ancestry, mount-location identity, trusted volume selection,
/// and rule scope are not yet available. No variant grants planning or
/// execution authority.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProtectedRootDisposition {
    Denied {
        form: ProtectedPathForm,
        kind: ProtectedPathKind,
        policy_revision: u32,
    },
    SpecificRuleRequired {
        // `kind` is diagnostic taxonomy, not a grant key. A later grant table
        // must add a stable per-boundary ID before it can satisfy this result.
        form: ProtectedPathForm,
        kind: ProtectedPathKind,
        policy_revision: u32,
    },
    NoTextualMatch {
        policy_revision: u32,
    },
}

impl ProtectedRootDisposition {
    pub(crate) fn policy_revision(self) -> u32 {
        match self {
            Self::Denied {
                policy_revision, ..
            }
            | Self::SpecificRuleRequired {
                policy_revision, ..
            }
            | Self::NoTextualMatch { policy_revision } => policy_revision,
        }
    }
}

#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub(crate) enum ProtectedRootError {
    #[error(
        "the current account home directory could not be discovered from the OS account database"
    )]
    CurrentAccountUnavailable,
    #[error("the current account identity is ambiguous for protected-root policy")]
    CurrentAccountAmbiguous,
    #[error("protected-root policy is unsupported on this platform")]
    UnsupportedPlatform,
    #[error("host path policy cannot represent the configured home directory")]
    InvalidHomeDirectory,
    #[error("host path policy cannot represent {form:?}")]
    InvalidPath { form: ProtectedPathForm },
    #[error("target snapshot does not match the supplied scan-root evidence")]
    MismatchedScanRootEvidence,
}

/// OS-account-derived home evidence retained only inside the path-validation
/// layer. The root and owner are captured from the same no-follow boundary
/// observation; callers cannot provide a path or UID to mint this value.
pub(crate) struct CurrentAccountHomeEvidence {
    root: CanonicalScanRoot,
    boundary: FilesystemBoundarySnapshot,
    uid: u32,
}

impl CurrentAccountHomeEvidence {
    pub(crate) fn capture() -> Result<Self, ProtectedRootError> {
        #[cfg(unix)]
        {
            let real_uid = getuid();
            let effective_uid = geteuid();
            if real_uid != effective_uid {
                return Err(ProtectedRootError::CurrentAccountAmbiguous);
            }
            let first = User::from_uid(effective_uid)
                .map_err(|_| ProtectedRootError::CurrentAccountUnavailable)?
                .ok_or(ProtectedRootError::CurrentAccountUnavailable)?;
            let second = User::from_uid(effective_uid)
                .map_err(|_| ProtectedRootError::CurrentAccountUnavailable)?
                .ok_or(ProtectedRootError::CurrentAccountUnavailable)?;
            if first.uid != second.uid || first.dir != second.dir {
                return Err(ProtectedRootError::CurrentAccountAmbiguous);
            }
            validate_account_home_path(&first.dir)?;
            let lexical = validate_scan_root(&first.dir)
                .map_err(|_| ProtectedRootError::InvalidHomeDirectory)?;
            let root =
                capture_scan_root(lexical).map_err(|_| ProtectedRootError::InvalidHomeDirectory)?;
            let boundary = capture_filesystem_boundary(&root)
                .map_err(|_| ProtectedRootError::InvalidHomeDirectory)?;
            if boundary.root_owner_uid() != Some(effective_uid.as_raw()) {
                return Err(ProtectedRootError::InvalidHomeDirectory);
            }
            let recaptured = capture_scan_root(
                validate_scan_root(root.requested_path())
                    .map_err(|_| ProtectedRootError::InvalidHomeDirectory)?,
            )
            .map_err(|_| ProtectedRootError::InvalidHomeDirectory)?;
            if recaptured.identity() != root.identity()
                || recaptured.canonical_path() != root.canonical_path()
            {
                return Err(ProtectedRootError::CurrentAccountAmbiguous);
            }
            let recaptured_boundary = capture_filesystem_boundary(&recaptured)
                .map_err(|_| ProtectedRootError::InvalidHomeDirectory)?;
            if recaptured_boundary != boundary {
                return Err(ProtectedRootError::CurrentAccountAmbiguous);
            }
            Ok(Self {
                root: recaptured,
                boundary: recaptured_boundary,
                uid: effective_uid.as_raw(),
            })
        }
        #[cfg(not(unix))]
        {
            Err(ProtectedRootError::UnsupportedPlatform)
        }
    }

    fn root(&self) -> &CanonicalScanRoot {
        &self.root
    }

    fn uid(&self) -> u32 {
        self.uid
    }

    fn boundary(&self) -> &FilesystemBoundarySnapshot {
        &self.boundary
    }
}

/// A macOS-first, non-cloneable location witness for a scan root at or below
/// the current account's OS-discovered home mount. This proves location only;
/// it does not grant a protected-root rule, clear `ProtectedPath`, or create
/// planning, approval, FFI, scheduling, or effect authority.
pub(crate) struct TrustedHomeMountWitness {
    home_root: CanonicalScanRoot,
    home_boundary: TrustedVolumeLocationWitness,
    scan_boundary: TrustedVolumeLocationWitness,
    uid: u32,
    proof_revision: u32,
}

#[derive(Debug, Error)]
pub(crate) enum TrustedHomeMountError {
    #[error("current-account home mount proof is unsupported on this platform")]
    UnsupportedPlatform,
    #[error("current-account home evidence could not be captured: {0}")]
    HomeEvidence(#[source] ProtectedRootError),
    #[error("home filesystem boundary could not be captured: {0}")]
    HomeBoundary(#[source] TrustedVolumeLocationError),
    #[error("scan-root filesystem boundary could not be captured: {0}")]
    ScanBoundary(#[source] TrustedVolumeLocationError),
    #[error("scan root is outside the current account home")]
    OutsideHome,
    #[error("scan-root ancestry does not retain the current account home identity")]
    MissingHomeAncestry,
    #[error("scan root and current account home do not share the exact mount")]
    DifferentMount,
    #[error("current-account home mount boundary changed")]
    Changed,
    #[error("current-account home mount proof revision is unsupported")]
    UnsupportedRevision,
}

impl TrustedHomeMountWitness {
    pub(crate) fn capture(scan_root: &CanonicalScanRoot) -> Result<Self, TrustedHomeMountError> {
        #[cfg(not(target_os = "macos"))]
        {
            let _ = scan_root;
            return Err(TrustedHomeMountError::UnsupportedPlatform);
        }
        #[cfg(target_os = "macos")]
        {
            let home = CurrentAccountHomeEvidence::capture()
                .map_err(TrustedHomeMountError::HomeEvidence)?;
            let CurrentAccountHomeEvidence {
                root: home_root,
                boundary: home_boundary_snapshot,
                uid: home_uid,
            } = home;
            let home_boundary =
                TrustedVolumeLocationWitness::from_boundary(&home_root, home_boundary_snapshot)
                    .map_err(TrustedHomeMountError::HomeBoundary)?;
            let scan_boundary = TrustedVolumeLocationWitness::capture(scan_root)
                .map_err(TrustedHomeMountError::ScanBoundary)?;
            if !scan_boundary.is_same_or_descendant_of(&home_boundary) {
                return Err(TrustedHomeMountError::OutsideHome);
            }
            if !scan_boundary.root_ancestry_contains(home_boundary.root_identity()) {
                return Err(TrustedHomeMountError::MissingHomeAncestry);
            }
            if !scan_boundary.same_mount(&home_boundary) {
                return Err(TrustedHomeMountError::DifferentMount);
            }
            if home_boundary.root_owner_uid() != Some(home_uid) {
                return Err(TrustedHomeMountError::HomeEvidence(
                    ProtectedRootError::InvalidHomeDirectory,
                ));
            }
            let witness = Self {
                home_root,
                home_boundary,
                scan_boundary,
                uid: home_uid,
                proof_revision: TRUSTED_HOME_MOUNT_PROOF_REVISION,
            };
            witness.revalidate()?;
            Ok(witness)
        }
    }

    pub(crate) const fn proof_revision(&self) -> u32 {
        self.proof_revision
    }

    pub(crate) fn matches_scan_boundary(&self, boundary: &FilesystemBoundarySnapshot) -> bool {
        self.scan_boundary.matches_boundary(boundary)
    }

    pub(crate) fn capacity_scope(
        &self,
    ) -> Result<crate::path_validation::FilesystemCapacityScope, TrustedHomeMountError> {
        self.revalidate()?;
        self.scan_boundary
            .capacity_scope()
            .ok_or(TrustedHomeMountError::Changed)
    }

    pub(crate) fn revalidate(&self) -> Result<(), TrustedHomeMountError> {
        if self.proof_revision != TRUSTED_HOME_MOUNT_PROOF_REVISION {
            return Err(TrustedHomeMountError::UnsupportedRevision);
        }
        let current_home =
            CurrentAccountHomeEvidence::capture().map_err(TrustedHomeMountError::HomeEvidence)?;
        if current_home.uid() != self.uid
            || current_home.root().requested_path() != self.home_root.requested_path()
            || current_home.root().canonical_path() != self.home_root.canonical_path()
            || current_home.root().identity() != self.home_root.identity()
        {
            return Err(TrustedHomeMountError::Changed);
        }
        self.home_boundary
            .revalidate()
            .map_err(|_| TrustedHomeMountError::Changed)?;
        self.scan_boundary
            .revalidate()
            .map_err(|_| TrustedHomeMountError::Changed)?;
        if !self
            .scan_boundary
            .is_same_or_descendant_of(&self.home_boundary)
            || !self
                .scan_boundary
                .root_ancestry_contains(self.home_boundary.root_identity())
            || !self.scan_boundary.same_mount(&self.home_boundary)
            || self.home_boundary.root_owner_uid() != Some(self.uid)
        {
            return Err(TrustedHomeMountError::Changed);
        }
        Ok(())
    }
}

/// Versioned, crate-private textual deny registry.
///
/// The registry is textual policy only. Production construction derives the
/// current account home from the OS account database, validates it as a live
/// no-follow directory, and checks that its owner is the current account. It
/// never reads HOME, USERPROFILE, or similar mutable environment values.
#[derive(Clone, Debug)]
pub(crate) struct ProtectedRootRegistry {
    policy: PlatformPolicy,
}

impl ProtectedRootRegistry {
    /// Build policy from the current account's OS-owned home directory.
    ///
    /// This creates no cleanup authority. It only supplies a validated,
    /// text-derived profile path to the protected-root classifier. Setuid
    /// ambiguity, missing account records, non-absolute homes, symlinks,
    /// replacement races, and ownership mismatches all fail closed.
    pub(crate) fn from_current_account() -> Result<Self, ProtectedRootError> {
        let evidence = CurrentAccountHomeEvidence::capture()?;
        Self::from_home_directory_evidence(evidence.root())
    }

    fn from_home_directory_evidence(
        home_directory: &CanonicalScanRoot,
    ) -> Result<Self, ProtectedRootError> {
        let platform = PolicyPlatform::current();
        let requested_home = PolicyPath::from_host(home_directory.requested_path())
            .ok_or(ProtectedRootError::InvalidHomeDirectory)?;
        let canonical_home = PolicyPath::from_host(home_directory.canonical_path())
            .ok_or(ProtectedRootError::InvalidHomeDirectory)?;
        if !valid_home_path(platform, &requested_home)
            || !valid_home_path(platform, &canonical_home)
        {
            return Err(ProtectedRootError::InvalidHomeDirectory);
        }
        let home_paths = if requested_home == canonical_home {
            vec![requested_home]
        } else {
            vec![requested_home, canonical_home]
        };
        Ok(Self {
            policy: PlatformPolicy {
                platform,
                profile_containers: profile_parents(&home_paths),
                home_paths,
            },
        })
    }

    /// Test-only injection of already-captured evidence.
    #[cfg(test)]
    fn with_home_directory_evidence(
        home_directory: &CanonicalScanRoot,
    ) -> Result<Self, ProtectedRootError> {
        Self::from_home_directory_evidence(home_directory)
    }

    #[cfg(test)]
    pub(crate) fn for_exact_review_fixture() -> Self {
        let home_paths = vec![PolicyPath::unix(&["fixture", "home"])]
            .into_iter()
            .collect::<Vec<_>>();
        Self {
            policy: PlatformPolicy {
                platform: PolicyPlatform::Linux,
                profile_containers: profile_parents(&home_paths),
                home_paths,
            },
        }
    }

    /// Runs the requested spelling through the registry before filesystem
    /// probing. A later canonical assessment is still mandatory.
    pub(crate) fn preflight(
        &self,
        target: &LexicalCleanupPath,
    ) -> Result<ProtectedRootDisposition, ProtectedRootError> {
        let scan_root =
            PolicyPath::from_host(target.scan_root()).ok_or(ProtectedRootError::InvalidPath {
                form: ProtectedPathForm::ScanRootRequested,
            })?;
        let target =
            PolicyPath::from_host(target.as_path()).ok_or(ProtectedRootError::InvalidPath {
                form: ProtectedPathForm::TargetRequested,
            })?;
        Ok(self.policy.assess(&[
            (
                ProtectedPathForm::ScanRootRequested,
                MatchContext::ScanScope,
                &scan_root,
            ),
            (
                ProtectedPathForm::TargetRequested,
                MatchContext::Target,
                &target,
            ),
        ]))
    }

    /// Rechecks requested and canonical scan/target locations after live
    /// identity capture. The returned disposition remains non-authoritative.
    pub(crate) fn assess(
        &self,
        scan_root: &CanonicalScanRoot,
        target: &CanonicalPathSnapshot,
    ) -> Result<ProtectedRootDisposition, ProtectedRootError> {
        if target.scan_root() != scan_root.canonical_path()
            || target
                .ancestors()
                .first()
                .map(|ancestor| ancestor.identity())
                != Some(scan_root.identity())
        {
            return Err(ProtectedRootError::MismatchedScanRootEvidence);
        }
        let scan_requested = parse(
            scan_root.requested_path(),
            ProtectedPathForm::ScanRootRequested,
        )?;
        let scan_canonical = parse(
            scan_root.canonical_path(),
            ProtectedPathForm::ScanRootCanonical,
        )?;
        let target_requested = parse(target.requested_path(), ProtectedPathForm::TargetRequested)?;
        let target_canonical = parse(target.canonical_path(), ProtectedPathForm::TargetCanonical)?;

        Ok(self.policy.assess(&[
            (
                ProtectedPathForm::ScanRootRequested,
                MatchContext::ScanScope,
                &scan_requested,
            ),
            (
                ProtectedPathForm::ScanRootCanonical,
                MatchContext::ScanScope,
                &scan_canonical,
            ),
            (
                ProtectedPathForm::TargetRequested,
                MatchContext::Target,
                &target_requested,
            ),
            (
                ProtectedPathForm::TargetCanonical,
                MatchContext::Target,
                &target_canonical,
            ),
        ]))
    }
}

#[cfg(unix)]
fn validate_account_home_path(path: &Path) -> Result<(), ProtectedRootError> {
    if !path.is_absolute() || path.as_os_str().is_empty() || path.to_str().is_none() {
        return Err(ProtectedRootError::InvalidHomeDirectory);
    }
    Ok(())
}

fn parse(path: &Path, form: ProtectedPathForm) -> Result<PolicyPath, ProtectedRootError> {
    PolicyPath::from_host(path).ok_or(ProtectedRootError::InvalidPath { form })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PolicyPlatform {
    MacOs,
    Linux,
    Windows,
    Unsupported,
}

impl PolicyPlatform {
    const fn current() -> Self {
        #[cfg(target_os = "macos")]
        {
            return Self::MacOs;
        }
        #[cfg(target_os = "linux")]
        {
            return Self::Linux;
        }
        #[cfg(windows)]
        {
            return Self::Windows;
        }
        #[allow(unreachable_code)]
        Self::Unsupported
    }

    fn components_equal(self, left: &str, right: &str) -> bool {
        match self {
            Self::Linux | Self::Unsupported => left == right,
            Self::MacOs | Self::Windows if left.is_ascii() && right.is_ascii() => {
                left.eq_ignore_ascii_case(right)
            }
            Self::MacOs | Self::Windows => left == right,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum PolicyRoot {
    Unix,
    Drive(char),
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PolicyPath {
    root: PolicyRoot,
    components: Vec<String>,
}

impl PolicyPath {
    #[cfg(unix)]
    fn from_host(path: &Path) -> Option<Self> {
        let mut components = path.components();
        if !matches!(components.next(), Some(Component::RootDir)) {
            return None;
        }
        Some(Self {
            root: PolicyRoot::Unix,
            components: components
                .map(|component| match component {
                    Component::Normal(component) => component.to_str().map(ToOwned::to_owned),
                    _ => None,
                })
                .collect::<Option<Vec<_>>>()?,
        })
    }

    #[cfg(windows)]
    fn from_host(path: &Path) -> Option<Self> {
        use std::path::Prefix;

        let mut components = path.components();
        let Component::Prefix(prefix) = components.next()? else {
            return None;
        };
        let drive = match prefix.kind() {
            Prefix::Disk(drive) | Prefix::VerbatimDisk(drive) => char::from(drive),
            _ => return None,
        };
        if !matches!(components.next(), Some(Component::RootDir)) {
            return None;
        }
        Some(Self {
            root: PolicyRoot::Drive(drive),
            components: components
                .map(|component| match component {
                    Component::Normal(component) => component.to_str().map(ToOwned::to_owned),
                    _ => None,
                })
                .collect::<Option<Vec<_>>>()?,
        })
    }

    #[cfg(not(any(unix, windows)))]
    fn from_host(_path: &Path) -> Option<Self> {
        None
    }

    fn unix(components: &[&str]) -> Self {
        Self {
            root: PolicyRoot::Unix,
            components: components.iter().map(|value| (*value).to_owned()).collect(),
        }
    }

    fn drive(drive: char, components: &[&str]) -> Self {
        Self {
            root: PolicyRoot::Drive(drive),
            components: components.iter().map(|value| (*value).to_owned()).collect(),
        }
    }

    fn starts_with(&self, platform: PolicyPlatform, prefix: &Self) -> bool {
        roots_equal(platform, &self.root, &prefix.root)
            && prefix.components.len() <= self.components.len()
            && self
                .components
                .iter()
                .zip(&prefix.components)
                .all(|(left, right)| platform.components_equal(left, right))
    }

    fn is_exact(&self, platform: PolicyPlatform, other: &Self) -> bool {
        self.components.len() == other.components.len() && self.starts_with(platform, other)
    }

    fn parent(&self) -> Option<Self> {
        if self.components.is_empty() {
            return None;
        }
        let mut parent = self.clone();
        parent.components.pop();
        Some(parent)
    }
}

fn roots_equal(platform: PolicyPlatform, left: &PolicyRoot, right: &PolicyRoot) -> bool {
    match (left, right) {
        (PolicyRoot::Unix, PolicyRoot::Unix) => true,
        (PolicyRoot::Drive(left), PolicyRoot::Drive(right))
            if platform == PolicyPlatform::Windows =>
        {
            left.eq_ignore_ascii_case(right)
        }
        _ => false,
    }
}

#[derive(Clone, Copy)]
enum MatchContext {
    ScanScope,
    Target,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ProtectionLevel {
    SpecificRuleRequired,
    Denied,
}

impl ProtectionLevel {
    fn priority(self) -> u8 {
        match self {
            Self::SpecificRuleRequired => 1,
            Self::Denied => 2,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ProtectionMatch {
    level: ProtectionLevel,
    kind: ProtectedPathKind,
}

#[derive(Clone, Debug)]
struct PlatformPolicy {
    platform: PolicyPlatform,
    home_paths: Vec<PolicyPath>,
    profile_containers: Vec<PolicyPath>,
}

impl PlatformPolicy {
    fn assess(
        &self,
        paths: &[(ProtectedPathForm, MatchContext, &PolicyPath)],
    ) -> ProtectedRootDisposition {
        let mut selected: Option<(ProtectedPathForm, ProtectionMatch)> = None;
        for (form, context, path) in paths {
            let matched = match context {
                MatchContext::ScanScope => self.classify_scan_scope(path),
                MatchContext::Target => self.classify_target(path),
            };
            if let Some(matched) = matched {
                selected = select(selected, (*form, matched));
            }
        }
        match selected {
            Some((
                form,
                ProtectionMatch {
                    level: ProtectionLevel::Denied,
                    kind,
                },
            )) => ProtectedRootDisposition::Denied {
                form,
                kind,
                policy_revision: PROTECTED_ROOT_POLICY_REVISION,
            },
            Some((
                form,
                ProtectionMatch {
                    level: ProtectionLevel::SpecificRuleRequired,
                    kind,
                },
            )) => ProtectedRootDisposition::SpecificRuleRequired {
                form,
                kind,
                policy_revision: PROTECTED_ROOT_POLICY_REVISION,
            },
            None => ProtectedRootDisposition::NoTextualMatch {
                policy_revision: PROTECTED_ROOT_POLICY_REVISION,
            },
        }
    }

    fn classify_target(&self, path: &PolicyPath) -> Option<ProtectionMatch> {
        let mut best = classify_static(self.platform, MatchContext::Target, path);
        best = prefer(
            best,
            self.classify_user_location(MatchContext::Target, path),
        );
        best
    }

    fn classify_scan_scope(&self, path: &PolicyPath) -> Option<ProtectionMatch> {
        let mut best = classify_static(self.platform, MatchContext::ScanScope, path);
        best = prefer(
            best,
            self.classify_user_location(MatchContext::ScanScope, path),
        );
        best
    }

    fn classify_user_location(
        &self,
        context: MatchContext,
        path: &PolicyPath,
    ) -> Option<ProtectionMatch> {
        let mut containers = self.profile_containers.clone();
        if let Some(container) = conventional_profile_container(self.platform)
            && let Some(container) = policy_root(self.platform, path, &[container])
            && !containers
                .iter()
                .any(|existing| existing.is_exact(self.platform, &container))
        {
            containers.push(container);
        }
        for container in containers {
            if matches!(context, MatchContext::Target) && path.is_exact(self.platform, &container) {
                return Some(ProtectionMatch {
                    level: ProtectionLevel::Denied,
                    kind: ProtectedPathKind::UserHomesContainer,
                });
            }
            if path.starts_with(self.platform, &container)
                && path.components.len() > container.components.len()
                && !self
                    .home_paths
                    .iter()
                    .any(|home| path.starts_with(self.platform, home))
            {
                return Some(ProtectionMatch {
                    level: ProtectionLevel::Denied,
                    kind: ProtectedPathKind::UserHome,
                });
            }
        }

        let mut best = None;
        for home in &self.home_paths {
            if matches!(context, MatchContext::Target) && path.is_exact(self.platform, home) {
                best = prefer_match(
                    best,
                    ProtectionMatch {
                        level: ProtectionLevel::Denied,
                        kind: ProtectedPathKind::UserHome,
                    },
                );
            }
            if path.starts_with(self.platform, home) {
                let relative = &path.components[home.components.len()..];
                let guarded_child = match self.platform {
                    PolicyPlatform::MacOs => Some("Library"),
                    PolicyPlatform::Windows => Some("AppData"),
                    PolicyPlatform::Linux | PolicyPlatform::Unsupported => None,
                };
                if guarded_child.is_some_and(|child| {
                    relative
                        .first()
                        .is_some_and(|value| self.platform.components_equal(value, child))
                }) {
                    let level = if matches!(context, MatchContext::Target) && relative.len() == 1 {
                        ProtectionLevel::Denied
                    } else {
                        ProtectionLevel::SpecificRuleRequired
                    };
                    best = prefer_match(
                        best,
                        ProtectionMatch {
                            level,
                            kind: ProtectedPathKind::UserLibrary,
                        },
                    );
                }
            }
        }
        best
    }
}

fn select(
    current: Option<(ProtectedPathForm, ProtectionMatch)>,
    candidate: (ProtectedPathForm, ProtectionMatch),
) -> Option<(ProtectedPathForm, ProtectionMatch)> {
    match current {
        Some(current) if current.1.level.priority() >= candidate.1.level.priority() => {
            Some(current)
        }
        _ => Some(candidate),
    }
}

fn prefer(
    current: Option<ProtectionMatch>,
    candidate: Option<ProtectionMatch>,
) -> Option<ProtectionMatch> {
    match candidate {
        Some(candidate) => prefer_match(current, candidate),
        None => current,
    }
}

fn prefer_match(
    current: Option<ProtectionMatch>,
    candidate: ProtectionMatch,
) -> Option<ProtectionMatch> {
    match current {
        Some(current) if current.level.priority() >= candidate.level.priority() => Some(current),
        _ => Some(candidate),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ProtectedCoverage {
    Exact,
    HardTree,
    GuardedTree,
}

#[derive(Clone, Copy)]
struct StaticRule {
    components: &'static [&'static str],
    coverage: ProtectedCoverage,
    kind: ProtectedPathKind,
}

impl StaticRule {
    const fn new(
        components: &'static [&'static str],
        coverage: ProtectedCoverage,
        kind: ProtectedPathKind,
    ) -> Self {
        Self {
            components,
            coverage,
            kind,
        }
    }

    fn classify(
        &self,
        platform: PolicyPlatform,
        context: MatchContext,
        path: &PolicyPath,
    ) -> Option<ProtectionMatch> {
        let root = policy_root(platform, path, self.components)?;
        let exact = path.is_exact(platform, &root);
        let descendant = path.starts_with(platform, &root) && !exact;
        let level = match (context, self.coverage, exact, descendant) {
            (MatchContext::Target, ProtectedCoverage::Exact, true, _)
            | (MatchContext::Target, ProtectedCoverage::HardTree, true, _)
            | (MatchContext::Target, ProtectedCoverage::HardTree, _, true)
            | (MatchContext::Target, ProtectedCoverage::GuardedTree, true, _)
            | (MatchContext::ScanScope, ProtectedCoverage::HardTree, true, _)
            | (MatchContext::ScanScope, ProtectedCoverage::HardTree, _, true) => {
                Some(ProtectionLevel::Denied)
            }
            (MatchContext::Target, ProtectedCoverage::GuardedTree, _, true)
            | (MatchContext::ScanScope, ProtectedCoverage::GuardedTree, true, _)
            | (MatchContext::ScanScope, ProtectedCoverage::GuardedTree, _, true) => {
                Some(ProtectionLevel::SpecificRuleRequired)
            }
            _ => None,
        }?;
        Some(ProtectionMatch {
            level,
            kind: self.kind,
        })
    }
}

fn policy_root(
    platform: PolicyPlatform,
    path: &PolicyPath,
    components: &[&str],
) -> Option<PolicyPath> {
    match platform {
        PolicyPlatform::Windows => match path.root {
            PolicyRoot::Drive(drive) => Some(PolicyPath::drive(drive, components)),
            PolicyRoot::Unix => None,
        },
        PolicyPlatform::MacOs | PolicyPlatform::Linux | PolicyPlatform::Unsupported => {
            Some(PolicyPath::unix(components))
        }
    }
}

fn conventional_profile_container(platform: PolicyPlatform) -> Option<&'static str> {
    match platform {
        PolicyPlatform::MacOs | PolicyPlatform::Windows => Some("Users"),
        PolicyPlatform::Linux => Some("home"),
        PolicyPlatform::Unsupported => None,
    }
}

fn valid_home_path(platform: PolicyPlatform, path: &PolicyPath) -> bool {
    !path.components.is_empty() && classify_static(platform, MatchContext::Target, path).is_none()
}

fn profile_parents(home_paths: &[PolicyPath]) -> Vec<PolicyPath> {
    let mut parents = Vec::new();
    for home in home_paths {
        if let Some(parent) = home.parent()
            && !parents.contains(&parent)
        {
            parents.push(parent);
        }
    }
    parents
}

fn classify_static(
    platform: PolicyPlatform,
    context: MatchContext,
    path: &PolicyPath,
) -> Option<ProtectionMatch> {
    if matches!(context, MatchContext::Target) && path.components.is_empty() {
        return Some(ProtectionMatch {
            level: ProtectionLevel::Denied,
            kind: ProtectedPathKind::FilesystemRoot,
        });
    }
    if platform == PolicyPlatform::Unsupported {
        return Some(ProtectionMatch {
            level: ProtectionLevel::Denied,
            kind: ProtectedPathKind::FilesystemRoot,
        });
    }
    rules(platform).iter().fold(None, |best, rule| {
        prefer(best, rule.classify(platform, context, path))
    })
}

fn rules(platform: PolicyPlatform) -> &'static [StaticRule] {
    const MACOS: &[StaticRule] = &[
        StaticRule::new(
            &["System"],
            ProtectedCoverage::HardTree,
            ProtectedPathKind::OperatingSystem,
        ),
        StaticRule::new(
            &["bin"],
            ProtectedCoverage::HardTree,
            ProtectedPathKind::OperatingSystem,
        ),
        StaticRule::new(
            &["sbin"],
            ProtectedCoverage::HardTree,
            ProtectedPathKind::OperatingSystem,
        ),
        StaticRule::new(
            &["usr"],
            ProtectedCoverage::HardTree,
            ProtectedPathKind::OperatingSystem,
        ),
        StaticRule::new(
            &["etc"],
            ProtectedCoverage::HardTree,
            ProtectedPathKind::OperatingSystem,
        ),
        StaticRule::new(
            &["private"],
            ProtectedCoverage::HardTree,
            ProtectedPathKind::OperatingSystem,
        ),
        StaticRule::new(
            &["dev"],
            ProtectedCoverage::HardTree,
            ProtectedPathKind::OperatingSystem,
        ),
        StaticRule::new(
            &["cores"],
            ProtectedCoverage::HardTree,
            ProtectedPathKind::OperatingSystem,
        ),
        StaticRule::new(
            &["Network"],
            ProtectedCoverage::HardTree,
            ProtectedPathKind::OperatingSystem,
        ),
        StaticRule::new(
            &["net"],
            ProtectedCoverage::HardTree,
            ProtectedPathKind::OperatingSystem,
        ),
        StaticRule::new(
            &["Applications"],
            ProtectedCoverage::GuardedTree,
            ProtectedPathKind::ApplicationInstallations,
        ),
        StaticRule::new(
            &["Library"],
            ProtectedCoverage::GuardedTree,
            ProtectedPathKind::SharedSystemLibrary,
        ),
        StaticRule::new(
            &["opt"],
            ProtectedCoverage::GuardedTree,
            ProtectedPathKind::ManagedSoftware,
        ),
        StaticRule::new(
            &["Users"],
            ProtectedCoverage::Exact,
            ProtectedPathKind::UserHomesContainer,
        ),
        StaticRule::new(
            &["Volumes"],
            ProtectedCoverage::Exact,
            ProtectedPathKind::VolumeMountContainer,
        ),
        StaticRule::new(
            &["var"],
            ProtectedCoverage::Exact,
            ProtectedPathKind::OperatingSystem,
        ),
        StaticRule::new(
            &["tmp"],
            ProtectedCoverage::Exact,
            ProtectedPathKind::OperatingSystem,
        ),
        StaticRule::new(
            &["home"],
            ProtectedCoverage::Exact,
            ProtectedPathKind::UserHomesContainer,
        ),
    ];
    const LINUX: &[StaticRule] = &[
        StaticRule::new(
            &["boot"],
            ProtectedCoverage::HardTree,
            ProtectedPathKind::OperatingSystem,
        ),
        StaticRule::new(
            &["dev"],
            ProtectedCoverage::HardTree,
            ProtectedPathKind::OperatingSystem,
        ),
        StaticRule::new(
            &["etc"],
            ProtectedCoverage::HardTree,
            ProtectedPathKind::OperatingSystem,
        ),
        StaticRule::new(
            &["proc"],
            ProtectedCoverage::HardTree,
            ProtectedPathKind::OperatingSystem,
        ),
        StaticRule::new(
            &["root"],
            ProtectedCoverage::HardTree,
            ProtectedPathKind::UserHome,
        ),
        StaticRule::new(
            &["run"],
            ProtectedCoverage::HardTree,
            ProtectedPathKind::OperatingSystem,
        ),
        StaticRule::new(
            &["sys"],
            ProtectedCoverage::HardTree,
            ProtectedPathKind::OperatingSystem,
        ),
        StaticRule::new(
            &["usr"],
            ProtectedCoverage::HardTree,
            ProtectedPathKind::OperatingSystem,
        ),
        StaticRule::new(
            &["bin"],
            ProtectedCoverage::HardTree,
            ProtectedPathKind::OperatingSystem,
        ),
        StaticRule::new(
            &["sbin"],
            ProtectedCoverage::HardTree,
            ProtectedPathKind::OperatingSystem,
        ),
        StaticRule::new(
            &["lib"],
            ProtectedCoverage::HardTree,
            ProtectedPathKind::OperatingSystem,
        ),
        StaticRule::new(
            &["lib64"],
            ProtectedCoverage::HardTree,
            ProtectedPathKind::OperatingSystem,
        ),
        StaticRule::new(
            &["lib32"],
            ProtectedCoverage::HardTree,
            ProtectedPathKind::OperatingSystem,
        ),
        StaticRule::new(
            &["libx32"],
            ProtectedCoverage::HardTree,
            ProtectedPathKind::OperatingSystem,
        ),
        StaticRule::new(
            &["opt"],
            ProtectedCoverage::HardTree,
            ProtectedPathKind::ManagedSoftware,
        ),
        StaticRule::new(
            &["srv"],
            ProtectedCoverage::HardTree,
            ProtectedPathKind::ServiceData,
        ),
        StaticRule::new(
            &["nix"],
            ProtectedCoverage::HardTree,
            ProtectedPathKind::PackageStore,
        ),
        StaticRule::new(
            &["snap"],
            ProtectedCoverage::HardTree,
            ProtectedPathKind::PackageStore,
        ),
        StaticRule::new(
            &["var"],
            ProtectedCoverage::HardTree,
            ProtectedPathKind::OperatingSystem,
        ),
        StaticRule::new(
            &["lost+found"],
            ProtectedCoverage::HardTree,
            ProtectedPathKind::OperatingSystem,
        ),
        StaticRule::new(
            &["home"],
            ProtectedCoverage::Exact,
            ProtectedPathKind::UserHomesContainer,
        ),
        StaticRule::new(
            &["mnt"],
            ProtectedCoverage::Exact,
            ProtectedPathKind::VolumeMountContainer,
        ),
        StaticRule::new(
            &["media"],
            ProtectedCoverage::Exact,
            ProtectedPathKind::VolumeMountContainer,
        ),
        StaticRule::new(
            &["tmp"],
            ProtectedCoverage::Exact,
            ProtectedPathKind::OperatingSystem,
        ),
    ];
    const WINDOWS: &[StaticRule] = &[
        StaticRule::new(
            &["Windows"],
            ProtectedCoverage::HardTree,
            ProtectedPathKind::OperatingSystem,
        ),
        StaticRule::new(
            &["$Recycle.Bin"],
            ProtectedCoverage::HardTree,
            ProtectedPathKind::OperatingSystem,
        ),
        StaticRule::new(
            &["System Volume Information"],
            ProtectedCoverage::HardTree,
            ProtectedPathKind::OperatingSystem,
        ),
        StaticRule::new(
            &["Recovery"],
            ProtectedCoverage::HardTree,
            ProtectedPathKind::OperatingSystem,
        ),
        StaticRule::new(
            &["Boot"],
            ProtectedCoverage::HardTree,
            ProtectedPathKind::OperatingSystem,
        ),
        StaticRule::new(
            &["PerfLogs"],
            ProtectedCoverage::HardTree,
            ProtectedPathKind::OperatingSystem,
        ),
        StaticRule::new(
            &["Program Files"],
            ProtectedCoverage::GuardedTree,
            ProtectedPathKind::ProgramFiles,
        ),
        StaticRule::new(
            &["Program Files (x86)"],
            ProtectedCoverage::GuardedTree,
            ProtectedPathKind::ProgramFiles,
        ),
        StaticRule::new(
            &["ProgramData"],
            ProtectedCoverage::GuardedTree,
            ProtectedPathKind::ProgramData,
        ),
        StaticRule::new(
            &["Users"],
            ProtectedCoverage::Exact,
            ProtectedPathKind::UserHomesContainer,
        ),
    ];
    match platform {
        PolicyPlatform::MacOs => MACOS,
        PolicyPlatform::Linux => LINUX,
        PolicyPlatform::Windows => WINDOWS,
        PolicyPlatform::Unsupported => &[],
    }
}

#[cfg(fuzzing)]
pub(super) fn assert_fuzz_invariants(input: &[u8]) {
    let (tag, payload) = input.split_first().unwrap_or((&b'A', &[]));
    match *tag {
        b'S' => fuzz_static_policy(payload),
        b'G' => fuzz_dynamic_policy(payload),
        b'P' => fuzz_precedence(payload),
        b'A' => fuzz_arbitrary_policy(payload),
        value => match value % 4 {
            0 => fuzz_static_policy(input),
            1 => fuzz_dynamic_policy(input),
            2 => fuzz_precedence(input),
            _ => fuzz_arbitrary_policy(input),
        },
    }
}

#[cfg(fuzzing)]
#[derive(Clone, Copy)]
enum FuzzCoverage {
    Exact,
    HardTree,
    GuardedTree,
}

#[cfg(fuzzing)]
fn fuzz_static_policy(input: &[u8]) {
    const CASES: &[(PolicyPlatform, &[&str], FuzzCoverage)] = &[
        (PolicyPlatform::MacOs, &["System"], FuzzCoverage::HardTree),
        (
            PolicyPlatform::MacOs,
            &["Applications"],
            FuzzCoverage::GuardedTree,
        ),
        (PolicyPlatform::MacOs, &["Users"], FuzzCoverage::Exact),
        (PolicyPlatform::Linux, &["etc"], FuzzCoverage::HardTree),
        (PolicyPlatform::Linux, &["opt"], FuzzCoverage::HardTree),
        (PolicyPlatform::Linux, &["home"], FuzzCoverage::Exact),
        (
            PolicyPlatform::Windows,
            &["Windows"],
            FuzzCoverage::HardTree,
        ),
        (
            PolicyPlatform::Windows,
            &["Program Files"],
            FuzzCoverage::GuardedTree,
        ),
        (PolicyPlatform::Windows, &["Users"], FuzzCoverage::Exact),
    ];
    let (platform, components, coverage) =
        CASES[usize::from(input.first().copied().unwrap_or_default()) % CASES.len()];
    let exact = match platform {
        PolicyPlatform::Windows => PolicyPath::drive('C', components),
        _ => PolicyPath::unix(components),
    };
    let mut descendant = exact.clone();
    descendant.components.push("fuzz-child".to_owned());
    let mut sibling = exact.clone();
    sibling.components[0].push_str("-lookalike");
    let expected = match coverage {
        FuzzCoverage::Exact => (Some(ProtectionLevel::Denied), None, None, None),
        FuzzCoverage::HardTree => (
            Some(ProtectionLevel::Denied),
            Some(ProtectionLevel::Denied),
            Some(ProtectionLevel::Denied),
            Some(ProtectionLevel::Denied),
        ),
        FuzzCoverage::GuardedTree => (
            Some(ProtectionLevel::Denied),
            Some(ProtectionLevel::SpecificRuleRequired),
            Some(ProtectionLevel::SpecificRuleRequired),
            Some(ProtectionLevel::SpecificRuleRequired),
        ),
    };
    assert_eq!(
        classify_static(platform, MatchContext::Target, &exact).map(|value| value.level),
        expected.0
    );
    assert_eq!(
        classify_static(platform, MatchContext::Target, &descendant).map(|value| value.level),
        expected.1
    );
    assert_eq!(
        classify_static(platform, MatchContext::ScanScope, &exact).map(|value| value.level),
        expected.2
    );
    assert_eq!(
        classify_static(platform, MatchContext::ScanScope, &descendant).map(|value| value.level),
        expected.3
    );
    assert_eq!(
        classify_static(platform, MatchContext::Target, &sibling),
        None
    );
}

#[cfg(fuzzing)]
fn fuzz_dynamic_policy(input: &[u8]) {
    let platform = match input.first().copied().unwrap_or_default() % 3 {
        0 => PolicyPlatform::MacOs,
        1 => PolicyPlatform::Linux,
        _ => PolicyPlatform::Windows,
    };
    let suffix = input
        .iter()
        .skip(1)
        .take(16)
        .map(|byte| char::from(b'a' + (byte % 26)))
        .collect::<String>();
    let current = format!("current-{suffix}");
    let foreign = format!("foreign-{suffix}");
    let (home, foreign_home, guarded_root, guarded_child) = match platform {
        PolicyPlatform::MacOs => (
            PolicyPath::unix(&["Users", &current]),
            PolicyPath::unix(&["Users", &foreign, "Downloads"]),
            PolicyPath::unix(&["Users", &current, "Library"]),
            PolicyPath::unix(&["Users", &current, "Library", "Caches"]),
        ),
        PolicyPlatform::Linux => (
            PolicyPath::unix(&["home", &current]),
            PolicyPath::unix(&["home", &foreign, ".cache"]),
            PolicyPath::unix(&["home", &current, ".cache"]),
            PolicyPath::unix(&["home", &current, ".cache", "tool"]),
        ),
        PolicyPlatform::Windows => (
            PolicyPath::drive('C', &["Users", &current]),
            PolicyPath::drive('C', &["Users", &foreign, "Downloads"]),
            PolicyPath::drive('C', &["Users", &current, "AppData"]),
            PolicyPath::drive('C', &["Users", &current, "AppData", "Local"]),
        ),
        PolicyPlatform::Unsupported => unreachable!(),
    };
    let home_paths = vec![home.clone()];
    let policy = PlatformPolicy {
        platform,
        profile_containers: profile_parents(&home_paths),
        home_paths,
    };
    assert_eq!(
        policy.classify_target(&home).map(|value| value.level),
        Some(ProtectionLevel::Denied)
    );
    assert_eq!(
        policy
            .classify_target(&foreign_home)
            .map(|value| value.level),
        Some(ProtectionLevel::Denied)
    );
    if platform == PolicyPlatform::Linux {
        assert_eq!(policy.classify_target(&guarded_root), None);
        assert_eq!(policy.classify_target(&guarded_child), None);
    } else {
        assert_eq!(
            policy
                .classify_target(&guarded_root)
                .map(|value| value.level),
            Some(ProtectionLevel::Denied)
        );
        assert_eq!(
            policy
                .classify_target(&guarded_child)
                .map(|value| value.level),
            Some(ProtectionLevel::SpecificRuleRequired)
        );
    }
}

#[cfg(fuzzing)]
fn fuzz_precedence(input: &[u8]) {
    let policy = PlatformPolicy {
        platform: PolicyPlatform::MacOs,
        home_paths: vec![PolicyPath::unix(&["Users", "fuzz-user"])],
        profile_containers: vec![PolicyPath::unix(&["Users"])],
    };
    let guarded = PolicyPath::unix(&["Applications", "Fuzz.app"]);
    let denied = PolicyPath::unix(&["System", "Library"]);
    let forms = [
        ProtectedPathForm::ScanRootRequested,
        ProtectedPathForm::ScanRootCanonical,
        ProtectedPathForm::TargetRequested,
        ProtectedPathForm::TargetCanonical,
    ];
    let guarded_form = forms[usize::from(input.first().copied().unwrap_or_default()) % forms.len()];
    let denied_form = forms[usize::from(input.get(1).copied().unwrap_or_default()) % forms.len()];
    let context = |form| match form {
        ProtectedPathForm::ScanRootRequested | ProtectedPathForm::ScanRootCanonical => {
            MatchContext::ScanScope
        }
        ProtectedPathForm::TargetRequested | ProtectedPathForm::TargetCanonical => {
            MatchContext::Target
        }
    };
    assert!(matches!(
        policy.assess(&[(guarded_form, context(guarded_form), &guarded)]),
        ProtectedRootDisposition::SpecificRuleRequired { .. }
    ));
    for paths in [
        vec![
            (guarded_form, context(guarded_form), &guarded),
            (denied_form, context(denied_form), &denied),
        ],
        vec![
            (denied_form, context(denied_form), &denied),
            (guarded_form, context(guarded_form), &guarded),
        ],
    ] {
        assert!(matches!(
            policy.assess(&paths),
            ProtectedRootDisposition::Denied { .. }
        ));
    }
}

#[cfg(fuzzing)]
fn fuzz_arbitrary_policy(input: &[u8]) {
    let platform = match input.first().copied().unwrap_or_default() % 3 {
        0 => PolicyPlatform::MacOs,
        1 => PolicyPlatform::Linux,
        _ => PolicyPlatform::Windows,
    };
    let home = match platform {
        PolicyPlatform::Windows => PolicyPath::drive('C', &["Users", "fuzz-user"]),
        PolicyPlatform::MacOs => PolicyPath::unix(&["Users", "fuzz-user"]),
        _ => PolicyPath::unix(&["home", "fuzz-user"]),
    };
    let home_paths = vec![home];
    let policy = PlatformPolicy {
        platform,
        profile_containers: profile_parents(&home_paths),
        home_paths,
    };
    let Ok(arbitrary) = std::str::from_utf8(input) else {
        return;
    };
    let components = arbitrary
        .split(['/', '\\'])
        .filter(|component| !component.is_empty())
        .take(32)
        .map(|component| component.chars().take(64).collect::<String>())
        .collect::<Vec<_>>();
    if components.is_empty() {
        return;
    }
    let refs = components.iter().map(String::as_str).collect::<Vec<_>>();
    let path = match platform {
        PolicyPlatform::Windows => PolicyPath::drive('D', &refs),
        _ => PolicyPath::unix(&refs),
    };
    let first = policy.assess(&[(
        ProtectedPathForm::TargetRequested,
        MatchContext::Target,
        &path,
    )]);
    let second = policy.assess(&[(
        ProtectedPathForm::TargetRequested,
        MatchContext::Target,
        &path,
    )]);
    assert_eq!(first, second);
    assert_eq!(first.policy_revision(), PROTECTED_ROOT_POLICY_REVISION);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::path_validation::{
        capture_path_snapshot, capture_scan_root, validate_cleanup_path, validate_scan_root,
    };
    use tempfile::tempdir_in;

    fn policy(platform: PolicyPlatform, home: PolicyPath) -> PlatformPolicy {
        let home_paths = vec![home];
        PlatformPolicy {
            platform,
            profile_containers: profile_parents(&home_paths),
            home_paths,
        }
    }

    fn target(policy: &PlatformPolicy, path: &PolicyPath) -> Option<ProtectionMatch> {
        policy.classify_target(path)
    }

    fn denied(kind: ProtectedPathKind) -> Option<ProtectionMatch> {
        Some(ProtectionMatch {
            level: ProtectionLevel::Denied,
            kind,
        })
    }

    fn guarded(kind: ProtectedPathKind) -> Option<ProtectionMatch> {
        Some(ProtectionMatch {
            level: ProtectionLevel::SpecificRuleRequired,
            kind,
        })
    }

    #[test]
    fn policy_tables_are_versioned_unique_and_have_expected_sizes() {
        assert_eq!(PROTECTED_ROOT_POLICY_REVISION, 2);
        for (platform, expected) in [
            (PolicyPlatform::MacOs, 18),
            (PolicyPlatform::Linux, 24),
            (PolicyPlatform::Windows, 10),
        ] {
            let rules = rules(platform);
            assert_eq!(rules.len(), expected);
            for (index, rule) in rules.iter().enumerate() {
                assert!(!rule.components.is_empty());
                assert!(!rules[..index].iter().any(|prior| {
                    prior.components.len() == rule.components.len()
                        && prior
                            .components
                            .iter()
                            .zip(rule.components)
                            .all(|(left, right)| platform.components_equal(left, right))
                }));
            }
        }
    }

    #[test]
    fn every_static_rule_has_exhaustive_target_and_scan_coverage() {
        for platform in [
            PolicyPlatform::MacOs,
            PolicyPlatform::Linux,
            PolicyPlatform::Windows,
        ] {
            for rule in rules(platform) {
                let exact = match platform {
                    PolicyPlatform::Windows => PolicyPath::drive('Q', rule.components),
                    _ => PolicyPath::unix(rule.components),
                };
                let mut descendant = exact.clone();
                descendant.components.push("child".to_owned());
                let matched = |level| {
                    Some(ProtectionMatch {
                        level,
                        kind: rule.kind,
                    })
                };
                let expected = match rule.coverage {
                    ProtectedCoverage::Exact => {
                        (matched(ProtectionLevel::Denied), None, None, None)
                    }
                    ProtectedCoverage::HardTree => (
                        matched(ProtectionLevel::Denied),
                        matched(ProtectionLevel::Denied),
                        matched(ProtectionLevel::Denied),
                        matched(ProtectionLevel::Denied),
                    ),
                    ProtectedCoverage::GuardedTree => (
                        matched(ProtectionLevel::Denied),
                        matched(ProtectionLevel::SpecificRuleRequired),
                        matched(ProtectionLevel::SpecificRuleRequired),
                        matched(ProtectionLevel::SpecificRuleRequired),
                    ),
                };
                assert_eq!(
                    (
                        rule.classify(platform, MatchContext::Target, &exact),
                        rule.classify(platform, MatchContext::Target, &descendant),
                        rule.classify(platform, MatchContext::ScanScope, &exact),
                        rule.classify(platform, MatchContext::ScanScope, &descendant),
                    ),
                    expected,
                    "wrong coverage for {platform:?} {:?}",
                    rule.components
                );
            }
        }
    }

    #[test]
    fn policy_revision_two_has_a_checked_full_table_fingerprint() {
        let mut signature = String::new();
        for platform in [
            PolicyPlatform::MacOs,
            PolicyPlatform::Linux,
            PolicyPlatform::Windows,
        ] {
            for rule in rules(platform) {
                use std::fmt::Write;
                writeln!(
                    signature,
                    "{platform:?}|{}|{:?}|{:?}",
                    rule.components.join("/"),
                    rule.coverage,
                    rule.kind
                )
                .unwrap();
            }
        }
        let fingerprint = signature
            .bytes()
            .fold(0xcbf29ce484222325_u64, |hash, byte| {
                (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
            });
        const POLICY_REVISION_2_STATIC_TABLE_FINGERPRINT: u64 = 10_830_234_716_710_889_208;
        assert_eq!(fingerprint, POLICY_REVISION_2_STATIC_TABLE_FINGERPRINT);
    }

    #[test]
    fn obvious_non_home_boundaries_are_rejected_as_home_evidence() {
        for (platform, paths) in [
            (
                PolicyPlatform::MacOs,
                vec![
                    PolicyPath::unix(&[]),
                    PolicyPath::unix(&["Users"]),
                    PolicyPath::unix(&["System"]),
                    PolicyPath::unix(&["Applications"]),
                ],
            ),
            (
                PolicyPlatform::Linux,
                vec![
                    PolicyPath::unix(&[]),
                    PolicyPath::unix(&["home"]),
                    PolicyPath::unix(&["usr"]),
                    PolicyPath::unix(&["opt"]),
                ],
            ),
            (
                PolicyPlatform::Windows,
                vec![
                    PolicyPath::drive('C', &[]),
                    PolicyPath::drive('C', &["Users"]),
                    PolicyPath::drive('C', &["Windows"]),
                    PolicyPath::drive('C', &["Program Files"]),
                ],
            ),
        ] {
            for path in paths {
                assert!(!valid_home_path(platform, &path), "accepted {path:?}");
            }
        }
    }

    #[test]
    fn macos_hard_guarded_and_exact_roots_have_distinct_coverage() {
        let policy = policy(PolicyPlatform::MacOs, PolicyPath::unix(&["Users", "alice"]));
        assert_eq!(
            target(&policy, &PolicyPath::unix(&["System", "Library"])),
            denied(ProtectedPathKind::OperatingSystem)
        );
        assert_eq!(
            target(&policy, &PolicyPath::unix(&["Applications"])),
            denied(ProtectedPathKind::ApplicationInstallations)
        );
        assert_eq!(
            target(&policy, &PolicyPath::unix(&["Applications", "Tool.app"])),
            guarded(ProtectedPathKind::ApplicationInstallations)
        );
        assert_eq!(
            target(&policy, &PolicyPath::unix(&["Volumes"])),
            denied(ProtectedPathKind::VolumeMountContainer)
        );
        assert_eq!(
            target(
                &policy,
                &PolicyPath::unix(&["Volumes", "External", "cache"])
            ),
            None
        );
        assert_eq!(target(&policy, &PolicyPath::unix(&["var", "tmp"])), None);
        assert_eq!(
            target(&policy, &PolicyPath::unix(&["private", "var", "tmp"])),
            denied(ProtectedPathKind::OperatingSystem)
        );
    }

    #[test]
    fn macos_current_home_and_library_are_gated_while_foreign_homes_are_denied() {
        let policy = policy(PolicyPlatform::MacOs, PolicyPath::unix(&["Users", "alice"]));
        assert_eq!(
            target(&policy, &PolicyPath::unix(&["Users", "alice"])),
            denied(ProtectedPathKind::UserHome)
        );
        assert_eq!(
            target(&policy, &PolicyPath::unix(&["Users", "alice", "Library"])),
            denied(ProtectedPathKind::UserLibrary)
        );
        assert_eq!(
            target(
                &policy,
                &PolicyPath::unix(&["Users", "alice", "Library", "Caches"])
            ),
            guarded(ProtectedPathKind::UserLibrary)
        );
        assert_eq!(
            target(
                &policy,
                &PolicyPath::unix(&["Users", "alice", "Downloads", "file"])
            ),
            None
        );
        assert_eq!(
            target(&policy, &PolicyPath::unix(&["Users", "bob", "Downloads"])),
            denied(ProtectedPathKind::UserHome)
        );
    }

    #[test]
    fn linux_covers_system_package_service_and_mount_anchors() {
        let policy = policy(PolicyPlatform::Linux, PolicyPath::unix(&["home", "alice"]));
        for (components, kind) in [
            (&["usr", "bin"][..], ProtectedPathKind::OperatingSystem),
            (&["var", "cache"][..], ProtectedPathKind::OperatingSystem),
            (&["opt", "tool"][..], ProtectedPathKind::ManagedSoftware),
            (&["srv", "data"][..], ProtectedPathKind::ServiceData),
            (&["nix", "store"][..], ProtectedPathKind::PackageStore),
            (&["snap", "core"][..], ProtectedPathKind::PackageStore),
        ] {
            assert_eq!(target(&policy, &PolicyPath::unix(components)), denied(kind));
        }
        assert_eq!(
            target(&policy, &PolicyPath::unix(&["mnt"])),
            denied(ProtectedPathKind::VolumeMountContainer)
        );
        assert_eq!(
            target(&policy, &PolicyPath::unix(&["mnt", "disk", "cache"])),
            None
        );
        assert_eq!(
            target(&policy, &PolicyPath::unix(&["home", "bob", ".cache"])),
            denied(ProtectedPathKind::UserHome)
        );
        assert_eq!(
            target(&policy, &PolicyPath::unix(&["home", "alice", ".cache"])),
            None
        );
    }

    #[test]
    fn windows_rules_apply_on_every_drive_with_component_boundaries() {
        let policy = policy(
            PolicyPlatform::Windows,
            PolicyPath::drive('C', &["Users", "Alice"]),
        );
        assert_eq!(
            target(&policy, &PolicyPath::drive('D', &["WINDOWS", "System32"])),
            denied(ProtectedPathKind::OperatingSystem)
        );
        assert_eq!(
            target(&policy, &PolicyPath::drive('D', &["Program Files", "Tool"])),
            guarded(ProtectedPathKind::ProgramFiles)
        );
        assert_eq!(
            target(
                &policy,
                &PolicyPath::drive('C', &["Users", "Bob", "Downloads"])
            ),
            denied(ProtectedPathKind::UserHome)
        );
        assert_eq!(
            target(
                &policy,
                &PolicyPath::drive('C', &["Users", "Alice", "AppData"])
            ),
            denied(ProtectedPathKind::UserLibrary)
        );
        assert_eq!(
            target(
                &policy,
                &PolicyPath::drive('C', &["Users", "Alice", "AppData", "Local"])
            ),
            guarded(ProtectedPathKind::UserLibrary)
        );
        assert_eq!(
            target(&policy, &PolicyPath::drive('C', &["Windows.old", "file"])),
            None
        );
        assert_eq!(
            target(&policy, &PolicyPath::drive('C', &[])),
            denied(ProtectedPathKind::FilesystemRoot)
        );
    }

    #[test]
    fn relocated_profile_container_siblings_are_denied() {
        let windows = policy(
            PolicyPlatform::Windows,
            PolicyPath::drive('D', &["Profiles", "Ålice"]),
        );
        assert_eq!(
            target(
                &windows,
                &PolicyPath::drive('D', &["Profiles", "Bob", "Downloads"])
            ),
            denied(ProtectedPathKind::UserHome)
        );
        assert_eq!(
            target(
                &windows,
                &PolicyPath::drive('D', &["Profiles", "Ålice", "Downloads"])
            ),
            None
        );
        assert_eq!(
            target(
                &windows,
                &PolicyPath::drive('D', &["Profiles", "ålice", "Downloads"])
            ),
            denied(ProtectedPathKind::UserHome)
        );

        let macos = policy(PolicyPlatform::MacOs, PolicyPath::unix(&["home", "alice"]));
        assert_eq!(
            target(&macos, &PolicyPath::unix(&["home", "bob", "file"])),
            denied(ProtectedPathKind::UserHome)
        );
    }

    #[test]
    fn foreign_profile_deny_overrides_library_and_appdata_guards() {
        let macos = policy(PolicyPlatform::MacOs, PolicyPath::unix(&["Users", "alice"]));
        assert_eq!(
            target(
                &macos,
                &PolicyPath::unix(&["Users", "bob", "Library", "Caches"])
            ),
            denied(ProtectedPathKind::UserHome)
        );
        let windows = policy(
            PolicyPlatform::Windows,
            PolicyPath::drive('C', &["Users", "Alice"]),
        );
        assert_eq!(
            target(
                &windows,
                &PolicyPath::drive('C', &["Users", "Bob", "AppData", "Local"])
            ),
            denied(ProtectedPathKind::UserHome)
        );
    }

    #[test]
    fn unresolved_windows_legacy_alias_never_becomes_positive_authority() {
        let policy = policy(
            PolicyPlatform::Windows,
            PolicyPath::drive('C', &["Users", "Alice"]),
        );
        let alias = PolicyPath::drive('C', &["PROGRA~1", "Tool"]);
        assert_eq!(
            policy.assess(&[(
                ProtectedPathForm::TargetRequested,
                MatchContext::Target,
                &alias,
            )]),
            ProtectedRootDisposition::NoTextualMatch {
                policy_revision: PROTECTED_ROOT_POLICY_REVISION,
            }
        );
    }

    #[test]
    fn scan_scope_propagates_only_hard_and_guarded_tree_restrictions() {
        let macos = policy(PolicyPlatform::MacOs, PolicyPath::unix(&["Users", "alice"]));
        assert_eq!(
            macos.classify_scan_scope(&PolicyPath::unix(&["System", "Library"])),
            denied(ProtectedPathKind::OperatingSystem)
        );
        assert_eq!(
            macos.classify_scan_scope(&PolicyPath::unix(&["Applications"])),
            guarded(ProtectedPathKind::ApplicationInstallations)
        );
        assert_eq!(
            macos.classify_scan_scope(&PolicyPath::unix(&["Users"])),
            None
        );
        assert_eq!(
            macos.classify_scan_scope(&PolicyPath::unix(&["Users", "alice"])),
            None
        );
        assert_eq!(
            macos.classify_scan_scope(&PolicyPath::unix(&["Users", "bob"])),
            denied(ProtectedPathKind::UserHome)
        );
    }

    #[test]
    fn hard_deny_overrides_guard_across_requested_and_canonical_forms() {
        let policy = policy(PolicyPlatform::MacOs, PolicyPath::unix(&["Users", "alice"]));
        let requested = PolicyPath::unix(&["Users", "alice", "Library", "Caches"]);
        let canonical = PolicyPath::unix(&["System", "Volumes", "Data", "cache"]);
        assert_eq!(
            policy.assess(&[
                (
                    ProtectedPathForm::TargetRequested,
                    MatchContext::Target,
                    &requested
                ),
                (
                    ProtectedPathForm::TargetCanonical,
                    MatchContext::Target,
                    &canonical
                ),
            ]),
            ProtectedRootDisposition::Denied {
                form: ProtectedPathForm::TargetCanonical,
                kind: ProtectedPathKind::OperatingSystem,
                policy_revision: PROTECTED_ROOT_POLICY_REVISION,
            }
        );
    }

    #[test]
    fn each_scan_and_target_path_form_can_independently_deny() {
        let policy = policy(PolicyPlatform::MacOs, PolicyPath::unix(&["Users", "alice"]));
        let hard = PolicyPath::unix(&["System", "Library"]);
        for (form, context) in [
            (
                ProtectedPathForm::ScanRootRequested,
                MatchContext::ScanScope,
            ),
            (
                ProtectedPathForm::ScanRootCanonical,
                MatchContext::ScanScope,
            ),
            (ProtectedPathForm::TargetRequested, MatchContext::Target),
            (ProtectedPathForm::TargetCanonical, MatchContext::Target),
        ] {
            assert_eq!(
                policy.assess(&[(form, context, &hard)]),
                ProtectedRootDisposition::Denied {
                    form,
                    kind: ProtectedPathKind::OperatingSystem,
                    policy_revision: PROTECTED_ROOT_POLICY_REVISION,
                }
            );
        }
    }

    #[test]
    fn custom_home_is_exactly_protected_without_ambient_environment_reads() {
        let policy = policy(
            PolicyPlatform::Linux,
            PolicyPath::unix(&["custom", "alice"]),
        );
        assert_eq!(
            target(&policy, &PolicyPath::unix(&["custom", "alice"])),
            denied(ProtectedPathKind::UserHome)
        );
        assert_eq!(
            target(&policy, &PolicyPath::unix(&["custom", "alice", ".cache"])),
            None
        );
        assert_eq!(
            target(&policy, &PolicyPath::unix(&["home", "alice", ".cache"])),
            denied(ProtectedPathKind::UserHome)
        );
    }

    #[test]
    fn matching_is_component_aware_and_platform_case_policy_is_conservative() {
        let linux = policy(PolicyPlatform::Linux, PolicyPath::unix(&["home", "alice"]));
        let macos = policy(PolicyPlatform::MacOs, PolicyPath::unix(&["Users", "Ålice"]));
        assert_eq!(
            target(&linux, &PolicyPath::unix(&["usr-local", "cache"])),
            None
        );
        assert_eq!(target(&linux, &PolicyPath::unix(&["USR", "bin"])), None);
        assert_eq!(
            target(&macos, &PolicyPath::unix(&["system", "library"])),
            denied(ProtectedPathKind::OperatingSystem)
        );
        assert_eq!(
            target(&macos, &PolicyPath::unix(&["Users", "ålice", "Downloads"])),
            denied(ProtectedPathKind::UserHome)
        );
    }

    #[test]
    fn every_no_match_disposition_is_explicitly_non_authoritative_and_versioned() {
        let policy = policy(PolicyPlatform::Linux, PolicyPath::unix(&["home", "alice"]));
        let path = PolicyPath::unix(&["home", "alice", ".cache"]);
        let disposition = policy.assess(&[(
            ProtectedPathForm::TargetRequested,
            MatchContext::Target,
            &path,
        )]);
        assert_eq!(
            disposition,
            ProtectedRootDisposition::NoTextualMatch {
                policy_revision: PROTECTED_ROOT_POLICY_REVISION,
            }
        );
        assert_eq!(
            disposition.policy_revision(),
            PROTECTED_ROOT_POLICY_REVISION
        );
    }

    #[test]
    fn diagnostics_never_include_user_paths() {
        let error = ProtectedRootError::InvalidPath {
            form: ProtectedPathForm::TargetCanonical,
        };
        assert!(!error.to_string().contains("Users"));
        assert!(!std::mem::needs_drop::<ProtectedRootError>());
    }

    #[test]
    fn live_preflight_and_canonical_assessment_do_not_mutate_the_target() {
        let current = std::env::current_dir().unwrap();
        let temp = tempdir_in(current).unwrap();
        let home_path = std::fs::canonicalize(temp.path()).unwrap();
        let target_path = home_path.join("ordinary-cache");
        std::fs::write(&target_path, b"unchanged").unwrap();
        let lexical_home = validate_scan_root(&home_path).unwrap();
        let canonical_home = capture_scan_root(lexical_home.clone()).unwrap();
        let lexical_target = validate_cleanup_path(&lexical_home, &target_path).unwrap();
        let registry =
            ProtectedRootRegistry::with_home_directory_evidence(&canonical_home).unwrap();
        assert!(matches!(
            registry.preflight(&lexical_target).unwrap(),
            ProtectedRootDisposition::NoTextualMatch { .. }
        ));
        let snapshot = capture_path_snapshot(&canonical_home, lexical_target).unwrap();
        assert!(matches!(
            registry.assess(&canonical_home, &snapshot).unwrap(),
            ProtectedRootDisposition::NoTextualMatch { .. }
        ));
        assert_eq!(std::fs::read(&target_path).unwrap(), b"unchanged");
    }

    #[test]
    #[expect(
        clippy::disallowed_methods,
        reason = "test replaces a TempDir-owned scan root to exercise identity rejection"
    )]
    fn replacement_scan_root_at_the_same_path_rejects_old_target_evidence() {
        let current = std::env::current_dir().unwrap();
        let container = tempdir_in(current).unwrap();
        let root_path = container.path().join("scan-root");
        let moved_root_path = container.path().join("moved-scan-root");
        std::fs::create_dir(&root_path).unwrap();
        let target_path = root_path.join("ordinary-cache");
        std::fs::write(&target_path, b"old-root").unwrap();

        let old_lexical_root = validate_scan_root(&root_path).unwrap();
        let old_canonical_root = capture_scan_root(old_lexical_root.clone()).unwrap();
        let old_lexical_target = validate_cleanup_path(&old_lexical_root, &target_path).unwrap();
        let old_snapshot = capture_path_snapshot(&old_canonical_root, old_lexical_target).unwrap();
        let registry =
            ProtectedRootRegistry::with_home_directory_evidence(&old_canonical_root).unwrap();

        // DUX-DESTRUCTIVE: allow=test-protected-replaced-root -- replace a TempDir-owned root to verify stale root evidence rejection
        std::fs::rename(&root_path, &moved_root_path).unwrap();
        std::fs::create_dir(&root_path).unwrap();
        let replacement_root = capture_scan_root(validate_scan_root(&root_path).unwrap()).unwrap();
        assert_ne!(old_canonical_root.identity(), replacement_root.identity());
        assert_eq!(
            registry.assess(&replacement_root, &old_snapshot),
            Err(ProtectedRootError::MismatchedScanRootEvidence)
        );
    }

    #[test]
    fn unsupported_policy_denies_every_target() {
        let policy = policy(
            PolicyPlatform::Unsupported,
            PolicyPath::unix(&["home", "alice"]),
        );
        assert_eq!(
            target(&policy, &PolicyPath::unix(&["tmp", "file"])),
            denied(ProtectedPathKind::FilesystemRoot)
        );
    }

    #[cfg(unix)]
    #[test]
    fn current_account_registry_uses_os_account_home_and_stays_text_only() {
        let registry = ProtectedRootRegistry::from_current_account().unwrap();
        let home = User::from_uid(geteuid()).unwrap().unwrap().dir;
        let lexical_home = validate_scan_root(&home).unwrap();
        let lexical_target = validate_cleanup_path(&lexical_home, &home.join("Library")).unwrap();
        assert!(matches!(
            registry.preflight(&lexical_target).unwrap(),
            ProtectedRootDisposition::Denied { .. }
                | ProtectedRootDisposition::SpecificRuleRequired { .. }
        ));
        assert!(!format!("{registry:?}").contains("HOME"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn trusted_home_mount_witness_is_current_account_bound_and_revalidatable() {
        let home = CurrentAccountHomeEvidence::capture().unwrap();
        let witness = TrustedHomeMountWitness::capture(home.root()).unwrap();
        assert_eq!(witness.proof_revision(), TRUSTED_HOME_MOUNT_PROOF_REVISION);
        witness.revalidate().unwrap();
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn trusted_home_mount_witness_rejects_a_foreign_scan_root() {
        let foreign = std::fs::canonicalize("/tmp").unwrap();
        let root = capture_scan_root(validate_scan_root(&foreign).unwrap()).unwrap();
        assert!(TrustedHomeMountWitness::capture(&root).is_err());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn trusted_home_mount_witness_fails_closed_outside_macos() {
        let root = capture_scan_root(validate_scan_root(Path::new("/tmp")).unwrap()).unwrap();
        assert!(matches!(
            TrustedHomeMountWitness::capture(&root),
            Err(TrustedHomeMountError::UnsupportedPlatform)
        ));
    }

    #[cfg(unix)]
    #[test]
    fn account_home_validation_rejects_ambiguous_or_lossy_paths() {
        for path in [Path::new("relative/home"), Path::new("")] {
            assert_eq!(
                validate_account_home_path(path),
                Err(ProtectedRootError::InvalidHomeDirectory)
            );
        }
        assert!(validate_account_home_path(Path::new("/tmp")).is_ok());
    }
}

#[cfg(test)]
#[path = "dangerous_path_tests.rs"]
mod dangerous_path_tests;
