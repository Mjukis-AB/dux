//! Non-authoritative scan completeness observations.
//!
//! These values explain what a scan did and did not observe. They are not
//! filesystem identity, cleanup policy, or mutation authorization.

use std::path::{Component, Path, PathBuf};

use thiserror::Error;

use super::{LocalizedTextKey, StableIdError};

/// Maximum number of distinct issue records retained for one scan.
///
/// Callers aggregate additional issues into [`ScanIssueKind::IssueLimitReached`]
/// rather than growing a scan result without bound.
pub(crate) const MAX_SCAN_ISSUES: usize = 256;

const MAX_PATH_ENCODED_UNITS: usize = 32 * 1_024;
pub(crate) const MAX_SCAN_ISSUE_OCCURRENCES: u32 = 1_000_000_000;

/// Optional quantitative coverage estimate, in thousandths of the intended
/// scan scope.
///
/// A zero value is a measured zero and remains distinct from an absent
/// measurement.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CoveragePermille(u16);

impl CoveragePermille {
    pub const COMPLETE: Self = Self(1_000);

    pub fn new(value: u16) -> Result<Self, CoveragePermilleError> {
        if value > Self::COMPLETE.get() {
            return Err(CoveragePermilleError::OutOfRange);
        }
        Ok(Self(value))
    }

    pub const fn get(self) -> u16 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
pub enum CoveragePermilleError {
    #[error("scan coverage permille must be between 0 and 1000")]
    OutOfRange,
}

/// Product-neutral reason that an intended part of a scan was not observed.
///
/// Persistence and client adapters must map these semantic variants explicitly
/// instead of storing Rust discriminants.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ScanIssueKind {
    PermissionDenied,
    TimedOut,
    DifferentFilesystem,
    NetworkOrVirtualFilesystem,
    SymlinkSkipped,
    FileChangedDuringScan,
    MetadataError,
    Cancelled,
    PolicyExcluded,
    DepthLimited,
    ProbePoolExhausted,
    FilesystemBoundaryUnknown,
    IssueLimitReached,
}

impl ScanIssueKind {
    /// Stable presentation/persistence ordering, independent of Rust enum
    /// declaration order and never used as a wire discriminant.
    pub(crate) const fn canonical_rank(self) -> u8 {
        match self {
            Self::PermissionDenied => 0,
            Self::TimedOut => 1,
            Self::DifferentFilesystem => 2,
            Self::NetworkOrVirtualFilesystem => 3,
            Self::SymlinkSkipped => 4,
            Self::FileChangedDuringScan => 5,
            Self::MetadataError => 6,
            Self::Cancelled => 7,
            Self::PolicyExcluded => 8,
            Self::DepthLimited => 9,
            Self::ProbePoolExhausted => 10,
            Self::FilesystemBoundaryUnknown => 11,
            Self::IssueLimitReached => 12,
        }
    }

    fn is_access_limitation(self) -> bool {
        matches!(self, Self::PermissionDenied)
    }

    fn permits_global_scope(self) -> bool {
        matches!(
            self,
            Self::Cancelled | Self::ProbePoolExhausted | Self::IssueLimitReached
        )
    }

    fn message_key(self) -> Result<LocalizedTextKey, StableIdError> {
        let value = match self {
            Self::PermissionDenied => "scan.issue.permission_denied",
            Self::TimedOut => "scan.issue.timed_out",
            Self::DifferentFilesystem => "scan.issue.different_filesystem",
            Self::NetworkOrVirtualFilesystem => "scan.issue.network_or_virtual_filesystem",
            Self::SymlinkSkipped => "scan.issue.symlink_skipped",
            Self::FileChangedDuringScan => "scan.issue.file_changed_during_scan",
            Self::MetadataError => "scan.issue.metadata_error",
            Self::Cancelled => "scan.issue.cancelled",
            Self::PolicyExcluded => "scan.issue.policy_excluded",
            Self::DepthLimited => "scan.issue.depth_limited",
            Self::ProbePoolExhausted => "scan.issue.probe_pool_exhausted",
            Self::FilesystemBoundaryUnknown => "scan.issue.filesystem_boundary_unknown",
            Self::IssueLimitReached => "scan.issue.issue_limit_reached",
        };
        LocalizedTextKey::new(value)
    }
}

