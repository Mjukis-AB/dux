# Changelog

All notable changes to DUX will be documented in this file.

## [Unreleased]

### Added
- Added a crate-private, versioned process-instance identity and tri-state
  liveness probe as a prerequisite for cleanup-journal recovery. The bounded
  128-byte owner representation binds PID and OS start time to a hashed macOS
  boot-session or Linux boot/PID-namespace scope plus a random claim nonce.
  Only a same-scope absence or start-token change is `DefinitelyGone`; changed
  scope, malformed/partial OS evidence, permissions, and unsupported host proof
  are `Unknown`. Windows can prove an exact live match through a retained
  process handle but deliberately cannot prove death until a reliable host
  scope exists. Native subprocess tests cover live owners plus graceful and
  abrupt death. This checkpoint changes no journal state and grants no cleanup,
  recovery, plan, or effect authority.
- Added a crate-private, permanent store-wide cleanup-effect lock distinct from
  SQLite's writer lock. Existing owned stores provision the exact private
  control file only while holding the writer lock. It flushes the lock and
  ready control before durably advancing the existing ownership marker from
  layout v1 to v2; v2 makes later absence fail closed instead of recreating a
  second lock identity. Retained handles, exact markers, root inventory, ownership,
  permissions, links, and path identity are revalidated around bounded lock
  acquisition. Windows retains both cleanup controls without delete sharing so
  `LockFileEx` cannot remain on a displaced file. Same-process, independent
  writer-lock, cross-process, malformed-layout, link, special-file, and native
  replacement regressions cover the boundary. The guard proves exclusion only:
  it carries no plan, owner, recovery, journal, target, or effect authority.
- Added crate-private planned cleanup journals on SQLite schema v2. One
  immutable cleanup plan is prepared and bounded before locking, matched
  exactly to complete persisted candidate observations, and atomically stored
  with contiguous items, paths, evidence, warnings, and each proposed effect
  even in dry-run mode. The exact-ID reader returns either an explicit legacy
  summary or a complete non-executable planned observation, rejects malformed
  or polluted children and newer schemas, and fits the shared query budget at
  the 256-path/512-evidence limit. Execution, owner claims, recovery, and
  journal transitions remain deliberately sealed.
- Added crate-private typed candidate history on SQLite schema v2. Complete
  deterministic findings are inserted atomically with ordered bounded paths,
  evidence, and blockers, then loaded only as non-executable observation
  records; migrated v1 rows return explicit incomplete summaries. Exact-ID
  reads bound SQLite work and validate storage types/lengths, ordinals, enum
  shapes, source-scan presence, policy pairs, and cloud-upload evidence before
  returning data. No status mutation, plan construction, FFI surface, or
  cleanup authority is introduced.
- Added checksummed SQLite schema v2 for complete, non-authoritative candidate
  and cleanup history. The atomic v1→v2 migration preserves legacy summaries
  under an explicit legacy format without inventing absent facts; new records
  have normalized ordered path/evidence/blocker data, nanosecond plan facts,
  frozen per-item proposed actions, multi-path operation journals, warnings,
  and owner-generation crash-recovery state. Exact per-version object
  inventories and fingerprints, populated legacy migration coverage, and
  fail-closed semantic constraints protect version skew and malformed rows.
  This schema grants no cleanup authority; typed candidate/session APIs and
  executor reconciliation remain separate checkpoints.
- Added the first typed persistence layer for non-authoritative scan history.
  Crate-private start, terminal compare-and-set, and exact-ID load operations
  preserve stable IDs, lossless accepted host-codec absolute roots, checked times and byte counts,
  reject duplicate or terminal rewrites, recheck schema compatibility while
  holding the coordinator and cross-process writer lease, and bound SQLite
  read work. Completed summaries survive coordinator teardown and reopen; this
  does not yet expose scan APIs to the engine, persist snapshots, or make
  historical paths actionable.
- Added a versioned private SQLite foundation owned by the shared engine. A
  checksummed v1 `STRICT` schema covers all planned aggregate/history tables
  with bounded fields, constrained semantic text, and a tested lossless
  UTF-8/UTF-16 path codec. Transactional migrations and live compatibility
  refresh use separate VM/deadline budgets, WAL, bounded lock waits, and a
  stable advisory writer lease; bounded schema materialization and runner-owned
  transaction control prevent crafted metadata or migration batches from
  escaping those limits. Valid newer schemas reopen read-only while
  foreign, unmarked, corrupt, drifted, or over-budget stores return path-free
  categories. Private staged provisioning atomically publishes an immutable
  ownership marker plus empty database without replacing a racing path, then
  durably records successful initialization so zero-length corruption cannot
  be mistaken for a fresh store, and
  permits the exact future `snapshots`, `ai`, and `logs` siblings. Unix stores
  enforce owner/mode/link/no-follow invariants, with deny-only publication-parent
  ACLs and exact final-object ACL rejection on macOS. Windows uses protected
  owner-only DACLs, handle-relative stage creation, handle-bound publication, a
  retained final-root rename guard, and exact SQLite-sidecar DACL repair. Real
  Unix crash regressions, cross-platform writer/version-race coverage, and native macOS/Windows
  storage tests cover the platform-specific boundaries. No cleanup authority
  is exposed by this checkpoint.
