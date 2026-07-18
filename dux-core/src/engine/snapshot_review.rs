use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::domain::{CandidateCategory, ScanId};
use crate::path_validation::{
    CanonicalPathError, FilesystemEntryKind, FilesystemIdentity, capture_path_snapshot,
    capture_scan_root, validate_cleanup_path, validate_scan_root,
};
use crate::persistence::HistoryErrorKind;
use crate::persistence::snapshot::{
    HostEncoding, HostValue, SnapshotCodecErrorKind, SnapshotDocument, SnapshotNode,
    SnapshotNodeKind, SnapshotRepositoryErrorKind, SnapshotReviewDocument as StoredReviewDocument,
    SnapshotReviewLease as StoredReviewLease, SnapshotScanFlags, SnapshotStorageErrorKind,
    SnapshotTimestamp,
};

pub const MAX_SNAPSHOT_REVIEW_NODE_PAGE_LIMIT: u16 = 200;
pub const MAX_SNAPSHOT_REVIEW_TREEMAP_CELLS: u16 = 64;
pub const MAX_SNAPSHOT_REVIEW_LARGE_FILE_RESULTS: u16 = 200;
pub const MAX_SNAPSHOT_REVIEW_PARENT_CONTEXT_COMPONENTS: usize = 8;
/// Defensive display-only join cap. Candidate persistence has separate, larger
/// materialization limits; Explorer discards the entire optional category join
/// instead of publishing a partial classification when this cap is exceeded.
pub(super) const MAX_SNAPSHOT_REVIEW_CATEGORY_ROOTS: usize = 4_096;
pub(super) const MAX_SNAPSHOT_REVIEW_CATEGORY_BYTES: usize = 1024 * 1024;
// Sorting runs on the engine's serial FFI executor. Keep a defensive ceiling
// well below the full snapshot-node budget until the million-node latency gate
// in M4 has measured a cancellable/background strategy.
const MAX_SNAPSHOT_REVIEW_SORTABLE_CHILDREN: u64 = 100_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapshotReviewNodeSort {
    NameAscending,
    LogicalBytesDescending,
    AllocatedBytesDescending,
    ModifiedNewest,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapshotReviewNodeKind {
    Directory,
    File,
    Symlink,
    Other,
    Error,
}

/// Deterministic historical display classification. This projection contains
/// no candidate identity, safety, action, reclaimability, or cleanup authority.
/// `Unclassified` is absence of an assertion and remains distinct from a rule
/// that explicitly classified an item as `UnknownStorage`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapshotReviewCategory {
    Unclassified,
    DeveloperArtifact,
    ApplicationCache,
    BrowserCache,
    LogAndDiagnostic,
    InstallerAndDownload,
    DeviceAndSimulatorData,
    CloudFile,
    LargeReviewItem,
    ProtectedSystemData,
    UnknownStorage,
}

