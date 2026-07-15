pub mod cache;
pub mod domain;
pub mod error;
// The validator is intentionally staged ahead of its protected-root consumer.
// Its constructors remain crate-private until that fail-closed layer lands.
#[allow(dead_code)]
pub(crate) mod path_validation;
pub mod projection;
pub mod scanner;
pub mod size;
pub mod tree;

mod time;

pub use cache::{
    CACHE_MAGIC, CACHE_VERSION, CacheMetadata, CachedScanConfig, cache_path_for, get_mtime,
    is_cache_valid, load_cache, save_cache, spot_check_mtimes,
};
pub use domain::{
    ActivityGuard, BlockReason, CLEANUP_PLAN_VALIDITY, Candidate, CandidateAction,
    CandidateCategory, CandidateId, CandidateValidationError, CleanupMode, CleanupPlan,
    CleanupPlanId, CleanupPlanItem, CleanupPlanValidationError, Evidence, EvidenceKind,
    LocalizedTextKey, OperationStatus, PlanWarning, ProvenanceUrl, Rule, RuleDefinition,
    RuleGuards, RuleId, RuleMatcher, RuleMatcherDefinition, RuleRef, RuleRevision, RuleScope,
    RuleValidationError, SafetyTier, ScanId, StableIdError,
};
pub use error::{DuxError, Result};
pub use projection::{
    ArtifactClassification, ArtifactKind, BuildArtifactEntry, LargeFileEntry, StaleThreshold,
    classify_artifact, project_build_artifacts_at, project_large_files,
    refresh_artifact_staleness_at,
};
pub use scanner::{CancellationToken, ScanConfig, ScanMessage, ScanProgress, Scanner};
pub use size::{format_count, format_size, format_size_short, size_percentage};
pub use tree::{DiskTree, NodeId, NodeKind, TreeNode};
