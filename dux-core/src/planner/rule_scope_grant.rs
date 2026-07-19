//! Trusted deterministic-rule scope authorization.
//!
//! This is the first authority-bearing planner token, but it is deliberately
//! not a plan or an effect capability. It binds an allowlisted deterministic
//! rule to the current-account home/mount proof, a fresh protected-root
//! assessment, and one exact no-follow target snapshot. Future plan and
//! executor boundaries must consume and revalidate this token again.

use thiserror::Error;

use crate::domain::RuleRef;
use crate::path_validation::{
    CanonicalPathError, CanonicalPathSnapshot, CanonicalScanRoot, ProtectedRootDisposition,
    ProtectedRootError, ProtectedRootRegistry, TrustedHomeMountError, TrustedHomeMountWitness,
    capture_filesystem_boundary, capture_path_snapshot, capture_scan_root, validate_cleanup_path,
    validate_scan_root,
};

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
    location: TrustedHomeMountWitness,
    registry: ProtectedRootRegistry,
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
    #[error("the authorized target changed before revalidation completed")]
    ChangedSinceAuthorization,
    #[error(transparent)]
    Filesystem(#[from] CanonicalPathError),
    #[error(transparent)]
    Lexical(#[from] crate::path_validation::LexicalPathError),
}

/// Mint a grant for an exact target snapshot and a known production rule.
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
        target,
        location,
        registry,
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
        self.location
            .revalidate()
            .map_err(RuleScopeGrantError::Location)?;
        let boundary = capture_filesystem_boundary(&self.scan_root)?;
        if !self.location.matches_scan_boundary(&boundary)
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
        let disposition = self.registry.assess(&self.scan_root, &self.target)?;
        if !matches!(disposition, ProtectedRootDisposition::NoTextualMatch { .. }) {
            return Err(RuleScopeGrantError::ProtectedPath);
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