/// One bounded, localized scan-coverage observation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScanIssue {
    kind: ScanIssueKind,
    path: Option<PathBuf>,
    occurrence_count: u32,
    message_key: LocalizedTextKey,
}

impl ScanIssue {
    pub(crate) fn try_new(
        kind: ScanIssueKind,
        path: Option<PathBuf>,
        occurrence_count: u32,
    ) -> Result<Self, ScanIssueValidationError> {
        if occurrence_count == 0 || occurrence_count > MAX_SCAN_ISSUE_OCCURRENCES {
            return Err(ScanIssueValidationError::InvalidOccurrenceCount);
        }
        match path.as_deref() {
            Some(path) => validate_observed_path(path)?,
            None if !kind.permits_global_scope() => {
                return Err(ScanIssueValidationError::MissingPath);
            }
            None => {}
        }
        let message_key = kind
            .message_key()
            .map_err(ScanIssueValidationError::InvalidCanonicalMessageKey)?;

        Ok(Self {
            kind,
            path,
            occurrence_count,
            message_key,
        })
    }

    pub fn kind(&self) -> ScanIssueKind {
        self.kind
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    pub fn occurrence_count(&self) -> u32 {
        self.occurrence_count
    }

    pub fn message_key(&self) -> &LocalizedTextKey {
        &self.message_key
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub(crate) enum ScanIssueValidationError {
    #[error("path-scoped scan issue must include an observed path")]
    MissingPath,
    #[error("scan issue occurrence count must be between 1 and 1000000000")]
    InvalidOccurrenceCount,
    #[error("scan issue path must be absolute")]
    RelativePath,
    #[error("scan issue path must use a lossless host-native Unicode representation")]
    InvalidPathText,
    #[error("scan issue path contains an ambiguous parent component")]
    AmbiguousPath,
    #[error("scan issue path must be at most 32768 host-encoded units")]
    PathTooLong,
    #[error("canonical scan issue message key is invalid: {0}")]
    InvalidCanonicalMessageKey(StableIdError),
}

/// Honest, bounded status for one intended scan scope.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ScanCoverageStatus {
    Unknown,
    Complete,
    LimitedAccess,
    Partial,
}

/// Coverage and its complete drill-down issue set.
///
/// Construction preserves these invariants:
///
/// - `Unknown` has neither a measurement nor issues.
/// - `Complete` is exactly 1000 permille and has no issues.
/// - `LimitedAccess` has only permission-denied issues.
/// - `Partial` contains at least one non-permission issue.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScanCoverage {
    status: ScanCoverageStatus,
    measured_permille: Option<CoveragePermille>,
    issues: Vec<ScanIssue>,
}

impl ScanCoverage {
    pub fn unknown() -> Self {
        Self {
            status: ScanCoverageStatus::Unknown,
            measured_permille: None,
            issues: Vec::new(),
        }
    }

    pub(crate) fn try_new(
        status: ScanCoverageStatus,
        measured_permille: Option<CoveragePermille>,
        mut issues: Vec<ScanIssue>,
    ) -> Result<Self, ScanCoverageValidationError> {
        canonicalize_issues(&mut issues)?;
        match status {
            ScanCoverageStatus::Unknown => {
                if measured_permille.is_some() || !issues.is_empty() {
                    return Err(ScanCoverageValidationError::InvalidUnknown);
                }
            }
            ScanCoverageStatus::Complete => {
                if measured_permille != Some(CoveragePermille::COMPLETE) || !issues.is_empty() {
                    return Err(ScanCoverageValidationError::InvalidComplete);
                }
            }
            ScanCoverageStatus::LimitedAccess => {
                validate_incomplete_measurement(measured_permille)?;
                if issues.is_empty()
                    || !issues.iter().all(|issue| issue.kind.is_access_limitation())
                {
                    return Err(ScanCoverageValidationError::InvalidLimitedAccess);
                }
            }
            ScanCoverageStatus::Partial => {
                validate_incomplete_measurement(measured_permille)?;
                if issues.is_empty() || issues.iter().all(|issue| issue.kind.is_access_limitation())
                {
                    return Err(ScanCoverageValidationError::InvalidPartial);
                }
            }
        }

        Ok(Self {
            status,
            measured_permille,
            issues,
        })
    }