- Added the shared core engine handle and bounded per-session FIFO task registry with explicit disjoint storage paths, fixed workers/queue/event/result retention, opaque non-reused task IDs, typed cancellation and terminal state, panic containment, nonblocking close with quiescence, and a bounded read-only formatting operation. The application architecture owns one session; the UniFFI engine remains smoke-only, and no scan, domain-persistence operation, AI, plan, or cleanup authority is exposed by this checkpoint.
- Added a temporary core-owned adapter for the legacy CLI permanent-delete path. Single and batch requests consume opaque target-bound plans through one guarded effect boundary, and repository policy prevents the adapter from being exposed through FFI or the macOS app.
- Added a CI-enforced destructive-call boundary: compiler-resolved per-platform Rust filesystem/process denials plus a cross-language repository scanner with one-use registered exceptions, executable/shebang discovery, release enforcement, and executable policy tests. The XCFramework builder now rejects arbitrary destinations and symlinked output parents before any build tool or destructive mutation runs.
- Added the normative DUX security design: an implementation-status-aware threat model and cleanup shipping gate covering the one-way observation-to-executor authority chain, path and protected-root evidence, rule provenance, Trash/permanent/eviction semantics, automation, AI isolation, private persistence, TCC and unsandboxed authority, FFI/CLI boundaries, supply-chain integrity, incident response, and the explicit gaps in the current legacy CLI deletion path.
- Added a versioned dangerous-path corpus and deterministic path-safety properties covering host-native lexical rejection, raw encoding failures, all independently authored protected-root entries, dynamic home/profile guards, component boundaries, evidence-form precedence, and explicit non-authoritative known gaps. An isolated, dependency-audited libFuzzer target uses a bounded filesystem-free `cfg(fuzzing)` adapter, retained seeds, pinned smoke CI, and a weekly campaign; fuzz crashes must become permanent corpus regressions.
- Added a revisioned, crate-private protected-root registry with host-independent macOS, Linux, and Windows policy tables. Component-aware rules distinguish exact structural anchors, hard-denied subtrees, and descendants that require a future code-owned deterministic-rule grant; configured validated home roots and other profiles beneath conventional/configured profile containers fail closed, and requested/canonical scan and target spellings are assessed with hard denies taking precedence. Production construction remains sealed until trusted account/known-folder discovery exists. The registry returns only a non-authoritative textual disposition—not an “allowed” witness—until trusted account/volume discovery, full root ancestry, mount-location identity, and executor revalidation exist.
- Added a crate-internal, non-authoritative path-validation service with raw host-native lexical checks, strict scan-root binding, descriptor-relative no-follow inspection on Unix, repeated no-follow handle inspection on Windows, alias-preserving canonical scope verification, stable volume/object identity, ordered ancestor evidence, target kind, and hard-link counts. It rejects ambiguous syntax, invalid/lossy encoding, symlinks and reparse points, special entries, cross-volume descendants, and paths that change while being inspected; it performs no mutation and grants no cleanup authority. Windows evidence remains full-path based and cannot become actionable until later handle-bound executor revalidation exists.
- Added immutable cleanup-plan and plan-item domain types with fixed expiration, checked estimates, exact candidate/rule provenance, explicit dry-run/Trash/permanent/cloud-eviction semantics, mandatory warnings, and fail-closed structural invariants. Construction remains sealed pending validated path witnesses, and the types provide no approval or execution capability.
- Added a versioned deterministic-rule catalog schema and an internal fail-closed JSON loader with explicit fields, bounded documents and collections, stable ordering, duplicate rejection, exhaustive policy validation, an ASCII byte-bounded matcher grammar, and synthetic schema/domain fixtures. The loader is crate-private and no catalog or fixture ships in the app.
- Added product-neutral candidate and deterministic rule domain types with stable ID/revision pairs, typed evidence and blockers, validated scope/matcher/guard policy, strict safety/action compatibility, localization keys, provenance, and exact per-path cloud-upload evidence for proposed eviction. Candidate construction remains internal and non-authoritative; no cleanup rule or execution path is enabled.
- Accepted architecture decision records now define the native SwiftUI app, direct Developer ID DMG distribution, unsandboxed access model, and shared Rust engine/FFI boundary. Local AI subprocess adapters are explicitly gated on an adversarial macOS permission and confinement spike.
- Added the initial `dux-ffi` UniFFI crate with a version handshake and typed formatted-size smoke result, producing Rust library, static-library, and dynamic-library artifacts for later Swift packaging.
- Added a fail-closed macOS build script and CI job that compile `dux-ffi` for arm64 and x86_64, merge the static libraries, and package a validated universal `DuxFFI.xcframework`.
- Added reproducible UniFFI Swift/header/module-map generation and a minimal unsigned SwiftUI app that imports and links the generated bindings through the universal XCFramework.
- Added an off-main-thread Swift engine service, `@MainActor` application model, rendered typed Rust smoke result, and linked macOS test coverage for the concurrency handoff.
- Added a clean-checkout macOS build gate that regenerates configuration-matched Rust and UniFFI artifacts, tests Debug, and builds unsigned universal Debug and Release apps.
- Accepted UniFFI as the private Swift/Rust transport, with strict version, generation, concurrency, error, lifetime, and batching boundaries plus a documented C-ABI replacement path.
- Added the first opaque `DuxEngine` session with FFI contract v2, typed closed-session errors, idempotent close, off-main service error mapping, and linked Swift/Rust lifetime coverage.
- Added the menu bar-first macOS shell with a window-style `MenuBarExtra`, singleton Explorer window, native Settings scene, shared application model, accessible actions, String Catalog, and `LSUIElement` agent configuration.
- Added off-main startup-volume capacity sampling for the macOS shell. The menu bar and Explorer prefer Foundation's important-usage capacity, explicitly fall back to ordinary filesystem availability, and render an accessible capacity summary without requiring a directory scan.

