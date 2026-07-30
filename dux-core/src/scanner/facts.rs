use std::collections::HashMap;
use std::path::Path;
use std::time::SystemTime;

use crate::tree::{DiskTree, NodeId, NodeKind};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum ScanObjectIdentity {
    #[cfg(unix)]
    Unix { device: u64, inode: u64 },
    #[cfg(windows)]
    Windows {
        volume_serial: u64,
        file_id: [u8; 16],
    },
}

impl ScanObjectIdentity {
    pub(super) const fn volume_key(self) -> u64 {
        match self {
            #[cfg(unix)]
            Self::Unix { device, .. } => device,
            #[cfg(windows)]
            Self::Windows { volume_serial, .. } => volume_serial,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn captured(
        logical_bytes: u64,
        allocated_bytes: Option<u64>,
        identity: Option<ScanObjectIdentity>,
        link_count: Option<u64>,
    ) -> ScanNodeFacts {
        ScanNodeFacts::captured(
            logical_bytes,
            allocated_bytes,
            None,
            None,
            identity,
            link_count,
        )
    }

    #[cfg(unix)]
    fn object_identity() -> ScanObjectIdentity {
        ScanObjectIdentity::Unix {
            device: 7,
            inode: 11,
        }
    }

    #[cfg(windows)]
    fn object_identity() -> ScanObjectIdentity {
        ScanObjectIdentity::Windows {
            volume_serial: 7,
            file_id: [11; 16],
        }
    }

    #[test]
    fn unknown_allocation_is_never_replaced_with_logical_bytes() {
        let root = PathBuf::from(if cfg!(windows) {
            r"C:\fixture"
        } else {
            "/fixture"
        });
        let mut tree = DiskTree::new(root.clone());
        let file = tree.add_node(
            "payload".to_owned(),
            NodeKind::File,
            root.join("payload"),
            NodeId::ROOT,
        );
        let known = tree.add_node(
            "known".to_owned(),
            NodeKind::File,
            root.join("known"),
            NodeId::ROOT,
        );
        let mut builder = ScanFactsBuilder::new();
        builder
            .record(NodeId::ROOT, captured(0, Some(0), None, None))
            .unwrap();
        builder
            .record(file, captured(41, None, None, None))
            .unwrap();
        builder
            .record(known, captured(23, Some(16), None, None))
            .unwrap();

        let finalized = builder.finalize(&mut tree, |_| {}).unwrap();

        assert_eq!(finalized.facts.node(file).unwrap().logical_bytes, 41);
        assert_eq!(finalized.facts.node(file).unwrap().allocated_bytes, None);
        assert_eq!(
            finalized.facts.node(NodeId::ROOT).unwrap().logical_bytes,
            64
        );
        assert_eq!(
            finalized.facts.node(NodeId::ROOT).unwrap().allocated_bytes,
            None
        );
        assert_eq!(tree.get(file).unwrap().size, 0);
        assert_eq!(tree.get(known).unwrap().size, 16);
        assert_eq!(tree.root().size, 16);
    }

    #[test]
    fn conflicting_hard_link_observations_fail_allocation_closed() {
        let root = PathBuf::from(if cfg!(windows) {
            r"C:\fixture"
        } else {
            "/fixture"
        });
        let mut tree = DiskTree::new(root.clone());
        let z = tree.add_node("z".to_owned(), NodeKind::File, root.join("z"), NodeId::ROOT);
        let a = tree.add_node("a".to_owned(), NodeKind::File, root.join("a"), NodeId::ROOT);
        let identity = Some(object_identity());
        let mut builder = ScanFactsBuilder::new();
        builder
            .record(NodeId::ROOT, captured(0, Some(0), None, None))
            .unwrap();
        builder
            .record(z, captured(20, Some(8), identity, Some(2)))
            .unwrap();
        builder
            .record(a, captured(21, Some(8), identity, Some(2)))
            .unwrap();

        let mut changed_paths = Vec::new();
        let finalized = builder
            .finalize(&mut tree, |path| changed_paths.push(path.to_path_buf()))
            .unwrap();

        assert_eq!(changed_paths, vec![root.join("a"), root.join("z")]);
        assert_eq!(finalized.facts.node(a).unwrap().allocated_bytes, None);
        assert_eq!(finalized.facts.node(z).unwrap().allocated_bytes, None);
        assert_eq!(
            finalized.facts.node(NodeId::ROOT).unwrap().allocated_bytes,
            None
        );
        assert!(
            !finalized
                .facts
                .node(z)
                .unwrap()
                .flags
                .contains(ScanNodeFlags::HARD_LINK_DUPLICATE)
        );
        assert!(
            !finalized
                .facts
                .node(a)
                .unwrap()
                .flags
                .contains(ScanNodeFlags::HARD_LINK_DUPLICATE)
        );
    }

    #[test]
    fn identity_reuse_with_too_small_link_count_fails_closed() {
        let root = PathBuf::from(if cfg!(windows) {
            r"C:\fixture"
        } else {
            "/fixture"
        });
        let mut tree = DiskTree::new(root.clone());
        let first = tree.add_node(
            "first".to_owned(),
            NodeKind::File,
            root.join("first"),
            NodeId::ROOT,
        );
        let second = tree.add_node(
            "second".to_owned(),
            NodeKind::File,
            root.join("second"),
            NodeId::ROOT,
        );
        let identity = Some(object_identity());
        let mut builder = ScanFactsBuilder::new();
        builder
            .record(NodeId::ROOT, captured(0, Some(0), None, None))
            .unwrap();
        builder
            .record(first, captured(20, Some(8), identity, Some(1)))
            .unwrap();
        builder
            .record(second, captured(20, Some(8), identity, Some(1)))
            .unwrap();
        let mut changed = Vec::new();

        let finalized = builder
            .finalize(&mut tree, |path| changed.push(path.to_path_buf()))
            .unwrap();

        assert_eq!(changed, vec![root.join("first"), root.join("second")]);
        assert_eq!(finalized.facts.node(first).unwrap().allocated_bytes, None);
        assert_eq!(finalized.facts.node(second).unwrap().allocated_bytes, None);
        assert!(
            !finalized
                .facts
                .node(first)
                .unwrap()
                .flags
                .contains(ScanNodeFlags::HARD_LINK_DUPLICATE)
        );
        assert!(
            !finalized
                .facts
                .node(second)
                .unwrap()
                .flags
                .contains(ScanNodeFlags::HARD_LINK_DUPLICATE)
        );
        assert_eq!(
            finalized.facts.node(NodeId::ROOT).unwrap().allocated_bytes,
            None
        );
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ScanNodeFlags(u8);

impl ScanNodeFlags {
    pub(crate) const HARD_LINK_DUPLICATE: Self = Self(1 << 0);
    pub(crate) const MOUNT_BOUNDARY: Self = Self(1 << 1);

    pub(crate) const fn contains(self, flag: Self) -> bool {
        self.0 & flag.0 == flag.0
    }

    pub(super) fn insert(&mut self, flag: Self) {
        self.0 |= flag.0;
    }

    fn remove(&mut self, flag: Self) {
        self.0 &= !flag.0;
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ScanNodeFacts {
    pub(crate) logical_bytes: u64,
    pub(crate) allocated_bytes: Option<u64>,
    pub(crate) modified_at: Option<SystemTime>,
    pub(crate) accessed_at: Option<SystemTime>,
    pub(crate) identity: Option<ScanObjectIdentity>,
    pub(crate) link_count: Option<u64>,
    pub(crate) flags: ScanNodeFlags,
    observed_allocated_bytes: Option<u64>,
}

impl ScanNodeFacts {
    pub(super) fn captured(
        logical_bytes: u64,
        allocated_bytes: Option<u64>,
        modified_at: Option<SystemTime>,
        accessed_at: Option<SystemTime>,
        identity: Option<ScanObjectIdentity>,
        link_count: Option<u64>,
    ) -> Self {
        Self {
            logical_bytes,
            allocated_bytes,
            modified_at,
            accessed_at,
            identity,
            link_count,
            flags: ScanNodeFlags::default(),
            observed_allocated_bytes: allocated_bytes,
        }
    }
}

/// Private same-scan witness aligned exactly with fresh `DiskTree` node IDs.
/// A cache-loaded or client-constructed tree cannot manufacture this value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FreshScanFacts {
    nodes: Vec<ScanNodeFacts>,
}

impl FreshScanFacts {
    pub(crate) fn node(&self, id: NodeId) -> Option<&ScanNodeFacts> {
        self.nodes.get(id.index())
    }

    pub(crate) fn len(&self) -> usize {
        self.nodes.len()
    }

    #[cfg(test)]
    pub(crate) fn set_allocated_bytes_for_test(
        &mut self,
        id: NodeId,
        allocated_bytes: Option<u64>,
    ) {
        self.nodes[id.index()].allocated_bytes = allocated_bytes;
    }
}

#[derive(Debug)]
pub(super) struct FinalizedScanFacts {
    pub(super) facts: FreshScanFacts,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FinalizeFactsError {
    Misaligned,
    Overflow,
}

pub(super) struct ScanFactsBuilder {
    nodes: Vec<ScanNodeFacts>,
}

enum IdentityNodes {
    One(NodeId),
    Many(Vec<NodeId>),
}

impl IdentityNodes {
    fn push(&mut self, id: NodeId) {
        match self {
            Self::One(first) => {
                let first = *first;
                *self = Self::Many(vec![first, id]);
            }
            Self::Many(ids) => ids.push(id),
        }
    }

    fn multiple_mut(&mut self) -> Option<&mut Vec<NodeId>> {
        match self {
            Self::One(_) => None,
            Self::Many(ids) => Some(ids),
        }
    }
}

impl ScanFactsBuilder {
    pub(super) fn new() -> Self {
        Self { nodes: Vec::new() }
    }

    pub(super) fn record(
        &mut self,
        id: NodeId,
        facts: ScanNodeFacts,
    ) -> Result<(), FinalizeFactsError> {
        if id.index() != self.nodes.len() {
            return Err(FinalizeFactsError::Misaligned);
        }
        self.nodes.push(facts);
        Ok(())
    }

    pub(super) fn node(&self, id: NodeId) -> Option<&ScanNodeFacts> {
        self.nodes.get(id.index())
    }

    pub(super) fn finalize<F>(
        self,
        tree: &mut DiskTree,
        mut record_changed: F,
    ) -> Result<FinalizedScanFacts, FinalizeFactsError>
    where
        F: FnMut(&Path),
    {
        if self.nodes.len() != tree.len() || tree.live_count() != tree.len() {
            return Err(FinalizeFactsError::Misaligned);
        }
        let mut nodes = self.nodes;

        // Most files have one link. Keep their first node inline so detecting
        // actual shared identities does not allocate one Vec per ordinary file.
        let mut hard_links: HashMap<ScanObjectIdentity, IdentityNodes> = HashMap::new();
        for node in tree.iter() {
            let facts = nodes
                .get_mut(node.id.index())
                .ok_or(FinalizeFactsError::Misaligned)?;
            facts.allocated_bytes = facts.observed_allocated_bytes;
            facts.flags.remove(ScanNodeFlags::HARD_LINK_DUPLICATE);
            if node.kind == NodeKind::File
                && let Some(identity) = facts.identity
            {
                hard_links
                    .entry(identity)
                    .and_modify(|nodes| nodes.push(node.id))
                    .or_insert(IdentityNodes::One(node.id));
            }
        }

        for nodes_for_identity in hard_links.values_mut() {
            let Some(ids) = nodes_for_identity.multiple_mut() else {
                continue;
            };
            ids.sort_by(|left, right| {
                tree.get(*left)
                    .expect("fresh fact node remains live")
                    .path
                    .cmp(&tree.get(*right).expect("fresh fact node remains live").path)
            });
            let first = &nodes[ids[0].index()];
            let observed_paths =
                u64::try_from(ids.len()).map_err(|_| FinalizeFactsError::Overflow)?;
            let observations_conflict = first.link_count.is_none_or(|count| count < observed_paths)
                || ids.iter().skip(1).any(|id| {
                    let facts = &nodes[id.index()];
                    facts.logical_bytes != first.logical_bytes
                        || facts.observed_allocated_bytes != first.observed_allocated_bytes
                        || facts.link_count != first.link_count
                        || facts.modified_at != first.modified_at
                });
            if observations_conflict {
                for &id in ids.iter() {
                    nodes[id.index()].allocated_bytes = None;
                    record_changed(&tree.get(id).expect("fresh fact node remains live").path);
                }
            }
            if !observations_conflict {
                for &duplicate in ids.iter().skip(1) {
                    let facts = &mut nodes[duplicate.index()];
                    if facts.allocated_bytes.is_some() {
                        facts.allocated_bytes = Some(0);
                    }
                    facts.flags.insert(ScanNodeFlags::HARD_LINK_DUPLICATE);
                }
            }
        }

        for index in (0..nodes.len()).rev() {
            let tree_node = tree
                .get(NodeId(index))
                .ok_or(FinalizeFactsError::Misaligned)?;
            if tree_node.kind.is_directory() {
                let children = tree_node.children.clone();
                let mut logical_bytes = 0_u64;
                let mut allocated_bytes = 0_u64;
                let mut known_ui_bytes = 0_u64;
                let mut file_count = 0_u64;
                let mut all_children_allocated = true;
                for child in children {
                    let child_facts = nodes
                        .get(child.index())
                        .ok_or(FinalizeFactsError::Misaligned)?;
                    logical_bytes = logical_bytes
                        .checked_add(child_facts.logical_bytes)
                        .ok_or(FinalizeFactsError::Overflow)?;
                    let child_count = tree
                        .get(child)
                        .ok_or(FinalizeFactsError::Misaligned)?
                        .file_count;
                    file_count = file_count
                        .checked_add(child_count)
                        .ok_or(FinalizeFactsError::Overflow)?;
                    known_ui_bytes = known_ui_bytes
                        .checked_add(tree.get(child).ok_or(FinalizeFactsError::Misaligned)?.size)
                        .ok_or(FinalizeFactsError::Overflow)?;
                    if let Some(bytes) = child_facts.allocated_bytes {
                        allocated_bytes = allocated_bytes
                            .checked_add(bytes)
                            .ok_or(FinalizeFactsError::Overflow)?;
                    } else {
                        all_children_allocated = false;
                    }
                }
                let facts = &mut nodes[index];
                facts.logical_bytes = logical_bytes;
                facts.allocated_bytes = all_children_allocated.then_some(allocated_bytes);
                let node = tree
                    .get_mut(NodeId(index))
                    .ok_or(FinalizeFactsError::Misaligned)?;
                node.file_count = file_count;
                node.size = known_ui_bytes;
            } else {
                let facts = &nodes[index];
                let node = tree
                    .get_mut(NodeId(index))
                    .ok_or(FinalizeFactsError::Misaligned)?;
                node.file_count = u64::from(node.kind == NodeKind::File);
                node.size = facts.allocated_bytes.unwrap_or(0);
            }
        }

        Ok(FinalizedScanFacts {
            facts: FreshScanFacts { nodes },
        })
    }
}