impl From<CandidateCategory> for SnapshotReviewCategory {
    fn from(value: CandidateCategory) -> Self {
        match value {
            CandidateCategory::DeveloperArtifact => Self::DeveloperArtifact,
            CandidateCategory::ApplicationCache => Self::ApplicationCache,
            CandidateCategory::BrowserCache => Self::BrowserCache,
            CandidateCategory::LogAndDiagnostic => Self::LogAndDiagnostic,
            CandidateCategory::InstallerAndDownload => Self::InstallerAndDownload,
            CandidateCategory::DeviceAndSimulatorData => Self::DeviceAndSimulatorData,
            CandidateCategory::CloudFile => Self::CloudFile,
            CandidateCategory::LargeReviewItem => Self::LargeReviewItem,
            CandidateCategory::ProtectedSystemData => Self::ProtectedSystemData,
            CandidateCategory::UnknownStorage => Self::UnknownStorage,
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct SnapshotReviewCategoryRoot {
    path: PathBuf,
    category: SnapshotReviewCategory,
}

#[derive(Clone, Copy)]
struct SnapshotReviewCategoryEntry {
    category: SnapshotReviewCategory,
    conflicting: bool,
}

#[derive(Default)]
struct SnapshotReviewCategoryIndex {
    roots: HashMap<PathBuf, SnapshotReviewCategoryEntry>,
}

impl SnapshotReviewCategoryIndex {
    fn from_roots(roots: Vec<SnapshotReviewCategoryRoot>) -> Self {
        let Some(total_bytes) = roots.iter().try_fold(0_usize, |total, root| {
            total.checked_add(category_path_bytes(&root.path))
        }) else {
            return Self::default();
        };
        if roots.len() > MAX_SNAPSHOT_REVIEW_CATEGORY_ROOTS
            || total_bytes > MAX_SNAPSHOT_REVIEW_CATEGORY_BYTES
        {
            return Self::default();
        }
        let mut index = HashMap::new();
        if index.try_reserve(roots.len()).is_err() {
            return Self::default();
        }
        for root in roots {
            let Some(path) = normalize_category_path(&root.path) else {
                continue;
            };
            index
                .entry(path)
                .and_modify(|entry: &mut SnapshotReviewCategoryEntry| {
                    if entry.category != root.category {
                        entry.conflicting = true;
                    }
                })
                .or_insert(SnapshotReviewCategoryEntry {
                    category: root.category,
                    conflicting: false,
                });
        }
        Self { roots: index }
    }

    fn clear(&mut self) {
        self.roots = HashMap::new();
    }
}

pub(super) fn category_path_bytes(path: &Path) -> usize {
    path.as_os_str().len()
}

impl SnapshotReviewCategoryRoot {
    pub(super) fn new(path: PathBuf, category: CandidateCategory) -> Self {
        Self {
            path,
            category: category.into(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapshotReviewNameEncoding {
    UnixBytes,
    WindowsUtf16LittleEndian,
}

/// Lossless historical root/name observation for display and navigation only.
/// It is deliberately not a current path or cleanup authority.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapshotReviewName {
    pub encoding: SnapshotReviewNameEncoding,
    pub encoded_bytes: Arc<[u8]>,
    pub display: Arc<str>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SnapshotReviewTimestamp {
    pub seconds_since_unix_epoch: u64,
    pub nanoseconds: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SnapshotReviewScanFlags {
    pub inaccessible: bool,
    pub timed_out: bool,
    pub hard_link_duplicate: bool,
    pub mount_boundary: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapshotReviewNode {
    pub id: u64,
    pub parent_id: Option<u64>,
    pub depth: u32,
    pub kind: SnapshotReviewNodeKind,
    pub category: SnapshotReviewCategory,
    pub name: SnapshotReviewName,
    pub logical_bytes: u64,
    pub allocated_bytes: Option<u64>,
    pub file_count: u64,
    pub child_count: u64,
    pub modified_at: Option<SnapshotReviewTimestamp>,
    pub accessed_at: Option<SnapshotReviewTimestamp>,
    pub scan_flags: SnapshotReviewScanFlags,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapshotReviewNodePage {
    pub parent_id: u64,
    pub offset: u64,
    pub total_children: u64,
    pub has_more: bool,
    pub nodes: Vec<SnapshotReviewNode>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapshotReviewTreemapCell {
    pub node: SnapshotReviewNode,
    /// Zero-based position in the complete logical-bytes-descending direct-
    /// child ordering for this parent.
    pub logical_rank: u64,
}

/// Bounded direct-child logical-size projection for one retained snapshot.
///
/// `other_*` accounts exactly for every child omitted from `cells`, including
/// zero-logical-byte observations. This remains historical display data and
/// carries no live path or cleanup authority.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapshotReviewTreemap {
    pub parent_id: u64,
    pub total_children: u64,
    pub total_child_logical_bytes: u64,
    pub other_child_count: u64,
    pub other_logical_bytes: u64,
    pub zero_logical_child_count: u64,
    pub cells: Vec<SnapshotReviewTreemapCell>,
}

/// One file observed in a retained snapshot, with bounded historical display
/// context. The context excludes the scan root and grants no live path or
/// cleanup authority.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapshotReviewLargeFile {
    pub node: SnapshotReviewNode,
    /// Root-to-parent historical name components, excluding the scan root.
    pub parent_context: Vec<SnapshotReviewName>,
    pub context_truncated: bool,
}

/// Exact match accounting plus a bounded deterministic top-file projection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapshotReviewLargeFilePage {
    pub total_matching_files: u64,
    pub total_matching_logical_bytes: u64,
    pub has_more: bool,
    pub files: Vec<SnapshotReviewLargeFile>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapshotReviewLiveTargetPurpose {
    Reveal,
    CopyPath,
    QuickLook,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapshotReviewLiveTargetKind {
    Directory,
    File,
}

/// A current path whose complete ancestor chain and target identity matched
/// this retained snapshot moments before return. It is read-only action
/// evidence and never cleanup authority; filesystem state can change again
/// immediately after this value is produced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapshotReviewLiveTarget {
    pub node_id: u64,
    pub purpose: SnapshotReviewLiveTargetPurpose,
    pub kind: SnapshotReviewLiveTargetKind,
    pub path: PathBuf,
}

/// Unforgeable owner shared by one engine and the reviews it created.
pub(super) struct SnapshotReviewOwner;

/// Current, no-follow directory evidence sealed inside the core. The path is
/// never transported and the identity must be revalidated by the scan worker.
pub(super) struct SnapshotReviewSubtreeTarget {
    pub(super) path: PathBuf,
    pub(super) identity: FilesystemIdentity,
}

/// Stable, path-free failures from an Explorer snapshot-review session.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum SnapshotReviewError {
    #[error("engine session is closed")]
    Closed,
    #[error("the scan does not exist")]
    ScanNotFound,
    #[error("the scan has no available snapshot")]
    SnapshotUnavailable,
    #[error("the review lease expired")]
    LeaseExpired,
    #[error("the snapshot node does not exist")]
    NodeNotFound,
    #[error("the snapshot node is not a directory")]
    NodeNotDirectory,
    #[error("the snapshot node page is invalid")]
    InvalidPage,
    #[error("the snapshot treemap cell budget is invalid")]
    InvalidTreemapBudget,
    #[error("the snapshot large-file request is invalid")]
    InvalidLargeFileRequest,
    #[error("the requested platform action is unsupported for this snapshot item")]
    LiveTargetUnsupported,
    #[error("the snapshot item has no safely usable current path")]
    LivePathUnavailable,
    #[error("the snapshot item no longer exists at its observed location")]
    LivePathMissing,
    #[error("the snapshot item or one of its ancestors is now a symbolic link")]
    LivePathSymlink,
    #[error("the snapshot item now crosses a filesystem boundary")]
    LivePathCrossVolume,
    #[error("the current filesystem item no longer matches the snapshot")]
    LivePathChanged,
    #[error("the current filesystem item cannot be inspected")]
    LivePathAccessDenied,
    #[error("the durable engine store is read-only")]
    ReadOnlyStore,
    #[error("the durable engine schema is incompatible")]
    IncompatibleSchema,
    #[error("snapshot review is temporarily busy")]
    Busy,
    #[error("snapshot storage failed its safety checks")]
    UnsafeStorage,
    #[error("the bounded review operation exceeded its budget")]
    BudgetExceeded,
    #[error("durable snapshot state is corrupt")]
    CorruptData,
    #[error("the snapshot format is incompatible")]
    IncompatibleSnapshot,
    #[error("snapshot review storage is unavailable")]
    Unavailable,
    #[error("the review operation outcome is unknown")]
    OutcomeUnknown,
    #[error("snapshot review reached an invalid internal state")]
    InternalState,
}

/// Result of explicitly ending an Explorer review session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapshotReviewReleaseOutcome {
    Released,
    AlreadyReleased,
}

/// One exact expiring review pin and retained immutable snapshot handle.
///
/// This type intentionally exposes no digest, file handle, or raw decoded
/// tree. Its only live-path projection is purpose-bound, freshly identity
/// checked, and read-only; it never grants cleanup authority.
/// Dropping it closes local resources without entering SQLite; callers should
/// explicitly release it when review ends.
#[must_use = "retain the session while Explorer is reviewing the snapshot"]
pub struct SnapshotReviewSession {
    owner: Arc<SnapshotReviewOwner>,
    scan_id: ScanId,
    lease: Option<StoredReviewLease>,
    document: Option<StoredReviewDocument>,
    sorted_children: Option<SortedChildCache>,
    category_index: SnapshotReviewCategoryIndex,
}

struct SortedChildCache {
    parent_id: u64,
    sort: SnapshotReviewNodeSort,
    indices: Vec<u32>,
}

impl SnapshotReviewSession {
    pub(super) fn new(
        owner: Arc<SnapshotReviewOwner>,
        scan_id: ScanId,
        lease: StoredReviewLease,
        category_roots: Vec<SnapshotReviewCategoryRoot>,
    ) -> Self {
        Self {
            owner,
            scan_id,
            lease: Some(lease),
            document: None,
            sorted_children: None,
            category_index: SnapshotReviewCategoryIndex::from_roots(category_roots),
        }
    }

    pub fn scan_id(&self) -> &ScanId {
        &self.scan_id
    }

    pub(super) fn belongs_to(&self, owner: &Arc<SnapshotReviewOwner>) -> bool {
        Arc::ptr_eq(&self.owner, owner)
    }

    /// Resolve a historical directory into a sealed current scan target. This
    /// deliberately reuses the strict live-target ancestry/identity checks but
    /// does not expose the resulting path outside the Rust core.
    pub(super) fn subtree_scan_target(
        &mut self,
        node_id: u64,
    ) -> Result<SnapshotReviewSubtreeTarget, SnapshotReviewError> {
        self.ensure_document(SystemTime::now())?;
        let document = self
            .document
            .as_deref()
            .ok_or(SnapshotReviewError::InternalState)?;
        let node_index = usize::try_from(node_id).map_err(|_| SnapshotReviewError::NodeNotFound)?;
        let expected_identity = document
            .nodes
            .get(node_index)
            .filter(|node| node.id == node_id)
            .ok_or(SnapshotReviewError::NodeNotFound)?
            .unix_identity
            .ok_or(SnapshotReviewError::LivePathUnavailable)?;
        let target =
            resolve_live_target(document, node_id, SnapshotReviewLiveTargetPurpose::Reveal)?;
        if target.kind != SnapshotReviewLiveTargetKind::Directory {
            return Err(SnapshotReviewError::NodeNotDirectory);
        }
        let lexical = validate_scan_root(&target.path)
            .map_err(|_| SnapshotReviewError::LivePathUnavailable)?;
        let current = capture_scan_root(lexical).map_err(map_live_path_error)?;
        if current.identity().volume() != expected_identity.device()
            || current.identity().object() != u128::from(expected_identity.inode())
        {
            return Err(SnapshotReviewError::LivePathChanged);
        }
        // The additional root capture both supplies the worker fence and
        // closes the interval after descendant validation.
        self.ensure_document(SystemTime::now())?;
        Ok(SnapshotReviewSubtreeTarget {
            path: current.canonical_path().to_path_buf(),
            identity: current.identity(),
        })
    }

    pub fn expires_at(&self) -> Result<SystemTime, SnapshotReviewError> {
        self.lease
            .as_ref()
            .ok_or(SnapshotReviewError::LeaseExpired)?
            .expires_at()
            .map_err(|error| map_repository_error(error.kind))
    }

    pub fn expires_at_unix_ms(&self) -> Result<i64, SnapshotReviewError> {
        let duration = self
            .expires_at()?
            .duration_since(UNIX_EPOCH)
            .map_err(|_| SnapshotReviewError::InternalState)?;
        i64::try_from(duration.as_millis()).map_err(|_| SnapshotReviewError::InternalState)
    }

    /// Renew this exact unexpired lease using the core-owned clock.
    pub fn renew(&mut self) -> Result<SystemTime, SnapshotReviewError> {
        self.renew_at(SystemTime::now())
    }

    pub fn renew_unix_ms(&mut self) -> Result<i64, SnapshotReviewError> {
        self.renew()?;
        self.expires_at_unix_ms()
    }

    pub fn release(&mut self) -> Result<SnapshotReviewReleaseOutcome, SnapshotReviewError> {
        self.document = None;
        self.sorted_children = None;
        self.category_index.clear();
        let Some(lease) = self.lease.take() else {
            return Ok(SnapshotReviewReleaseOutcome::AlreadyReleased);
        };
        lease
            .release()
            .map_err(|error| map_repository_error(error.kind))?;
        Ok(SnapshotReviewReleaseOutcome::Released)
    }

    pub fn is_released(&self) -> bool {
        self.lease.is_none()
    }

    /// Return the snapshot root after revalidating the live review lease.
    pub fn root_node(&mut self) -> Result<SnapshotReviewNode, SnapshotReviewError> {
        self.ensure_document(SystemTime::now())?;
        let document = self
            .document
            .as_deref()
            .ok_or(SnapshotReviewError::InternalState)?;
        let root = document
            .nodes
            .first()
            .ok_or(SnapshotReviewError::CorruptData)?;
        let category = category_for_node(document, root, &self.category_index)?;
        Ok(project_node(root, &document.metadata.root, category))
    }

    /// Return one deterministic bounded page of direct children.
    pub fn child_nodes(
        &mut self,
        parent_id: u64,
        sort: SnapshotReviewNodeSort,
        offset: u64,
        limit: u16,
    ) -> Result<SnapshotReviewNodePage, SnapshotReviewError> {
        if limit == 0 || limit > MAX_SNAPSHOT_REVIEW_NODE_PAGE_LIMIT {
            return Err(SnapshotReviewError::InvalidPage);
        }
        self.ensure_document(SystemTime::now())?;
        let rebuild_cache = self
            .sorted_children
            .as_ref()
            .is_none_or(|cache| cache.parent_id != parent_id || cache.sort != sort);
        if rebuild_cache {
            let document = self
                .document
                .as_ref()
                .ok_or(SnapshotReviewError::InternalState)?;
            let indices = sorted_child_indices(document, parent_id, sort, offset)?;
            self.sorted_children = Some(SortedChildCache {
                parent_id,
                sort,
                indices,
            });
        }
        let document = self
            .document
            .as_deref()
            .ok_or(SnapshotReviewError::InternalState)?;
        let cache = self
            .sorted_children
            .as_ref()
            .ok_or(SnapshotReviewError::InternalState)?;
        build_child_page(
            document,
            &cache.indices,
            parent_id,
            offset,
            limit,
            &self.category_index,
        )
    }

    /// Return a bounded, deterministic direct-child logical-size projection.
    pub fn treemap(
        &mut self,
        parent_id: u64,
        max_cells: u16,
    ) -> Result<SnapshotReviewTreemap, SnapshotReviewError> {
        if max_cells == 0 || max_cells > MAX_SNAPSHOT_REVIEW_TREEMAP_CELLS {
            return Err(SnapshotReviewError::InvalidTreemapBudget);
        }
        self.ensure_document(SystemTime::now())?;
        let sort = SnapshotReviewNodeSort::LogicalBytesDescending;
        let rebuild_cache = self
            .sorted_children
            .as_ref()
            .is_none_or(|cache| cache.parent_id != parent_id || cache.sort != sort);
        if rebuild_cache {
            let document = self
                .document
                .as_ref()
                .ok_or(SnapshotReviewError::InternalState)?;
            let indices = sorted_child_indices(document, parent_id, sort, 0)?;
            self.sorted_children = Some(SortedChildCache {
                parent_id,
                sort,
                indices,
            });
        }
        let document = self
            .document
            .as_deref()
            .ok_or(SnapshotReviewError::InternalState)?;
        let cache = self
            .sorted_children
            .as_ref()
            .ok_or(SnapshotReviewError::InternalState)?;
        build_treemap(
            document,
            &cache.indices,
            parent_id,
            max_cells,
            &self.category_index,
        )
    }

    /// Return the largest matching regular files from this exact retained
    /// snapshot. This is read-only historical evidence, never cleanup
    /// authority.
    pub fn large_files(
        &mut self,
        minimum_logical_bytes: u64,
        modified_before: Option<SnapshotReviewTimestamp>,
        max_results: u16,
    ) -> Result<SnapshotReviewLargeFilePage, SnapshotReviewError> {
        if minimum_logical_bytes == 0
            || max_results == 0
            || max_results > MAX_SNAPSHOT_REVIEW_LARGE_FILE_RESULTS
            || modified_before.is_some_and(|timestamp| timestamp.nanoseconds >= 1_000_000_000)
        {
            return Err(SnapshotReviewError::InvalidLargeFileRequest);
        }
        self.ensure_document(SystemTime::now())?;
        let document = self
            .document
            .as_deref()
            .ok_or(SnapshotReviewError::InternalState)?;
        let page = build_large_file_page(
            document,
            minimum_logical_bytes,
            modified_before,
            max_results,
            &self.category_index,
        )?;
        // Revalidate after the O(n) projection so an expiring lease cannot
        // authorize returning results computed past its lifetime.
        self.ensure_document(SystemTime::now())?;
        Ok(page)
    }

    /// Resolve one snapshot node to a freshly identity-checked current path.
    /// Symlinks, special entries, missing identities, host-incompatible paths,
    /// and any changed ancestor are rejected instead of returning a guessed
    /// path.
    pub fn live_target(
        &mut self,
        node_id: u64,
        purpose: SnapshotReviewLiveTargetPurpose,
    ) -> Result<SnapshotReviewLiveTarget, SnapshotReviewError> {
        self.ensure_document(SystemTime::now())?;
        let document = self
            .document
            .as_deref()
            .ok_or(SnapshotReviewError::InternalState)?;
        let live_target = resolve_live_target(document, node_id, purpose)?;
        // Filesystem validation may take time. The immutable snapshot must
        // still be pinned when its match evidence crosses the API boundary.
        self.ensure_document(SystemTime::now())?;
        Ok(live_target)
    }

    fn renew_at(&mut self, observed_at: SystemTime) -> Result<SystemTime, SnapshotReviewError> {
        let result = self
            .lease
            .as_mut()
            .ok_or(SnapshotReviewError::LeaseExpired)?
            .renew(observed_at)
            .map_err(|error| map_repository_error(error.kind));
        if result.is_err() {
            self.document = None;
            self.sorted_children = None;
            self.category_index.clear();
        }
        result
    }

    fn ensure_document(&mut self, observed_at: SystemTime) -> Result<(), SnapshotReviewError> {
        let lease = self
            .lease
            .as_ref()
            .ok_or(SnapshotReviewError::LeaseExpired)?;
        if self.document.is_none() {
            let document = match lease
                .load_for_review(observed_at)
                .map_err(|error| map_repository_error(error.kind))
            {
                Ok(document) => document,
                Err(error) => {
                    self.category_index.clear();
                    return Err(error);
                }
            };
            self.document = Some(document);
        } else {
            if let Err(error) = lease
                .validate(observed_at)
                .map_err(|error| map_repository_error(error.kind))
            {
                self.document = None;
                self.sorted_children = None;
                self.category_index.clear();
                return Err(error);
            }
        }
        Ok(())
    }

    #[cfg(test)]
    pub(super) fn renew_at_for_test(
        &mut self,
        observed_at: SystemTime,
    ) -> Result<SystemTime, SnapshotReviewError> {
        self.renew_at(observed_at)
    }

    #[cfg(test)]
    pub(super) fn category_root_count_for_test(&self) -> usize {
        self.category_index.roots.len()
    }
}

fn resolve_live_target(
    document: &SnapshotDocument,
    node_id: u64,
    purpose: SnapshotReviewLiveTargetPurpose,
) -> Result<SnapshotReviewLiveTarget, SnapshotReviewError> {
    let target_index = usize::try_from(node_id).map_err(|_| SnapshotReviewError::NodeNotFound)?;
    let target = document
        .nodes
        .get(target_index)
        .filter(|node| node.id == node_id)
        .ok_or(SnapshotReviewError::NodeNotFound)?;
    let kind = match target.kind {
        SnapshotNodeKind::Directory => SnapshotReviewLiveTargetKind::Directory,
        SnapshotNodeKind::File => SnapshotReviewLiveTargetKind::File,
        SnapshotNodeKind::Symlink | SnapshotNodeKind::Other | SnapshotNodeKind::Error => {
            return Err(SnapshotReviewError::LiveTargetUnsupported);
        }
    };
    if purpose == SnapshotReviewLiveTargetPurpose::QuickLook
        && kind != SnapshotReviewLiveTargetKind::File
    {
        return Err(SnapshotReviewError::LiveTargetUnsupported);
    }

    let mut chain = Vec::new();
    chain
        .try_reserve_exact(usize::try_from(target.depth).unwrap_or(0).saturating_add(1))
        .map_err(|_| SnapshotReviewError::BudgetExceeded)?;
    let mut current_id = Some(node_id);
    while let Some(id) = current_id {
        let index = usize::try_from(id).map_err(|_| SnapshotReviewError::CorruptData)?;
        let node = document
            .nodes
            .get(index)
            .filter(|node| node.id == id)
            .ok_or(SnapshotReviewError::CorruptData)?;
        chain.push(node);
        current_id = node.parent;
    }
    chain.reverse();
    if chain.first().is_none_or(|node| node.id != 0)
        || chain.len()
            != usize::try_from(target.depth)
                .unwrap_or(usize::MAX)
                .saturating_add(1)
    {
        return Err(SnapshotReviewError::CorruptData);
    }

    let root_path = document
        .metadata
        .root
        .to_path_buf()
        .map_err(|_| SnapshotReviewError::CorruptData)?;
    let lexical_root =
        validate_scan_root(&root_path).map_err(|_| SnapshotReviewError::LivePathUnavailable)?;
    let live_root = capture_scan_root(lexical_root.clone()).map_err(map_live_path_error)?;
    ensure_live_identity(chain[0], live_root.identity())?;

    if node_id == 0 {
        return Ok(SnapshotReviewLiveTarget {
            node_id,
            purpose,
            kind,
            path: live_root.canonical_path().to_path_buf(),
        });
    }

    let mut requested_path = root_path;
    for node in chain.iter().skip(1) {
        let component = node
            .name
            .as_ref()
            .ok_or(SnapshotReviewError::CorruptData)?
            .to_path_buf()
            .map_err(|_| SnapshotReviewError::CorruptData)?;
        requested_path.push(component);
    }
    let lexical_target = validate_cleanup_path(&lexical_root, &requested_path)
        .map_err(|_| SnapshotReviewError::LivePathUnavailable)?;
    let live = capture_path_snapshot(&live_root, lexical_target).map_err(map_live_path_error)?;
    let expected_kind = match kind {
        SnapshotReviewLiveTargetKind::Directory => FilesystemEntryKind::Directory,
        SnapshotReviewLiveTargetKind::File => FilesystemEntryKind::RegularFile,
    };
    if live.target_kind() != expected_kind {
        return Err(SnapshotReviewError::LivePathChanged);
    }
    if live.ancestors().len() != chain.len() - 1 {
        return Err(SnapshotReviewError::LivePathChanged);
    }
    for (snapshot_node, live_ancestor) in chain.iter().zip(live.ancestors()) {
        ensure_live_identity(snapshot_node, live_ancestor.identity())?;
    }
    ensure_live_identity(target, live.target_identity())?;

    Ok(SnapshotReviewLiveTarget {
        node_id,
        purpose,
        kind,
        path: live.canonical_path().to_path_buf(),
    })
}

fn ensure_live_identity(
    node: &SnapshotNode,
    live: crate::path_validation::FilesystemIdentity,
) -> Result<(), SnapshotReviewError> {
    let expected = node
        .unix_identity
        .ok_or(SnapshotReviewError::LivePathUnavailable)?;
    if expected.device() != live.volume() || u128::from(expected.inode()) != live.object() {
        return Err(SnapshotReviewError::LivePathChanged);
    }
    Ok(())
}

fn map_live_path_error(error: CanonicalPathError) -> SnapshotReviewError {
    match error {
        CanonicalPathError::AccessDenied { .. } => SnapshotReviewError::LivePathAccessDenied,
        CanonicalPathError::Missing { .. } => SnapshotReviewError::LivePathMissing,
        CanonicalPathError::SymlinkOrReparsePoint { .. } => SnapshotReviewError::LivePathSymlink,
        CanonicalPathError::CrossVolume { .. } => SnapshotReviewError::LivePathCrossVolume,
        CanonicalPathError::ChangedDuringValidation { .. }
        | CanonicalPathError::MismatchedScanRoot
        | CanonicalPathError::CanonicalEscapesScanRoot
        | CanonicalPathError::CanonicalPathMismatch { .. }
        | CanonicalPathError::ScanRootNotDirectory
        | CanonicalPathError::NonDirectoryAncestor { .. }
        | CanonicalPathError::UnsupportedTargetKind => SnapshotReviewError::LivePathChanged,
        CanonicalPathError::Io { kind, .. } => match kind {
            std::io::ErrorKind::NotFound => SnapshotReviewError::LivePathMissing,
            std::io::ErrorKind::PermissionDenied => SnapshotReviewError::LivePathAccessDenied,
            _ => SnapshotReviewError::LivePathUnavailable,
        },
        CanonicalPathError::CanonicalizationFailed { source, .. } => match source.kind() {
            std::io::ErrorKind::NotFound => SnapshotReviewError::LivePathMissing,
            std::io::ErrorKind::PermissionDenied => SnapshotReviewError::LivePathAccessDenied,
            _ => SnapshotReviewError::LivePathUnavailable,
        },
        CanonicalPathError::IdentityUnavailable { .. }
        | CanonicalPathError::UnsupportedPlatform => SnapshotReviewError::LivePathUnavailable,
    }
}

fn sorted_child_indices(
    document: &StoredReviewDocument,
    parent_id: u64,
    sort: SnapshotReviewNodeSort,
    offset: u64,
) -> Result<Vec<u32>, SnapshotReviewError> {
    let parent_index = usize::try_from(parent_id).map_err(|_| SnapshotReviewError::NodeNotFound)?;
    let parent = document
        .nodes
        .get(parent_index)
        .filter(|node| node.id == parent_id)
        .ok_or(SnapshotReviewError::NodeNotFound)?;
    if parent.kind != SnapshotNodeKind::Directory {
        return Err(SnapshotReviewError::NodeNotDirectory);
    }
    if offset > parent.child_count {
        return Err(SnapshotReviewError::InvalidPage);
    }
    if parent.child_count > MAX_SNAPSHOT_REVIEW_SORTABLE_CHILDREN {
        return Err(SnapshotReviewError::BudgetExceeded);
    }
    let child_capacity =
        usize::try_from(parent.child_count).map_err(|_| SnapshotReviewError::BudgetExceeded)?;
    let direct = document
        .direct_child_indices(parent_index)
        .map_err(|error| map_repository_error(error.kind))?;
    if direct.len() != child_capacity {
        return Err(SnapshotReviewError::CorruptData);
    }
    let mut child_indices = Vec::<u32>::new();
    child_indices
        .try_reserve_exact(child_capacity)
        .map_err(|_| SnapshotReviewError::BudgetExceeded)?;
    child_indices.extend_from_slice(direct);
    child_indices.sort_unstable_by(|left, right| {
        compare_nodes(
            &document.nodes[*left as usize],
            &document.nodes[*right as usize],
            sort,
        )
    });
    Ok(child_indices)
}

fn build_child_page(
    document: &SnapshotDocument,
    child_indices: &[u32],
    parent_id: u64,
    offset: u64,
    limit: u16,
    category_index: &SnapshotReviewCategoryIndex,
) -> Result<SnapshotReviewNodePage, SnapshotReviewError> {
    let parent_index = usize::try_from(parent_id).map_err(|_| SnapshotReviewError::NodeNotFound)?;
    let parent = document
        .nodes
        .get(parent_index)
        .filter(|node| node.id == parent_id)
        .ok_or(SnapshotReviewError::NodeNotFound)?;
    let start = usize::try_from(offset).map_err(|_| SnapshotReviewError::InvalidPage)?;
    if start > child_indices.len() {
        return Err(SnapshotReviewError::InvalidPage);
    }
    let end = start
        .saturating_add(usize::from(limit))
        .min(child_indices.len());
    let mut nodes = Vec::new();
    nodes
        .try_reserve_exact(end - start)
        .map_err(|_| SnapshotReviewError::BudgetExceeded)?;
    for index in &child_indices[start..end] {
        let node = &document.nodes[*index as usize];
        let name = node.name.as_ref().ok_or(SnapshotReviewError::CorruptData)?;
        let category = category_for_node(document, node, category_index)?;
        nodes.push(project_node(node, name, category));
    }
    Ok(SnapshotReviewNodePage {
        parent_id,
        offset,
        total_children: parent.child_count,
        has_more: end < child_indices.len(),
        nodes,
    })
}

fn build_treemap(
    document: &SnapshotDocument,
    child_indices: &[u32],
    parent_id: u64,
    max_cells: u16,
    category_index: &SnapshotReviewCategoryIndex,
) -> Result<SnapshotReviewTreemap, SnapshotReviewError> {
    let parent_index = usize::try_from(parent_id).map_err(|_| SnapshotReviewError::NodeNotFound)?;
    let parent = document
        .nodes
        .get(parent_index)
        .filter(|node| node.id == parent_id)
        .ok_or(SnapshotReviewError::NodeNotFound)?;
    if parent.kind != SnapshotNodeKind::Directory {
        return Err(SnapshotReviewError::NodeNotDirectory);
    }
    let total_children =
        u64::try_from(child_indices.len()).map_err(|_| SnapshotReviewError::BudgetExceeded)?;
    if total_children != parent.child_count {
        return Err(SnapshotReviewError::CorruptData);
    }

    let mut total_child_logical_bytes = 0_u64;
    let mut zero_logical_child_count = 0_u64;
    for index in child_indices {
        let node = document
            .nodes
            .get(*index as usize)
            .ok_or(SnapshotReviewError::CorruptData)?;
        total_child_logical_bytes = total_child_logical_bytes
            .checked_add(node.logical_bytes)
            .ok_or(SnapshotReviewError::CorruptData)?;
        if node.logical_bytes == 0 {
            zero_logical_child_count = zero_logical_child_count
                .checked_add(1)
                .ok_or(SnapshotReviewError::CorruptData)?;
        }
    }

    let cell_capacity = usize::from(max_cells).min(child_indices.len());
    let mut cells = Vec::new();
    cells
        .try_reserve_exact(cell_capacity)
        .map_err(|_| SnapshotReviewError::BudgetExceeded)?;
    let mut represented_logical_bytes = 0_u64;
    for (rank, index) in child_indices.iter().enumerate() {
        if cells.len() == cell_capacity {
            break;
        }
        let node = document
            .nodes
            .get(*index as usize)
            .ok_or(SnapshotReviewError::CorruptData)?;
        if node.logical_bytes == 0 {
            break;
        }
        let name = node.name.as_ref().ok_or(SnapshotReviewError::CorruptData)?;
        represented_logical_bytes = represented_logical_bytes
            .checked_add(node.logical_bytes)
            .ok_or(SnapshotReviewError::CorruptData)?;
        let category = category_for_node(document, node, category_index)?;
        cells.push(SnapshotReviewTreemapCell {
            node: project_node(node, name, category),
            logical_rank: u64::try_from(rank).map_err(|_| SnapshotReviewError::BudgetExceeded)?,
        });
    }

    let represented_children =
        u64::try_from(cells.len()).map_err(|_| SnapshotReviewError::BudgetExceeded)?;
    Ok(SnapshotReviewTreemap {
        parent_id,
        total_children,
        total_child_logical_bytes,
        other_child_count: total_children
            .checked_sub(represented_children)
            .ok_or(SnapshotReviewError::CorruptData)?,
        other_logical_bytes: total_child_logical_bytes
            .checked_sub(represented_logical_bytes)
            .ok_or(SnapshotReviewError::CorruptData)?,
        zero_logical_child_count,
        cells,
    })
}

#[derive(Clone, Copy)]
struct LargeFileCandidate<'document> {
    node: &'document SnapshotNode,
}

impl PartialEq for LargeFileCandidate<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.node.id == other.node.id
    }
}

impl Eq for LargeFileCandidate<'_> {}

impl PartialOrd for LargeFileCandidate<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for LargeFileCandidate<'_> {
    fn cmp(&self, other: &Self) -> Ordering {
        compare_large_file_nodes(self.node, other.node)
    }
}

fn build_large_file_page(
    document: &SnapshotDocument,
    minimum_logical_bytes: u64,
    modified_before: Option<SnapshotReviewTimestamp>,
    max_results: u16,
    category_index: &SnapshotReviewCategoryIndex,
) -> Result<SnapshotReviewLargeFilePage, SnapshotReviewError> {
    let capacity = usize::from(max_results);
    let mut selected = BinaryHeap::new();
    selected
        .try_reserve(capacity)
        .map_err(|_| SnapshotReviewError::BudgetExceeded)?;
    let mut total_matching_files = 0_u64;
    let mut total_matching_logical_bytes = 0_u64;

    for node in &document.nodes {
        if node.kind != SnapshotNodeKind::File
            || node.logical_bytes < minimum_logical_bytes
            || !modified_before_matches(node.modified_at, modified_before)
        {
            continue;
        }
        if node.name.is_none() {
            return Err(SnapshotReviewError::CorruptData);
        }
        total_matching_files = total_matching_files
            .checked_add(1)
            .ok_or(SnapshotReviewError::BudgetExceeded)?;
        total_matching_logical_bytes = total_matching_logical_bytes
            .checked_add(node.logical_bytes)
            .ok_or(SnapshotReviewError::CorruptData)?;
        let candidate = LargeFileCandidate { node };
        if selected.len() < capacity {
            selected.push(candidate);
        } else if selected
            .peek()
            .is_some_and(|worst| candidate.cmp(worst) == Ordering::Less)
        {
            selected.pop();
            selected.push(candidate);
        }
    }

    let mut selected = selected.into_vec();
    selected.sort_unstable();
    let mut files = Vec::new();
    files
        .try_reserve_exact(selected.len())
        .map_err(|_| SnapshotReviewError::BudgetExceeded)?;
    for candidate in selected {
        let name = candidate
            .node
            .name
            .as_ref()
            .ok_or(SnapshotReviewError::CorruptData)?;
        let (parent_context, context_truncated) = build_parent_context(document, candidate.node)?;
        let category = category_for_node(document, candidate.node, category_index)?;
        files.push(SnapshotReviewLargeFile {
            node: project_node(candidate.node, name, category),
            parent_context,
            context_truncated,
        });
    }

    Ok(SnapshotReviewLargeFilePage {
        total_matching_files,
        total_matching_logical_bytes,
        has_more: total_matching_files
            > u64::try_from(files.len()).map_err(|_| SnapshotReviewError::BudgetExceeded)?,
        files,
    })
}

fn modified_before_matches(
    observed: Option<SnapshotTimestamp>,
    cutoff: Option<SnapshotReviewTimestamp>,
) -> bool {
    match cutoff {
        None => true,
        Some(cutoff) => observed.is_some_and(|observed| {
            timestamp_key(observed) < (cutoff.seconds_since_unix_epoch, cutoff.nanoseconds)
        }),
    }
}

fn build_parent_context(
    document: &SnapshotDocument,
    file: &SnapshotNode,
) -> Result<(Vec<SnapshotReviewName>, bool), SnapshotReviewError> {
    let mut reverse_context = Vec::new();
    reverse_context
        .try_reserve_exact(MAX_SNAPSHOT_REVIEW_PARENT_CONTEXT_COMPONENTS)
        .map_err(|_| SnapshotReviewError::BudgetExceeded)?;
    let mut next_parent = file.parent;

    while let Some(parent_id) = next_parent {
        let parent_index =
            usize::try_from(parent_id).map_err(|_| SnapshotReviewError::CorruptData)?;
        let parent = document
            .nodes
            .get(parent_index)
            .filter(|node| node.id == parent_id && node.kind == SnapshotNodeKind::Directory)
            .ok_or(SnapshotReviewError::CorruptData)?;
        if parent.id == 0 {
            reverse_context.reverse();
            return Ok((reverse_context, false));
        }
        if reverse_context.len() == MAX_SNAPSHOT_REVIEW_PARENT_CONTEXT_COMPONENTS {
            reverse_context.reverse();
            return Ok((reverse_context, true));
        }
        let name = parent
            .name
            .as_ref()
            .ok_or(SnapshotReviewError::CorruptData)?;
        reverse_context.push(project_name(name));
        next_parent = parent.parent;
    }

    Err(SnapshotReviewError::CorruptData)
}

fn compare_large_file_nodes(left: &SnapshotNode, right: &SnapshotNode) -> Ordering {
    right
        .logical_bytes
        .cmp(&left.logical_bytes)
        .then_with(|| compare_names(left, right))
        .then_with(|| left.id.cmp(&right.id))
}

fn compare_nodes(
    left: &SnapshotNode,
    right: &SnapshotNode,
    sort: SnapshotReviewNodeSort,
) -> Ordering {
    let ordering = match sort {
        SnapshotReviewNodeSort::NameAscending => compare_names(left, right),
        SnapshotReviewNodeSort::LogicalBytesDescending => right
            .logical_bytes
            .cmp(&left.logical_bytes)
            .then_with(|| compare_names(left, right)),
        SnapshotReviewNodeSort::AllocatedBytesDescending => right
            .allocated_bytes
            .cmp(&left.allocated_bytes)
            .then_with(|| right.logical_bytes.cmp(&left.logical_bytes))
            .then_with(|| compare_names(left, right)),
        SnapshotReviewNodeSort::ModifiedNewest => right
            .modified_at
            .map(timestamp_key)
            .cmp(&left.modified_at.map(timestamp_key))
            .then_with(|| compare_names(left, right)),
    };
    ordering.then_with(|| left.id.cmp(&right.id))
}

fn compare_names(left: &SnapshotNode, right: &SnapshotNode) -> Ordering {
    match (left.name.as_ref(), right.name.as_ref()) {
        (Some(left), Some(right)) => encoding_key(left.encoding())
            .cmp(&encoding_key(right.encoding()))
            .then_with(|| left.bytes().cmp(right.bytes())),
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Less,
        (Some(_), None) => Ordering::Greater,
    }
}

const fn encoding_key(encoding: HostEncoding) -> u8 {
    match encoding {
        HostEncoding::UnixBytes => 0,
        HostEncoding::WindowsUtf16Le => 1,
    }
}

const fn timestamp_key(timestamp: SnapshotTimestamp) -> (u64, u32) {
    (
        timestamp.seconds_since_unix_epoch(),
        timestamp.nanoseconds(),
    )
}

fn category_for_node(
    document: &SnapshotDocument,
    node: &SnapshotNode,
    category_index: &SnapshotReviewCategoryIndex,
) -> Result<SnapshotReviewCategory, SnapshotReviewError> {
    if category_index.roots.is_empty() {
        return Ok(SnapshotReviewCategory::Unclassified);
    }
    let scan_root = normalize_category_path(
        &document
            .metadata
            .root
            .to_path_buf()
            .map_err(|_| SnapshotReviewError::CorruptData)?,
    )
    .ok_or(SnapshotReviewError::CorruptData)?;
    let mut node_path = normalize_category_path(&historical_node_path(document, node)?)
        .ok_or(SnapshotReviewError::CorruptData)?;
    if !node_path.starts_with(&scan_root) {
        return Err(SnapshotReviewError::CorruptData);
    }
    loop {
        if let Some(entry) = category_index.roots.get(&node_path) {
            return Ok(if entry.conflicting {
                SnapshotReviewCategory::Unclassified
            } else {
                entry.category
            });
        }
        if node_path == scan_root {
            break;
        }
        if !node_path.pop() || !node_path.starts_with(&scan_root) {
            return Err(SnapshotReviewError::CorruptData);
        }
    }
    Ok(SnapshotReviewCategory::Unclassified)
}

fn normalize_category_path(path: &Path) -> Option<PathBuf> {
    if !path.is_absolute() {
        return None;
    }
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::Prefix(_)
            | std::path::Component::RootDir
            | std::path::Component::Normal(_) => normalized.push(component.as_os_str()),
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => return None,
        }
    }
    Some(normalized)
}

fn historical_node_path(
    document: &SnapshotDocument,
    target: &SnapshotNode,
) -> Result<PathBuf, SnapshotReviewError> {
    let mut path = document
        .metadata
        .root
        .to_path_buf()
        .map_err(|_| SnapshotReviewError::CorruptData)?;
    if target.id == 0 {
        return Ok(path);
    }
    let capacity =
        usize::try_from(target.depth).map_err(|_| SnapshotReviewError::BudgetExceeded)?;
    let mut components = Vec::new();
    components
        .try_reserve_exact(capacity)
        .map_err(|_| SnapshotReviewError::BudgetExceeded)?;
    let mut current = target;
    while current.id != 0 {
        components.push(
            current
                .name
                .as_ref()
                .ok_or(SnapshotReviewError::CorruptData)?,
        );
        let parent_id = current.parent.ok_or(SnapshotReviewError::CorruptData)?;
        let parent_index =
            usize::try_from(parent_id).map_err(|_| SnapshotReviewError::CorruptData)?;
        current = document
            .nodes
            .get(parent_index)
            .filter(|node| node.id == parent_id)
            .ok_or(SnapshotReviewError::CorruptData)?;
        if components.len() > capacity {
            return Err(SnapshotReviewError::CorruptData);
        }
    }
    if components.len() != capacity {
        return Err(SnapshotReviewError::CorruptData);
    }
    for component in components.into_iter().rev() {
        path.push(
            component
                .to_path_buf()
                .map_err(|_| SnapshotReviewError::CorruptData)?,
        );
    }
    Ok(path)
}

fn project_node(
    node: &SnapshotNode,
    name: &HostValue,
    category: SnapshotReviewCategory,
) -> SnapshotReviewNode {
    SnapshotReviewNode {
        id: node.id,
        parent_id: node.parent,
        depth: node.depth,
        kind: match node.kind {
            SnapshotNodeKind::Directory => SnapshotReviewNodeKind::Directory,
            SnapshotNodeKind::File => SnapshotReviewNodeKind::File,
            SnapshotNodeKind::Symlink => SnapshotReviewNodeKind::Symlink,
            SnapshotNodeKind::Other => SnapshotReviewNodeKind::Other,
            SnapshotNodeKind::Error => SnapshotReviewNodeKind::Error,
        },
        category,
        name: project_name(name),
        logical_bytes: node.logical_bytes,
        allocated_bytes: node.allocated_bytes,
        file_count: node.file_count,
        child_count: node.child_count,
        modified_at: node.modified_at.map(project_timestamp),
        accessed_at: node.accessed_at.map(project_timestamp),
        scan_flags: project_flags(node.scan_flags),
    }
}

fn project_name(name: &HostValue) -> SnapshotReviewName {
    SnapshotReviewName {
        encoding: match name.encoding() {
            HostEncoding::UnixBytes => SnapshotReviewNameEncoding::UnixBytes,
            HostEncoding::WindowsUtf16Le => SnapshotReviewNameEncoding::WindowsUtf16LittleEndian,
        },
        encoded_bytes: Arc::from(name.bytes()),
        display: Arc::from(name.display_lossy()),
    }
}

const fn project_timestamp(timestamp: SnapshotTimestamp) -> SnapshotReviewTimestamp {
    SnapshotReviewTimestamp {
        seconds_since_unix_epoch: timestamp.seconds_since_unix_epoch(),
        nanoseconds: timestamp.nanoseconds(),
    }
}

const fn project_flags(flags: SnapshotScanFlags) -> SnapshotReviewScanFlags {
    SnapshotReviewScanFlags {
        inaccessible: flags.contains(SnapshotScanFlags::INACCESSIBLE),
        timed_out: flags.contains(SnapshotScanFlags::TIMED_OUT),
        hard_link_duplicate: flags.contains(SnapshotScanFlags::HARD_LINK_DUPLICATE),
        mount_boundary: flags.contains(SnapshotScanFlags::MOUNT_BOUNDARY),
    }
}

pub(super) const fn map_repository_error(kind: SnapshotRepositoryErrorKind) -> SnapshotReviewError {
    match kind {
        SnapshotRepositoryErrorKind::ReadOnly => SnapshotReviewError::ReadOnlyStore,
        SnapshotRepositoryErrorKind::MissingStore
        | SnapshotRepositoryErrorKind::MissingSnapshot
        | SnapshotRepositoryErrorKind::SnapshotUnavailable
        | SnapshotRepositoryErrorKind::ReferenceMismatch => {
            SnapshotReviewError::SnapshotUnavailable
        }
        SnapshotRepositoryErrorKind::IncompatibleVersion => {
            SnapshotReviewError::IncompatibleSnapshot
        }
        SnapshotRepositoryErrorKind::ReviewLeaseExpired => SnapshotReviewError::LeaseExpired,
        SnapshotRepositoryErrorKind::Codec(kind) => match kind {
            SnapshotCodecErrorKind::LimitExceeded => SnapshotReviewError::BudgetExceeded,
            SnapshotCodecErrorKind::IncompatibleVersion => {
                SnapshotReviewError::IncompatibleSnapshot
            }
            SnapshotCodecErrorKind::Io => SnapshotReviewError::Unavailable,
            SnapshotCodecErrorKind::InvalidMagic
            | SnapshotCodecErrorKind::InvalidLength
            | SnapshotCodecErrorKind::ChecksumMismatch
            | SnapshotCodecErrorKind::CorruptData => SnapshotReviewError::CorruptData,
            SnapshotCodecErrorKind::InvalidInput => SnapshotReviewError::InternalState,
        },
        SnapshotRepositoryErrorKind::Storage(kind) => match kind {
            SnapshotStorageErrorKind::UnsafeRoot
            | SnapshotStorageErrorKind::UnsafeObject
            | SnapshotStorageErrorKind::UnrecognizedStore => SnapshotReviewError::UnsafeStorage,
            SnapshotStorageErrorKind::Busy => SnapshotReviewError::Busy,
            SnapshotStorageErrorKind::Unavailable => SnapshotReviewError::Unavailable,
            SnapshotStorageErrorKind::InvalidConfiguration
            | SnapshotStorageErrorKind::InternalState => SnapshotReviewError::InternalState,
        },
        SnapshotRepositoryErrorKind::History(kind) => match kind {
            HistoryErrorKind::IncompatibleSchema => SnapshotReviewError::IncompatibleSchema,
            HistoryErrorKind::QueryLimitExceeded => SnapshotReviewError::BudgetExceeded,
            HistoryErrorKind::Busy => SnapshotReviewError::Busy,
            HistoryErrorKind::UnsafeStorage => SnapshotReviewError::UnsafeStorage,
            HistoryErrorKind::CorruptData => SnapshotReviewError::CorruptData,
            HistoryErrorKind::DatabaseUnavailable => SnapshotReviewError::Unavailable,
            HistoryErrorKind::OutcomeUnknown => SnapshotReviewError::OutcomeUnknown,
            HistoryErrorKind::NotFound => SnapshotReviewError::ScanNotFound,
            HistoryErrorKind::InvalidInput
            | HistoryErrorKind::AlreadyExists
            | HistoryErrorKind::InvalidTransition
            | HistoryErrorKind::InternalState => SnapshotReviewError::InternalState,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(
        id: u64,
        name: &[u8],
        logical_bytes: u64,
        allocated_bytes: Option<u64>,
        modified_at: Option<SnapshotTimestamp>,
    ) -> SnapshotNode {
        SnapshotNode {
            id,
            parent: Some(0),
            depth: 1,
            kind: SnapshotNodeKind::File,
            name: Some(HostValue::from_encoded_bytes_for_test(
                HostEncoding::UnixBytes,
                name.to_vec(),
            )),
            logical_bytes,
            allocated_bytes,
            file_count: 1,
            child_count: 0,
            modified_at,
            accessed_at: None,
            scan_flags: SnapshotScanFlags::NONE,
            unix_identity: None,
        }
    }

    #[test]
    fn allocated_and_modified_sorts_put_unknown_last_and_use_stable_ties() {
        let older = SnapshotTimestamp::new(10, 0).unwrap();
        let newer = SnapshotTimestamp::new(20, 0).unwrap();
        let mut nodes = [
            node(3, b"unknown", 99, None, None),
            node(2, b"beta", 10, Some(20), Some(older)),
            node(1, b"alpha", 10, Some(20), Some(newer)),
            node(4, b"small", 20, Some(10), Some(newer)),
        ];

        nodes.sort_unstable_by(|left, right| {
            compare_nodes(
                left,
                right,
                SnapshotReviewNodeSort::AllocatedBytesDescending,
            )
        });
        assert_eq!(
            nodes.iter().map(|node| node.id).collect::<Vec<_>>(),
            [1, 2, 4, 3]
        );

        nodes.sort_unstable_by(|left, right| {
            compare_nodes(left, right, SnapshotReviewNodeSort::ModifiedNewest)
        });
        assert_eq!(
            nodes.iter().map(|node| node.id).collect::<Vec<_>>(),
            [1, 4, 2, 3]
        );
    }

    #[test]
    fn projection_preserves_encoded_bytes_and_explicit_lossy_display() {
        let unix_name =
            HostValue::from_encoded_bytes_for_test(HostEncoding::UnixBytes, vec![b'f', 0x80]);
        let unix = project_node(
            &node(1, b"placeholder", 0, None, None),
            &unix_name,
            SnapshotReviewCategory::Unclassified,
        );
        assert_eq!(unix.name.encoding, SnapshotReviewNameEncoding::UnixBytes);
        assert_eq!(unix.name.encoded_bytes.as_ref(), [b'f', 0x80]);
        assert_eq!(unix.name.display.as_ref(), "f\u{fffd}");

        let windows_name =
            HostValue::from_encoded_bytes_for_test(HostEncoding::WindowsUtf16Le, vec![0x00, 0xd8]);
        let windows = project_node(
            &node(2, b"placeholder", 0, None, None),
            &windows_name,
            SnapshotReviewCategory::Unclassified,
        );
        assert_eq!(
            windows.name.encoding,
            SnapshotReviewNameEncoding::WindowsUtf16LittleEndian
        );
        assert_eq!(windows.name.encoded_bytes.as_ref(), [0x00, 0xd8]);
        assert_eq!(windows.name.display.as_ref(), "\u{fffd}");
    }

    #[test]
    fn modified_before_is_strict_and_excludes_unknown_observations() {
        let cutoff = SnapshotReviewTimestamp {
            seconds_since_unix_epoch: 20,
            nanoseconds: 5,
        };
        assert!(modified_before_matches(
            Some(SnapshotTimestamp::new(20, 4).unwrap()),
            Some(cutoff)
        ));
        assert!(!modified_before_matches(
            Some(SnapshotTimestamp::new(20, 5).unwrap()),
            Some(cutoff)
        ));
        assert!(!modified_before_matches(None, Some(cutoff)));
        assert!(modified_before_matches(None, None));
    }

    #[test]
    fn every_candidate_category_has_an_explicit_display_mapping() {
        for (candidate, expected) in [
            (
                CandidateCategory::DeveloperArtifact,
                SnapshotReviewCategory::DeveloperArtifact,
            ),
            (
                CandidateCategory::ApplicationCache,
                SnapshotReviewCategory::ApplicationCache,
            ),
            (
                CandidateCategory::BrowserCache,
                SnapshotReviewCategory::BrowserCache,
            ),
            (
                CandidateCategory::LogAndDiagnostic,
                SnapshotReviewCategory::LogAndDiagnostic,
            ),
            (
                CandidateCategory::InstallerAndDownload,
                SnapshotReviewCategory::InstallerAndDownload,
            ),
            (
                CandidateCategory::DeviceAndSimulatorData,
                SnapshotReviewCategory::DeviceAndSimulatorData,
            ),
            (
                CandidateCategory::CloudFile,
                SnapshotReviewCategory::CloudFile,
            ),
            (
                CandidateCategory::LargeReviewItem,
                SnapshotReviewCategory::LargeReviewItem,
            ),
            (
                CandidateCategory::ProtectedSystemData,
                SnapshotReviewCategory::ProtectedSystemData,
            ),
            (
                CandidateCategory::UnknownStorage,
                SnapshotReviewCategory::UnknownStorage,
            ),
        ] {
            assert_eq!(SnapshotReviewCategory::from(candidate), expected);
        }
    }

    #[test]
    fn historical_categories_inherit_nearest_root_without_coloring_siblings() {
        let document = category_document();
        let roots = vec![
            SnapshotReviewCategoryRoot::new(
                PathBuf::from("/scan/projects"),
                CandidateCategory::ApplicationCache,
            ),
            SnapshotReviewCategoryRoot::new(
                PathBuf::from("/scan/projects/target"),
                CandidateCategory::DeveloperArtifact,
            ),
        ];
        let index = SnapshotReviewCategoryIndex::from_roots(roots);

        assert_eq!(
            category_for_node(&document, &document.nodes[0], &index).unwrap(),
            SnapshotReviewCategory::Unclassified
        );
        assert_eq!(
            category_for_node(&document, &document.nodes[1], &index).unwrap(),
            SnapshotReviewCategory::ApplicationCache
        );
        assert_eq!(
            category_for_node(&document, &document.nodes[2], &index).unwrap(),
            SnapshotReviewCategory::DeveloperArtifact
        );
        assert_eq!(
            category_for_node(&document, &document.nodes[3], &index).unwrap(),
            SnapshotReviewCategory::DeveloperArtifact
        );
        assert_eq!(
            category_for_node(&document, &document.nodes[4], &index).unwrap(),
            SnapshotReviewCategory::ApplicationCache
        );
        assert_eq!(
            category_for_node(&document, &document.nodes[5], &index).unwrap(),
            SnapshotReviewCategory::Unclassified
        );
    }

    #[test]
    fn equal_depth_category_conflicts_and_unavailable_joins_fail_closed() {
        let document = category_document();
        let conflicting = vec![
            SnapshotReviewCategoryRoot::new(
                PathBuf::from("/scan/projects/target"),
                CandidateCategory::DeveloperArtifact,
            ),
            SnapshotReviewCategoryRoot::new(
                PathBuf::from("/scan/projects/target"),
                CandidateCategory::BrowserCache,
            ),
        ];
        let conflicting = SnapshotReviewCategoryIndex::from_roots(conflicting);
        assert_eq!(
            category_for_node(&document, &document.nodes[3], &conflicting).unwrap(),
            SnapshotReviewCategory::Unclassified
        );
        assert_eq!(
            category_for_node(
                &document,
                &document.nodes[3],
                &SnapshotReviewCategoryIndex::default(),
            )
            .unwrap(),
            SnapshotReviewCategory::Unclassified
        );
        let outside =
            SnapshotReviewCategoryIndex::from_roots(vec![SnapshotReviewCategoryRoot::new(
                PathBuf::from("/elsewhere"),
                CandidateCategory::DeveloperArtifact,
            )]);
        assert_eq!(
            category_for_node(&document, &document.nodes[3], &outside).unwrap(),
            SnapshotReviewCategory::Unclassified
        );
    }

    #[test]
    fn category_index_rejects_payload_above_its_independent_byte_bound() {
        let oversized = PathBuf::from(format!(
            "/{}",
            "x".repeat(MAX_SNAPSHOT_REVIEW_CATEGORY_BYTES)
        ));
        let index = SnapshotReviewCategoryIndex::from_roots(vec![SnapshotReviewCategoryRoot::new(
            oversized,
            CandidateCategory::DeveloperArtifact,
        )]);
        assert!(index.roots.is_empty());
    }

    fn category_document() -> SnapshotDocument {
        use crate::persistence::snapshot::{SnapshotMetadata, SnapshotTotals};

        let nodes = vec![
            category_node(0, None, 0, SnapshotNodeKind::Directory, None, 2),
            category_node(
                1,
                Some(0),
                1,
                SnapshotNodeKind::Directory,
                Some(b"projects"),
                2,
            ),
            category_node(
                2,
                Some(1),
                2,
                SnapshotNodeKind::Directory,
                Some(b"target"),
                1,
            ),
            category_node(3, Some(2), 3, SnapshotNodeKind::File, Some(b"artifact"), 0),
            category_node(4, Some(1), 2, SnapshotNodeKind::File, Some(b"sibling"), 0),
            category_node(5, Some(0), 1, SnapshotNodeKind::File, Some(b"other"), 0),
        ];
        SnapshotDocument {
            metadata: SnapshotMetadata {
                scan_id: ScanId::new("scan:category-unit").unwrap(),
                root: HostValue::from_root(std::path::Path::new("/scan")).unwrap(),
                captured_at: SnapshotTimestamp::new(1, 0).unwrap(),
                totals: SnapshotTotals {
                    directory_count: 3,
                    file_count: 2,
                    logical_bytes: 2,
                    allocated_bytes: None,
                },
            },
            nodes,
        }
    }

    fn category_node(
        id: u64,
        parent: Option<u64>,
        depth: u32,
        kind: SnapshotNodeKind,
        name: Option<&[u8]>,
        child_count: u64,
    ) -> SnapshotNode {
        SnapshotNode {
            id,
            parent,
            depth,
            kind,
            name: name.map(|value| {
                HostValue::from_encoded_bytes_for_test(HostEncoding::UnixBytes, value.to_vec())
            }),
            logical_bytes: u64::from(kind == SnapshotNodeKind::File),
            allocated_bytes: None,
            file_count: u64::from(kind == SnapshotNodeKind::File),
            child_count,
            modified_at: None,
            accessed_at: None,
            scan_flags: SnapshotScanFlags::NONE,
            unix_identity: None,
        }
    }
}