    /// Derive a terminal scan's truthful status from its issue taxonomy.
    ///
    /// A terminal traversal with no issues is complete by definition. An
    /// absent quantitative estimate is canonicalized to 1000 in that case;
    /// a conflicting measured value is rejected.
    #[cfg(test)]
    pub(crate) fn try_from_terminal(
        measured_permille: Option<CoveragePermille>,
        issues: Vec<ScanIssue>,
    ) -> Result<Self, ScanCoverageValidationError> {
        validate_issue_bound(&issues)?;
        if issues.is_empty() {
            if measured_permille.is_some_and(|value| value != CoveragePermille::COMPLETE) {
                return Err(ScanCoverageValidationError::InvalidComplete);
            }
            return Self::try_new(
                ScanCoverageStatus::Complete,
                Some(CoveragePermille::COMPLETE),
                issues,
            );
        }

        let status = if issues.iter().all(|issue| issue.kind.is_access_limitation()) {
            ScanCoverageStatus::LimitedAccess
        } else {
            ScanCoverageStatus::Partial
        };
        Self::try_new(status, measured_permille, issues)
    }

    /// Infallible terminal construction for an already validated, canonical
    /// scanner accumulator. Keeping this crate-private prevents clients from
    /// manufacturing a false Complete result.
    pub(crate) fn from_validated_terminal_issues(issues: Vec<ScanIssue>) -> Self {
        debug_assert!(validate_issue_bound(&issues).is_ok());
        debug_assert!(issues.windows(2).all(|pair| {
            pair[0].kind.canonical_rank() < pair[1].kind.canonical_rank()
                || (pair[0].kind == pair[1].kind && pair[0].path < pair[1].path)
        }));

        if issues.is_empty() {
            return Self {
                status: ScanCoverageStatus::Complete,
                measured_permille: Some(CoveragePermille::COMPLETE),
                issues,
            };
        }
        let status = if issues.iter().all(|issue| issue.kind.is_access_limitation()) {
            ScanCoverageStatus::LimitedAccess
        } else {
            ScanCoverageStatus::Partial
        };
        Self {
            status,
            measured_permille: None,
            issues,
        }
    }

    pub fn status(&self) -> ScanCoverageStatus {
        self.status
    }

    pub fn measured_permille(&self) -> Option<CoveragePermille> {
        self.measured_permille
    }

