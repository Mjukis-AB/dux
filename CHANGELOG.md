# Changelog

All notable changes to DUX will be documented in this file.

## [Unreleased]

### Fixed
- Prevented deletion of the active scan root.
- Failed filesystem deletions no longer remove items from the displayed or cached tree.
- Unicode paths are truncated safely without slicing through UTF-8 characters.
- Scanner path exclusions now use component-aware rules, and crossed filesystems are classified before traversal instead of treating every `/Volumes` path as unsafe.
- Scanner metadata and filesystem probes now use a process-wide bounded worker pool instead of spawning one detached thread per directory.
- Permanent-delete dialogs now require an explicit `y` and show the effective item count; Enter no longer confirms deletion.
- Graceful quit now waits for active deletion workers, applies their results, and orders cache writes so an older snapshot cannot overwrite the post-delete tree.
- Permanent deletes now capture non-following filesystem identity before confirmation and re-check it immediately before removal; replaced or missing entries are skipped without changing tree state or deletion statistics.
- Build-artifact entries now require exact, regular, non-symlink marker files instead of directory names alone. Ambiguous names such as `DerivedData`, `Build`, `dist`, `vendor`, and `.cache` are omitted until stronger rules exist.
- Artifact marker identities and every directory identity from the scan root to the target parent are re-checked before permanent deletion. Changed evidence, replaced ancestors, and symlink/reparse ancestors fail closed.
- Followed symlinks retain path provenance in scan snapshots, and deletion is refused below a known followed-symlink ancestor.

### Changed
- Cache version bumped to v5 so scans created with the previous scanner policy or without symlink provenance auto-invalidate.
- The Build Artifacts view now labels entries as “Marker-matched”; classification identifies likely tooling ownership but does not assert that contents are automatically safe or reproducible.

## [0.5.0]

### Added
- **Multi-select**: Press `v` to toggle items in/out of selection, then navigate with arrow keys or `j`/`k` to extend the range. `K`/`J` (uppercase) also extend selection. Selection count and total size shown in purple in the footer.
- **Multi-delete**: Press `d` with items selected to delete them all at once. Confirmation dialog shows up to 5 paths with sizes and a total. Deletions run concurrently with a progress overlay showing a bar, completed/total count, and freed bytes.
- **Selecting mode**: When `v` is pressed, entering selecting mode where all navigation automatically extends the selection. Press `v` on a selected item to unselect it, or `Esc` to clear all.
- **Large Files view**: Flat list of all files sorted by size, helping find big files buried deep in the tree. Press `Tab` to switch views.
- **Build Artifacts view**: Detects known build directories (`target/`, `node_modules/`, `DerivedData/`, etc.) with a staleness indicator. Press `s` to cycle the stale threshold (1d/7d/30d/90d/All).
- **View switching**: `Tab`/`Shift-Tab` cycles between Tree, Large Files, and Build Artifacts views. All views support navigation, deletion, and open-in-Finder.
- Help overlay now includes a Views section documenting the new key bindings.

### Fixed
- Scanner no longer hangs on cloud storage FUSE mounts (Google Drive, OneDrive, iCloud Drive). Added these paths to the skip list.
- Scanner now probes directories with a 5-second metadata timeout before descending. Directories that don't respond in time (slow FUSE, hung NFS, etc.) are automatically skipped.
- **Build Artifacts staleness**: Now uses the newest mtime of any descendant directory, not just the top-level directory. Recently-built `target/` directories no longer incorrectly show as stale.
- **Build Artifacts dedup**: Subdirectories inside an artifact (e.g. `target/debug/build`) no longer appear as separate entries.
- **Stale threshold cycling**: No longer triggers a full tree rebuild — updates `is_stale` flags in place.

### Changed
- `Tab` now switches views instead of toggling expand/collapse (use `Space` for toggle).
- Footer hints update dynamically based on the active view.
- Header shows the active view name for non-Tree views.
- `Esc` now clears selection first (if any), then goes back.

## [0.4.0]

### Added
- **Scan caching**: Persist scan results to disk and reload on subsequent runs if the root directory hasn't changed. Cache files are stored in the system cache directory (`~/.cache/dux/` on Linux/macOS). Use `--no-cache` to force a fresh scan.
- **Incremental tree updates**: After deleting files/directories, the tree is updated in-place without requiring a rescan. Sizes and file counts propagate correctly up to the root.
- **Delete statistics**: Track and display space freed during the session. The footer shows "Freed: X.X GB (N items)" when items have been deleted.
- **Cache indicator**: Header shows "(cached)" when the tree was loaded from cache.
- **Save cache on quit**: Deletions made in dux are now persisted to cache so deleted items no longer reappear on next launch.
- **Smarter cache invalidation**: Spot-check mtimes of the 32 largest directories on cache load to detect deep filesystem changes that root mtime alone misses.

### Changed
- Tree data structure now uses tombstones for deleted nodes, allowing efficient in-place updates.
- `DiskTree` is now serializable with serde for cache persistence.
- Cache format version bumped to v3 (old caches auto-invalidate).

## [0.1.0] - Initial Release

### Added
- Interactive TUI disk usage analyzer
- Parallel filesystem scanning with jwalk
- Tree view with expand/collapse
- Drill-down navigation
- Open in Finder (macOS)
- Delete files/directories with confirmation
- Keyboard navigation (vim-style and arrow keys)
- Size bar visualization
- Progress indicator during scan
