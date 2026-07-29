#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use tempfile::TempDir;

#[cfg(target_os = "macos")]
use nix::unistd::{User, geteuid};

use super::rust_target::{
    RustTargetLiveValidationError, RustTargetValidationSource, validate_live_rust_target_for_test,
    validate_rust_target_effect,
};
use crate::domain::{
    BlockReason, Candidate, CandidateAction, CandidateCategory, CandidateId, CandidateInput,
    CleanupMode, CleanupPlan, CleanupPlanId, CleanupPlanValidationError, Evidence,
    LocalizedTextKey, ProvenanceUrl, Rule, RuleDefinition, RuleGuards, RuleId, RuleMatcher,
    RuleMatcherDefinition, RuleRef, RuleRevision, RuleScope, SafetyTier, ScanId,
};
use crate::path_validation::{
    FilesystemEntryKind, capture_path_snapshot, capture_scan_root, validate_cleanup_path,
    validate_scan_root,
};

pub(super) const CARGO_CACHE_TAG_SIGNATURE: &[u8] = b"Signature: 8a477f597d28d172789f06886806bc55";

pub(super) struct Fixture {
    _temp: TempDir,
    pub(super) root: PathBuf,
    pub(super) target: PathBuf,
    pub(super) manifest: PathBuf,
    pub(super) cache_tag: PathBuf,
    pub(super) scan_id: ScanId,
}

impl Fixture {
    pub(super) fn new(tag_bytes: &[u8]) -> Self {
        Self::with_manifest(tag_bytes, true)
    }

    #[cfg(target_os = "macos")]
    pub(super) fn try_in_current_account_home(tag_bytes: &[u8]) -> Option<Self> {
        let home = User::from_uid(geteuid()).ok().flatten()?.dir;
        let temp = TempDir::new_in(home).ok()?;
        Some(Self::from_temp(temp, tag_bytes, true))
    }

    fn with_symlinked_manifest(tag_bytes: &[u8]) -> Self {
        use std::os::unix::fs::symlink;

        let fixture = Self::with_manifest(tag_bytes, false);
        let replacement = fixture.root.join("real-manifest");
        std::fs::write(&replacement, b"[workspace]\n").unwrap();
        symlink(&replacement, &fixture.manifest).unwrap();
        fixture
    }

    fn with_manifest(tag_bytes: &[u8], create_manifest: bool) -> Self {
        Self::with_manifest_in(tag_bytes, create_manifest, None)
    }

    fn with_manifest_in(tag_bytes: &[u8], create_manifest: bool, parent: Option<&Path>) -> Self {
        let temp = match parent {
            Some(parent) => TempDir::new_in(parent).unwrap(),
            None => TempDir::new().unwrap(),
        };
        Self::from_temp(temp, tag_bytes, create_manifest)
    }

    fn from_temp(temp: TempDir, tag_bytes: &[u8], create_manifest: bool) -> Self {
        let root = temp.path().join("scan-root");
        let target = root.join("project/target");
        std::fs::create_dir_all(&target).unwrap();
        let root = root.canonicalize().unwrap();
        let target = root.join("project/target");
        let manifest = root.join("project/Cargo.toml");
        let cache_tag = target.join("CACHEDIR.TAG");
        if create_manifest {
            std::fs::write(&manifest, b"[package]\nname = \"fixture\"\n").unwrap();
        }
        std::fs::write(&cache_tag, tag_bytes).unwrap();
        Self {
            _temp: temp,
            root,
            target,
            manifest,
            cache_tag,
            scan_id: ScanId::new("scan:rust-live-fixture").unwrap(),
        }
    }

    pub(super) fn candidate(&self) -> Candidate {
        candidate(
            &self.scan_id,
            &self.target,
            rust_rule(false),
            exact_evidence(&self.target),
            vec![BlockReason::ProtectedPath],
        )
    }

    pub(super) fn source(&self) -> RustTargetValidationSource<'_> {
        RustTargetValidationSource::new(&self.scan_id, &self.root)
    }
}

