READ ~/mjukis/projects/agent-scripts/AGENTS.md BEFORE ANYTHING (skip if missing).

READ ~/agent-rules/AGENTS.md BEFORE ANYTHING (skip if missing).

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
- Tree and cache state are updated only after filesystem deletion succeeds
- Failed deletions remain visible and surface an error in the UI
- Delete confirmation captures the entry's filesystem identity without following symlinks. It also captures every ancestor directory identity from the scan root to the target parent; the worker re-checks ancestors and the target before removal and skips changed, missing, symlinked, or reparse-point paths.
- Build-artifact deletes additionally capture the exact marker-file identities used for classification and re-check them at execution time.
- User can continue browsing while deletion happens in background
- Deletion workers are tracked; graceful quit waits for them, applies their results to the tree, then persists the cache. External force termination can still interrupt a filesystem operation.

## Build Artifact Classification

- Classification is fail-closed and tree-aware; directory names alone are not evidence.
- Markers must be exact-case, direct sibling/child regular files and must not be symlinks.
- Current rules: Cargo `target` + sibling `Cargo.toml`; Node `node_modules` + sibling `package.json`; Gradle `build`/`.gradle` + a sibling Gradle build/settings script; Python `__pycache__` + sibling `.py`, `.tox` + sibling `tox.ini`, and `.venv`/`venv` + child `pyvenv.cfg`; CocoaPods `Pods` + sibling `Podfile` + child `Manifest.lock`; Next/Nuxt output + sibling `package.json` + matching framework config.
- `DerivedData`, `Build`, `dist`, `vendor`, and `.cache` are intentionally not classified yet.
- “Marker-matched” means likely tooling ownership, not guaranteed reproducibility. Permanent deletion remains manually confirmed.

## Git Hooks

Global hooks at `~/.git-hooks/` run `cargo fmt --check` and `cargo clippy` for Rust projects.
