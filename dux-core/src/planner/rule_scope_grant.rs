//! Trusted deterministic-rule scope authorization.
//!
//! This is the first authority-bearing planner token, but it is deliberately
//! not a plan or an effect capability. It binds an allowlisted deterministic
//! rule to the current-account home/mount proof, a fresh protected-root
//! assessment, and one exact no-follow target snapshot. Future plan and
//! executor boundaries must consume and revalidate this token again.

use thiserror::Error;

#[cfg(unix)]
use super::rust_target_cargo::{RustTargetRuleBoundaryError, RustTargetRuleBoundaryEvidence};
use crate::domain::RuleRef;
#[cfg(unix)]
use crate::domain::{CandidateId, ScanId};
use crate::path_validation::{
    CanonicalPathError, CanonicalPathSnapshot, CanonicalScanRoot, ProtectedRootDisposition,
    ProtectedRootError, ProtectedRootRegistry, TrustedHomeMountError, TrustedHomeMountWitness,
    capture_filesystem_boundary, capture_path_snapshot, capture_scan_root, validate_cleanup_path,
    validate_scan_root,
};

const TRUSTED_VOLUME_GRANT_REVISION: u32 = 1;
const PROTECTED_RULE_GRANT_REVISION: u32 = 1;
const RUST_TARGET_PROTECTED_GRANT_KEY: &str =
    "dux:protected-rule:developer.rust.target:2:policy-2:volume-1";
const PYTHON_PYCACHE_PROTECTED_GRANT_KEY: &str =
    "dux:protected-rule:developer.python.pycache:2:policy-2:volume-1";

/// A private authority-bearing wrapper around the current-account home/mount
/// proof. The underlying observation is consumed at construction and can only
/// be revalidated or used for an exact scan-boundary comparison.
#[cfg(unix)]
struct TrustedVolumeGrant {
    witness: TrustedHomeMountWitness,
    revision: u32,
}

#[cfg(unix)]
#[derive(Debug, Error)]
pub(crate) enum TrustedVolumeGrantError {
    #[error("trusted volume grant revision is unsupported")]
    UnsupportedRevision,
    #[error("trusted volume grant changed")]
    Changed,
}

#[cfg(unix)]
impl TrustedVolumeGrant {
    fn capture(witness: TrustedHomeMountWitness) -> Result<Self, TrustedVolumeGrantError> {
        witness
            .revalidate()
            .map_err(|_| TrustedVolumeGrantError::Changed)?;
        let grant = Self {
            witness,
            revision: TRUSTED_VOLUME_GRANT_REVISION,
        };
        grant.revalidate()?;
        Ok(grant)
    }

    fn revalidate(&self) -> Result<(), TrustedVolumeGrantError> {
        if self.revision != TRUSTED_VOLUME_GRANT_REVISION {
            return Err(TrustedVolumeGrantError::UnsupportedRevision);
        }
        self.witness
            .revalidate()
            .map_err(|_| TrustedVolumeGrantError::Changed)
    }

    fn matches_scan_boundary(
        &self,
        boundary: &crate::path_validation::FilesystemBoundarySnapshot,
    ) -> bool {
        self.revision == TRUSTED_VOLUME_GRANT_REVISION
            && self.witness.matches_scan_boundary(boundary)
    }
}

/// A private grant for one exact rule target after both requested and
/// canonical protected-root forms have produced the revisioned
/// `NoTextualMatch` result. It is a policy grant, not a generic allow-list:
/// target identity and policy revision are rechecked on every use.
#[cfg(unix)]
struct ProtectedRuleGrant {
    rule: RuleRef,
    boundary_key: &'static str,
    registry: ProtectedRootRegistry,
    scan_root: CanonicalScanRoot,
    target: CanonicalPathSnapshot,
    requested_policy_revision: u32,
    canonical_policy_revision: u32,
    revision: u32,
}