    pub fn issues(&self) -> &[ScanIssue] {
        &self.issues
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
pub(crate) enum ScanCoverageValidationError {
    #[error("scan coverage contains more than 256 issue records")]
    TooManyIssues,
    #[error("scan coverage issues must be aggregated by kind and path")]
    DuplicateIssue,
    #[error("unknown scan coverage cannot contain a measurement or issues")]
    InvalidUnknown,
    #[error("complete scan coverage must be exactly 1000 permille with no issues")]
    InvalidComplete,
    #[error("limited-access coverage requires only permission-denied issues")]
    InvalidLimitedAccess,
    #[error("partial coverage requires at least one non-permission issue")]
    InvalidPartial,
    #[error("incomplete scan coverage cannot be measured as 1000 permille")]
    CompleteMeasurementWithIssues,
}

fn validate_issue_bound(issues: &[ScanIssue]) -> Result<(), ScanCoverageValidationError> {
    if issues.len() > MAX_SCAN_ISSUES {
        return Err(ScanCoverageValidationError::TooManyIssues);
    }
    Ok(())
}

fn canonicalize_issues(issues: &mut [ScanIssue]) -> Result<(), ScanCoverageValidationError> {
    validate_issue_bound(issues)?;
    issues.sort_by(|left, right| {
        left.kind
            .canonical_rank()
            .cmp(&right.kind.canonical_rank())
            .then_with(|| left.path.cmp(&right.path))
    });
    if issues
        .windows(2)
        .any(|pair| pair[0].kind == pair[1].kind && pair[0].path == pair[1].path)
    {
        return Err(ScanCoverageValidationError::DuplicateIssue);
    }
    Ok(())
}

fn validate_incomplete_measurement(
    measured_permille: Option<CoveragePermille>,
) -> Result<(), ScanCoverageValidationError> {
    if measured_permille == Some(CoveragePermille::COMPLETE) {
        return Err(ScanCoverageValidationError::CompleteMeasurementWithIssues);
    }
    Ok(())
}

fn validate_observed_path(path: &Path) -> Result<(), ScanIssueValidationError> {
    if !path.is_absolute() {
        return Err(ScanIssueValidationError::RelativePath);
    }
    if path
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(ScanIssueValidationError::AmbiguousPath);
    }
    validate_host_path_text(path)
}

#[cfg(unix)]
fn validate_host_path_text(path: &Path) -> Result<(), ScanIssueValidationError> {
    use std::os::unix::ffi::OsStrExt;

    let bytes = path.as_os_str().as_bytes();
    if bytes.len() > MAX_PATH_ENCODED_UNITS {
        return Err(ScanIssueValidationError::PathTooLong);
    }
    if bytes.contains(&0) || std::str::from_utf8(bytes).is_err() {
        return Err(ScanIssueValidationError::InvalidPathText);
    }
    Ok(())
}

#[cfg(windows)]
fn validate_host_path_text(path: &Path) -> Result<(), ScanIssueValidationError> {
    use std::os::windows::ffi::OsStrExt;

    let units = path.as_os_str().encode_wide().collect::<Vec<_>>();
    if units.len() > MAX_PATH_ENCODED_UNITS {
        return Err(ScanIssueValidationError::PathTooLong);
    }
    if units.contains(&0) || String::from_utf16(&units).is_err() {
        return Err(ScanIssueValidationError::InvalidPathText);
    }
    Ok(())
}

#[cfg(not(any(unix, windows)))]
fn validate_host_path_text(path: &Path) -> Result<(), ScanIssueValidationError> {
    let value = path
        .to_str()
        .ok_or(ScanIssueValidationError::InvalidPathText)?;
    if value.len() > MAX_PATH_ENCODED_UNITS {
        return Err(ScanIssueValidationError::PathTooLong);
    }
    if value.as_bytes().contains(&0) {
        return Err(ScanIssueValidationError::InvalidPathText);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path_for(name: &str) -> PathBuf {
        if cfg!(windows) {
            PathBuf::from(format!(r"C:\fixture\{name}"))
        } else {
            PathBuf::from(format!("/fixture/{name}"))
        }
    }

    fn issue(kind: ScanIssueKind) -> ScanIssue {
        let path = if kind.permits_global_scope() {
            None
        } else {
            Some(path_for("entry"))
        };
        ScanIssue::try_new(kind, path, 1).unwrap()
    }

    #[test]
    fn measured_zero_is_distinct_from_unknown() {
        let unknown = ScanCoverage::unknown();
        let zero = CoveragePermille::new(0).unwrap();
        let partial =
            ScanCoverage::try_from_terminal(Some(zero), vec![issue(ScanIssueKind::MetadataError)])
                .unwrap();

        assert_eq!(unknown.status(), ScanCoverageStatus::Unknown);
        assert_eq!(unknown.measured_permille(), None);
        assert_eq!(partial.status(), ScanCoverageStatus::Partial);
        assert_eq!(partial.measured_permille(), Some(zero));
        assert_eq!(
            CoveragePermille::new(1_001),
            Err(CoveragePermilleError::OutOfRange)
        );
    }

    #[test]
    fn terminal_status_is_derived_from_issue_semantics() {
        let complete = ScanCoverage::try_from_terminal(None, Vec::new()).unwrap();
        assert_eq!(complete.status(), ScanCoverageStatus::Complete);
        assert_eq!(
            complete.measured_permille(),
            Some(CoveragePermille::COMPLETE)
        );

        let limited =
            ScanCoverage::try_from_terminal(None, vec![issue(ScanIssueKind::PermissionDenied)])
                .unwrap();
        assert_eq!(limited.status(), ScanCoverageStatus::LimitedAccess);

        let partial = ScanCoverage::try_from_terminal(
            None,
            vec![
                issue(ScanIssueKind::PermissionDenied),
                issue(ScanIssueKind::TimedOut),
            ],
        )
        .unwrap();
        assert_eq!(partial.status(), ScanCoverageStatus::Partial);
    }

    #[test]
    fn invalid_status_combinations_fail_closed() {
        let permission = issue(ScanIssueKind::PermissionDenied);
        let metadata = issue(ScanIssueKind::MetadataError);

        assert_eq!(
            ScanCoverage::try_new(
                ScanCoverageStatus::Unknown,
                Some(CoveragePermille::new(0).unwrap()),
                Vec::new(),
            ),
            Err(ScanCoverageValidationError::InvalidUnknown)
        );
        assert_eq!(
            ScanCoverage::try_new(
                ScanCoverageStatus::Complete,
                Some(CoveragePermille::COMPLETE),
                vec![permission.clone()],
            ),
            Err(ScanCoverageValidationError::InvalidComplete)
        );
        assert_eq!(
            ScanCoverage::try_new(
                ScanCoverageStatus::LimitedAccess,
                None,
                vec![metadata.clone()],
            ),
            Err(ScanCoverageValidationError::InvalidLimitedAccess)
        );
        assert_eq!(
            ScanCoverage::try_new(ScanCoverageStatus::Partial, None, vec![permission],),
            Err(ScanCoverageValidationError::InvalidPartial)
        );
        assert_eq!(
            ScanCoverage::try_new(
                ScanCoverageStatus::Partial,
                Some(CoveragePermille::COMPLETE),
                vec![metadata],
            ),
            Err(ScanCoverageValidationError::CompleteMeasurementWithIssues)
        );
    }

    #[test]
    fn issue_taxonomy_has_fixed_localized_keys() {
        let cases = [
            (
                ScanIssueKind::PermissionDenied,
                "scan.issue.permission_denied",
            ),
            (ScanIssueKind::TimedOut, "scan.issue.timed_out"),
            (
                ScanIssueKind::DifferentFilesystem,
                "scan.issue.different_filesystem",
            ),
            (
                ScanIssueKind::NetworkOrVirtualFilesystem,
                "scan.issue.network_or_virtual_filesystem",
            ),
            (ScanIssueKind::SymlinkSkipped, "scan.issue.symlink_skipped"),
            (
                ScanIssueKind::FileChangedDuringScan,
                "scan.issue.file_changed_during_scan",
            ),
            (ScanIssueKind::MetadataError, "scan.issue.metadata_error"),
            (ScanIssueKind::Cancelled, "scan.issue.cancelled"),
            (ScanIssueKind::PolicyExcluded, "scan.issue.policy_excluded"),
            (ScanIssueKind::DepthLimited, "scan.issue.depth_limited"),
            (
                ScanIssueKind::ProbePoolExhausted,
                "scan.issue.probe_pool_exhausted",
            ),
            (
                ScanIssueKind::FilesystemBoundaryUnknown,
                "scan.issue.filesystem_boundary_unknown",
            ),
            (
                ScanIssueKind::IssueLimitReached,
                "scan.issue.issue_limit_reached",
            ),
        ];

        for (kind, expected) in cases {
            assert_eq!(issue(kind).message_key().as_str(), expected);
        }
    }

    #[test]
    fn issue_paths_and_counts_are_bounded() {
        assert_eq!(
            ScanIssue::try_new(ScanIssueKind::MetadataError, None, 1),
            Err(ScanIssueValidationError::MissingPath)
        );
        assert_eq!(
            ScanIssue::try_new(
                ScanIssueKind::MetadataError,
                Some(PathBuf::from("relative")),
                1
            ),
            Err(ScanIssueValidationError::RelativePath)
        );
        assert_eq!(
            ScanIssue::try_new(ScanIssueKind::MetadataError, Some(path_for("entry")), 0),
            Err(ScanIssueValidationError::InvalidOccurrenceCount)
        );
        assert_eq!(
            ScanIssue::try_new(
                ScanIssueKind::MetadataError,
                Some(path_for("entry")),
                MAX_SCAN_ISSUE_OCCURRENCES + 1,
            ),
            Err(ScanIssueValidationError::InvalidOccurrenceCount)
        );
        let issue = ScanIssue::try_new(
            ScanIssueKind::MetadataError,
            Some(path_for("entry")),
            MAX_SCAN_ISSUE_OCCURRENCES,
        )
        .unwrap();
        assert_eq!(issue.path(), Some(path_for("entry").as_path()));
        assert_eq!(issue.occurrence_count(), MAX_SCAN_ISSUE_OCCURRENCES);
    }

    #[test]
    fn issue_collection_is_bounded() {
        let issues = (0..=MAX_SCAN_ISSUES)
            .map(|_| issue(ScanIssueKind::PermissionDenied))
            .collect();
        assert_eq!(
            ScanCoverage::try_from_terminal(None, issues),
            Err(ScanCoverageValidationError::TooManyIssues)
        );
    }

    #[test]
    fn exact_issue_bound_and_canonical_kind_order_are_stable() {
        let issues = (0..MAX_SCAN_ISSUES)
            .map(|index| {
                ScanIssue::try_new(
                    ScanIssueKind::MetadataError,
                    Some(path_for(&format!("entry-{index:03}"))),
                    1,
                )
                .unwrap()
            })
            .collect();
        assert_eq!(
            ScanCoverage::try_from_terminal(None, issues)
                .unwrap()
                .issues()
                .len(),
            MAX_SCAN_ISSUES
        );

        let kinds = [
            ScanIssueKind::PermissionDenied,
            ScanIssueKind::TimedOut,
            ScanIssueKind::DifferentFilesystem,
            ScanIssueKind::NetworkOrVirtualFilesystem,
            ScanIssueKind::SymlinkSkipped,
            ScanIssueKind::FileChangedDuringScan,
            ScanIssueKind::MetadataError,
            ScanIssueKind::Cancelled,
            ScanIssueKind::PolicyExcluded,
            ScanIssueKind::DepthLimited,
            ScanIssueKind::ProbePoolExhausted,
            ScanIssueKind::FilesystemBoundaryUnknown,
            ScanIssueKind::IssueLimitReached,
        ];
        assert_eq!(
            kinds.map(ScanIssueKind::canonical_rank),
            [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]
        );
    }

    #[test]
    fn only_global_issue_kinds_allow_a_missing_path() {
        let kinds = [
            ScanIssueKind::PermissionDenied,
            ScanIssueKind::TimedOut,
            ScanIssueKind::DifferentFilesystem,
            ScanIssueKind::NetworkOrVirtualFilesystem,
            ScanIssueKind::SymlinkSkipped,
            ScanIssueKind::FileChangedDuringScan,
            ScanIssueKind::MetadataError,
            ScanIssueKind::Cancelled,
            ScanIssueKind::PolicyExcluded,
            ScanIssueKind::DepthLimited,
            ScanIssueKind::ProbePoolExhausted,
            ScanIssueKind::FilesystemBoundaryUnknown,
            ScanIssueKind::IssueLimitReached,
        ];
        for kind in kinds {
            let result = ScanIssue::try_new(kind, None, 1);
            assert_eq!(result.is_ok(), kind.permits_global_scope(), "{kind:?}");
        }
    }

    #[test]
    fn issue_collection_is_canonical_and_requires_aggregation() {
        let permission =
            ScanIssue::try_new(ScanIssueKind::PermissionDenied, Some(path_for("z")), 1).unwrap();
        let metadata =
            ScanIssue::try_new(ScanIssueKind::MetadataError, Some(path_for("a")), 1).unwrap();
        let coverage =
            ScanCoverage::try_from_terminal(None, vec![metadata.clone(), permission.clone()])
                .unwrap();
        assert_eq!(coverage.issues(), &[permission.clone(), metadata]);

        assert_eq!(
            ScanCoverage::try_from_terminal(None, vec![permission.clone(), permission]),
            Err(ScanCoverageValidationError::DuplicateIssue)
        );
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_and_nul_paths_are_rejected_without_lossy_conversion() {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;

        let non_utf8 = PathBuf::from(OsString::from_vec(vec![b'/', 0xff]));
        assert_eq!(
            ScanIssue::try_new(ScanIssueKind::MetadataError, Some(non_utf8), 1),
            Err(ScanIssueValidationError::InvalidPathText)
        );

        let nul = PathBuf::from(OsString::from_vec(b"/fixture/\0entry".to_vec()));
        assert_eq!(
            ScanIssue::try_new(ScanIssueKind::MetadataError, Some(nul), 1),
            Err(ScanIssueValidationError::InvalidPathText)
        );
    }
}
