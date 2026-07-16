use std::collections::HashSet;
use std::path::Component;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::domain::{ScanCoverage, ScanId};
use crate::persistence::history::ScanCounts;
use crate::scanner::{CompletedScanArtifact, ScanNodeFlags, ScanObjectIdentity};
use crate::tree::{NodeId, NodeKind};

use super::{
    HostValue, MAX_SNAPSHOT_DEPTH, MAX_SNAPSHOT_NODES, SnapshotDocument, SnapshotMetadata,
    SnapshotNode, SnapshotNodeKind, SnapshotScanFlags, SnapshotTimestamp, SnapshotTotals,
    SnapshotUnixIdentity, validate_snapshot_document,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ScanSnapshotErrorKind {
    InvalidGraph,
    LimitExceeded,
    UnsupportedFollowedSymlink,
    InvalidTimestamp,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("completed scan cannot become a snapshot: {kind:?}")]
pub(crate) struct ScanSnapshotError {
    pub(crate) kind: ScanSnapshotErrorKind,
}

fn error(kind: ScanSnapshotErrorKind) -> ScanSnapshotError {
    ScanSnapshotError { kind }
}

#[derive(Debug)]
pub(crate) struct PreparedCompletedScan {
    document: SnapshotDocument,
    counts: ScanCounts,
    coverage: ScanCoverage,
}

impl PreparedCompletedScan {
    pub(crate) fn into_parts(self) -> (SnapshotDocument, ScanCounts, ScanCoverage) {
        (self.document, self.counts, self.coverage)
    }

    #[cfg(test)]
    fn document(&self) -> &SnapshotDocument {
        &self.document
    }
}

struct PendingNode {
    tree_id: NodeId,
    expected_tree_parent: Option<NodeId>,
    snapshot_parent: Option<u64>,
    depth: u32,
}

pub(crate) fn prepare_completed_scan(
    scan_id: ScanId,
    captured_at: SystemTime,
    artifact: &CompletedScanArtifact,
) -> Result<PreparedCompletedScan, ScanSnapshotError> {
    let (tree, facts, coverage) = artifact.parts();
    if facts.len() != tree.len() || tree.live_count() != tree.len() {
        return Err(error(ScanSnapshotErrorKind::InvalidGraph));
    }
    let node_count =
        u64::try_from(tree.len()).map_err(|_| error(ScanSnapshotErrorKind::LimitExceeded))?;
    if node_count > MAX_SNAPSHOT_NODES {
        return Err(error(ScanSnapshotErrorKind::LimitExceeded));
    }
    let captured_at = snapshot_timestamp(captured_at)
        .ok_or_else(|| error(ScanSnapshotErrorKind::InvalidTimestamp))?;
    let root = HostValue::from_root(tree.root_path()).map_err(map_codec_error)?;
    let mut nodes = Vec::new();
    nodes
        .try_reserve_exact(tree.len())
        .map_err(|_| error(ScanSnapshotErrorKind::LimitExceeded))?;
    let mut visited = HashSet::new();
    visited
        .try_reserve(tree.len())
        .map_err(|_| error(ScanSnapshotErrorKind::LimitExceeded))?;
    let mut pending = vec![PendingNode {
        tree_id: NodeId::ROOT,
        expected_tree_parent: None,
        snapshot_parent: None,
        depth: 0,
    }];
    let mut directory_count = 0_u64;

    while let Some(current) = pending.pop() {
        if current.depth > MAX_SNAPSHOT_DEPTH {
            return Err(error(ScanSnapshotErrorKind::LimitExceeded));
        }
        if !visited.insert(current.tree_id) {
            return Err(error(ScanSnapshotErrorKind::InvalidGraph));
        }
        let tree_node = tree
            .get(current.tree_id)
            .ok_or_else(|| error(ScanSnapshotErrorKind::InvalidGraph))?;
        let node_facts = facts
            .node(current.tree_id)
            .ok_or_else(|| error(ScanSnapshotErrorKind::InvalidGraph))?;
        if tree_node.parent != current.expected_tree_parent {
            return Err(error(ScanSnapshotErrorKind::InvalidGraph));
        }
        if tree_node.path_is_symlink && tree_node.kind != NodeKind::Symlink {
            return Err(error(ScanSnapshotErrorKind::UnsupportedFollowedSymlink));
        }

        let snapshot_id = nodes.len() as u64;
        let name = if let Some(parent_id) = current.expected_tree_parent {
            let parent = tree
                .get(parent_id)
                .ok_or_else(|| error(ScanSnapshotErrorKind::InvalidGraph))?;
            if tree_node.path.parent() != Some(parent.path.as_path()) {
                return Err(error(ScanSnapshotErrorKind::InvalidGraph));
            }
            let relative = tree_node
                .path
                .strip_prefix(&parent.path)
                .map_err(|_| error(ScanSnapshotErrorKind::InvalidGraph))?;
            let mut components = relative.components();
            let Some(Component::Normal(component)) = components.next() else {
                return Err(error(ScanSnapshotErrorKind::InvalidGraph));
            };
            if components.next().is_some() {
                return Err(error(ScanSnapshotErrorKind::InvalidGraph));
            }
            Some(HostValue::from_component(component).map_err(map_codec_error)?)
        } else {
            if current.tree_id != NodeId::ROOT || tree_node.path != tree.root_path() {
                return Err(error(ScanSnapshotErrorKind::InvalidGraph));
            }
            None
        };

        let kind = snapshot_kind(tree_node.kind);
        if kind == SnapshotNodeKind::Directory {
            directory_count = directory_count
                .checked_add(1)
                .ok_or_else(|| error(ScanSnapshotErrorKind::LimitExceeded))?;
        }
        let mut scan_flags = SnapshotScanFlags::NONE;
        if node_facts
            .flags
            .contains(ScanNodeFlags::HARD_LINK_DUPLICATE)
        {
            scan_flags = scan_flags.union(SnapshotScanFlags::HARD_LINK_DUPLICATE);
        }
        if node_facts.flags.contains(ScanNodeFlags::MOUNT_BOUNDARY) {
            scan_flags = scan_flags.union(SnapshotScanFlags::MOUNT_BOUNDARY);
        }
        let unix_identity = snapshot_unix_identity(node_facts.identity);
        let modified_at = node_facts.modified_at.and_then(snapshot_timestamp);
        let accessed_at = node_facts.accessed_at.and_then(snapshot_timestamp);
        let mut children = tree_node.children.clone();
        children.sort_by(|left, right| {
            tree.get(*left)
                .map(|node| node.path.as_path())
                .cmp(&tree.get(*right).map(|node| node.path.as_path()))
        });
        let child_count = u64::try_from(children.len())
            .map_err(|_| error(ScanSnapshotErrorKind::LimitExceeded))?;
        nodes.push(SnapshotNode {
            id: snapshot_id,
            parent: current.snapshot_parent,
            depth: current.depth,
            kind,
            name,
            logical_bytes: node_facts.logical_bytes,
            allocated_bytes: node_facts.allocated_bytes,
            file_count: tree_node.file_count,
            child_count,
            modified_at,
            accessed_at,
            scan_flags,
            unix_identity,
        });
        for child in children.into_iter().rev() {
            pending.push(PendingNode {
                tree_id: child,
                expected_tree_parent: Some(current.tree_id),
                snapshot_parent: Some(snapshot_id),
                depth: current
                    .depth
                    .checked_add(1)
                    .ok_or_else(|| error(ScanSnapshotErrorKind::LimitExceeded))?,
            });
        }
    }

    if visited.len() != tree.len() {
        return Err(error(ScanSnapshotErrorKind::InvalidGraph));
    }
    let root_facts = facts
        .node(NodeId::ROOT)
        .ok_or_else(|| error(ScanSnapshotErrorKind::InvalidGraph))?;
    let totals = SnapshotTotals {
        directory_count,
        file_count: tree.root().file_count,
        logical_bytes: root_facts.logical_bytes,
        allocated_bytes: root_facts.allocated_bytes,
    };
    let counts = ScanCounts {
        directory_count,
        file_count: totals.file_count,
        logical_bytes: totals.logical_bytes,
        allocated_bytes: totals.allocated_bytes,
    };
    counts
        .validate_for_storage()
        .map_err(|_| error(ScanSnapshotErrorKind::LimitExceeded))?;
    let document = SnapshotDocument {
        metadata: SnapshotMetadata {
            scan_id,
            root,
            captured_at,
            totals,
        },
        nodes,
    };
    validate_snapshot_document(&document).map_err(map_codec_error)?;
    Ok(PreparedCompletedScan {
        document,
        counts,
        coverage: coverage.clone(),
    })
}

fn snapshot_kind(kind: NodeKind) -> SnapshotNodeKind {
    match kind {
        NodeKind::Directory => SnapshotNodeKind::Directory,
        NodeKind::File => SnapshotNodeKind::File,
        NodeKind::Symlink => SnapshotNodeKind::Symlink,
        NodeKind::Other => SnapshotNodeKind::Other,
        NodeKind::Error => SnapshotNodeKind::Error,
    }
}

fn snapshot_timestamp(time: SystemTime) -> Option<SnapshotTimestamp> {
    let duration = time.duration_since(UNIX_EPOCH).ok()?;
    SnapshotTimestamp::new(duration.as_secs(), duration.subsec_nanos()).ok()
}

#[cfg(unix)]
fn snapshot_unix_identity(identity: Option<ScanObjectIdentity>) -> Option<SnapshotUnixIdentity> {
    identity
        .map(|ScanObjectIdentity::Unix { device, inode }| SnapshotUnixIdentity::new(device, inode))
}

#[cfg(windows)]
fn snapshot_unix_identity(_identity: Option<ScanObjectIdentity>) -> Option<SnapshotUnixIdentity> {
    None
}

fn map_codec_error(codec_error: super::SnapshotCodecError) -> ScanSnapshotError {
    match codec_error.kind {
        super::SnapshotCodecErrorKind::LimitExceeded
        | super::SnapshotCodecErrorKind::InvalidLength => {
            error(ScanSnapshotErrorKind::LimitExceeded)
        }
        _ => error(ScanSnapshotErrorKind::InvalidGraph),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::Duration;

    use tempfile::TempDir;

    use super::*;
    use crate::scanner::{ScanConfig, ScanTermination, Scanner};

    fn scan(root: &std::path::Path, config: ScanConfig) -> CompletedScanArtifact {
        let (messages, handle) = Scanner::new(config).scan(root.to_path_buf());
        for _ in messages {}
        let outcome = handle.join().unwrap();
        assert_eq!(outcome.termination(), ScanTermination::Completed);
        outcome.into_completed_artifact().unwrap()
    }

    fn prepare(root: &std::path::Path, config: ScanConfig) -> PreparedCompletedScan {
        prepare_completed_scan(
            ScanId::new("scan:fresh-fixture").unwrap(),
            UNIX_EPOCH + Duration::from_secs(1_750_000_000),
            &scan(root, config),
        )
        .unwrap()
    }

    #[test]
    fn completed_scan_builds_a_canonical_valid_snapshot() {
        let temp = TempDir::new().unwrap();
        fs::create_dir(temp.path().join("nested")).unwrap();
        fs::write(temp.path().join("nested/file"), b"payload").unwrap();
        let prepared = prepare(temp.path(), ScanConfig::default());
        let document = prepared.document();

        assert_eq!(document.nodes[0].id, 0);
        assert_eq!(document.nodes[0].parent, None);
        assert_eq!(document.nodes[0].kind, SnapshotNodeKind::Directory);
        assert_eq!(document.metadata.totals.directory_count, 2);
        assert_eq!(document.metadata.totals.file_count, 1);
        assert_eq!(document.metadata.totals.logical_bytes, 7);
        assert_eq!(document.nodes.len(), 3);
        assert_eq!(document.nodes[1].parent, Some(0));
        assert_eq!(document.nodes[2].parent, Some(1));
        validate_snapshot_document(document).unwrap();

        let mut encoded = Vec::new();
        let digest = crate::persistence::snapshot::encode_snapshot(document, &mut encoded).unwrap();
        let (decoded, decoded_digest) =
            crate::persistence::snapshot::decode_snapshot(&mut std::io::Cursor::new(encoded))
                .unwrap();
        assert_eq!(decoded, *document);
        assert_eq!(decoded_digest, digest);
    }

    #[test]
    fn empty_directories_have_known_zero_aggregates() {
        let temp = TempDir::new().unwrap();
        fs::create_dir(temp.path().join("empty")).unwrap();
        let prepared = prepare(temp.path(), ScanConfig::default());
        let (document, counts, coverage) = prepared.into_parts();

        assert_eq!(document.metadata.totals.directory_count, 2);
        assert_eq!(document.metadata.totals.file_count, 0);
        assert_eq!(document.metadata.totals.logical_bytes, 0);
        assert_eq!(document.metadata.totals.allocated_bytes, Some(0));
        assert!(document.nodes.iter().all(|node| {
            node.kind != SnapshotNodeKind::Directory || node.allocated_bytes == Some(0)
        }));
        assert_eq!(counts.directory_count, 2);
        assert_eq!(counts.file_count, 0);
        assert_eq!(counts.logical_bytes, 0);
        assert_eq!(counts.allocated_bytes, Some(0));
        assert_eq!(coverage.status(), crate::ScanCoverageStatus::Complete);
    }

    #[test]
    fn hard_links_count_logical_paths_but_allocate_once_deterministically() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("z-link"), b"hard-link-payload").unwrap();
        fs::hard_link(temp.path().join("z-link"), temp.path().join("a-link")).unwrap();
        let prepared = prepare(temp.path(), ScanConfig::default());
        let document = prepared.document();
        let files = document
            .nodes
            .iter()
            .filter(|node| node.kind == SnapshotNodeKind::File)
            .collect::<Vec<_>>();

        assert_eq!(files.len(), 2);
        assert_eq!(document.metadata.totals.file_count, 2);
        assert_eq!(document.metadata.totals.logical_bytes, 34);
        assert_eq!(
            files
                .iter()
                .filter(|node| node
                    .scan_flags
                    .contains(SnapshotScanFlags::HARD_LINK_DUPLICATE))
                .count(),
            1
        );
        let canonical_name = HostValue::from_component(std::ffi::OsStr::new("a-link")).unwrap();
        let canonical = files
            .iter()
            .find(|node| node.name.as_ref() == Some(&canonical_name))
            .unwrap();
        let duplicate = files
            .iter()
            .find(|node| {
                node.scan_flags
                    .contains(SnapshotScanFlags::HARD_LINK_DUPLICATE)
            })
            .unwrap();
        assert!(
            !canonical
                .scan_flags
                .contains(SnapshotScanFlags::HARD_LINK_DUPLICATE)
        );
        assert_eq!(duplicate.allocated_bytes, Some(0));
        assert_eq!(
            canonical.allocated_bytes,
            document.metadata.totals.allocated_bytes
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;

            assert_eq!(
                canonical.allocated_bytes,
                Some(fs::metadata(temp.path().join("a-link")).unwrap().blocks() * 512)
            );
        }
        let summed_allocation = files
            .iter()
            .map(|node| node.allocated_bytes.unwrap())
            .sum::<u64>();
        assert_eq!(
            document.metadata.totals.allocated_bytes,
            Some(summed_allocation)
        );

        let parallel = prepare(
            temp.path(),
            ScanConfig {
                num_threads: 4,
                ..ScanConfig::default()
            },
        );
        let stable_shape = |document: &SnapshotDocument| {
            document
                .nodes
                .iter()
                .map(|node| {
                    (
                        node.id,
                        node.parent,
                        node.depth,
                        node.kind,
                        node.name.clone(),
                        node.logical_bytes,
                        node.allocated_bytes,
                        node.file_count,
                        node.child_count,
                        node.scan_flags,
                        node.unix_identity,
                    )
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(stable_shape(parallel.document()), stable_shape(document));
    }

    #[cfg(unix)]
    #[test]
    fn sparse_file_keeps_logical_and_allocated_bytes_distinct() {
        use std::fs::OpenOptions;
        use std::io::{Seek, SeekFrom, Write};

        let temp = TempDir::new().unwrap();
        let mut sparse = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(temp.path().join("sparse"))
            .unwrap();
        sparse.seek(SeekFrom::Start(8 * 1024 * 1024 - 1)).unwrap();
        sparse.write_all(&[0]).unwrap();
        let prepared = prepare(temp.path(), ScanConfig::default());
        let file = prepared
            .document()
            .nodes
            .iter()
            .find(|node| node.kind == SnapshotNodeKind::File)
            .unwrap();

        assert_eq!(file.logical_bytes, 8 * 1024 * 1024);
        assert!(file.allocated_bytes.unwrap() < file.logical_bytes);
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn non_utf8_component_is_never_rebuilt_from_the_lossy_display_name() {
        use std::ffi::{OsStr, OsString};
        use std::os::unix::ffi::{OsStrExt, OsStringExt};

        let temp = TempDir::new().unwrap();
        let raw_name = vec![b'n', b'o', b'n', b'-', 0xff];
        fs::write(temp.path().join(OsString::from_vec(raw_name.clone())), b"x").unwrap();
        let prepared = prepare(temp.path(), ScanConfig::default());
        let file = prepared
            .document()
            .nodes
            .iter()
            .find(|node| node.kind == SnapshotNodeKind::File)
            .unwrap();

        assert_eq!(
            file.name.as_ref(),
            Some(&HostValue::from_component(OsStr::from_bytes(&raw_name)).unwrap())
        );
    }

    #[cfg(unix)]
    #[test]
    fn followed_directory_symlink_is_rejected_until_the_wire_can_preserve_it() {
        use std::os::unix::fs::symlink;

        let temp = TempDir::new().unwrap();
        fs::create_dir(temp.path().join("target")).unwrap();
        fs::write(temp.path().join("target/file"), b"x").unwrap();
        symlink(temp.path().join("target"), temp.path().join("followed")).unwrap();
        let artifact = scan(
            temp.path(),
            ScanConfig {
                follow_symlinks: true,
                ..ScanConfig::default()
            },
        );

        assert_eq!(
            prepare_completed_scan(
                ScanId::new("scan:followed-link").unwrap(),
                UNIX_EPOCH + Duration::from_secs(1_750_000_000),
                &artifact,
            )
            .unwrap_err()
            .kind,
            ScanSnapshotErrorKind::UnsupportedFollowedSymlink
        );
    }

    #[cfg(unix)]
    #[test]
    fn followed_file_symlink_is_rejected_until_the_wire_can_preserve_it() {
        use std::os::unix::fs::symlink;

        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("target"), b"x").unwrap();
        symlink(temp.path().join("target"), temp.path().join("followed")).unwrap();
        let artifact = scan(
            temp.path(),
            ScanConfig {
                follow_symlinks: true,
                ..ScanConfig::default()
            },
        );

        assert_eq!(
            prepare_completed_scan(
                ScanId::new("scan:followed-file-link").unwrap(),
                UNIX_EPOCH + Duration::from_secs(1_750_000_000),
                &artifact,
            )
            .unwrap_err()
            .kind,
            ScanSnapshotErrorKind::UnsupportedFollowedSymlink
        );
    }

    #[test]
    fn pre_epoch_capture_time_is_rejected_without_consuming_the_artifact() {
        let temp = TempDir::new().unwrap();
        let artifact = scan(temp.path(), ScanConfig::default());
        let before_epoch = UNIX_EPOCH.checked_sub(Duration::from_secs(1)).unwrap();

        assert_eq!(
            prepare_completed_scan(
                ScanId::new("scan:before-epoch").unwrap(),
                before_epoch,
                &artifact,
            )
            .unwrap_err()
            .kind,
            ScanSnapshotErrorKind::InvalidTimestamp
        );
        prepare_completed_scan(
            ScanId::new("scan:retry-after-invalid-time").unwrap(),
            UNIX_EPOCH + Duration::from_secs(1),
            &artifact,
        )
        .unwrap();
    }
}