#[cfg(unix)]
#[derive(Debug, Error)]
pub(crate) enum ProtectedRuleGrantError {
    #[error("protected rule grant revision is unsupported")]
    UnsupportedRevision,
    #[error("protected-root policy could not be evaluated: {0}")]
    Policy(#[from] ProtectedRootError),
    #[error("protected rule grant target does not match the reviewed scan")]
    TargetMismatch,
    #[error("protected rule grant requires a non-textually-protected target")]
    ProtectedPath,
}

#[cfg(unix)]
impl ProtectedRuleGrant {
    fn capture(
        registry: ProtectedRootRegistry,
        scan_root: &CanonicalScanRoot,
        target: &CanonicalPathSnapshot,
        rule: &RuleRef,
    ) -> Result<Self, ProtectedRuleGrantError> {
        let boundary_key =
            trusted_rule_boundary_key(rule).ok_or(ProtectedRuleGrantError::TargetMismatch)?;
        let lexical_root = validate_scan_root(scan_root.requested_path())
            .map_err(|_| ProtectedRuleGrantError::TargetMismatch)?;
        let requested = validate_cleanup_path(&lexical_root, target.requested_path())
            .map_err(|_| ProtectedRuleGrantError::TargetMismatch)?;
        let requested = registry.preflight(&requested)?;
        let canonical = registry.assess(scan_root, target)?;
        let (requested_policy_revision, canonical_policy_revision) = match (requested, canonical) {
            (
                ProtectedRootDisposition::NoTextualMatch {
                    policy_revision: requested_revision,
                },
                ProtectedRootDisposition::NoTextualMatch {
                    policy_revision: canonical_revision,
                },
            ) if requested_revision == canonical_revision => {
                (requested_revision, canonical_revision)
            }
            _ => return Err(ProtectedRuleGrantError::ProtectedPath),
        };
        let grant = Self {
            rule: rule.clone(),
            boundary_key,
            registry,
            scan_root: scan_root.clone(),
            target: target.clone(),
            requested_policy_revision,
            canonical_policy_revision,
            revision: PROTECTED_RULE_GRANT_REVISION,
        };
        grant.revalidate()?;
        Ok(grant)
    }

    fn revalidate(&self) -> Result<(), ProtectedRuleGrantError> {
        if self.revision != PROTECTED_RULE_GRANT_REVISION {
            return Err(ProtectedRuleGrantError::UnsupportedRevision);
        }
        if trusted_rule_boundary_key(&self.rule) != Some(self.boundary_key) {
            return Err(ProtectedRuleGrantError::TargetMismatch);
        }
        let lexical_root = validate_scan_root(self.scan_root.requested_path())
            .map_err(|_| ProtectedRuleGrantError::TargetMismatch)?;
        let requested = validate_cleanup_path(&lexical_root, self.target.requested_path())
            .map_err(|_| ProtectedRuleGrantError::TargetMismatch)?;
        let requested = self.registry.preflight(&requested)?;
        let canonical = self.registry.assess(&self.scan_root, &self.target)?;
        match (requested, canonical) {
            (
                ProtectedRootDisposition::NoTextualMatch {
                    policy_revision: requested_revision,
                },
                ProtectedRootDisposition::NoTextualMatch {
                    policy_revision: canonical_revision,
                },
            ) if requested_revision == self.requested_policy_revision
                && canonical_revision == self.canonical_policy_revision =>
            {
                Ok(())
            }
            _ => Err(ProtectedRuleGrantError::ProtectedPath),
        }
    }
}

#[cfg(unix)]
fn trusted_rule_boundary_key(rule: &RuleRef) -> Option<&'static str> {
    match (rule.id().as_str(), rule.revision().get()) {
        ("developer.rust.target", 2) => Some(RUST_TARGET_PROTECTED_GRANT_KEY),
        ("developer.python.pycache", 2) => Some(PYTHON_PYCACHE_PROTECTED_GRANT_KEY),
        _ => None,
    }
}

pub(crate) const TRUSTED_RULE_SCOPE_GRANT_REVISION: u32 = 1;
const RUST_TARGET_RULE: &str = "developer.rust.target";
const PYTHON_PYCACHE_RULE: &str = "developer.python.pycache";
const SAFE_RULE_REVISION: u32 = 2;

/// A code-owned authorization for one known deterministic rule and one exact
/// target. It is intentionally non-Clone, non-serializable, and exposes only
/// revalidation/release; no path, plan, approval, scheduling, or effect API is
/// available from this token.
#[must_use = "rule scope authorization must be consumed by a reviewed planner"]
pub(crate) struct RuleScopeAuthorization {
    revision: u32,
    rule: RuleRef,
    scan_root: CanonicalScanRoot,
    target: CanonicalPathSnapshot,
    #[cfg(unix)]
    volume: TrustedVolumeGrant,
    #[cfg(unix)]
    protected: ProtectedRuleGrant,
    #[cfg(not(unix))]
    location: TrustedHomeMountWitness,
    #[cfg(not(unix))]
    registry: ProtectedRootRegistry,
    #[cfg(unix)]
    cargo_boundary: Option<RustTargetRuleBoundaryEvidence>,
    #[cfg(test)]
    test_only_without_cargo: bool,
}