pub(super) fn rust_rule(schedule_eligible: bool) -> Rule {
    Rule::try_new(RuleDefinition {
        reference: RuleRef::new(
            RuleId::new("developer.rust.target").unwrap(),
            RuleRevision::new(crate::domain::SAFE_RUST_RULE_REVISION).unwrap(),
        ),
        title_key: LocalizedTextKey::new("rule.developer.rust.target.title").unwrap(),
        category: CandidateCategory::DeveloperArtifact,
        scope: RuleScope::SelectedScanRoot,
        matcher: RuleMatcher::try_new(RuleMatcherDefinition {
            path_component: Some("target".to_owned()),
            required_ancestor_markers_any: vec!["Cargo.toml".to_owned()],
            required_markers_all: vec!["CACHEDIR.TAG".to_owned()],
            forbidden_markers_any: Vec::new(),
            exact_bundle_identifiers: Vec::new(),
            excluded_descendants: Vec::new(),
            protected_descendants: Vec::new(),
        })
        .unwrap(),
        guards: RuleGuards::try_new(
            Some(crate::domain::SAFE_RUST_RULE_MINIMUM_AGE),
            0,
            Vec::new(),
            false,
        )
        .unwrap(),
        safety: SafetyTier::SafeRegenerable,
        action: CandidateAction::RemoveKnownRegenerableContents,
        schedule_eligible,
        explanation_key: LocalizedTextKey::new("rule.developer.rust.target.explanation").unwrap(),
        provenance: vec![
            ProvenanceUrl::new("https://doc.rust-lang.org/cargo/reference/build-cache.html")
                .unwrap(),
        ],
    })
    .unwrap()
}

pub(super) fn exact_evidence(target: &Path) -> Vec<Evidence> {
    vec![
        Evidence::MatchedPath {
            path: target.to_path_buf(),
        },
        Evidence::RequiredMarker {
            path: target.parent().unwrap().join("Cargo.toml"),
        },
        Evidence::RequiredMarker {
            path: target.join("CACHEDIR.TAG"),
        },
        Evidence::MinimumAge {
            newest_mtime: rust_candidate_newest_mtime(),
            minimum_age: crate::domain::SAFE_RUST_RULE_MINIMUM_AGE,
        },
    ]
}

pub(super) fn rust_candidate_newest_mtime() -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(86_400)
}

pub(crate) fn set_subtree_modified_at(path: &Path, modified_at: SystemTime) {
    if path.is_dir() {
        for entry in std::fs::read_dir(path).unwrap() {
            set_subtree_modified_at(&entry.unwrap().path(), modified_at);
        }
    }
    std::fs::File::open(path)
        .unwrap()
        .set_times(std::fs::FileTimes::new().set_modified(modified_at))
        .unwrap();
}

pub(super) fn candidate(
    scan_id: &ScanId,
    target: &Path,
    rule: Rule,
    evidence: Vec<Evidence>,
    blockers: Vec<BlockReason>,
) -> Candidate {
    Candidate::try_from_rule(
        &rule,
        CandidateInput::new(
            CandidateId::new("candidate:rust-live-fixture").unwrap(),
            vec![target.to_path_buf()],
            4_096,
            Some(rust_candidate_newest_mtime()),
            evidence,
            blockers,
            scan_id.clone(),
        ),
    )
    .unwrap()
}

#[test]
fn exact_signature_and_arbitrary_suffix_create_non_authoritative_witness() {
    let mut tag = CARGO_CACHE_TAG_SIGNATURE.to_vec();
    tag.extend_from_slice(b"\n# arbitrary standards-compliant suffix\n");
    let fixture = Fixture::new(&tag);
    let candidate = fixture.candidate();

    let witness = validate_live_rust_target_for_test(fixture.source(), &candidate).unwrap();

    assert_eq!(witness.witness_revision(), 1);
    assert_eq!(witness.source_scan_id(), &fixture.scan_id);
    assert_eq!(witness.candidate_id(), candidate.id());
    assert_eq!(witness.scan_root().canonical_path(), fixture.root);
    assert_eq!(
        witness.target().target_kind(),
        FilesystemEntryKind::Directory
    );
    assert_eq!(
        witness.manifest().target_kind(),
        FilesystemEntryKind::RegularFile
    );
    assert_eq!(witness.cache_tag().prefix(), CARGO_CACHE_TAG_SIGNATURE);
    assert!(witness.protected_path_is_still_unresolved());
    assert_eq!(candidate.blockers(), [BlockReason::ProtectedPath]);
    assert!(!candidate.rule_marks_schedule_eligible());
}

#[test]
fn malformed_or_truncated_cache_tag_fails_closed() {
    for invalid in [
        b"".as_slice(),
        b"Signature: 8a477f597d28d172789f06886806bc5".as_slice(),
        b"\nSignature: 8a477f597d28d172789f06886806bc55".as_slice(),
        b"signature: 8a477f597d28d172789f06886806bc55".as_slice(),
        b"Signature: 8a477f597d28d172789f06886806bc54".as_slice(),
    ] {
        let fixture = Fixture::new(invalid);
        assert!(
            validate_live_rust_target_for_test(fixture.source(), &fixture.candidate()).is_err()
        );
    }
}