### Fixed
- Legacy CLI deletion planning now validates the complete target as an absolute, control-free, valid-text, normal-component strict descendant and rejects filesystem-boundary crossings for the target, ancestors, and marker evidence. This closes a forged-cache terminal `.`/`..` alias that could otherwise escape or collapse the selected scan root before permanent removal.
- Exact `$HOME/Library` and `$HOME/AppData` roots are now denied by protected-path policy; only their descendants and scan scopes return the non-authoritative specific-rule requirement.
- Prevented deletion of the active scan root.
- Failed filesystem deletions no longer remove items from the displayed or cached tree.
- Unicode paths are truncated safely without slicing through UTF-8 characters.
- Scanner path exclusions now use component-aware rules, and crossed filesystems are classified before traversal instead of treating every `/Volumes` path as unsafe.
- Scanner metadata and filesystem probes now use a process-wide bounded worker pool instead of spawning one detached thread per directory.
- Probe results completed after their end-to-end deadline are rejected even if the waiting scanner thread was descheduled while the result arrived.
- Permanent-delete dialogs now require an explicit `y` and show the effective item count; Enter no longer confirms deletion.
- Graceful quit now waits for active deletion workers, applies their results, and orders cache writes so an older snapshot cannot overwrite the post-delete tree.
- Permanent deletes now capture non-following filesystem identity before confirmation and re-check it immediately before removal; replaced or missing entries are skipped without changing tree state or deletion statistics.
- Build-artifact entries now require exact, regular, non-symlink marker files instead of directory names alone. Ambiguous names such as `DerivedData`, `Build`, `dist`, `vendor`, and `.cache` are omitted until stronger rules exist.
- Artifact marker identities and every directory identity from the scan root to the target parent are re-checked before permanent deletion. Changed evidence, replaced ancestors, and symlink/reparse ancestors fail closed.
- Followed symlinks retain path provenance in scan snapshots, and deletion is refused below a known followed-symlink ancestor.
- Multi-delete now uses a fixed pool of at most four workers instead of spawning one thread per selected item. Per-item panics become failures without stranding later queued deletions, and graceful quit continues waiting for the complete confirmed batch.
- Cache writers now use exclusively created, process-specific temporary files instead of one shared `.tmp` path, preventing concurrent writers from truncating each other's in-progress snapshots.
- Footer selection bytes no longer double-count descendants when their selected ancestor already includes their size; the displayed bytes now match the effective multi-delete roots.

### Changed
- CLI cleanup surfaces now consistently identify deletion as permanent and label successful-item byte totals as scan estimates rather than measured freed capacity.
- Moved artifact classification, large-file projection, and staleness evaluation from the TUI into deterministic `dux-core` APIs. The CLI now retains only view caching and presentation while continuing to pass core marker evidence through pre-delete identity validation.
- File and directory scan nodes now retain modification times from their size metadata snapshot, allowing shared projections to account for recent file activity without another filesystem query. Unsupported pre-Unix-epoch values remain explicitly unknown so they cannot break cache writes or become trusted age evidence.
- Cache version bumped to v6 so scans created with the previous scanner policy, without symlink provenance, or without file modification times auto-invalidate.
- The Build Artifacts view now labels entries as “Marker-matched”; classification identifies likely tooling ownership but does not assert that contents are automatically safe or reproducible.
- Cached headers show the original scan age. Press `r` while browsing to rescan directly from the filesystem; the previous tree remains available if the replacement scan fails.
- CI now enforces RustSec and dependency source/license policy. Release automation tests and builds every target before publishing, produces a verified `SHA256SUMS`, publishes GitHub assets through a draft, and updates Homebrew without a third-party action handling the tap token.
- The application lockfile is now committed. Crossbeam was advanced past RUSTSEC-2026-0204, Postcard's unused heapless defaults were disabled, and Ratatui/Crossterm were upgraded to remove unmaintained and yanked transitive crates. The declared minimum Rust version is now 1.88.
- The scanner diagnostics example now lives in `dux-core/examples/debug_scan.rs`, so Cargo discovers it normally and includes it in the published `dux-core` package.

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