#[derive(Debug, Error)]
pub(crate) enum RuleScopeGrantError {
    #[error("trusted rule scope grant revision is unsupported")]
    UnsupportedRevision,
    #[error("the rule is not allowlisted for a trusted deterministic scope")]
    UnsupportedRule,
    #[error("trusted current-account home/mount evidence could not be captured: {0}")]
    Location(#[from] TrustedHomeMountError),
    #[error("protected-root policy could not be evaluated: {0}")]
    ProtectedPolicy(#[from] ProtectedRootError),
    #[error("the target is outside the trusted scan boundary")]
    BoundaryMismatch,
    #[error("the target is not free of textual protected-root requirements")]
    ProtectedPath,
    #[cfg(unix)]
    #[error("the Rust-target grant requires the retained Cargo planning boundary")]
    CargoProvenanceMissing,
    #[cfg(unix)]
    #[error("the retained Cargo planning boundary does not match the reviewed target")]
    CargoBoundaryMismatch,
    #[cfg(unix)]
    #[error("the retained Cargo planning boundary could not be revalidated: {0}")]
    CargoBoundary(#[source] RustTargetRuleBoundaryError),
    #[error("the authorized target changed before revalidation completed")]
    ChangedSinceAuthorization,
    #[cfg(unix)]
    #[error("trusted volume grant failed: {0}")]
    VolumeGrant(#[source] TrustedVolumeGrantError),
    #[cfg(unix)]
    #[error("protected rule grant failed: {0}")]
    ProtectedGrant(#[source] ProtectedRuleGrantError),
    #[error(transparent)]
    Filesystem(#[from] CanonicalPathError),
    #[error(transparent)]
    Lexical(#[from] crate::path_validation::LexicalPathError),
}

/// Mint a grant for an exact target snapshot and a known production rule.
#[cfg(test)]
pub(crate) fn authorize_rule_target(
    scan_root: &CanonicalScanRoot,
    target: CanonicalPathSnapshot,
    rule: &RuleRef,
) -> Result<RuleScopeAuthorization, RuleScopeGrantError> {
    validate_rule(rule)?;
    let location = TrustedHomeMountWitness::capture(scan_root)?;
    let registry = ProtectedRootRegistry::from_current_account()?;
    let authorization = RuleScopeAuthorization {
        revision: TRUSTED_RULE_SCOPE_GRANT_REVISION,
        rule: rule.clone(),
        scan_root: scan_root.clone(),
        target: target.clone(),
        #[cfg(unix)]
        volume: TrustedVolumeGrant::capture(location).map_err(RuleScopeGrantError::VolumeGrant)?,
        #[cfg(unix)]
        protected: ProtectedRuleGrant::capture(registry, scan_root, &target, rule)
            .map_err(RuleScopeGrantError::ProtectedGrant)?,
        #[cfg(not(unix))]
        location,
        #[cfg(not(unix))]
        registry,
        #[cfg(unix)]
        cargo_boundary: None,
        #[cfg(test)]
        test_only_without_cargo: true,
    };
    authorization.validate_current()?;
    Ok(authorization)
}

/// Consume the exact Cargo/read-set rule-boundary evidence into the next
/// private planning token. This join is still observational: it cannot clear
/// `ProtectedPath`, create a plan, approve, schedule, cross FFI, or mutate.
#[cfg(unix)]
pub(crate) fn authorize_rust_target(
    boundary: RustTargetRuleBoundaryEvidence,
    source_scan_id: &ScanId,
    candidate_id: &CandidateId,
    scan_root: &CanonicalScanRoot,
    target: CanonicalPathSnapshot,
    rule: &RuleRef,
) -> Result<RuleScopeAuthorization, RuleScopeGrantError> {
    if rule.id().as_str() != RUST_TARGET_RULE || rule.revision().get() != SAFE_RULE_REVISION {
        return Err(RuleScopeGrantError::UnsupportedRule);
    }
    boundary
        .revalidate()
        .map_err(RuleScopeGrantError::CargoBoundary)?;
    if !boundary.matches_target_binding(source_scan_id, candidate_id, scan_root, &target) {
        return Err(RuleScopeGrantError::CargoBoundaryMismatch);
    }
    let location = TrustedHomeMountWitness::capture(scan_root)?;
    let registry = ProtectedRootRegistry::from_current_account()?;
    let authorization = RuleScopeAuthorization {
        revision: TRUSTED_RULE_SCOPE_GRANT_REVISION,
        rule: rule.clone(),
        scan_root: scan_root.clone(),
        target: target.clone(),
        volume: TrustedVolumeGrant::capture(location).map_err(RuleScopeGrantError::VolumeGrant)?,
        protected: ProtectedRuleGrant::capture(registry, scan_root, &target, rule)
            .map_err(RuleScopeGrantError::ProtectedGrant)?,
        cargo_boundary: Some(boundary),
        #[cfg(test)]
        test_only_without_cargo: false,
    };
    authorization.validate_current()?;
    Ok(authorization)
}

fn validate_rule(rule: &RuleRef) -> Result<(), RuleScopeGrantError> {
    let allowed = matches!(
        (rule.id().as_str(), rule.revision().get()),
        (RUST_TARGET_RULE, SAFE_RULE_REVISION) | (PYTHON_PYCACHE_RULE, SAFE_RULE_REVISION)
    );
    if allowed {
        Ok(())
    } else {
        Err(RuleScopeGrantError::UnsupportedRule)
    }
}

impl RuleScopeAuthorization {
    pub(super) fn matches(&self, rule: &RuleRef, target: &CanonicalPathSnapshot) -> bool {
        &self.rule == rule && &self.target == target
    }

    /// Revalidate the exact trusted target and return its private snapshot for
    /// the next rule-specific executor witness. The snapshot remains
    /// crate-private and is never exposed through FFI or a public plan API.
    pub(super) fn revalidated_target_snapshot(
        &self,
    ) -> Result<CanonicalPathSnapshot, RuleScopeGrantError> {
        self.revalidate()?;
        Ok(self.target.clone())
    }

    fn validate_current(&self) -> Result<(), RuleScopeGrantError> {
        if self.revision != TRUSTED_RULE_SCOPE_GRANT_REVISION {
            return Err(RuleScopeGrantError::UnsupportedRevision);
        }
        validate_rule(&self.rule)?;
        #[cfg(unix)]
        if self.rule.id().as_str() == RUST_TARGET_RULE {
            #[cfg(test)]
            let test_only_without_cargo = self.test_only_without_cargo;
            #[cfg(not(test))]
            let test_only_without_cargo = false;
            if !test_only_without_cargo {
                let cargo_boundary = self
                    .cargo_boundary
                    .as_ref()
                    .ok_or(RuleScopeGrantError::CargoProvenanceMissing)?;
                cargo_boundary
                    .revalidate()
                    .map_err(RuleScopeGrantError::CargoBoundary)?;
                if !cargo_boundary.matches_bound_target(&self.scan_root, &self.target) {
                    return Err(RuleScopeGrantError::CargoBoundaryMismatch);
                }
            }
        }
        #[cfg(unix)]
        {
            self.volume
                .revalidate()
                .map_err(RuleScopeGrantError::VolumeGrant)?;
            self.protected
                .revalidate()
                .map_err(RuleScopeGrantError::ProtectedGrant)?;
        }
        #[cfg(not(unix))]
        self.location
            .revalidate()
            .map_err(RuleScopeGrantError::Location)?;
        let boundary = capture_filesystem_boundary(&self.scan_root)?;
        #[cfg(unix)]
        let location_matches = self.volume.matches_scan_boundary(&boundary);
        #[cfg(not(unix))]
        let location_matches = self.location.matches_scan_boundary(&boundary);
        if !location_matches
            || self.target.scan_root() != self.scan_root.canonical_path()
            || self
                .target
                .ancestors()
                .first()
                .map(|ancestor| ancestor.identity())
                != Some(self.scan_root.identity())
        {
            return Err(RuleScopeGrantError::BoundaryMismatch);
        }
        #[cfg(not(unix))]
        {
            let disposition = self.registry.assess(&self.scan_root, &self.target)?;
            if !matches!(disposition, ProtectedRootDisposition::NoTextualMatch { .. }) {
                return Err(RuleScopeGrantError::ProtectedPath);
            }
        }
        Ok(())
    }

    /// Repeat account, mount, boundary, protected-root, and exact target
    /// evidence. Any replacement, removal, or ancestry drift fails closed.
    pub(crate) fn revalidate(&self) -> Result<(), RuleScopeGrantError> {
        self.validate_current()?;
        let lexical_root = validate_scan_root(self.scan_root.requested_path())?;
        let live_root = capture_scan_root(lexical_root.clone())?;
        if live_root != self.scan_root {
            return Err(RuleScopeGrantError::ChangedSinceAuthorization);
        }
        let lexical_target = validate_cleanup_path(&lexical_root, self.target.requested_path())?;
        let live_target = capture_path_snapshot(&live_root, lexical_target)?;
        if live_target != self.target {
            return Err(RuleScopeGrantError::ChangedSinceAuthorization);
        }
        Ok(())
    }

    pub(crate) fn release(self) {}
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use std::fs::{create_dir_all, rename, write};

    use tempfile::TempDir;

    use super::*;
    use crate::domain::{RuleId, RuleRevision};
    use crate::path_validation::{capture_path_snapshot, capture_scan_root, validate_cleanup_path};

    fn fixture() -> (TempDir, CanonicalScanRoot, CanonicalPathSnapshot) {
        let temp = TempDir::new_in(std::env::current_dir().unwrap()).unwrap();
        let root_path = std::fs::canonicalize(temp.path()).unwrap();
        let lexical_root = validate_scan_root(&root_path).unwrap();
        let root = capture_scan_root(lexical_root.clone()).unwrap();
        let target_path = root_path.join("target");
        create_dir_all(&target_path).unwrap();
        write(
            target_path.join("CACHEDIR.TAG"),
            b"Signature: 8a477f597d28d172789f06886806bc55",
        )
        .unwrap();
        let lexical_target = validate_cleanup_path(&lexical_root, &target_path).unwrap();
        let target = capture_path_snapshot(&root, lexical_target).unwrap();
        (temp, root, target)
    }

    fn rust_rule() -> RuleRef {
        RuleRef::new(
            RuleId::new(RUST_TARGET_RULE).unwrap(),
            RuleRevision::new(SAFE_RULE_REVISION).unwrap(),
        )
    }

    #[test]
    fn unknown_or_wrong_revision_never_mints_location_evidence() {
        let (_temp, root, target) = fixture();
        let unknown = RuleRef::new(
            RuleId::new("fixture.unknown").unwrap(),
            RuleRevision::new(1).unwrap(),
        );
        assert!(matches!(
            authorize_rule_target(&root, target.clone(), &unknown),
            Err(RuleScopeGrantError::UnsupportedRule)
        ));
        let wrong_revision = RuleRef::new(
            RuleId::new(RUST_TARGET_RULE).unwrap(),
            RuleRevision::new(1).unwrap(),
        );
        assert!(matches!(
            authorize_rule_target(&root, target, &wrong_revision),
            Err(RuleScopeGrantError::UnsupportedRule)
        ));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_grant_revalidates_exact_target_and_rejects_replacement() {
        let (temp, root, target) = fixture();
        let authorization = authorize_rule_target(&root, target, &rust_rule()).unwrap();
        authorization.revalidate().unwrap();
        let old = temp.path().join("target");
        rename(&old, temp.path().join("target-old")).unwrap();
        create_dir_all(&old).unwrap();
        write(old.join("CACHEDIR.TAG"), b"replacement").unwrap();
        assert!(matches!(
            authorization.revalidate(),
            Err(RuleScopeGrantError::ChangedSinceAuthorization)
                | Err(RuleScopeGrantError::Filesystem(_))
        ));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_grant_rejects_boundary_key_policy_and_volume_drift() {
        let (_temp, root, target) = fixture();
        let mut authorization = authorize_rule_target(&root, target, &rust_rule()).unwrap();

        authorization.protected.boundary_key = "dux:foreign-boundary";
        assert!(matches!(
            authorization.revalidate(),
            Err(RuleScopeGrantError::ProtectedGrant(
                ProtectedRuleGrantError::TargetMismatch
            ))
        ));

        authorization.protected.boundary_key = RUST_TARGET_PROTECTED_GRANT_KEY;
        authorization.protected.requested_policy_revision += 1;
        assert!(matches!(
            authorization.revalidate(),
            Err(RuleScopeGrantError::ProtectedGrant(
                ProtectedRuleGrantError::ProtectedPath
            ))
        ));

        authorization.protected.requested_policy_revision =
            authorization.protected.canonical_policy_revision;
        authorization.volume.revision += 1;
        assert!(matches!(
            authorization.revalidate(),
            Err(RuleScopeGrantError::VolumeGrant(
                TrustedVolumeGrantError::UnsupportedRevision
            ))
        ));
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn non_macos_grant_fails_closed() {
        let (_temp, root, target) = fixture();
        assert!(matches!(
            authorize_rule_target(&root, target, &rust_rule()),
            Err(RuleScopeGrantError::Location(
                TrustedHomeMountError::UnsupportedPlatform
            ))
        ));
    }
}
