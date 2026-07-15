# DUX macOS Product and Implementation Roadmap

Status: Draft implementation specification

Last updated: 2026-07-14

Primary platform: macOS 14 or later

Distribution: Direct download, Developer ID signed and notarized, not Mac App Store sandboxed

Existing products retained: `dux-core` and the `dux` terminal UI

## 1. Purpose of this document

This document is the implementation source of truth for evolving DUX from a terminal disk-usage analyzer into a native macOS disk-pressure assistant with:

- a persistent menu bar status item;
- a normal, resizable Explorer window;
- visual disk-usage and growth analysis;
- deterministic, reviewable cleanup recommendations;
- low-disk monitoring and notifications;
- opt-in scheduled cleanup of narrowly defined regenerable data;
- AI-assisted explanation and grouping that is never authorized to delete data; and
- an optional CLI installed from Settings while preserving the existing standalone CLI distribution.

The roadmap is deliberately explicit. An implementer should not need to invent safety policy, product boundaries, data contracts, or milestone order while working through it. If implementation reality conflicts with this document, update the document in the same pull request that changes the decision.

As normative specifications land (`SECURITY_DESIGN.md`, the rule schema, the AI contract), extract them into standalone documents and let this roadmap shrink toward product decisions and milestones — smaller focused documents keep the update-in-the-same-PR rule cheap to honor. Mirror milestone checkboxes into repository issues so progress is visible outside this file.

Normative terms:

- **MUST** is a correctness or safety requirement.
- **SHOULD** is the default unless a documented reason requires another choice.
- **MAY** is optional.

## 2. Product definition

DUX is a disk-pressure assistant, not a generic “Mac optimizer.” Its promise is:

> DUX explains where storage is going, identifies trustworthy ways to recover it, and keeps the user in control of every destructive action.

The product should answer four questions in order:

1. How much usable storage remains, and is the situation urgent?
2. What changed recently?
3. What can be reclaimed with the least effort and risk?
4. What is using the rest of the disk, including storage commonly grouped under “System Data”?

Use **storage** or **disk space** in user-facing copy. Do not call disk usage “memory”; on macOS, memory normally means RAM.

## 3. Fixed product decisions

These decisions are defaults for implementation and do not require further product discovery before work begins.

### 3.1 Platform and distribution

- Build a native SwiftUI macOS application.
- Target macOS 14 or later initially.
- Produce one universal application supporting Apple Silicon and Intel.
- Distribute outside the Mac App Store.
- Enable Hardened Runtime, Developer ID signing, notarization, and stapling.
- Do not enable App Sandbox for the primary build.
- Do not assume that being unsandboxed bypasses TCC privacy protections. The app MUST detect and explain incomplete access.
- Use `SMAppService.mainApp` for the user-controlled “Launch at Login” setting.
- Keep the app running as the menu bar process. Do not add a privileged daemon or launch daemon in the first release.

### 3.2 Technology boundaries

- Keep filesystem scanning, classification, cleanup planning, safety validation, and shared domain models in Rust.
- Keep macOS scenes, navigation, charts, notifications, settings UI, and command-provider configuration in Swift.
- Add a small Rust FFI crate and generate Swift bindings with UniFFI unless the Phase 0 spike demonstrates an unacceptable Swift concurrency or packaging problem.
- If UniFFI is rejected, use a narrow, versioned C ABI. Do not reimplement core policy in Swift.
- Use SQLite for durable history and settings that need queries.
- Keep large full-tree scan snapshots in versioned binary files rather than inserting millions of nodes into SQLite.

### 3.3 Safety boundaries

- AI MUST NOT create cleanup targets, change a safety tier, approve a cleanup, execute a command, or invoke filesystem tools.
- Arbitrary files selected in Explorer MUST go to Trash by default.
- Moving a file to Trash MUST NOT be reported as “space freed”; it is “moved to Trash” until the filesystem reports more available capacity.
- Permanent cleanup MAY be offered only for deterministic, tested, regenerable candidates or through an explicit advanced user action.
- Scheduled cleanup MUST be limited to rules marked safe and schedule-eligible by the shipped deterministic policy.
- DUX MUST fail closed. An uncertain candidate is shown for understanding, not cleanup.
- DUX MUST never empty the user’s entire Trash as a side effect of another cleanup.
- DUX MUST not require root or `sudo` in the first production release.

### 3.4 Privacy boundaries

- Scans and history stay local.
- No telemetry is required for core product operation.
- AI receives structured metadata only by default, never file contents.
- Absolute paths MUST be shortened to home-relative paths before being sent to AI unless the user explicitly enables full paths.
- Credentials, keychains, tokens, browser profiles, messages, mail, notes, cloud documents, password-manager data, and security-tool state MUST be excluded from AI payloads and cleanup suggestions.
- Local databases, snapshots, and caches contain full path listings of the user’s disk and are sensitive at rest: create them user-only (0700 directories, 0600 files) and cover data-at-rest handling in `SECURITY_DESIGN.md`.

### 3.5 Language and localization

- Ship 1.0 in English only.
- All user-facing strings MUST be centralized (String Catalogs in Swift, a single strings module in Rust) from the first commit so localization is a translation task, not a refactor.
- Swedish is the first localization candidate after 1.0.
- Copy rules in this document (for example “storage,” never “memory”) apply to every locale.

## 4. Current repository assessment

The current workspace has two crates:

- `dux-core`: scanner, tree arena, size formatting, and a versioned scan cache.
- `dux-cli`: Ratatui application, interaction state, computed views, and destructive filesystem operations.

Useful foundations already present:

- parallel scanning with progress and cancellation;
- physical allocation sizing on Unix via `st_blocks`;
- an arena tree with drill-down and incremental removal;
- versioned cached snapshots with checksum validation;
- large-file projection;
- build-artifact detection and staleness;
- Finder reveal from the CLI;
- multi-selection and asynchronous deletion;
- macOS, Linux, and Windows CLI CI/release coverage.

Verified defects in the current implementation (code-grounded review, 2026-07-14). These are fixed by the Milestone 0 hardening tasks:

1. `SLOW_PATTERNS` in `dux-core/src/scanner/walker.rs` matches substrings of the whole path, so any directory whose path contains `/dev/`, `/proc/`, `/sys/`, or `/Volumes/` is silently dropped. Confirmed empirically: a folder named `dev` under the scan root reports 0 bytes with no error and no coverage note.
2. Deletion does not survive quitting: deletes run on detached threads that die at process exit, so quitting mid-delete can leave a half-deleted directory and a cache that still records it intact. Documentation claiming deletion continues after quit is wrong.
3. Enter confirms permanent deletion in the confirm dialogs while also being the drill-down key in browsing mode, and the dialogs only advertise `[y]`/`[n]`.
4. Build-artifact classification matches directory names only (`target`, `build`, `vendor`, and so on) with no marker evidence, and feeds multi-select permanent deletion.
5. Cache invalidation (root mtime plus spot-checking the 32 largest directory mtimes) cannot see in-place file growth; the UI shows “(cached)” without the scan age and offers no rescan key.
6. `metadata_with_timeout` spawns one OS thread per directory scanned.
7. Multi-delete spawns one unbounded thread per selected item.
8. The footer selection total double-counts nested selections (parent plus child).
9. Concurrent dux instances scanning the same root race on the same cache temp file name (CRC32 makes this self-healing, but the rename race exists).

Required architectural corrections:

1. `dux-cli/src/app/views.rs` owns artifact classification. Move product-neutral classifications into Rust core so both UIs use identical results.
2. `dux-cli/src/main.rs` owns scan/cache orchestration. Introduce an engine service usable by CLI and FFI.
3. `dux-cli/src/app/state.rs` performs permanent deletion directly. Replace this with centralized validation, cleanup plans, platform execution, and operation records.
4. The current cache answers “load the latest tree,” not “what grew over time?” Add aggregate history.
5. Scan skips and permission failures are not a first-class result. Add coverage and incompleteness data.
6. Current nodes lack enough identity/evidence for cleanup safety, hard-link deduplication, and longitudinal comparison.
7. There is no volume-pressure model independent of a full directory scan.

Baseline gate before Phase 1:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

At the time of writing, all 24 existing Rust tests pass. Preserve that baseline while extracting behavior.

## 5. Mole research: lessons to adopt and avoid

