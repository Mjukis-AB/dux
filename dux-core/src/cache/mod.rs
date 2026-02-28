mod metadata;

pub use metadata::{CACHE_MAGIC, CACHE_VERSION, CacheMetadata, CachedScanConfig};

use std::collections::hash_map::DefaultHasher;
use std::fs::{self, File};
use std::hash::{Hash, Hasher};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::Result;
use crate::tree::DiskTree;

/// Get the cache file path for a given root directory
pub fn cache_path_for(root: &Path, cache_dir: &Path) -> PathBuf {
    let hash = hash_path(root);
    cache_dir.join(format!("{:016x}.dux", hash))
}

/// Hash a path to a u64 for cache filename
fn hash_path(path: &Path) -> u64 {
    let mut hasher = DefaultHasher::new();
    path.hash(&mut hasher);
    hasher.finish()
}

/// Save a tree to cache file
///
/// File format:
/// [4B] Magic "DUXC"
/// [4B] Version (u32 LE)
/// [4B] Metadata length (u32 LE)
/// [NB] Metadata (postcard)
/// [4B] Tree length (u32 LE)
/// [MB] Tree (postcard)
/// [4B] CRC32 checksum of all preceding bytes
pub fn save_cache(path: &Path, tree: &DiskTree, meta: &CacheMetadata) -> Result<()> {
    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut data = Vec::new();

    // Magic
    data.extend_from_slice(&CACHE_MAGIC);

    // Version
    data.extend_from_slice(&CACHE_VERSION.to_le_bytes());

    // Metadata
    let meta_bytes = postcard::to_allocvec(meta)
        .map_err(|e| crate::DuxError::Cache(format!("Failed to serialize metadata: {}", e)))?;
    data.extend_from_slice(&(meta_bytes.len() as u32).to_le_bytes());
    data.extend_from_slice(&meta_bytes);

    // Tree
    let tree_bytes = postcard::to_allocvec(tree)
        .map_err(|e| crate::DuxError::Cache(format!("Failed to serialize tree: {}", e)))?;
    data.extend_from_slice(&(tree_bytes.len() as u32).to_le_bytes());
    data.extend_from_slice(&tree_bytes);

    // CRC32 checksum
    let checksum = crc32fast::hash(&data);
    data.extend_from_slice(&checksum.to_le_bytes());

    // Write atomically by writing to temp file then renaming
    let temp_path = path.with_extension("tmp");
    let mut file = File::create(&temp_path)?;
    file.write_all(&data)?;
    file.sync_all()?;
    drop(file);

    fs::rename(&temp_path, path)?;

    Ok(())
}

/// Load a tree from cache file
pub fn load_cache(path: &Path) -> Result<(CacheMetadata, DiskTree)> {
    let mut file = File::open(path)?;

    let mut data = Vec::new();
    file.read_to_end(&mut data)?;

    // Need at least: magic(4) + version(4) + meta_len(4) + tree_len(4) + checksum(4) = 20 bytes
    if data.len() < 20 {
        return Err(crate::DuxError::Cache("Cache file too small".to_string()));
    }

    // Verify checksum (last 4 bytes)
    let checksum_offset = data.len() - 4;
    let stored_checksum = u32::from_le_bytes([
        data[checksum_offset],
        data[checksum_offset + 1],
        data[checksum_offset + 2],
        data[checksum_offset + 3],
    ]);
    let computed_checksum = crc32fast::hash(&data[..checksum_offset]);
    if stored_checksum != computed_checksum {
        return Err(crate::DuxError::Cache(
            "Cache checksum mismatch".to_string(),
        ));
    }

    let mut offset = 0;

    // Magic
    let magic: [u8; 4] = data[offset..offset + 4].try_into().unwrap();
    if magic != CACHE_MAGIC {
        return Err(crate::DuxError::Cache("Invalid cache magic".to_string()));
    }
    offset += 4;

    // Version
    let version = u32::from_le_bytes(data[offset..offset + 4].try_into().unwrap());
    if version != CACHE_VERSION {
        return Err(crate::DuxError::Cache(format!(
            "Cache version mismatch: expected {}, got {}",
            CACHE_VERSION, version
        )));
    }
    offset += 4;

    // Metadata
    let meta_len = u32::from_le_bytes(data[offset..offset + 4].try_into().unwrap()) as usize;
    offset += 4;
    if offset + meta_len > checksum_offset {
        return Err(crate::DuxError::Cache(
            "Invalid metadata length".to_string(),
        ));
    }
    let meta: CacheMetadata = postcard::from_bytes(&data[offset..offset + meta_len])
        .map_err(|e| crate::DuxError::Cache(format!("Failed to deserialize metadata: {}", e)))?;
    offset += meta_len;

    // Tree
    let tree_len = u32::from_le_bytes(data[offset..offset + 4].try_into().unwrap()) as usize;
    offset += 4;
    if offset + tree_len > checksum_offset {
        return Err(crate::DuxError::Cache("Invalid tree length".to_string()));
    }
    let mut tree: DiskTree = postcard::from_bytes(&data[offset..offset + tree_len])
        .map_err(|e| crate::DuxError::Cache(format!("Failed to deserialize tree: {}", e)))?;

    // Reconstruct paths (not serialized to save space)
    tree.rebuild_paths();

    Ok((meta, tree))
}