#[test]
fn source_policy_and_exact_evidence_are_all_required() {
    let fixture = Fixture::new(CARGO_CACHE_TAG_SIGNATURE);
    let wrong_scan = ScanId::new("scan:other").unwrap();
    assert!(matches!(
        validate_live_rust_target_for_test(
            RustTargetValidationSource::new(&wrong_scan, &fixture.root),
            &fixture.candidate(),
        ),
        Err(RustTargetLiveValidationError::SourceScanMismatch)
    ));

    let scheduled = candidate(
        &fixture.scan_id,
        &fixture.target,
        rust_rule(true),
        exact_evidence(&fixture.target),
        vec![BlockReason::ProtectedPath],
    );
    assert!(matches!(
        validate_live_rust_target_for_test(fixture.source(), &scheduled),
        Err(RustTargetLiveValidationError::CandidatePolicyMismatch)
    ));

    let mut wrong_evidence = exact_evidence(&fixture.target);
    wrong_evidence[2] = Evidence::RequiredMarker {
        path: fixture.target.join("not-the-cache-tag"),
    };
    let forged = candidate(
        &fixture.scan_id,
        &fixture.target,
        rust_rule(false),
        wrong_evidence,
        vec![BlockReason::ProtectedPath],
    );
    assert!(matches!(
        validate_live_rust_target_for_test(fixture.source(), &forged),
        Err(RustTargetLiveValidationError::CandidateEvidenceMismatch)
    ));
}

#[test]
fn symlinked_or_multiply_linked_markers_fail_closed() {
    let symlink_fixture = Fixture::with_symlinked_manifest(CARGO_CACHE_TAG_SIGNATURE);
    assert!(
        validate_live_rust_target_for_test(symlink_fixture.source(), &symlink_fixture.candidate(),)
            .is_err()
    );

    let linked_fixture = Fixture::new(CARGO_CACHE_TAG_SIGNATURE);
    std::fs::hard_link(
        &linked_fixture.cache_tag,
        linked_fixture.target.join("second-tag-link"),
    )
    .unwrap();
    assert!(matches!(
        validate_live_rust_target_for_test(linked_fixture.source(), &linked_fixture.candidate()),
        Err(RustTargetLiveValidationError::MultiplyLinkedMarker)
    ));
}

#[test]
fn witness_does_not_remove_blocker_or_unlock_plan_construction() {
    let fixture = Fixture::new(CARGO_CACHE_TAG_SIGNATURE);
    let candidate = fixture.candidate();
    let _witness = validate_live_rust_target_for_test(fixture.source(), &candidate).unwrap();

    assert_eq!(candidate.blockers(), [BlockReason::ProtectedPath]);
    assert_eq!(
        CleanupPlan::try_from_candidates_for_persistence_test(
            CleanupPlanId::new("plan:rust-live-fixture").unwrap(),
            SystemTime::now(),
            CleanupMode::PermanentSafe,
            std::slice::from_ref(&candidate),
        ),
        Err(CleanupPlanValidationError::BlockedCandidate { candidate_index: 0 })
    );
}

#[test]
fn permanent_effect_witness_revalidates_markers_without_mutating_target() {
    let fixture = Fixture::new(CARGO_CACHE_TAG_SIGNATURE);
    let lexical_root = validate_scan_root(&fixture.root).unwrap();
    let scan_root = capture_scan_root(lexical_root.clone()).unwrap();
    let lexical_target = validate_cleanup_path(&lexical_root, &fixture.target).unwrap();
    let target = capture_path_snapshot(&scan_root, lexical_target).unwrap();

    let witness = validate_rust_target_effect(target, SystemTime::now()).unwrap();
    witness.revalidate_current().unwrap();
    assert_eq!(witness.target_path(), fixture.target);
    assert!(fixture.target.exists());

    std::fs::write(&fixture.cache_tag, b"invalid").unwrap();
    assert!(matches!(
        witness.revalidate_current(),
        Err(RustTargetLiveValidationError::InvalidCacheTagSignature)
            | Err(RustTargetLiveValidationError::ChangedDuringValidation)
            | Err(RustTargetLiveValidationError::FilePrefix(_))
    ));
    assert!(fixture.target.exists());
}
