READ ~/mjukis/projects/agent-scripts/AGENTS.md BEFORE ANYTHING (skip if missing).

READ ~/agent-rules/AGENTS.md BEFORE ANYTHING (skip if missing).

# DUX - Development Notes

## Release Checklist

Before creating a release tag:

1. **Bump version** in `Cargo.toml` (workspace version) and update `dux-cli/Cargo.toml` dependency on `dux-core` to match
2. **Run checks locally**:
   ```bash
   cargo fmt --all -- --check
   cargo clippy --workspace --all-targets --locked -- -D warnings
   cargo test --workspace --all-targets --locked
   cargo audit --deny warnings
   cargo deny --locked check
   cargo package --workspace --locked
   ```
3. **Consider cross-platform issues**: Code inside `#[cfg(target_os = "macos")]` won't be compiled on Linux CI - ensure no unused variables leak out
4. **Commit and tag**: `git tag vX.Y.Z && git push origin main --tags`

The tag workflow is strictly ordered: validation/security, four-target builds, crates.io, draft GitHub release plus `SHA256SUMS`, then Homebrew. All actions must remain pinned to full commit SHAs. `HOMEBREW_TAP_TOKEN` must be a fine-grained PAT scoped only to `mjukis-ab/homebrew-tap` with Contents read/write; it is passed only to inline Git in the final job.

## Architecture

- **dux-core**: Core library with tree data structures, scanner, and cache
- **dux-cli**: TUI application using ratatui
- **dux-ffi**: Minimal UniFFI boundary; keep exports coarse, owned, versioned, and free of presentation or cleanup policy
- ADR 0005 selects the locked UniFFI release for the private in-process Swift/Rust transport. Keep generated APIs behind `EngineService`, keep checksum checks enabled, and treat the documented C ABI as a replacement path rather than a parallel API.
- `DuxEngine` is one application-scoped opaque session. Calls and close operations are serialized by `EngineService`; close is explicit and idempotent, generated errors are mapped before render state, and ARC release is not a substitute for cancellation or shutdown semantics. `live_engine_instance_count` exists only for linked binding-lifetime diagnostics.
- The macOS scene order is an invariant: keep the window-style `MenuBarExtra` first, followed by the singleton `Window(id: "explorer")` and `Settings`. On the macOS 14 deployment target, the first-scene rule keeps Explorer closed on a fresh launch. Open/focus windows only through `AppActivation`, which uses `NSApplication.activate()` rather than the API deprecated in macOS 14. Keep `LSUIElement` enabled and user-facing shell strings in `Localizable.xcstrings`.
- Build the universal macOS Rust artifact with `./dux-macos/scripts/build-rust-xcframework.sh`; generated output belongs under ignored `dux-macos/Generated/`.
- Regenerate matching Swift bindings and the header-bearing XCFramework with `CONFIGURATION=Debug ./dux-macos/scripts/generate-bindings.sh` or `CONFIGURATION=Release ./dux-macos/scripts/generate-bindings.sh` before building that same Xcode configuration from a clean checkout.
- Synchronous UniFFI calls belong in `EngineService`'s dedicated queue. Convert generated records to Sendable app DTOs before resuming Swift concurrency, and publish render state through the `@MainActor` `AppModel`.
- `VolumeMonitor` samples raw startup-volume capacity through Foundation on a dedicated utility queue. Prefer important-usage capacity for the headline value, retain ordinary available capacity for usage accounting, make fallback provenance explicit, and never infer free space from scan totals. Disk-pressure labels and hysteresis belong in the Rust evaluator, not Swift UI code.
- The macOS app and shared-engine target architecture is defined by the accepted ADRs in `docs/adr/`; keep `ROADMAP.md` and those decisions aligned.
- Run the scanner diagnostics example with `cargo run -p dux-core --example debug_scan -- /path/to/scan`.

## Cache System

- Cache stored in `~/.cache/dux/` (or platform equivalent via `dirs` crate)
- Format: Magic + Version + Metadata + Tree (postcard) + CRC32
- `TreeNode.path` and `is_expanded` are `#[serde(skip)]` - reconstructed on load via `rebuild_paths()`
- Bump `CACHE_VERSION` in `dux-core/src/cache/metadata.rs` when format changes
- Cache age always means the tree's original scan time; deletion-only cache updates must preserve it.
- `r` starts a cache-bypassing rescan. Keep the previous tree and dirty state until the replacement scan succeeds so failure or quit cannot lose the last usable snapshot.
- Writers use exclusively created `.<cache>.<pid>.<counter>.tmp` files and atomic rename. Join older in-process writers before publishing a newer snapshot; cross-process writes are still last-writer-wins.

## Tree Structure

- Arena-allocated with `Vec<Option<TreeNode>>` - `None` = tombstone (deleted)
- `remove_node()` tombstones node + descendants and propagates size changes up to root
- Footer selection bytes and multi-delete planning use the same effective selection: selected descendants are omitted when an ancestor is selected. The displayed selection count remains the raw number of highlighted rows.

## Deletion

- Deletion runs in a background thread to keep UI responsive
- Tree and cache state are updated only after filesystem deletion succeeds
- Failed deletions remain visible and surface an error in the UI
- Delete confirmation captures the entry's filesystem identity without following symlinks. It also captures every ancestor directory identity from the scan root to the target parent; the worker re-checks ancestors and the target before removal and skips changed, missing, symlinked, or reparse-point paths.
- Build-artifact deletes additionally capture the exact marker-file identities used for classification and re-check them at execution time.
- Confirmed multi-delete batches use at most four named workers fed by a shared queue. All handles remain tracked; graceful quit and `Drop` wait for active and queued items, and an individual task panic is reported as that item's failure without stopping the pool.
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