/// Check if a cache is still valid for the given configuration
pub fn is_cache_valid(meta: &CacheMetadata, root: &Path, config: &CachedScanConfig) -> bool {
    // Config must match
    if meta.config != *config {
        return false;
    }

    // Root path must match
    if meta.root_path != root {
        return false;
    }

    // Check root directory mtime hasn't changed
    // This is a quick heuristic - if the root dir mtime changed, something in it changed
    if let Ok(root_meta) = fs::metadata(root) {
        if let Ok(mtime) = root_meta.modified() {
            if mtime != meta.root_mtime {
                return false;
            }
        } else {
            return false;
        }
    } else {
        return false;
    }

    true
}

/// Get the modification time of a path
pub fn get_mtime(path: &Path) -> Option<SystemTime> {
    fs::metadata(path).ok()?.modified().ok()
}

/// Spot-check directory mtimes to detect deep changes that root mtime misses.
///
/// Collects all directory nodes with stored mtimes, sorts by size descending
/// (largest dirs are most impactful if stale), and stats the top `limit` entries.
/// Returns `true` if all checked mtimes match (cache is likely valid).
pub fn spot_check_mtimes(tree: &crate::tree::DiskTree, limit: usize) -> bool {
    use crate::tree::NodeKind;

    // Collect (size, path, stored_mtime) for all directories with mtimes
    let mut dirs: Vec<(u64, &Path, SystemTime)> = tree
        .iter()
        .filter(|n| n.kind == NodeKind::Directory)
        .filter_map(|n| n.mtime.map(|mt| (n.size, n.path.as_path(), mt)))
        .collect();

    // Sort by size descending — largest dirs cover the most of the tree
    dirs.sort_by(|a, b| b.0.cmp(&a.0));

    for (_, path, stored_mtime) in dirs.into_iter().take(limit) {
        match fs::metadata(path).and_then(|m| m.modified()) {
            Ok(current_mtime) if current_mtime != stored_mtime => return false,
            Err(_) => return false, // directory gone or inaccessible
            _ => {}                 // matches
        }
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tempfile::TempDir;

    #[test]
    fn test_cache_path_generation() {
        let cache_dir = PathBuf::from("/tmp/dux-cache");
        let path1 = cache_path_for(Path::new("/home/user/data"), &cache_dir);
        let path2 = cache_path_for(Path::new("/home/user/other"), &cache_dir);

        assert!(path1.to_string_lossy().ends_with(".dux"));
        assert!(path2.to_string_lossy().ends_with(".dux"));
        assert_ne!(path1, path2);
    }

    #[test]
    fn test_save_load_cache() {
        let temp = TempDir::new().unwrap();
        let cache_path = temp.path().join("test.dux");

        // Create a simple tree
        let tree = DiskTree::new(temp.path().to_path_buf());

        let meta = CacheMetadata {
            version: CACHE_VERSION,
            root_path: temp.path().to_path_buf(),
            scan_time: SystemTime::now(),
            root_mtime: SystemTime::now() - Duration::from_secs(100),
            total_size: 1024,
            node_count: 1,
            config: CachedScanConfig {
                follow_symlinks: false,
                same_filesystem: true,
                max_depth: None,
            },
        };

        // Save
        save_cache(&cache_path, &tree, &meta).unwrap();

        // Load
        let (loaded_meta, loaded_tree) = load_cache(&cache_path).unwrap();

        assert_eq!(loaded_meta.total_size, 1024);
        assert_eq!(loaded_tree.len(), 1);
    }

    #[test]
    fn test_paths_reconstructed_after_load() {
        use crate::tree::NodeKind;

        let temp = TempDir::new().unwrap();
        let cache_path = temp.path().join("test.dux");
        let root_path = temp.path().to_path_buf();

        // Create a tree with nested structure
        let mut tree = DiskTree::new(root_path.clone());
        let subdir_id = tree.add_node(
            "subdir".to_string(),
            NodeKind::Directory,
            root_path.join("subdir"),
            crate::tree::NodeId::ROOT,
        );
        let file_id = tree.add_node(
            "file.txt".to_string(),
            NodeKind::File,
            root_path.join("subdir").join("file.txt"),
            subdir_id,
        );

        // Verify original paths
        assert_eq!(tree.get(subdir_id).unwrap().path, root_path.join("subdir"));
        assert_eq!(
            tree.get(file_id).unwrap().path,
            root_path.join("subdir").join("file.txt")
        );

        let meta = CacheMetadata {
            version: CACHE_VERSION,
            root_path: root_path.clone(),
            scan_time: SystemTime::now(),
            root_mtime: SystemTime::now(),
            total_size: 0,
            node_count: 3,
            config: CachedScanConfig {
                follow_symlinks: false,
                same_filesystem: true,
                max_depth: None,
            },
        };

        // Save and reload
        save_cache(&cache_path, &tree, &meta).unwrap();
        let (_, loaded_tree) = load_cache(&cache_path).unwrap();

        // Verify paths were reconstructed correctly
        assert_eq!(loaded_tree.root().path, root_path);
        assert_eq!(
            loaded_tree.get(subdir_id).unwrap().path,
            root_path.join("subdir")
        );
        assert_eq!(
            loaded_tree.get(file_id).unwrap().path,
            root_path.join("subdir").join("file.txt")
        );
    }
}
