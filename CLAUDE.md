# DUX - Development Notes

## Release Checklist

Before creating a release tag:

1. **Bump version** in `Cargo.toml` (workspace version) and update `dux-cli/Cargo.toml` dependency on `dux-core` to match
2. **Run checks locally**:
   ```bash
   cargo fmt --all
   cargo clippy --workspace --all-targets -- -D warnings
   cargo test
   ```
3. **Consider cross-platform issues**: Code inside `#[cfg(target_os = "macos")]` won't be compiled on Linux CI - ensure no unused variables leak out
4. **Commit and tag**: `git tag vX.Y.Z && git push origin main --tags`

## Architecture

- **dux-core**: Core library with tree data structures, scanner, and cache
- **dux-cli**: TUI application using ratatui

## Cache System

- Cache stored in `~/.cache/dux/` (or platform equivalent via `dirs` crate)
- Format: Magic + Version + Metadata + Tree (postcard) + CRC32
- `TreeNode.path` and `is_expanded` are `#[serde(skip)]` - reconstructed on load via `rebuild_paths()`
- Bump `CACHE_VERSION` in `dux-core/src/cache/metadata.rs` when format changes

## Tree Structure

- Arena-allocated with `Vec<Option<TreeNode>>` - `None` = tombstone (deleted)
- `remove_node()` tombstones node + descendants and propagates size changes up to root

## Deletion

- Deletion runs in a background thread to keep UI responsive
- Tree is updated optimistically (immediately) before filesystem deletion completes
- If user quits during deletion, the deletion continues to completion in the background
- Footer shows "Deleting..." during the operation

## Git Hooks

Global hooks at `~/.git-hooks/` run `cargo fmt --check` and `cargo clippy` for Rust projects.