Research source: [tw93/Mole](https://github.com/tw93/Mole), inspected at commit `7c681aef35d0dca6d1c3bdd85333ff571dc4be00` from 2026-07-11.

Mole is GPL-3.0. DUX is MIT. DUX MUST use clean-room behavioral inspiration only. Do not copy Mole source, protection lists, shell code, test corpora, wording, or proprietary Mole for Mac assets. New DUX rules must have independent reasoning, provenance, and tests.

### 5.1 Behaviors worth adopting

- A preview/dry-run path that uses the same planner and validator as execution.
- Centralized destructive operations; no ad hoc recursive deletion scattered through UI code.
- Layered absolute-path, traversal, control-character, protected-root, and symlink checks.
- Explicit sensitive-category protection in addition to root-path protection.
- Fail-closed handling when access, metadata, ownership, or safety is ambiguous.
- Separate “analyze and Trash” from deterministic cache cleanup.
- An operation history that distinguishes removed, trashed, skipped, rejected, and failed actions.
- Machine-readable CLI output for status, analysis, recommendations, and history.
- User-configurable project roots for build-artifact discovery.
- Recency-based artifact selection where recent projects are visible but not preselected.
- Parent/child candidate deduplication.
- Recognition of standard cache markers such as a valid, non-symlinked `CACHEDIR.TAG` as evidence, not as sole deletion authority.
- Fast and slow monitoring cadences rather than running every expensive probe every second.
- Explicit external-volume filtering and filesystem-type handling.
- Fuzz/property testing around path validation and destructive boundaries.
- CI that prevents new unguarded destructive calls from entering the codebase.
- Security design documentation next to implementation and tests.

### 5.2 Behaviors to adapt rather than copy

- Mole includes broad cleanup, uninstall, optimization, CPU, RAM, battery, and network functionality. DUX should stay focused on storage until the storage experience is excellent.
- A combined “health score” hides important distinctions. DUX should show disk pressure and evidence directly instead of producing a vague system-health number.
- Mole has permanent cleanup paths. DUX must make the execution mode and recoverability explicit for every plan item.
- Mole uses shell and Go implementations. DUX should express policy as typed Rust data and use native macOS APIs where appropriate.
- Mole has extensive sudo behavior. DUX should ship without privileged cleanup and add it only through a separately reviewed future design.
- Mole protection data evolves from its own incident history. DUX needs its own independently researched rule registry, security review, and adversarial tests.

### 5.3 New roadmap items added because of Mole research

- Add `SECURITY_DESIGN.md` before enabling any app cleanup.
- Add a dangerous-path regression corpus and Rust fuzz target.
- Add a lint/CI check banning raw destructive filesystem calls outside the executor module.
- Add dry-run and operation history to the core engine, not only to CLI presentation.
- Add candidate overlap resolution and symlink/volume boundary checks.
- Add distinct execution modes: Trash, permanent-safe cleanup, and reveal-only.
- Add protection-registry maintenance to each macOS major-version review.
- Add JSON CLI contracts so the GUI and external automation can inspect engine behavior.

## 6. Target repository layout

Evolve toward this layout incrementally:

```text
dux/
├── Cargo.toml
├── ROADMAP.md
├── SECURITY_DESIGN.md
├── dux-core/
│   ├── src/
│   │   ├── engine/
│   │   ├── scanner/
│   │   ├── tree/
│   │   ├── volume/
│   │   ├── candidates/
│   │   ├── rules/
│   │   ├── safety/
│   │   ├── cleanup/
│   │   ├── history/
│   │   └── cache/
│   └── resources/
│       ├── cleanup-rules/
│       └── protected-paths/
├── dux-ffi/
│   ├── src/lib.rs
│   └── uniffi.toml
├── dux-cli/
│   └── src/
├── dux-macos/
│   ├── Dux.xcodeproj/
│   ├── Dux/
│   │   ├── App/
│   │   ├── Models/
│   │   ├── Services/
│   │   ├── MenuBar/
│   │   ├── Explorer/
│   │   ├── Recommendations/
│   │   ├── History/
│   │   ├── Settings/
│   │   └── Resources/
│   ├── DuxTests/
│   └── scripts/
│       ├── build-rust-xcframework.sh
│       └── generate-bindings.sh
└── fixtures/
    ├── scans/
    ├── rules/
    └── dangerous-paths.txt
```

Avoid creating many crates before boundaries are proven. `dux-core`, `dux-ffi`, `dux-cli`, and the Xcode application are sufficient initially.

## 7. Core domain model

Implement these concepts before exposing them through FFI. Names may change, but responsibilities and invariants must remain.

### 7.1 Volume status

```rust
pub struct VolumeStatus {
    pub id: VolumeId,
    pub mount_path: PathBuf,
    pub display_name: String,
    pub total_bytes: u64,
    pub available_bytes: u64,
    pub important_available_bytes: Option<u64>,
    pub filesystem: Option<String>,
    pub is_internal: bool,
    pub is_removable: bool,
    pub sampled_at: SystemTime,
    pub pressure: DiskPressure,
}

pub enum DiskPressure {
    Healthy,
    Warning,
    Critical,
    Unknown,
}
```

Default startup-volume thresholds:

- Critical when important available capacity is at or below min(10 GiB, 5% of total capacity).
- Warning when at or below min(30 GiB, 10% of total capacity).
- Healthy otherwise.
- Use `min`, not or-semantics: “10 GiB or 5%, whichever triggers first” would put a 4 TB volume into Critical at 205 GiB free. With `min`, a 4 TB volume goes Critical at 10 GiB and a 64 GB volume at roughly 3.2 GiB.
- Thresholds are user-configurable, but the UI must always show the actual bytes and percent alongside the label.
- Apply hysteresis so a volume does not oscillate between states near a threshold. Default recovery margin: 2 GiB and 1 percentage point.

On macOS, obtain `volumeAvailableCapacityForImportantUsage` in Swift/Foundation and pass it to the Rust pressure evaluator. Use portable filesystem values as a fallback. Do not infer volume free space from scanned node totals.

### 7.2 Scan identity and coverage

Extend scan output with:

```rust
pub struct ScanSummary {
    pub id: ScanId,
    pub root: PathBuf,
    pub started_at: SystemTime,
    pub completed_at: Option<SystemTime>,
    pub logical_bytes: u64,
    pub allocated_bytes: u64,
    pub file_count: u64,
    pub directory_count: u64,
    pub errors: Vec<ScanIssue>,
    pub skipped: Vec<ScanIssue>,
    pub coverage: ScanCoverage,
    pub snapshot_path: Option<PathBuf>,
}

pub struct ScanIssue {
    pub path: PathBuf,
    pub kind: ScanIssueKind,
    pub user_message: String,
}

pub enum ScanIssueKind {
    PermissionDenied,
    TimedOut,
    DifferentFilesystem,
    NetworkOrVirtualFilesystem,
    SymlinkSkipped,
    FileChangedDuringScan,
    MetadataError,
    Cancelled,
}
```

Coverage is a first-class product value. The Explorer must show “Complete,” “Limited access,” or “Partial,” with a drill-down list. Never silently turn skipped bytes into zero.

### 7.3 Node metadata

Add only metadata required for correctness or product features:

- logical byte length;
- allocated byte count;
- modification time for files and directories (the current scanner records directories only; extending to files requires a cache format bump and adds per-node memory — plan both);
- optional access time, clearly marked as unreliable;
- device and inode identity on Unix for hard-link deduplication;
- file type;
- scan flags such as inaccessible, timed out, hard-link duplicate, and mount boundary;
- stable snapshot-local node ID;
- optional content-type/category determined without opening file contents.

Do not promise exact APFS clone accounting. Document that physical allocation totals can differ from Finder because of purgeable data, snapshots, clones, and filesystem accounting.

### 7.4 Candidate

```rust
pub struct Candidate {
    pub id: CandidateId,
    pub rule_id: RuleId,
    pub title: String,
    pub category: CandidateCategory,
    pub paths: Vec<PathBuf>,
    pub estimated_bytes: u64,
    pub newest_mtime: Option<SystemTime>,
    pub evidence: Vec<Evidence>,
    pub safety: SafetyTier,
    pub action: CandidateAction,
    pub schedule_eligible: bool,
    pub blocked_reason: Option<BlockReason>,
    pub source_scan_id: ScanId,
}

pub enum SafetyTier {
    SafeRegenerable,
    SafeEvictable,
    ReviewRequired,
    Informational,
    Protected,
}

pub enum CandidateAction {
    RemoveKnownRegenerableContents,
    EvictLocalCopy,
    MoveToTrash,
    RevealOnly,
    NoAction,
}
```

Candidate invariants:

- Every actionable candidate has exactly one shipped deterministic `rule_id`.
- AI output cannot populate or mutate any candidate field.
- A candidate path must be canonical, absolute, on an allowed volume, and inside the rule’s resolved scope.
- Candidates cannot overlap after planning. If parent and child match, the planner retains one according to explicit rule precedence.
- An unavailable or changed path is re-evaluated at execution time.
- Estimated bytes are estimates until execution and post-action capacity verification complete.
- `SafeEvictable`/`EvictLocalCopy` applies only to confirmed fully uploaded cloud items (for example iCloud Drive via the ubiquitous-item eviction API). Eviction MUST never target items with local-only changes, MUST be labeled non-destructive-but-requires-network-to-re-download, and MUST NOT be reported as deletion.

### 7.5 Cleanup plan and result

```rust
pub struct CleanupPlan {
    pub id: CleanupPlanId,
    pub created_at: SystemTime,
    pub source_scan_id: ScanId,
    pub mode: CleanupMode,
    pub items: Vec<CleanupPlanItem>,
    pub estimated_bytes: u64,
    pub warnings: Vec<PlanWarning>,
    pub expires_at: SystemTime,
}

pub enum CleanupMode {
    DryRun,
    Trash,
    PermanentSafe,
}

pub enum OperationStatus {
    Planned,
    DryRun,
    Trashed,
    Removed,
    Skipped,
    Rejected,
    Failed,
    ChangedSincePlan,
}
```

Plans expire after a short interval, initially 15 minutes. Execution MUST re-stat and revalidate every path. A plan created from a stale scan may be displayed, but it cannot execute without refresh.

## 8. Deterministic rule registry

Rules are bundled versioned data validated at build time and load time. Prefer JSON plus a checked-in JSON Schema. Do not allow arbitrary shell fragments in rules.

Required fields:

```json
{
  "id": "developer.rust.target",
  "version": 1,
  "title": "Rust build output",
  "category": "developer_artifact",
  "scope": "configured_project_roots",
  "path_component": "target",
  "required_ancestor_markers_any": ["Cargo.toml"],
  "required_markers_all": [],
  "excluded_descendants": [],
  "minimum_age_days": 7,
  "minimum_bytes": 104857600,
  "safety": "safe_regenerable",
  "action": "remove_known_regenerable_contents",
  "schedule_eligible": true,
  "explanation": "Build output regenerated by Cargo.",
  "provenance": ["https://doc.rust-lang.org/cargo/guide/build-cache.html"]
}
```

The exact Rust-target rule must be independently verified before shipping; the example above defines schema shape, not automatic approval.

Rule engine requirements:

- Expand only recognized variables such as home, user cache directory, and configured project roots.
- Reject unknown variables.
- Match path components, not unsafe substring patterns.
- Support required and forbidden marker files.
- Support exact bundle identifiers where app ownership matters.
- Support minimum age, size, and inactive-process guards.
- Support excluded descendants and protected child patterns.
- Include human explanation and provenance.
- Include a rule revision in every history item.
- Validate that schedule-eligible implies `SafeRegenerable` and permanent-safe action.
- Validate that protected paths cannot be weakened by a cleanup rule.
- Rules ship inside the signed app bundle. If remote rule updates are ever introduced, they MUST be signature-verified; unsigned remote rules are forbidden.
- Ship rules with positive, negative, nested, symlink, and changed-after-scan fixtures.

Initial independently researched categories:

Safe-regenerable candidates to investigate first:

- Rust `target` directories beneath confirmed Cargo projects;
- Xcode DerivedData with age and active-project guards;
- Swift Package Manager build output;
- Node package-manager caches and project `node_modules`, with recent projects unselected;
- Python bytecode and explicitly identified virtual environments;
- Gradle caches and confirmed project build output;
- CocoaPods caches versus project Pods, treated separately;
- browser cache subdirectories, excluding history, cookies, sessions, profiles, and credentials;
- crash reports and diagnostic logs with age limits;
- old application update packages and package-manager download caches;
- valid cache directories carrying a non-symlinked `CACHEDIR.TAG`, as supporting evidence only.

Evictable candidates (non-destructive local-space recovery):

- fully uploaded iCloud Drive items evicted with the Foundation ubiquitous-item API, disclosed as “stays in iCloud, re-downloads on demand”;
- other cloud providers only where a supported eviction API exists — never by deleting provider-managed files directly.

Review-required candidates:

- old installers in Downloads/Desktop;
- device backups;
- large archives;
- duplicate files;
- unused applications and their support data;
- simulator devices;
- old media downloads and offline caches;
- project artifacts without strong project markers.

Informational/protected areas:

- Time Machine local snapshots;
- swap and virtual memory;
- iCloud and other cloud-provider placeholders;
- Messages, Mail, Photos, Notes, and document libraries;
- container/VM disk images that may be active;
- endpoint security and device-management software;
- credentials, tokens, keychains, password managers, and browser profiles;
- unknown “System Data.”

## 9. Safety architecture

Create `SECURITY_DESIGN.md` during Phase 1 and keep it aligned with code.

### Layer 1: plan-only UI

UI code requests candidates and plans. It does not directly remove paths. Remove all direct `std::fs::remove_file`, `remove_dir_all`, shell `rm`, and Swift `FileManager.removeItem` calls from feature code.

### Layer 2: centralized executor

Only `dux-core/src/cleanup/executor.rs` and platform-specific modules may perform destructive filesystem operations. Add a CI script that fails if forbidden calls appear elsewhere, with narrow annotations for test temporary directories.

Where practical, enforce this at the type level: the executor’s mutating entry points take a witness type constructible only from a reviewed, unexpired, non-dry-run plan, so dry-run code paths cannot compile into mutations.

### Layer 3: lexical validation

Reject:

- empty paths;
- relative paths;
- `..` path components;
- NUL and control characters;
- bare filesystem roots;
- a user home root;
- top-level `/Applications`, `/Library`, `/Users`, `/Volumes`, `/System`, `/bin`, `/sbin`, `/usr`, `/etc`, `/var`, `/private`, and similar critical roots;
- a cleanup root equal to the scan root unless a narrowly defined rule permits only children.

`dux-core` is shared with the cross-platform CLI, so the validator MUST carry per-platform protected-root sets: `~/Library` itself on macOS (children reachable only through specific rules); `/home`, `/root`, `/opt`, `/srv`, and `/nix` on Linux; drive roots, `C:\Windows`, `C:\Users`, and `C:\Program Files` on Windows.

### Layer 4: canonical and symlink validation

- Use descriptor-relative or equivalent race-resistant operations where practical.
- Resolve symlinks without following them into protected roots.
- Refuse a symlinked cleanup base.
- Record (device, inode) identity for every path at plan time; execution MUST re-stat and compare both, returning `ChangedSincePlan` on mismatch.
- Reject paths that cross to a different volume unless the rule and plan explicitly target that volume.
- Treat disappearance as skipped, not success.

### Layer 5: protected categories

Maintain a separate deny registry for sensitive data. Denies override cleanup allows. The registry needs independently authored tests and a macOS-major-version review process.

### Layer 6: rule evidence

Execution requires that markers, bundle identity, age, and process guards still match. If evidence changed since planning, return `ChangedSincePlan`.

### Layer 7: explicit execution mode

- Explorer ad hoc selection defaults to Trash.
- Safe-regenerable cleanup defaults to a review screen that states permanent deletion is required to free space immediately.
- Scheduled cleanup can use only permanent-safe mode.
- Permanent mode requires a typed confirmation sentence the first time it is enabled globally; ordinary per-run confirmation can be simpler afterward.
- Settings must provide a global “Disable permanent cleanup” switch.

Trash execution requirements:

- macOS Trash MUST use `FileManager.trashItem(at:)` through the platform executor — never a manual move into `~/.Trash` (that loses put-back metadata, breaks on external volumes, and degrades into copy-plus-delete across volumes).
- The Linux CLI implements the XDG Trash specification or refuses Trash mode; it MUST NOT fake it with moves.
- Ad hoc Trash of a symlink trashes the link itself, never the target, and says so in the review UI.

### Layer 8: audit and verification

- Start a cleanup session before the first operation.
- Log rejected and failed attempts as well as success.
- Measure available capacity before and after.
- Never equate sum of file sizes with verified freed capacity.
- Show both “items removed” and “available storage increased by.”
- Keep a local manifest containing rule ID, rule revision, path, estimated size, execution mode, status, and error category.

### Safety test suite

At minimum test:

- empty, relative, traversal, Unicode, control-character, and very long paths;
- repeated/trailing slashes;
- symlinks to protected roots;
- symlinked Trash or cleanup roots;
- parent/child overlap;
- hard links;
- mount changes;
- permission changes;
- file replacement between plan and execution;
- rule marker removed after planning;
- protected child beneath an otherwise cleanable cache;
- active-process guard;
- cancellation halfway through a plan;
- process exit or crash halfway through a plan leaves a consistent operation journal that the next launch reports truthfully;
- graceful shutdown drains or cleanly cancels in-flight operations before the process exits;
- partial failures and retry;
- dry-run producing the same decisions without mutations;
- forbidden destructive-call CI lint;
- fuzz target invariant: every accepted path is absolute, non-root, traversal-free, and outside protected scopes.

## 10. Scanning and refresh architecture

Use three cadences rather than one full-disk scan loop.

### 10.1 Capacity sampling

- Cheap startup-volume capacity check at app launch.
- Repeat every five minutes while the app runs.
- Repeat on wake and mounted-volume changes.
- Persist at most one routine sample per hour, plus threshold transitions.
- Menu bar status reads this data and does not wait for a directory scan.

### 10.2 Targeted reclaim scan

- Scan known rule roots quickly when disk pressure enters Warning or Critical.
- Prefer user caches and configured project roots.
- Produce recommendations before starting an expensive home scan.
- Mark estimates with the scan time.

### 10.3 Explorer scan

- User-triggered scan of Home, a selected folder, startup volume, or external volume.
- Stream child summaries so the UI paints progressively.
- Support pause/cancel.
- Preserve current results until a replacement scan is usable.
- Cache the last complete snapshot per root.
- Do not follow symlinks by default.
- Stay on one filesystem by default.
- Allow explicit external-volume scans.
- Deduplicate hard-linked files within one scan.
- Bound metadata timeouts and worker count.
- Skip decisions MUST match absolute path prefixes and whole path components, never substrings, and SHOULD use filesystem-type detection (statfs) instead of name patterns; every skipped subtree MUST surface as a scan issue.
- Probe slow mounts with a bounded prober pool, not one thread per directory.
- Surface skipped and timed-out paths.

M0 implementation note: the process-wide probe pool deliberately caps potentially wedged kernel calls at four threads. Filesystem syscalls cannot be cancelled in-process; if all four workers become permanently stuck, later probes time out until DUX restarts. The scan-coverage work must surface this as pool exhaustion and should add a circuit breaker or killable helper-process design so a long-running menu-bar session does not repeatedly spend the full deadline.

### 10.4 Incremental freshness

After the initial app release, add FSEvents invalidation:

- Observe only roots the user has scanned or configured.
- Mark affected snapshot branches stale rather than trying to maintain exact sizes from events alone.
- Coalesce bursts.
- Schedule bounded subtree rescans.
- Fall back to full rescan after dropped events or event-ID discontinuity.

Do not make FSEvents a blocker for the first beta.

## 11. Persistence design

Store application data under:

```text
~/Library/Application Support/Dux/
├── dux.sqlite3
├── snapshots/
├── ai/
└── logs/
```

Cache-only data belongs under `~/Library/Caches/Dux/`.

### 11.1 SQLite tables

Create migrations for:

```text
schema_migrations
volumes
disk_samples
scans
scan_aggregates
scan_issues
candidates
cleanup_sessions
cleanup_items
rule_outcomes
ai_insights
schedules
settings
```

Minimum columns:

- `volumes`: stable ID, mount path, display name, filesystem, internal/removable flags, first/last seen.
- `disk_samples`: volume ID, sampled time, total, available, important available, pressure.
- `scans`: scan ID, root, start/end, status, snapshot version/path, counts, bytes, coverage.
- `scan_aggregates`: scan ID, category/rule/top-level path key, bytes, file count.
- `scan_issues`: scan ID, shortened path, issue kind, count/message.
- `candidates`: candidate ID, scan ID, rule ID/revision, tier, bytes, created time, status.
- `cleanup_sessions`: plan ID, start/end, mode, estimate, verified capacity delta, trigger source.
- `cleanup_items`: session ID, rule ID/revision, path, estimate, final status, error category.
- `rule_outcomes`: rule ID, cleaned time, bytes, next observed size, regrowth duration.
- `ai_insights`: input digest, provider, adapter version, model label if known, output, created time, expiration.
- `schedules`: rule/category scope, enabled, cadence, age, size cap, last/next run.

Do not store full millions-node trees in SQLite initially. Continue using versioned, checksummed snapshot files. Add atomic write and migration/invalidation behavior.

### 11.2 Retention

- Hourly disk samples: 30 days.
- Daily rolled-up samples: one year.
- Cleanup history: retained until user clears it.
- AI insights: default 30 days, user-clearable, and regenerated on input digest change.
- Full snapshots: latest two complete snapshots per root plus any snapshot referenced by an active cleanup review.
- The snapshots directory has a total size cap (default 2 GiB, configurable); evict oldest first. Settings shows DUX’s own disk footprint with a clear-data action — a disk-pressure tool must not be a storage thief itself.
- Never delete history during cleanup without a separate settings action.

### 11.3 Multi-process access

The app, the bundled CLI, the standalone CLI, and (later) the scheduler are separate processes sharing this data.

- Open SQLite in WAL mode with a busy timeout; writes go through one store coordinator per process, and an advisory lock serializes writers across processes.
- Snapshot and cache writers use per-process unique temp names plus atomic rename; never a shared fixed `.tmp` name.
- An older binary opening a newer schema MUST NOT write: it either falls back to read-only with a clear message or exits cleanly. Define and test both directions of version skew.
- All files under `Application Support/Dux` and `Caches/Dux` are created user-only (0700 directories, 0600 files).

## 12. macOS application specification

### 12.1 Scene structure

Use:

- `MenuBarExtra` with window style;
- a normal `WindowGroup` for Explorer;
- a Settings scene;
- `LSUIElement` so the app can behave as a menu bar utility;
- an explicit setting to show a Dock icon while Explorer is open if testing proves that improves window discoverability.

One shared `AppModel`/coordinator owns engine state. Menu bar and Explorer must never start duplicate scans accidentally.

### 12.2 Menu bar label

Support three user-selectable modes:

- icon only;
- icon plus free GiB;
- icon plus free percent.

The label must remain legible in light/dark menu bars and accessibility contrast modes. Do not rely only on red/amber/green; pair state with symbol shape or text.

### 12.3 Menu bar popover

Target width: approximately 340–400 points. Keep it glanceable.

Order:

1. Startup volume name and pressure label.
2. Large free/total value and compact capacity bar.
3. Trend: change over 24 hours and seven days.
4. “Easy recovery” card with verified estimate and last scanned time.
5. Top three deterministic recommendation groups.
6. Primary button: `Review and recover…` opens Explorer Recommendations.
7. Secondary actions: `Scan now`, `Open Explorer`, `Settings`, `Quit`.

Milestone availability: items 1–2 and 7 ship in Milestone 3; item 3 (trend) arrives with Milestone 6 samples; items 4–6 depend on Milestone 5 candidates. Earlier milestones omit those slots entirely rather than showing placeholder data.

During scanning:

- keep the last known values visible;
- show unobtrusive progress;
- allow cancellation;
- do not replace recommendations with a blank loading state.

Critical state:

- lead with “X GiB available”;
- show the fastest safe candidates first;
- distinguish “frees space now” from “moves to Trash.”

### 12.4 Explorer navigation

Sidebar destinations:

- Overview
- Recommendations
- Explore
- Large Files
- History
- Automations
- Settings shortcut

Toolbar:

- selected volume/root;
- scan freshness;
- scan/refresh button;
- search;
- coverage indicator;
- optional AI Explain button when configured.

### 12.5 Overview

Show:

- total, available, used, and estimated reclaimable storage;
- a segmented capacity bar with honest unknown/unscanned space;
- 30-day free-space line chart;
- top growth categories since the prior comparable scan;
- top recommendation groups;
- coverage and Full Disk Access status.

Charts must be accessible with text summaries and keyboard focus. Every segment opens a filtered detail view.

### 12.6 Recommendations

Group by user-understandable outcome, not raw path:

- Developer build output
- App and browser caches
- Logs and diagnostics
- Installers and downloads
- Device and simulator data
- Cloud files that can free local space (evictable)
- Large review items
- Unknown items to understand

Each group displays:

- estimated reclaimable bytes;
- safety tier;
- execution mode;
- number of paths;
- newest activity;
- why DUX believes it is recoverable;
- regeneration impact;
- schedule eligibility;
- disclosure list of exact paths and exclusions.

No group-level action runs immediately from the menu bar. It opens review. The final review screen lists the exact plan and mode.

### 12.7 Explore

Primary layout:

- interactive treemap;
- breadcrumb path;
- sortable child list synchronized with treemap selection;
- inspector panel with size, counts, timestamps, classification, coverage, and Finder actions.

Interactions:

- single click selects;
- double click/Return drills into a directory;
- Backspace or toolbar back returns;
- Space opens Quick Look where supported;
- Command-R rescans current subtree;
- Command-Delete opens a Trash review, never immediate deletion;
- context menu includes Reveal in Finder, Copy Path, Explain, Exclude, and Review for Trash.

Treemap requirements:

- stable layout during progressive updates when practical;
- minimum visible-cell threshold with an “Other” aggregate;
- color by category, not random directory;
- highlight reclaimable candidates with an overlay/badge rather than replacing category color;
- textual fallback list for accessibility and small windows.

### 12.8 Large Files

- Default threshold: 1 GiB, configurable.
- Filter by age, type, root, and candidate status.
- Do not preselect files for deletion.
- Show whether a file is a cloud placeholder, package content, backup, VM image, or unknown when deterministically known.
- Detect nested selections so deleting a directory and one of its child files cannot produce two plan items.

### 12.9 History

Show:

- free-space trend;
- scan timeline;
- cleanup sessions;
- estimated versus verified capacity change;
- trashed versus permanently removed item counts;
- failed/rejected/skipped operations;
- recurring rule groups and regrowth rate.

“Storage thieves” are computed from history, not AI:

- aggregate by deterministic rule ID;
- observe size after cleanup in later scans;
- rank by bytes regrown per day and repeated cleanup count;
- suggest automation only after at least two successful manual cleanups and one observed regrowth cycle;
- never auto-enable a schedule.

### 12.10 Permissions onboarding

On first launch:

1. Explain what DUX scans and that it runs locally.
2. Start with access available without Full Disk Access.
3. Show a coverage result.
4. Offer a guided Full Disk Access step only when broader analysis is requested.
5. Re-probe after the user returns from System Settings.

The product remains useful without Full Disk Access. Never show a false complete result.

### 12.11 Future feature candidates (post-first-beta)

Ideas that pass the §23 scope gate but are deliberately not scheduled yet:

- App-centric storage attribution: map `~/Library/{Application Support,Caches,Containers}` to bundle identifiers for a read-only “storage by app” view, including orphaned data from deleted apps as review candidates.
- Container/VM awareness: detect Docker/OrbStack/Colima images and simulator runtimes; where reclaim requires a vendor tool, show the exact command for the user to copy — DUX never executes it.
- Menu bar free-space sparkline.
- Exportable storage report (JSON or HTML) for support and team use.
- Duplicate detection stays deferred until hashing cost and UX are designed.

## 13. Low-disk monitoring and notifications

### 13.1 State machine

```text
Unknown -> Healthy -> Warning -> Critical
                    ^          |
                    |----------|
```

- Persist the prior state per volume.
- Notify on a transition into Warning or Critical, not on every sample.
- Default notification cooldown: 24 hours per level.
- A Critical transition may bypass a Warning cooldown.
- Recovery to Healthy clears the episode after hysteresis requirements are met.

### 13.2 Notification content

Warning example:

> 27 GiB remains. DUX found approximately 14 GiB of reviewable build output and caches.

Critical example:

> 7 GiB remains. Review the safest ways to recover storage now.

Do not say DUX “can free” an amount unless all included candidates are current, actionable, and permanent-safe. Otherwise use “found” or “reviewable.”

Clicking the notification opens Recommendations filtered to the affected volume and urgency.

### 13.3 Emergency recovery ordering

When Critical:

1. evictable cloud items (non-destructive; frees local space immediately when fully uploaded);
2. stale safe-regenerable candidates;
3. Trash size as information only;
4. old installers and archives requiring review;
5. large files;
6. guided storage exploration;
7. permission gaps that may hide large areas.

Do not recommend risky system modifications merely because capacity is critical.

## 14. AI advisor architecture

AI is an optional explanation layer. The deterministic engine must remain fully useful without it.

### 14.1 Supported provider model

Initial adapters:

- Claude CLI adapter;
- Codex CLI adapter;
- disabled/no-provider adapter.

Add a generic custom-command adapter only after the fixed adapters establish a safe contract.

The macOS app is launched outside a login shell, so provider discovery must:

- accept a user-selected executable path;
- probe common user binary directories without invoking a shell;
- canonicalize and display the resolved executable;
- show version/probe status;
- never execute a raw user-authored shell string;
- record the provider version ranges each adapter was tested against; the probe rejects unknown major versions instead of guessing at flags.

### 14.2 Invocation contract

- Launch executable plus an adapter-owned fixed argument vector.
- Never use `/bin/sh -c`.
- Use a sanitized environment with a minimal `PATH` and required provider auth environment only.
- Set working directory to a new empty temporary directory.
- Disable provider filesystem, shell, network-tool, MCP, and agentic tools where the provider supports it.
- If tools cannot be disabled reliably, mark the adapter unsupported.
- Send one JSON document over stdin.
- Enforce timeout, cancellation, output byte limit, and process-tree termination.
- Capture stderr separately and redact it before display/logging.
- Require output matching a versioned JSON Schema.
- Cache by a digest of redacted input plus adapter version.

### 14.3 AI input

```json
{
  "schema_version": 1,
  "task": "explain_storage_cluster",
  "root_label": "~/Library/Application Support/Example",
  "total_bytes": 123,
  "age_summary": {},
  "children": [],
  "known_classifications": [],
  "protected": false,
  "content_included": false
}
```

Limits:

- bounded child count and depth;
- no file content by default;
- no credentials or sensitive-category paths;
- no environment dump;
- no complete home directory listing in a single prompt;
- indicate omitted/aggregated children;
- file and directory names are untrusted input: they appear only as JSON data fields, are never concatenated into instruction text, and adapters assume they may contain prompt-injection attempts.

### 14.4 AI output

```json
{
  "schema_version": 1,
  "summary": "Likely generated support data for …",
  "labels": ["developer-tool", "cache-like"],
  "groups": [
    {
      "title": "Generated indexes",
      "input_node_ids": ["n1", "n2"],
      "reason": "…"
    }
  ],
  "questions": ["Do you still use …?"],
  "uncertainties": ["Could not determine …"]
}
```

Output rules:

- Node references must already exist in the input.
- Unknown node IDs invalidate the group.
- AI safety claims are rendered as AI text, never converted to DUX safety badges.
- AI cannot return paths or actions.
- The UI labels AI output and provider.
- The user can inspect exactly what metadata was sent.

### 14.5 AI product actions

Allowed:

- Explain selected folder.
- Summarize a large unknown cluster.
- Group existing nodes for presentation.
- Generate questions the user can answer.
- Suggest that maintainers research a future deterministic rule.

Forbidden:

- “Clean with AI.”
- Feeding AI output into `CleanupPlan`.
- Asking the provider to inspect the disk directly.
- Allowing AI to execute provider tools.
- Using AI confidence as a safety tier.
- Scheduled AI-triggered cleanup.

## 15. Periodic cleanup and automations

Automation ships only after manual cleanup history and safety tests are mature.

### 15.1 Eligibility

A rule is automation-eligible only when:

- shipped policy marks it `SafeRegenerable` and schedule-eligible;
- the user has manually cleaned it successfully at least twice;
- no failures or protected descendants were observed in the last two runs;
- the current candidate meets age and size thresholds;
- the relevant app/process is not active when an activity guard exists;
- the scan and rule evidence are fresh;
- execution does not require `sudo`.

### 15.2 User controls

Per automation:

- category/rule scope;
- cadence: weekly, monthly, or low-disk only;
- minimum age;
- minimum reclaimable size;
- maximum bytes per run;
- exclusions;
- notify before run;
- require confirmation versus fully automatic;
- pause and delete schedule.

Defaults:

- disabled;
- monthly;
- 30-day minimum age;
- 25 GiB maximum per run;
- notification before the first three runs;
- permanent-safe mode clearly disclosed.

### 15.3 Execution

- The menu bar app must be running.
- Use an in-process scheduler initially.
- Persist next-run state.
- On wake, run at most one missed job and only after revalidation.
- Avoid running during an active manual scan, high thermal pressure, or on battery below a conservative threshold.
- Cancel cleanly on app quit.
- Record trigger source as scheduled, low-disk, or manual.
- Notify with verified result and failures.

Do not add a launch agent until evidence shows launch-at-login plus a persistent menu bar process is insufficient.

## 16. CLI strategy

Retain the TUI and make it a companion to the app.

### 16.1 Commands

Preserve existing behavior and add incrementally:

```text
dux                         # existing TUI
dux scan [PATH]
dux status [--json]
dux recommendations [--json]
dux plan [--json]           # always a dry-run; plans never execute from this command
dux history [--json] [--limit N]
dux rules list [--json]
dux doctor [--json]         # coverage, Full Disk Access state, skipped roots, cache/database health
```

Do not add permanent cleanup CLI commands until the shared planner/executor and safety suite exist. CLI cleanup must use the same plans, validation, and history as the app.

### 16.2 Install from Settings

- Bundle the universal `dux` binary inside the app.
- Default install target: `~/.local/bin/dux`.
- Detect an existing target and show version/source before replacement.
- Write atomically.
- Never overwrite a non-DUX binary.
- Explain how to add `~/.local/bin` to `PATH` if missing.
- Offer Uninstall CLI.
- Continue publishing Homebrew/crates.io CLI releases independently.
- App and bundled CLI should share compatible database/snapshot schema versions; older binaries must fail gracefully rather than corrupting data.

### 16.3 JSON compatibility

- Every JSON object includes `schema_version`.
- Add golden tests.
- Add fields compatibly; reserve breaking changes for schema increments.
- Errors go to stderr and nonzero exit codes.
- When stdout is not a TTY, noninteractive commands may default to JSON only if explicitly documented and tested.

## 17. FFI boundary

Keep the FFI coarse-grained. Do not expose millions of Rust nodes as chatty one-call-per-node objects.

Suggested API:

```text
create_engine(config) -> EngineHandle
get_volume_status() -> [VolumeStatusDto]
start_scan(request, callback) -> TaskId
cancel_task(task_id)
get_snapshot_summary(scan_id) -> SnapshotSummaryDto
get_scan_issues(scan_id, page) -> ScanIssuePageDto
get_children(scan_id, node_id, sort, page) -> NodePageDto
get_treemap(scan_id, node_id, budget) -> TreemapDto
get_candidates(scan_id, filter) -> CandidatePageDto
create_cleanup_plan(candidate_ids, mode) -> CleanupPlanDto
execute_cleanup_plan(plan_id, callback) -> TaskId
get_history(query) -> HistoryPageDto
record_ai_insight(input_digest, insight)
```

Requirements:

- DTOs are immutable/versioned at the boundary.
- Pagination/budgets prevent huge crossings.
- Long work is cancellable.
- Callbacks deliver typed events, not UI strings.
- Rust panics must not cross FFI.
- Swift calls engine work off the main actor.
- Generated bindings and XCFramework are reproducible in CI.
- Add an FFI smoke-test host executable before building substantial UI.

## 18. Concurrency and state management

Rust engine:

- one task registry;
- one active full scan per volume/root scope;
- bounded worker pools;
- cancellation tokens;
- immutable published snapshots;
- serialized cleanup execution;
- SQLite writes through a single store coordinator or transaction-safe pool.

Swift app:

- `@MainActor AppModel` contains renderable state only;
- `EngineService` bridges callbacks into `AsyncStream`;
- `VolumeMonitor` owns cheap samples;
- `NotificationService` owns authorization and threshold notifications;
- `AIProviderService` owns external command processes;
- `LoginItemService` owns `SMAppService` state;
- `CLIInstallerService` owns optional CLI installation.

Task priority:

1. cleanup execution and user cancellation;
2. capacity sampling;
3. user-triggered subtree scan;
4. user-triggered full scan;
5. targeted recommendation scan;
6. history rollup and background maintenance.

## 19. Testing strategy

### 19.1 Rust unit/property tests

- tree aggregation and deletion propagation;
- hard-link deduplication;
- candidate classification;
- rule schema validation;
- overlap resolution;
- protected-path precedence;
- plan expiration and changed-since-plan;
- disk-pressure hysteresis;
- history/regrowth calculations;
- AI schema validation independent of provider execution;
- hostile-filename AI inputs (prompt-injection attempts embedded in names) yield schema-valid, non-actionable output or rejection;
- snapshot/database migration.

### 19.2 Fixture tests

Create filesystem fixtures representing:

- Rust, Node, Xcode, Swift, Python, Gradle, and mixed projects;
- recent and stale artifacts;
- nested artifact names;
- misleading directory names without required markers;
- symlink loops and symlinks into protected roots;
- inaccessible directories;
- hard links;
- cloud placeholders where fixtures are possible;
- active and inactive app guards;
- protected descendants inside cache-like parents.

### 19.3 macOS integration tests

- volume capacity conversion;
- Trash execution in an isolated test account/temp volume;
- Finder reveal;
- launch-at-login registration state without enabling it on CI machines;
- notification deep links;
- Full Disk Access probe result mapping;
- universal Rust XCFramework loading;
- CLI install/upgrade/uninstall in a temporary HOME;
- AI process timeout, malformed output, excessive output, cancellation, and missing executable.

### 19.4 Swift UI tests

- menu bar healthy/warning/critical snapshots;
- no-scan and partial-coverage states;
- recommendation review and execution-mode disclosure;
- progressive scan retains old results;
- treemap/list synchronization;
- keyboard navigation;
- accessibility labels and text alternatives;
- notification deep link;
- AI disabled/configured/error states.

### 19.5 Performance gates

Define synthetic fixtures and measure:

- app idle CPU near zero;
- capacity sample duration;
- memory for 1M and 5M node snapshots;
- time to first top-level scan result;
- time to rebuild candidate projections;
- FFI page transfer size;
- treemap layout time;
- SQLite history query latency;
- cancellation latency.

Initial targets are budgets, not promises:

- menu bar open under 150 ms using cached state;
- capacity sample under 500 ms normally;
- cancellation acknowledged under one second;
- Explorer list/treemap interactions remain responsive at 60 Hz;
- idle app uses no continuous full scan and no one-second polling.

## 20. Security, privacy, and release gates

Before any public build with cleanup:

- `SECURITY_DESIGN.md` exists and matches implementation.
- Dangerous-path corpus and fuzz seeds exist.
- No-raw-delete CI lint passes.
- Dry-run integration test proves no filesystem mutation.
- Trash and permanent-safe modes are visually distinct.
- Operation history includes rejected/failed actions.
- Rules have provenance and negative tests.
- AI cannot reach cleanup plan APIs.
- App works with AI disabled.
- App works without Full Disk Access and shows partial coverage.

Before each release:

```text
Rust fmt, clippy, tests
Swift build and tests on supported macOS versions
Universal architecture verification
Rule schema and fixture validation
Database/snapshot migration tests
Forbidden destructive-call scan
Dependency vulnerability/license review (cargo audit and cargo deny in CI, not manual)
Archive signing verification
Notarization and stapling verification
Sparkle/appcast signature verification if Sparkle is adopted
CLI release compatibility test
```

Add a macOS-major-version audit issue/template covering:

- protected system paths and app containers;
- TCC behavior;
- volume-capacity/APFS behavior;
- Full Disk Access guidance;
- menu bar and SwiftUI regressions;
- bundled CLI architectures;
- rule validity for Xcode/simulators and Apple developer tooling.

### 20.1 Release pipeline security (applies to the existing CLI pipeline immediately)

- Pin every GitHub Action to a full commit SHA; mutable tags and branches are forbidden. The current `copy_file_to_another_repo_action@main` holding a cross-repo PAT is the exact shape of a Homebrew-tap poisoning attack.
- Replace the third-party copy action with an inline `git clone`/`commit`/`push` using a fine-grained PAT scoped to the tap repository only.
- Order release jobs test → build all targets → publish to crates.io → upload assets → update tap. crates.io publishes are immutable and MUST come after everything else is verified.
- Publish a `SHA256SUMS` asset for every release; compute checksums in the build job that produced the artifacts.

## 21. Milestone plan

Each milestone should land as several reviewable pull requests. Do not combine core safety extraction, FFI, and a full UI in one change.

### Milestone 0: Baseline hardening and architecture spikes

Goal: fix what is broken in the shipped CLI, secure the release pipeline, and prove the risky integration choices without product expansion.

Hardening tasks (ship as v0.5.x patch releases; see the verified defect list in §4):

- [x] Fix scanner skip patterns: absolute-prefix and path-component matching plus statfs-based detection, with a regression fixture containing a directory named `dev`. Completed 2026-07-15; cache v5 invalidates snapshots created under the old scan policy and additionally persists followed-symlink provenance.
- [x] Replace one-thread-per-directory scanner probes with a process-wide four-worker bounded pool, one end-to-end deadline, cancellation polling, and abandoned-job suppression. Completed 2026-07-15.
- [x] Join in-flight deletions on quit (or require explicit confirmation to abandon them); correct the documentation that claims deletion survives quit. Completed 2026-07-15 with deferred quit, tracked worker handles, disconnected-worker recovery, and ordered cache persistence.
- [x] Remove Enter as a delete-confirmation key; show item counts in both confirm dialogs. Completed 2026-07-15 with explicit permanent-delete copy and focused key/render tests.
- [x] Re-stat and compare filesystem identity immediately before every delete. Completed 2026-07-15: identity is captured before the confirmation UI opens, then the shared single/batch deletion helper compares non-following `(device, inode)` metadata on Unix or `(volume serial, 128-bit file ID)` from a non-following Windows handle immediately before removal. Mismatch, disappearance, and inspection failure leave tree/cache/statistics state unchanged.
- [x] Gate artifact classification on marker evidence (for example, `target` requires a sibling `Cargo.toml`). Completed 2026-07-15 with the fail-closed M0 rule set and execution checks detailed below.
- [x] Bound multi-delete concurrency with a small worker pool. Completed 2026-07-15: confirmed batches are queued onto at most four named workers, per-item results retain the existing progress/tree/statistics behavior, and every pool handle remains tracked for deferred quit and `Drop` joining. A panic while processing one item is converted into that item's failure so later queued work can continue; concurrency-cap and larger real-batch fixtures cover the integration.
- [ ] Show cache age in the header; add a rescan keybinding; use per-process cache temp names.
- [ ] Fix the footer selection total double-counting nested selections.
- [ ] Apply §20.1 to the release workflow and add cargo audit/deny to CI.
- [ ] Move `debug_scan.rs` into `dux-core/examples/`.

M0 deletion-lifecycle note: graceful in-app quit deliberately waits for active permanent deletions and cannot cancel a filesystem syscall already in progress. A truly hung delete can therefore keep graceful quit waiting indefinitely; external force termination may leave partial filesystem work and stale cache state. A force-abandon flow belongs with the later centralized executor and must require explicit destructive-risk confirmation.

M0 artifact-evidence implementation:

- Classification requires a directory node with no followed-symlink ancestor. A marker is accepted only when its final path component is an exact-case, direct sibling/child regular file that is not itself a followed symlink.
- Current positive rules are: Cargo `target` + sibling `Cargo.toml`; Node `node_modules` + sibling `package.json`; Gradle `build` or `.gradle` + sibling `build.gradle`, `build.gradle.kts`, `settings.gradle`, or `settings.gradle.kts`; Python `__pycache__` + sibling `*.py`, `.tox` + sibling `tox.ini`, or `.venv`/`venv` + child `pyvenv.cfg`; CocoaPods `Pods` + sibling `Podfile` + child `Manifest.lock`; Next `.next` + sibling `package.json` + `next.config.{js,mjs,ts}`; Nuxt `.nuxt` + sibling `package.json` + `nuxt.config.{js,mjs,ts}`.
- `DerivedData`, `Build`, `dist`, `vendor`, and `.cache` deliberately remain unclassified: their names and plausible nearby files do not yet provide sufficiently specific evidence for a permanent-delete-backed entry.
- The UI says “Marker-matched”, not “safe”. In particular, `pyvenv.cfg` and a CocoaPods manifest identify ownership but do not prove that all contents can be reproduced. No entry is scheduled or automatically removed, and permanent deletion still requires explicit confirmation.
- A delete plan captures the identities of all classification markers, the target, and every non-followed directory from the scan root through the target parent. Execution rejects changed markers, changed/missing ancestors, and symlink/reparse ancestors before re-checking the target immediately before removal.
- Positive, missing-marker, partial-marker, wrong-case, wrong-location, wrong-kind, symlink-marker, symlink-ancestor, changed-evidence, nested-artifact, and live ancestor-replacement fixtures cover the rules. The evidence model follows the upstream tool layouts documented by [Cargo](https://doc.rust-lang.org/cargo/reference/build-cache.html), [npm](https://docs.npmjs.com/files/folders/), [Python bytecode](https://docs.python.org/3/faq/programming.html), [Python virtual environments](https://docs.python.org/3/library/venv.html), [Gradle](https://docs.gradle.org/current/userguide/gradle_directories.html), [CocoaPods](https://guides.cocoapods.org/using/the-podfile.html), [Next](https://nextjs.org/docs/pages/api-reference/config/next-config-js/distDir), and [Nuxt](https://nuxt.com/docs/3.x/directory-structure/nuxt).

M0 identity-check note: request-time capture protects the confirmation-to-execution interval, including marker and ancestor replacement, but it does not prove that a cached/previous scan still describes the object present when confirmation opens. The separate ancestor/target identity checks and removal syscall also leave a narrow final TOCTOU window, and an unchanged directory identity does not freeze its descendants. The later plan model must carry scan/plan identity, and the centralized executor should use descriptor-relative or handle-relative mutation where the platform permits it.

Spike tasks:

- [ ] Add architecture decision records for native SwiftUI, direct distribution, no sandbox, and shared Rust engine.
- [ ] Create minimal `dux-ffi` returning its version and one size-format result.
- [ ] Build Rust for arm64 and x86_64 and package a universal XCFramework.
- [ ] Generate/import Swift bindings in a minimal Xcode app.
- [ ] Call Rust off the main actor and render the result.
- [ ] Verify Debug and Release builds from a clean checkout.
- [ ] Decide UniFFI versus C ABI and record the decision.
- [ ] Prototype `MenuBarExtra` plus normal Explorer window and Settings.
- [ ] Prototype Foundation important-usage volume capacity.

Exit criteria:

- Hardening fixes have shipped in a v0.5.x release and the §4 defect list is resolved.
- A clean CI job builds the universal app shell.
- The menu bar can open the normal window.
- Swift receives a typed Rust value.
- The integration approach has no unresolved blocker.

### Milestone 1: Shared engine extraction and safety foundation

Goal: make CLI behavior reusable and remove UI-owned destructive authority.

Tasks:

- [ ] Move `ArtifactKind`, artifact classification, large-file projection, and staleness calculation from CLI into core.
- [ ] Record file modification times in scan nodes (cache format bump) to support age guards.
- [ ] Introduce candidate and rule domain types.
- [ ] Introduce rule schema/loader with fixture validation.
- [ ] Introduce cleanup-plan types without execution.
- [ ] Introduce lexical/canonical path validator.
- [ ] Add protected-root registry with per-platform sets (macOS, Linux, Windows).
- [ ] Add dangerous-path corpus and fuzz/property tests.
- [ ] Add `SECURITY_DESIGN.md`.
- [ ] Add forbidden destructive-call CI lint.
- [ ] Route existing CLI delete requests through a temporary centralized executor adapter.
- [ ] Preserve CLI behavior and tests; clearly label current permanent deletion until replaced.

Exit criteria:

- CLI and core tests pass on all existing platforms.
- Classification results are owned by core.
- No feature UI directly invokes recursive deletion.
- Planner can produce dry-run decisions from fixtures.
- Safety documentation and regression corpus are reviewed.

### Milestone 2: History and engine service

Goal: provide a durable orchestration layer shared by CLI and app.

Tasks:

- [ ] Add engine handle and task registry.
- [ ] Add versioned SQLite migrations.
- [ ] Add scan/session/candidate/cleanup persistence.
- [ ] Keep binary snapshots atomic and checksummed.
- [ ] Add capacity sample storage.
- [ ] Add typed scan coverage/issues.
- [ ] Add JSON CLI status/history scaffolding.
- [ ] Add history retention maintenance.
- [ ] Add engine integration tests with temporary HOME and database.

Exit criteria:

- A scan creates a durable summary and snapshot.
- History queries survive process restart.
- Corrupt/incompatible snapshots fail safely.
- CLI can print versioned JSON status and history.

### Milestone 3: Native menu bar MVP

Goal: ship a useful read-only disk-pressure companion.

Tasks:

- [ ] Build `AppModel`, `EngineService`, and `VolumeMonitor`.
- [ ] Implement capacity sampling and pressure hysteresis.
- [ ] Implement menu bar label modes.
- [ ] Implement popover layout with cached status and scan progress.
- [ ] Implement Explorer window shell and Overview.
- [ ] Implement launch-at-login setting.
- [ ] Implement notification permission UI but do not notify repeatedly.
- [ ] Implement permission/coverage onboarding.
- [ ] Add signed/notarized local release script.

Exit criteria:

- App starts at login when enabled.
- Menu bar displays accurate important available capacity without a scan.
- Popover opens Explorer reliably.
- Read-only app is useful with no AI and no Full Disk Access.
- Idle resource usage meets the initial budget.

### Milestone 4: Visual Explorer

Goal: reach feature parity with CLI navigation and materially improve clarity.

Tasks:

- [ ] Expose paged children and treemap-budget APIs over FFI.
- [ ] Add progressive scan events.
- [ ] Implement treemap, synchronized list, breadcrumbs, history navigation, and inspector.
- [ ] Implement Large Files.
- [ ] Implement scan coverage details.
- [ ] Implement reveal, copy path, and Quick Look.
- [ ] Add category colors and accessible text alternatives.
- [ ] Add scan cancellation and subtree refresh.
- [ ] Add performance fixtures for million-node snapshots.

Exit criteria:

- Users can scan, drill down, go back, and inspect large files.
- UI stays responsive during scans.
- Unknown/unscanned storage remains visible.
- No destructive action exists outside plan review.

### Milestone 5: Deterministic recommendations and reviewed cleanup

Goal: provide trustworthy recovery actions.

Tasks:

- [ ] Ship the first independently researched safe-regenerable rules.
- [ ] Implement candidate groups and overlap resolution.
- [ ] Implement exact-path plan review.
- [ ] Implement Trash executor for Explorer selections.
- [ ] Implement permanent-safe executor for approved rules.
- [ ] Implement execution-time revalidation.
- [ ] Implement cleanup session/item history.
- [ ] Implement pre/post capacity verification.
- [ ] Implement exclusions and global permanent-cleanup disable setting.
- [ ] Add partial failure, retry, cancellation, and changed-since-plan UI.

Exit criteria:

- Every mutation originates from a current, reviewed plan.
- Trash actions do not claim immediate freed space.
- Permanent actions are limited to safe deterministic rules.
- All actions, failures, rejections, and skips appear in History.
- Safety integration and fuzz tests pass.

### Milestone 6: Low-disk response and trends

Goal: make DUX proactive and explain recurring disk pressure.

Tasks:

- [ ] Persist disk samples and pressure episodes.
- [ ] Add 24-hour/7-day changes and 30-day chart.
- [ ] Add transition-based notifications and cooldown.
- [ ] Deep-link notifications to urgent Recommendations.
- [ ] Add targeted reclaim scan on Warning/Critical.
- [ ] Add emergency recovery ordering.
- [ ] Add rule outcome/regrowth measurement.
- [ ] Add recurring “storage thief” ranking.
- [ ] Add iCloud evictable candidates and an eviction executor (non-destructive; disclosed as re-download-on-demand). MAY ship after the first beta.
- [ ] Add snapshot diff mode in Explorer: tree/treemap colored by growth between the last two snapshots. MAY ship after the first beta.

Exit criteria:

- A simulated threshold transition produces one correct notification.
- Repeated samples do not spam.
- Critical flow presents current safe candidates quickly.
- History distinguishes estimated size from verified capacity change.

### Milestone 7: AI explanations

Goal: help users understand unknown storage without expanding deletion authority.

Tasks:

- [ ] Define versioned AI input/output schemas.
- [ ] Add privacy redaction and sensitive-path exclusion tests.
- [ ] Implement Claude CLI probe/invocation adapter.
- [ ] Implement Codex CLI probe/invocation adapter.
- [ ] Implement timeout, output limit, cancellation, and process-tree cleanup.
- [ ] Validate tools-disabled behavior for each adapter; reject adapters that cannot guarantee it.
- [ ] Implement “Explain selection” and group overlays.
- [ ] Add “View metadata sent” and clear-cache controls.
- [ ] Prove through type/module boundaries that AI cannot create plans.

Exit criteria:

- AI is entirely optional.
- Malformed or malicious output cannot reference unknown nodes or trigger actions.
- No file content is sent by default.
- Provider failure leaves deterministic UI unchanged.
- Security review confirms there is no AI-to-executor path.

### Milestone 8: Automations

Goal: handle repeatedly growing safe storage with explicit user consent.

Tasks:

- [ ] Add schedule model and Settings UI.
- [ ] Enforce eligibility rules in core.
- [ ] Suggest schedules only from repeated manual history.
- [ ] Add in-process scheduler and wake handling.
- [ ] Add age/size/run-cap controls.
- [ ] Add pre-run and result notifications.
- [ ] Add pause/delete schedule and global automation kill switch.
- [ ] Add test clock and deterministic scheduler tests.

Exit criteria:

- No schedule can target Review/Informational/Protected candidates.
- Schedules default off and cannot self-enable.
- Every run re-plans and revalidates.
- Missed jobs do not pile up.
- Failures disable or pause a schedule after a conservative threshold.

### Milestone 9: CLI companion and production distribution

Goal: make the macOS app primary without abandoning CLI users.

Tasks:

- [ ] Add Settings CLI installer/upgrader/remover.
- [ ] Add final JSON commands and golden schema tests.
- [ ] Preserve standalone Homebrew/crates.io release.
- [ ] Add app DMG or ZIP packaging.
- [ ] Add Developer ID signing, notarization, and stapling CI.
- [ ] Add update framework only after signing is stable.
- [ ] Publish privacy, security, and cleanup-rule documentation.
- [ ] Add crash-report opt-in only if desired; never include paths by default.

Exit criteria:

- Fresh install works on Intel and Apple Silicon.
- Gatekeeper accepts the notarized app.
- Bundled CLI install is atomic and reversible.
- App and standalone CLI safely share schemas.
- Release artifacts and update metadata are signed and verified.

## 22. Recommended first beta scope

Include:

- menu bar disk-pressure status;
- Overview with capacity and trend;
- Home/folder scanning;
- treemap and list drill-down;
- Large Files;
- visible scan coverage;
- a small, well-tested developer-artifact rule set;
- recommendation review;
- Trash for arbitrary Explorer items;
- permanent-safe cleanup for approved regenerable rules;
- cleanup history;
- low-disk notifications;
- existing CLI retained.

Exclude from first beta:

- AI;
- scheduled cleanup;
- app uninstalling;
- duplicate hashing;
- privileged/system cleanup;
- emptying Trash;
- RAM/CPU/network monitoring;
- generic “optimization” actions;
- remote rule updates;
- Mac App Store build.

## 23. Risks and mitigations

### Incorrect cleanup classification

Mitigation: narrow rule evidence, protected overrides, negative fixtures, manual review, execution-time revalidation, independent provenance, and staged rule rollout.

### Scanner totals disagree with Finder

Mitigation: use Foundation capacity for pressure, label scanned allocation separately, expose coverage, document APFS behavior, and compare verified capacity after cleanup.

### Full scans consume excessive memory/CPU

Mitigation: paged FFI, snapshot budgets, bounded workers, cancellation, progressive summaries, idle/background priorities, and performance gates.

### TCC prevents complete understanding

Mitigation: coverage model, guided permissions, useful partial mode, and no false “complete” claim.

### External AI command gains filesystem authority

Mitigation: fixed adapters, no shell, empty working directory, tools-disabled requirement, sanitized environment, structured metadata, timeout, and no connection to planner/executor.

### UniFFI/Swift concurrency friction

Mitigation: Phase 0 spike, coarse API, generated-binding smoke tests, and documented C ABI fallback.

### Trash does not free disk space

Mitigation: honest labels, capacity verification, permanent-safe mode only for deterministic rules, and never implying a Trash move freed bytes.

### GPL contamination from Mole inspiration

Mitigation: do not copy code/lists/text; independently research rules; record provenance; review diffs for suspiciously similar implementation; keep this document’s Mole notes behavioral.

### Scope expansion into a generic system utility

Mitigation: use the product definition as a gate. Features must improve disk understanding, recovery, or prevention.

### Release pipeline compromise

Mitigation: SHA-pinned actions, minimal scoped tokens, publishing only after build and test succeed, inline tap updates instead of third-party actions, published checksums, and the signing gates in §20.

## 24. Definition of done for any cleanup rule

A rule is not done until all are true:

- [ ] Rule has stable ID and revision.
- [ ] User-facing explanation is accurate.
- [ ] Regeneration behavior is independently documented.
- [ ] Positive fixture matches.
- [ ] Similar but unsafe negative fixture does not match.
- [ ] Recent/in-use case does not preselect or execute.
- [ ] Nested/overlapping case resolves correctly.
- [ ] Symlink case refuses safely.
- [ ] Protected descendants remain protected.
- [ ] Dry-run and execution use identical validation.
- [ ] Execution-time changed evidence refuses safely.
- [ ] History records rule and revision.
- [ ] Schedule eligibility has separate review.
- [ ] UI shows exact paths, estimate, safety, and execution mode.
- [ ] Provenance URL is recorded.

## 25. Definition of done for any milestone

- Implementation and this roadmap agree.
- Relevant Rust, Swift, integration, and UI tests pass.
- No unrelated user work is overwritten.
- New destructive behavior has regression tests and security-document updates.
- New settings have defaults, migration, and reset behavior.
- Loading, empty, partial, error, cancellation, and stale states are designed.
- Accessibility and keyboard behavior are implemented.
- Performance is measured for the affected path.
- Public behavior and JSON/FFI schema changes are documented.
- Release or migration risk is stated in the pull request.

## 26. Implementation order for the first working cycle

If starting immediately, use this exact order:

1. Ship the Milestone 0 hardening fixes for the current CLI and release pipeline (v0.5.x).
2. Land the minimal Swift/Rust FFI app-shell spike.
3. Move existing computed views from CLI to core without changing behavior.
4. Add candidate/rule/plan types with no deletion.
5. Add safety validator, protected roots, corpus, fuzz tests, and security document.
6. Centralize existing CLI deletion behind the executor boundary.
7. Add SQLite aggregate history and volume status.
8. Build the read-only menu bar and Overview.
9. Build Explorer treemap/list on the shared snapshot API.
10. Add the first one or two independently researched cleanup rules.
11. Add dry-run, exact plan review, Trash execution, and permanent-safe execution.
12. Add low-disk notifications and regrowth history.
13. Run a private beta before starting AI or automation.

This order intentionally earns trust in the deterministic engine before adding the two hardest features: AI interpretation and unattended cleanup.

## 27. Research references

Primary inspiration and platform references:

- [Mole repository](https://github.com/tw93/Mole)
- [Mole security policy](https://github.com/tw93/Mole/blob/main/SECURITY.md)
- [Mole security audit](https://github.com/tw93/Mole/blob/main/SECURITY_AUDIT.md)
- [Mole security design](https://github.com/tw93/Mole/blob/main/docs/SECURITY_DESIGN.md)
- [Apple MenuBarExtra](https://developer.apple.com/documentation/swiftui/menubarextra)
- [Apple volumeAvailableCapacityForImportantUsage](https://developer.apple.com/documentation/foundation/urlresourcevalues/volumeavailablecapacityforimportantusage)
- [Apple SMAppService](https://developer.apple.com/documentation/servicemanagement/smappservice)
- [Apple local notifications](https://developer.apple.com/documentation/usernotifications/scheduling-a-notification-locally-from-your-app)
- [Apple File System Events](https://developer.apple.com/documentation/coreservices/file_system_events)
- [UniFFI Swift bindings](https://mozilla.github.io/uniffi-rs/latest/swift/overview.html)

Future rule research must prefer vendor documentation and macOS primary sources. Community cleaner implementations can identify questions and edge cases, but must not be treated as proof that a path is safe to delete.
