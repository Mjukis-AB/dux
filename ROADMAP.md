# DUX macOS Product and Implementation Roadmap

Status: Draft implementation specification

Last updated: 2026-08-09

Primary platform: macOS 14 or later

Distribution: Direct download, Developer ID signed and notarized, not Mac App Store sandboxed

In-app updates: Sparkle 2 only; no custom updater and no App Store update path

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
- Use Sparkle 2 as the sole in-app updater, integrated through Swift Package
  Manager after the production bundle identity and Developer ID signing lane
  are stable; follow ADR 0002 and do not build a custom updater.
- Enable Hardened Runtime, Developer ID signing, notarization, and stapling.
- Do not enable App Sandbox for the primary build.
- Do not assume that being unsandboxed bypasses TCC privacy protections. The app MUST detect and explain incomplete access.
- Use `SMAppService.mainApp` for the user-controlled “Launch at Login” setting.
- Keep the app running as the menu bar process. Do not add a privileged daemon or launch daemon in the first release.

### 3.2 Technology boundaries

- Keep filesystem scanning, classification, cleanup planning, safety validation, and shared domain models in Rust.
- Keep macOS scenes, navigation, charts, notifications, settings UI, and command-provider configuration in Swift.
- Generate Swift bindings for the small Rust FFI crate with the locked UniFFI
  release selected by [ADR 0005](docs/adr/0005-uniffi-swift-rust-transport.md).
- Keep the narrow, versioned C ABI in ADR 0005 as a replacement path, not a
  parallel API. Transport changes must not reimplement core policy in Swift.
- Use SQLite for durable history and settings that need queries.
- Keep large full-tree scan snapshots in versioned binary files rather than inserting millions of nodes into SQLite.

### 3.3 Safety boundaries

- AI MUST NOT create cleanup targets, change a safety tier, approve a cleanup, execute a command, or invoke filesystem tools.
- Arbitrary files selected in Explorer MUST go to Trash by default.
- Moving a file to Trash MUST NOT be reported as “space freed”; it is “moved to Trash” until the filesystem reports more available capacity.
- Permanent cleanup MAY be offered only for deterministic, tested, regenerable candidates in the first production authority graph. Any future arbitrary-path advanced permanent action requires its own threat model and security-design revision; it cannot inherit safe or schedule-eligible status.
- Scheduled cleanup MUST be limited to rules marked safe and schedule-eligible by the shipped deterministic policy.
- DUX MUST fail closed. An uncertain candidate is shown for understanding, not cleanup.
- DUX MUST never empty the user’s entire Trash as a side effect of another cleanup.
- DUX MUST not require root or `sudo` in the first production release.

### 3.4 Privacy boundaries

- Scans and history stay local.
- No telemetry is required for core product operation.
- AI receives structured metadata only by default, never file contents.
- Paths MUST be replaced by non-hierarchical, path-free display labels before
  being sent to AI. V1 has no full-path or home-relative-path override.
- Credentials, keychains, tokens, browser profiles, messages, mail, notes, password-manager data, and security-tool state MUST be excluded from AI payloads and cleanup suggestions.
- Cloud-document contents and paths MUST be excluded from AI payloads and direct-deletion suggestions. A future metadata-only local-copy eviction recommendation is a separate non-destructive flow and requires confirmed full upload, no local-only changes, a supported provider API, and explicit re-download disclosure.
- Local databases, snapshots, and caches contain full path listings of the user’s disk and are sensitive at rest: create them user-only (0700 directories, 0600 files) and cover data-at-rest handling in `SECURITY_DESIGN.md`.

### 3.5 Language and localization

- Ship 1.0 in English only.
- All user-facing strings MUST be centralized (String Catalogs in Swift, a single strings module in Rust) from the first commit so localization is a translation task, not a refactor.
- Swedish is the first localization candidate after 1.0.
- Copy rules in this document (for example “storage,” never “memory”) apply to every locale.

## 4. Current repository assessment

The current repository has three Rust workspace crates plus the macOS app:

- `dux-core`: scanner, tree arena, size formatting, and a versioned scan cache.
- `dux-cli`: Ratatui application, interaction state, computed views, and destructive filesystem operations.
- `dux-ffi`: the private UniFFI engine-session boundary used by the app.
- `dux-macos`: the native SwiftUI menu bar, Explorer, and Settings shell.

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

At this checkpoint, the workspace has 192 passing Rust tests across core,
projection, CLI, and FFI targets. Preserve and extend that baseline while
extracting behavior.

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
├── docs/adr/
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
- modification time for files and directories (captured from the size metadata snapshot since cache v6; cache v7 additionally invalidates pre-deduplication hard-link totals without changing the serialized shape; the optional field already occupied the current in-memory node layout, while populated file values increase serialized cache size);
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
    pub rule: RuleRef,
    pub category: CandidateCategory,
    pub paths: Vec<PathBuf>,
    pub estimated_bytes: u64,
    pub newest_mtime: Option<SystemTime>,
    pub evidence: Vec<Evidence>,
    pub safety: SafetyTier,
    pub action: CandidateAction,
    pub rule_schedule_eligible: bool,
    pub blockers: Vec<BlockReason>,
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

- Every candidate has exactly one shipped deterministic rule ID and nonzero rule
  revision through `RuleRef`; the revision must survive into plans and history.
- Candidate construction is internal to deterministic discovery code. AI output
  cannot construct or mutate a candidate field.
- User-facing titles and explanations are resolved from stable localization keys;
  shared candidate records do not embed English product copy.
- Multiple blockers are preserved because partial coverage, recent activity,
  active use, and path protection may all apply simultaneously.
- A candidate path must be canonical, absolute, on an allowed volume, and inside the rule’s resolved scope.
- Candidates cannot overlap after planning. If parent and child match, the planner retains one according to explicit rule precedence.
- An unavailable or changed path is re-evaluated at execution time.
- Estimated bytes are estimates until execution and post-action capacity verification complete.
- `SafeEvictable`/`EvictLocalCopy` applies only to supported cloud items whose
  fresh provider and filesystem witnesses pass the reviewed final-effect
  policy (for example iCloud Drive via the ubiquitous-item eviction API).
  Point-in-time discovery metadata alone does not prove that no unflushed or
  concurrent writer exists. Eviction MUST never target an item with a known or
  possible local-only change, MUST be labeled
  non-destructive-but-requires-network-to-re-download, and MUST NOT be reported
  as deletion.

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
    EvictLocalCopy,
}

pub enum OperationStatus {
    Planned,
    DryRun,
    Trashed,
    Removed,
    Evicted,
    Skipped,
    Rejected,
    Failed,
    ChangedSincePlan,
}
```

Cloud eviction has its own mode and result because it is neither Trash nor
permanent removal. A dry run retains each item's proposed action so review copy
can distinguish all three outcomes accurately.

Plans expire after a short interval, initially 15 minutes. Execution MUST re-stat and revalidate every path. A plan created from a stale scan may be displayed, but it cannot execute without refresh.

## 8. Deterministic rule registry

Rules are bundled versioned data validated at build time and load time. Prefer JSON plus a checked-in JSON Schema. Do not allow arbitrary shell fragments in rules.

The v1 catalog envelope has its own schema version, distinct from each rule's
revision. Every rule field is explicit: disabled optional guards/selectors use
`null`, and empty collections remain present so security-reviewed policy diffs
cannot acquire hidden defaults.

```json
{
  "schema_version": 1,
  "rules": [
    {
      "id": "developer.rust.target",
      "revision": 1,
      "title_key": "rule.developer.rust.target.title",
      "category": "developer_artifact",
      "scope": "configured_project_roots",
      "path_component": "target",
      "required_ancestor_markers_any": ["Cargo.toml"],
      "required_markers_all": [],
      "forbidden_markers_any": [],
      "exact_bundle_identifiers": [],
      "excluded_descendants": [],
      "protected_descendants": [],
      "minimum_age_days": 7,
      "minimum_bytes": 104857600,
      "inactive_processes": [],
      "requires_cloud_upload_complete": false,
      "safety": "safe_regenerable",
      "action": "remove_known_regenerable_contents",
      "schedule_eligible": true,
      "explanation_key": "rule.developer.rust.target.explanation",
      "provenance": ["https://doc.rust-lang.org/cargo/guide/build-cache.html"]
    }
  ]
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
- fuzz/property invariants are stage-specific: lexical cleanup success is
  lossless, absolute, non-root, traversal-free, and a strict component
  descendant; known protected paths never weaken from denied to guarded/miss.
  Neither lexical success nor `NoTextualMatch` is an accepted/safe path or an
  execution witness.

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

Interactive Home scans now also admit a bounded retained-node budget (currently
200,000 nodes) and one traversal worker. Once the budget is reached, jwalk is
prevented from queuing more children, the scan finishes with a partial
`IssueLimitReached` coverage fact instead of pretending the result is complete.
CLI and explicitly configured library scans retain their current
unbounded default until a caller opts into `ScanConfig.max_nodes`.

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
- `disk_samples`: volume ID, raw/daily-rollup kind, sampled time, total,
  available, important available, pressure. Kind participates in uniqueness and
  retention selection so a daily rollup cannot collide with a raw sample at
  the same timestamp.
- `scans`: scan ID, root, start/end, status, snapshot version/path, counts,
  bytes, and first-class `unknown`/`complete`/`limited_access`/`partial`
  coverage. Quantitative coverage is nullable so unknown coverage is never
  encoded as zero.
- `scan_aggregates`: scan ID, category/rule/top-level path key, bytes, file count.
- `scan_issues`: scan ID, shortened path, issue kind, count/message.
- `candidates`: candidate ID, scan ID, rule ID/revision, tier, bytes, created time, status.
- `cleanup_sessions`: plan ID, start/end, mode, estimate, verified capacity delta, trigger source.
- `cleanup_items`: session ID, rule ID/revision, path, estimate, final status, error category.
- `rule_outcomes`: rule ID, cleaned time, bytes, next observed size, regrowth duration.
- `ai_insights`: reserved input digest, provider, adapter version, model label,
  output, created time, and expiration fields. No producer is authorized by the
  current schema.
- `schedules`: rule/category scope, enabled, cadence, age, size cap, last/next run.

Do not store full millions-node trees in SQLite initially. Continue using versioned, checksummed snapshot files. Add atomic write and migration/invalidation behavior.

Before the first AI cache write, migrate the reserved row: its current 16-MiB
payload limit and missing privacy/input revisions are not admissible. The new
sealed insert/load boundary must cap canonical validated output at 64 KiB and
bind the input digest, privacy-policy revision, input schema/digest revision,
output schema revision, provider, adapter revision, and exact model revision.

### 11.2 Retention

- Hourly disk samples: 30 days.
- Daily rolled-up samples: one year.
- Cleanup history: retained until user clears it.
- Admitted AI insights: default 30 days, user-clearable, and regenerated when
  any bound digest, policy/schema, provider, adapter, or model revision changes.
- Full snapshots: latest two physically present, logically available succeeded snapshots per exact losslessly encoded root, plus any snapshot protected by an explicit active Explorer or cleanup-review lease. Scan coverage remains visible metadata; it does not silently remove a succeeded snapshot from this retention set.
- The snapshots directory has a total size cap (default 2 GiB, configurable); charge the conservative maximum of logical length and filesystem allocation for every final, recognized temporary, and control file, while reporting both values separately. Directory metadata overhead is excluded. Evict eligible referenced snapshots oldest first; active, quiescent, and unleased temps, tombstoned residuals, and physical orphans are separate maintenance debt, never normal victims. Settings shows DUX’s own disk footprint with a clear-data action — a disk-pressure tool must not be a storage thief itself.
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
- a singleton normal `Window(id:)` for Explorer;
- a Settings scene;
- `LSUIElement` so the initial app is an accessory/menu-bar utility with no
  Dock icon by default;
- a focused AppKit activation bridge for opening/focusing Explorer. Runtime
  Dock-icon switching and a removable menu-extra setting are out of scope for
  v1.

One shared `AppModel`/coordinator owns engine state. Menu bar and Explorer must never start duplicate scans accidentally.

### 12.2 Menu bar label

Support three user-selectable modes:

- icon only;
- icon plus free GiB;
- icon plus free percent.

The label must remain legible in light/dark menu bars and accessibility contrast modes. Do not rely only on red/amber/green; pair state with symbol shape or text.

Menu bar visibility is a separate opt-in preference from the label mode:

- default to **Always visible**;
- offer **Only when free space is at or below** a configurable whole percentage
  from 1–100, defaulting to 10%;
- evaluate the same effective-available numerator and total-capacity denominator
  used by the visible status: important-use availability first, with the
  disclosed filesystem-availability fallback only when necessary;
- do not start a scan or a separate capacity query merely to decide visibility;
  consume the shared cached startup-volume sample;
- keep the item visible while capacity is unknown, unavailable, or has no valid
  cached sample, so uncertainty can never make DUX unreachable;
- refreshing and stale states may use the last confirmed sample but must retain
  their truthful stale/refreshing presentation when visible;
- reveal immediately at or below the configured threshold, then hide only after
  recovery above the threshold by at least one full percentage point. This
  one-point exit hysteresis prevents boundary flicker without delaying a low-disk
  warning;
- keep this percentage-only visibility policy independent from the Rust-owned
  Warning/Critical policy, whose byte-and-percentage `min(...)` semantics answer
  a different question;
- when a healthy-state policy has hidden the item, explicitly reopening the DUX
  application from Finder, Spotlight, or `open` must reveal it for the remainder
  of that process session so Explorer and Settings are reachable. Changing back
  to Always visible must also reveal it immediately;
- Launch at Login may legitimately start DUX with no visible item while storage
  is healthy, but the relaunch/reopen escape hatch and unknown-state fail-open
  behavior are mandatory before conditional visibility ships; and
- persist only the validated user preference, never the derived inserted/hidden
  state. Visibility evaluation is presentation-only and grants no scan,
  notification, scheduling, AI, plan, or cleanup authority.

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
- coverage and observed access limitations, with Full Disk Access guidance when
  relevant. macOS has no public authoritative Full Disk Access status query.

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

The initially proposed Claude CLI and Codex CLI adapters failed the security
gate in §14.2 and are prohibited by ADR 0009. They were intentionally not
implemented. [ADR 0013](docs/adr/0013-metadata-only-remote-ai-transport.md)
selects fixed, direct-vendor, metadata-only HTTPS as the v1 architecture, but
the first fixed Anthropic Messages v1 adapter is compiled only as an
unreachable contract with a DEBUG fake harness. No adapter is runtime-enabled;
disabled/no-provider remains the only runtime state.

The contract-v59 preview remains earlier than provider invocation. It can
prepare and validate one exact-review, path-free metadata preview for the
native service layer, but no AppModel or view consumes it and no method can
transmit it. A separately dormant Anthropic revision-1 adapter now freezes the
provider/model, retention policy, request limits, envelope, and extractor, but
cannot consume that preview or construct a native task. The final consent
preview and explicit **Explain selection** action must bind all of those values
to one single-use core capability later.

A future Settings picker may offer only separately reviewed built-in providers
such as Anthropic or OpenAI. Each adapter owns its exact HTTPS origin/path,
authentication shape, bounded model choices, request envelope, response
extractor, and data-retention disclosure. DUX accepts no arbitrary endpoint,
custom header map, executable, raw command, local provider, DUX-operated proxy,
or provider SDK. An App-Sandboxed component, staged workspace, or VM is a
different unapproved architecture and requires its own ADR and supported-
platform evidence.

### 14.2 Invocation contract

- Require one explicit user-invoked explanation after showing the exact
  path-free metadata disclosure; scanning, pressure monitoring, notifications,
  schedules, and app launch never invoke AI automatically.
- Accept only the exact core-minted privacy proof bound to one retained
  succeeded-snapshot review. Caller-authored or parsed JSON cannot satisfy it.
- Build one adapter-owned JSON request for one fixed HTTPS origin/path using an
  ephemeral native session and default platform TLS validation.
- Reject every redirect before authentication can be replayed. Disable cookie
  and URL-cache storage and accept no arbitrary URL, upload, file, image,
  remote-URL, streaming, background, telemetry, or provider-storage option.
- Store a DUX-managed API credential only as a non-synchronizing, device-only
  Keychain item. Never place it in settings, environment, payload, persistence,
  logs, diagnostics, or errors.
- Send no tool, function, MCP, web, file, computer, shell, code-execution, or
  other client/server tool capability. Reject tool-shaped output completely.
- Enforce fixed request and wall-clock limits, a 64-KiB response cap, explicit
  cancellation/task teardown, and no automatic retry.
- Require the existing versioned Rust output validator to accept the complete
  body and exact input digest before display or caching.
- Attach provider, model, and adapter revision from trusted adapter state, not
  model-authored output. Render accepted prose as visibly AI-authored,
  non-linkified inert presentation.

Direct-process controls are defense in depth, not confinement. The 2026-08-09
adversarial spike proved that a direct child of the unsandboxed app retains
ordinary same-user read authority despite the clean environment and empty
working directory. That fails the gate before Full Disk Access or TCC can make
the exposure broader. ADR 0009 therefore rejects direct local Claude, Codex,
and custom-command adapters. The deprecated `sandbox-exec` comparison is not a
production boundary. ADR 0013's remote architecture avoids executing provider
code under DUX's ambient authority; tool-disable flags remain mandatory defense
in depth but never grant cleanup authority or substitute for the closed
transport and privacy-proof boundary.

### 14.3 AI input

The exact v1 envelope and typed digest encoding are normative in
`docs/AI_CONTRACT.md`. The schema-valid example is checked directly from
`dux-core/tests/fixtures/ai/v1/input/schema-valid/explain-storage-cluster.json`;
do not duplicate a drifting path-bearing example here.

Limits:

- bounded child count and depth;
- no file content in v1;
- no credentials or sensitive-category paths;
- no environment dump;
- no complete home directory listing in a single prompt;
- indicate omitted/aggregated children;
- file and directory names are untrusted input: they appear only as JSON data fields, are never concatenated into instruction text, and adapters assume they may contain prompt-injection attempts.

### 14.4 AI output

The exact checked response example is
`dux-core/tests/fixtures/ai/v1/output/schema-valid/explanation.json`; its task,
input digest, questions, uncertainties, and research suggestions are all
required by the v1 schema.

Output rules:

- Node references must already exist in the input.
- Unknown node IDs invalidate the group.
- AI safety claims are rendered as AI text, never converted to DUX safety badges.
- AI cannot return structured paths, actions, commands, or authority-bearing
  fields. Free-form model prose remains untrusted even after the conservative
  path/action lexical filter and is rendered as non-linkified inert text.
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
dux doctor [--json]         # coverage/access evidence, skipped roots, cache/database health
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

The transport is UniFFI under
[ADR 0005](docs/adr/0005-uniffi-swift-rust-transport.md). Its generated API is a
private implementation detail contained by `EngineService`, not a public ABI.

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
prepare_ai_metadata_preview(review_handle, selected_node_id)
    -> AiMetadataPreviewSession
prepare_ai_explanation(metadata_preview_handle, built_in_provider,
                       built_in_model) -> OpaqueAiExplanationPreview
consume_ai_explanation_preview(preview_handle, callback) -> TaskId
```

The metadata-preview endpoint is implemented in v59 and stops before provider
selection or transmission. The later explanation pair remains a future
capability shape. Its preparation must consume the exact available metadata
preview, bind provider/model/adapter/retention disclosure, and require explicit
user consent before returning a final request capability. Consumption is
single-use and accepts no caller digest, payload, URL, headers, provider output,
or cache row. Only the engine may hand the exact request to the fixed native
adapter callback and validate the response before any persistence or display.

Current AI realization (FFI contract v59):
`prepare_ai_metadata_preview(parent_review, {record_version,
selected_node_id}) -> AiMetadataPreviewSession` derives complete coverage from
the exact retained succeeded-scan row and returns only `info()` plus idempotent
`release()`. One two-minute, parent-capped child is available per engine; it
strongly retains the exact review and is released before reviews during close
or reset. Info is the exact canonical path-free JSON/digest, aggregate privacy
disclosure, and generic structured projection with false content/path/name
flags. The request accepts no JSON, digest, coverage, privacy fact,
provider/model/URL/credential, callback, or plan. The sealed request-local node
mapping remains in Rust. The native adapter independently validates the record,
but no controller, view, provider, network, cache, task, CLI, or effect consumes
it.

Initial native volume realization (introduced in FFI contract v17):
`observe_startup_volume(versioned Foundation facts) -> versioned path-free
status` is the first production volume endpoint. It returns Rust-owned pressure,
headline source/boundaries, prior durable pressure, and history disposition.
The adapter fixes the mount to `/`; no path crosses from Swift, and incomplete
or important-only evidence is explicitly evaluation-only.

Contract v6 retains those startup records at v1 and adds separate versioned
`get_disk_pressure_policy`, `set_disk_pressure_policy`, and
`reset_disk_pressure_policy` endpoints. They expose exact integer values,
Default/Stored provenance, monotonic revision, optional update time, changed
disposition, and typed validation/storage failures. A policy is never supplied
with a capacity observation, and none of these DTOs can express a path, scan,
candidate, plan, notification, schedule, or cleanup action.

Contract v7 adds `start_scan(versioned discovery scope) -> opaque ScanTask`.
The request admits only a bounded, absolute, control-free root and grants
read-only observation scope, never cleanup or planning authority. The M3 native
adapter fixes that root to Home. Poll/cancel records retain their cursor inside
Rust and expose only authoritative phase, coarse stage, cancellation intent,
revision, optional cumulative path-free progress, sticky truncation evidence,
typed failure, and a path-free terminal summary. Missing progress is unknown,
not zero, and late cancellation cannot replace the task's terminal outcome.
Paged snapshot children, issue details, candidate details, planner inputs, and
execution remain separately gated later endpoints.

Contract v8 adds a bounded newest-first recent-scan history page used only to
select an Explorer review target. Its path-free records contain stable scan ID,
timestamps/status, succeeded counts, coverage, and a `snapshot_recorded` hint.
The hint is deliberately non-authoritative: selecting it must still acquire the
existing exact scan-bound review lease, which repeats history, tombstone,
identity, file, and format validation. Swift rejects malformed versions,
ordering, lifecycle/count shapes, timestamps, IDs, coverage, and snapshot hints
before publishing app-owned models. Snapshot nodes, issue details, candidate
paths/evidence, and treemap data remain sealed behind later bounded APIs.

Contract v9 adds an atomic-looking newest-available review operation: Rust
selects the exact newest succeeded snapshot not covered by an exact retention
tombstone, then acquires the existing expiring review lease for that reference.
The repository repeats history, tombstone, identity, file, and full-format
validation while pinning; a selection/retention race therefore fails closed.
Swift learns the selected stable scan ID from the acquired lease, validates it,
owns renewal independently from render state, generation-fences overlapping
requests, and explicitly releases stale or malformed handles. This still
transports no paths, nodes, candidates, plans, or cleanup authority.

Contract v10 adds the retained historical root and deterministic direct-child
pages, capped at 200 and sorted inside Rust. Contract v11 adds a coarse
logical-size treemap under that same exact lease: at most 64 represented
positive-size children, stable logical ranks, and exact Other child/byte
accounting including zero-size children. The projection reuses the retained
decoded document and the existing 100,000-direct-child sort budget. It exposes
no category assertion, current path, candidate, reclaimability estimate, plan,
AI input, or cleanup authority.

Contract v12 adds one whole-snapshot Large Files projection under the same
exact review lease. The request requires a positive logical-size threshold,
accepts an optional strict modification cutoff, and is capped at 200 results;
the app requests 100 and defaults to 1 GiB. Rust uses O(k) top-result memory,
returns deterministic logical-size/name/ID ordering, bounded historical parent
context, and exact matching file-count/logical-byte aggregates, then revalidates
the lease before returning. Unknown modification times do not match an age
filter. The response is historical discovery only and carries no current path,
reclaimability, candidate status, AI input, plan, or cleanup capability.

Contract v15 adds one display-only storage category to every snapshot node
projection. Rust joins only the exact scan's immutable, fully validated
candidate evaluation: a classified root and its descendants inherit the
nearest historical category, while missing, failed, legacy, or ambiguous
evidence remains Unclassified. The optional join is independently capped at
4,096 roots and 1 MiB of exact path payload, uses an exact-root index with
ancestor lookup, and is discarded on release or expiry. Candidate persistence
currently requires Unicode host paths; a candidate evaluation containing a
non-Unicode path fails closed, so Explorer shows Unclassified rather than
using a lossy name. The FFI enum intentionally copies the category
names without exposing candidate identity, paths, evidence, safety, action,
status, reclaimability, plan, AI input, or cleanup capability. Swift maps the
enum without filename or display-string inference and renders redundant color,
symbol, visible text, legend, inspector, and VoiceOver alternatives.

Contract v26 adds one review-only candidate status transition under an already
acquired exact snapshot lease. The command is one of four fixed enum values and
the result is only a versioned scan ID, candidate ID, and status. Rust performs
the complete durable candidate/source binding and transactional review-state
transition; Swift rejects malformed or cross-review results. This endpoint is
not a planner input and cannot create a plan, approval, journal claim, schedule,
AI request, or filesystem effect.

Contract v29 adds one exact-session cleanup-history observation selected only
by a bounded stable session ID copied from the recent summary feed. Core fully
validates the stored graph before FFI independently projects a versioned,
path-free session summary, ordered item summaries, and warnings. Verified
capacity delta, typed lifecycle/item statuses, bounded stable error categories,
complete-versus-legacy policy shape, and aggregate counts are preserved. No
paths, evidence payloads, candidate IDs, execution fences, claims, journal
receipts, clear/retry/recovery operation, approval, or executor authority cross
this endpoint.

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
- observed access-probe evidence and coverage mapping, without inventing an
  authoritative Full Disk Access boolean;
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
- Dry-run integration test proves no filesystem mutation. The 2026-07-29
  Rust-target core slice snapshots a real stale Cargo project before and after
  the complete production validator and proves byte content, object identity,
  kind, mode, link count, size, and mtime are unchanged. This closes the
  mutation-detector requirement for that implemented mode; future Trash and
  eviction dry runs require their own parity evidence before exposure.
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
Sparkle 2 signed-appcast and signed-enclosure verification
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
- [x] Show cache age in the header; add a rescan keybinding; use per-process cache temp names. Completed 2026-07-15: cached headers show compact age from the original scan timestamp; `r` starts a cache-bypassing scan while retaining the prior tree as failure fallback; successful replacement resets navigation and publishes a new timestamp; deletion-only cache updates preserve the tree's original timestamp. Cache writers exclusively create PID-and-counter-qualified temp files, clean them after write/rename failures, and join older in-process writers before a newer scan can publish. Concurrent processes remain last-writer-wins until later snapshot writer coordination.
- [x] Fix the footer selection total double-counting nested selections. Completed 2026-07-15: footer bytes and multi-delete planning now share one deterministic effective-selection calculation that removes any selected node with a selected ancestor, ignores stale/tombstoned IDs, excludes the root, and saturates byte addition. The footer continues to show the raw highlighted-row count while its byte total matches the roots that confirmation will act on.
- [x] Apply §20.1 to the release workflow and add cargo audit/deny to CI. Completed 2026-07-15: CI and tag builds use a committed lockfile, exact Rust/tool versions, read-only default permissions, and full-SHA action pins. Releases now gate four target builds on tests plus RustSec/license/source policy, compute per-producer checksums, verify and aggregate them into `SHA256SUMS`, publish crates.io before a draft-backed GitHub release, then update Homebrew through inline Git with the tap-only PAT. Exact-version crates.io reruns compare packaged bytes before continuing. Enabling the gates also upgraded the vulnerable Crossbeam lock entry, disabled Postcard's unused heapless defaults, and upgraded Ratatui/Crossterm to remove the remaining unmaintained/yanked transitive crates.
- [x] Move `debug_scan.rs` into `dux-core/examples/`. Completed 2026-07-15: the diagnostics binary now uses Cargo's conventional example discovery, needs no out-of-crate manifest path override, and is included when `dux-core` is packaged and published.

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

- [x] Add architecture decision records for native SwiftUI, direct distribution, no sandbox, and shared Rust engine. Completed 2026-07-15: accepted decisions and implementation gates are indexed in [`docs/adr/`](docs/adr/README.md), including the external-AI subprocess security gate.
- [x] Create minimal `dux-ffi` returning its version and one size-format result.
  Completed 2026-07-15: the non-published UniFFI crate builds `rlib`,
  `staticlib`, and `cdylib` artifacts; exports a library/FFI-contract version
  handshake plus a typed raw-bytes/display smoke record; delegates formatting
  to `dux-core`; and configures immutable Swift records. This proves the Rust
  boundary only—the display string is not a localization contract or final API.
- [x] Build Rust for arm64 and x86_64 and package a universal XCFramework.
  Completed 2026-07-15: a locked, fail-closed script supports Debug and Release,
  verifies both Rust targets, builds both macOS architectures, creates and
  validates one fat static library, then packages a single `macos-arm64_x86_64`
  `DuxFFI.xcframework` slice. Generated output is ignored, identical cached
  Release builds produce identical file hashes, and a dedicated macOS CI job
  exercises the clean build path. Headers and Swift source remain the next
  binding-generation task.
- [x] Generate/import Swift bindings in a minimal Xcode app. Completed
  2026-07-15: the pinned UniFFI 0.31.2 generator binary emits deterministic Swift,
  C-header, and module-map inputs from the universal archive; the header-bearing
  XCFramework and committed Swift source are produced together; and a generated
  Xcode project compiles and links them in an unsigned macOS 14 SwiftUI spike
  app. CI regenerates the boundary before building so mismatched source/library
  revisions fail during compilation or UniFFI initialization checks.
- [x] Call Rust off the main actor and render the result. Completed 2026-07-15:
  `EngineService` runs synchronous UniFFI work on a dedicated non-main dispatch
  queue, converts generated records into a Sendable app DTO before crossing the
  continuation, and returns it to an observable `@MainActor` model. The SwiftUI
  spike renders library/contract versions, raw bytes, Rust display text, and the
  verified execution context. Linked XCTest coverage asserts both the real Rust
  values and the main-actor state handoff, locally and in macOS CI.
- [x] Verify Debug and Release builds from a clean checkout. Completed
  2026-07-15 from a source-only temporary snapshot containing the tracked and
  intended untracked repository inputs but no `target/` directory or generated
  XCFramework. Each configuration independently regenerated its matching
  UniFFI bindings and Rust library before Xcode built an unsigned universal
  `arm64` + `x86_64` app. The macOS CI job now repeats the configuration-matched
  Debug build and linked tests plus the Release build from GitHub's clean
  checkout, preventing a cached or locally generated artifact from satisfying
  the gate.
- [x] Decide UniFFI versus C ABI and record the decision. Completed 2026-07-15:
  [ADR 0005](docs/adr/0005-uniffi-swift-rust-transport.md) accepts the locked
  UniFFI Swift bindings for the private in-process app boundary based on the
  typed-record, checksum, off-main concurrency, deterministic-generation, and
  universal Debug/Release spike evidence. Generated APIs stay contained inside
  `EngineService`; sync calls remain off-main; DTOs stay coarse, immutable, and
  owned; UniFFI and the DUX contract retain separate version checks. A narrow
  opaque-handle C ABI is documented as a replacement—not parallel—path with
  explicit triggers for packaging, concurrency, lifetime, safety, maintenance,
  or measured performance failures.
- [x] Before merging the first real engine handle, prove one typed fallible
  UniFFI export and the opaque-handle construction/use/close/release lifecycle
  through linked Swift tests, including use-after-close rejection. Completed
  2026-07-15: FFI contract v2 introduces one application-scoped `DuxEngine`
  object with atomic lifecycle state, idempotent explicit close, and fallible
  version/format methods returning the stable `EngineError::Closed`. Generated
  Swift errors are caught and mapped to app-owned `EngineServiceError` values;
  views never import them as render state. Rust tests cover close and drop, while
  four linked Swift tests cover the real typed values, main-actor handoff,
  construction, both close outcomes, direct typed use-after-close rejection,
  service error mapping, Swift object deallocation, and return to the baseline
  Rust live-instance count. The diagnostic counter is not application state.
- [x] Prototype `MenuBarExtra` plus normal Explorer window and Settings.
  Completed 2026-07-15: the SwiftUI lifecycle now starts with a persistent,
  window-style menu bar extra; a primary accessible action targets one
  `Window(id: "explorer")`, which SwiftUI orders forward instead of creating
  another instance; and the native `Settings` scene opens through the same
  menu surface. One `@State` `AppModel` and application-scoped `DuxEngine` are
  shared by all three scenes. `AppActivation` uses macOS 14's non-deprecated
  `NSApplication.activate()` request after opening Explorer or Settings, while
  the generated app has `LSUIElement=true` and therefore no default Dock icon.
  Keeping `MenuBarExtra` first relies on SwiftUI's macOS 14 automatic launch
  behavior so a fresh launch does not present Explorer; do not reorder the
  scenes without a launch regression check. A String Catalog owns the shell's
  user-facing keys, and VoiceOver labels cover the status item and actions.
  Six linked tests now include one concurrent menu/Explorer load assertion that
  proves the shared model starts only one engine request plus a built-bundle
  assertion for `LSUIElement`. Real scan-session deduplication remains an engine
  task because the Phase 0 handle does not expose scanning yet.
- [x] Prototype Foundation important-usage volume capacity. Completed
  2026-07-15: `VolumeMonitor` samples the startup volume (`/`) through Foundation
  on a dedicated utility queue and requests its localized name, total capacity,
  ordinary available capacity, and important-usage capacity. The immutable,
  timestamped snapshot prefers important-usage capacity for the headline value,
  retains ordinary availability for usage accounting, records fallback
  provenance, and rejects missing, negative, or internally inconsistent values.
  One shared `AppModel` deduplicates initial capacity work across scenes; the menu
  bar and Explorer show exact values plus a text-backed accessible capacity bar
  without deriving free space from directory scans. Five focused capacity tests,
  including a real startup-volume Foundation sample, plus the shared-load test
  cover preference, fallback, validation, off-main execution, and scene
  deduplication.
  Pressure thresholds and hysteresis deliberately remain unimplemented here:
  Milestone 3's Rust evaluator owns that policy so Swift cannot create a second
  source of truth.

M0 implementation is complete, but the milestone does not meet its exit criteria
until the hardening changes are actually released as a v0.5.x patch.

Exit criteria:

- Hardening fixes have shipped in a v0.5.x release and the §4 defect list is resolved.
- A clean CI job builds the universal app shell.
- The menu bar can open the normal window.
- Swift receives a typed Rust value.
- The integration approach has no unresolved blocker.

### Milestone 1: Shared engine extraction and safety foundation

Goal: make CLI behavior reusable and remove UI-owned destructive authority.

Tasks:

- [x] Move `ArtifactKind`, artifact classification, large-file projection, and
  staleness calculation from CLI into core. Completed 2026-07-15:
  `dux-core::projection` now owns the public product-neutral types and pure
  projections over `DiskTree`. Artifact matching preserves the complete M0
  marker, exact-kind/case/location, symlink-chain, evidence-order, and verified-
  ancestor suppression rules; its marker IDs/paths are explicitly snapshot
  evidence that deletion callers must revalidate. Large-file membership,
  relative paths, stable size ordering, percentages, newest-subtree mtime,
  strict stale boundaries, missing/future-clock behavior, and in-place threshold
  refresh are core calculations with an injected observation time. The CLI keeps
  only dirty-view coordination, threshold cycling, and presentation labels while
  consuming the shared results unchanged for deletion planning. Thirteen core
  projection regressions include the eight migrated classification fixtures and
  previously missing large-file/staleness edge cases; CLI state/deletion tests
  continue proving evidence revalidation and selection behavior.
- [x] Record file modification times in scan nodes (cache format bump) to
  support age guards. Completed 2026-07-15: the scanner now captures modification
  time for every discovered file and directory from the same metadata already
  used for its allocated size, so collection adds no filesystem query. Followed
  symlinks use target metadata while retaining path provenance; non-file/directory
  nodes, unavailable times, and pre-Unix-epoch values unsupported by Serde remain
  explicitly unknown instead of breaking cache writes. Cache
  version 6 rejects v5 snapshots before decoding, ensuring cached trees cannot
  silently omit all file activity. Scanner regressions prove exact file and
  directory capture (including followed-link target semantics), cache time
  normalization rejects unsupported pre-epoch values, the cache round-trip
  preserves a deterministic file time, the previous version is rejected, and
  the existing projection regression proves newest-file activity controls
  artifact staleness. The optional field
  already occupied each current `TreeNode`, so this changes serialized cache
  size rather than the present node layout. These timestamps remain scan
  evidence only: future actionable age guards must treat incomplete coverage as
  unknown/non-actionable and revalidate independently of this snapshot.
- [x] Introduce candidate and rule domain types. Completed 2026-07-15:
  `dux-core::domain` now defines opaque bounded candidate/scan IDs, strict dotted
  rule and localization keys, nonzero `RuleRevision`, and an inseparable
  `RuleRef`. Immutable validated rules carry unresolved scope, path-component or
  exact-bundle match selectors, required/forbidden markers, excluded/protected
  descendants, age/size/activity/cloud-upload guards, localized text keys,
  provenance URLs, safety, proposed action, and the original scheduling policy.
  The fail-closed safety/action matrix permits only regenerable removal,
  confirmed-cloud eviction, review-required Trash, or informational/protected
  reveal/no-action; only the regenerable-removal pair may be marked schedulable.
  Candidates copy all policy and rule identity from a validated rule, preserve
  typed snapshot evidence and multiple typed blockers, and expose only narrow
  discovery facts rather than “actionable” or “schedule now” authority. Their
  internal constructor requires unique paths and evidence; cloud eviction
  additionally requires exactly one matching upload-complete fact per candidate
  path and rejects missing, duplicate, or unrelated facts. Matcher construction
  rejects mixed selector families plus absolute, traversing, empty-segment,
  separator, Windows-prefix, wildcard, and trailing-dot path syntax. Nineteen
  focused domain tests cover
  ID/revision validity, the exhaustive policy matrix, provenance,
  matcher/guard validation, policy copying, blockers, and eviction evidence.
  No rule is shipped and existing marker matches are not promoted to safe
  candidates. JSON/Serde documents and loading, filesystem matching, candidate
  filesystem-path validation, overlap planning, persistence, FFI DTOs, AI, and
  execution deliberately remain at their later roadmap boundaries.
- [x] Introduce rule schema/loader with fixture validation. Completed
  2026-07-15: a self-contained Draft 2020-12 catalog schema now fixes the v1
  envelope, exhaustive required fields and enums, selector alternatives,
  collection/string/numeric budgets, the complete safety/action matrix,
  scheduling constraint, cloud-upload guard, HTTP(S) provenance shape, and
  denial of unknown properties. The internal byte-only Rust loader separately
  caps catalogs at 1 MiB and 256 rules, uses private deny-unknown wire DTOs with
  required nullable fields, rejects duplicate values and active rule IDs or
  revisions, converts every value through invariant-bearing domain
  constructors, and stores validated rules in stable ID order. Its API remains
  crate-private until a signed bundled-rule source exists: validation proves
  shape and policy consistency, not trusted origin or cleanup authority. Eight
  checked synthetic `fixture.*` catalogs cover path and bundle selectors,
  missing/unknown/mixed fields, duplicate IDs/values, and invalid provenance;
  nine focused tests also reject malformed/trailing/duplicate JSON, remove
  every required field, inject unknown fields at
  every object level, exhaust the 25 safety/action pairs, cover every category
  and scope, exercise scheduling/cloud constraints, typed limits/errors, schema
  meta-validation, deterministic ordering, and schema/runtime parity for the
  ASCII byte-bounded relative-path grammar. JSON Schema validation uses a
  dev-only no-resolver dependency, while release code adds only `serde_json`.
  No rule catalog or fixture is embedded in Rust, copied into the macOS resource
  phase, exposed over FFI, or made available to AI/CLI callers. Evaluator
  filesystem fixtures and independently researched production rules remain
  later tasks.
- [x] Introduce cleanup-plan types without execution. Completed 2026-07-15:
  immutable core plan/item types now preserve the source scan, candidate and
  rule revision, exact observed paths, evidence, policy, timestamps, scheduling
  eligibility, checked byte estimate, fixed 15-minute lifetime, and mandatory
  user-facing warning taxonomy. Explicit dry-run, Trash, permanent-safe, and
  cloud-eviction modes fail closed against blockers, non-cleanup candidates,
  mixed scans, duplicate IDs, incompatible policy, arithmetic overflow, and
  unresolved exact or parent/child path overlap. Cloud eviction has distinct
  `Evicted` result semantics rather than being mislabeled deletion. Nine focused
  tests cover frozen facts, expiration boundaries and overflow, exhaustive mode
  compatibility, overlap in either order, mandatory/deduplicated warnings,
  schedule-policy preservation, and the inert operation-status taxonomy. Plan
  construction remains private inside the domain module until the following
  path-validator and planner slices can supply validated target witnesses; the
  types perform no I/O, expose no mutation or approval API, and grant no
  execution authority. FFI, persistence, AI, CLI migration, plan-result
  journaling, and execution remain at their later roadmap boundaries.
- [x] Introduce lexical/canonical path validator.
  `dux-core::path_validation` now stages non-authoritative structural evidence
  behind crate-private constructors. Raw host-native syntax is inspected before
  `Path` normalization and rejects empty/relative paths, dot traversal,
  repeated or trailing separators, controls, invalid Unicode encoding,
  excessive encoded length, ambiguous Windows namespaces and aliases, bare
  cleanup roots, scan-root equality, and non-descendant prefix collisions.
  Cleanup lexical evidence carries its exact scan root and relative path, so it
  cannot be paired with another root. Cache-reconstructed tree paths are not
  integrated and are forbidden by policy because the current cache persists
  names through a lossy Unicode representation; a typed fresh/lossless scan-
  provenance witness remains required when the later planner adopts this API.

  The live stage walks each component without following symlinks (descriptor-
  relative `openat`/`fstatat` on Unix and repeated cumulative-path no-follow
  handles on Windows), refuses symlinks/reparse points and special entries,
  rejects detectable volume
  crossings, and binds canonical path, portable volume/object identity, target
  kind, hard-link count, and the ordered ancestor identity chain into an
  immutable snapshot. It requires canonicalization to preserve the exact
  platform path location, compares repeated captures plus canonical identity to
  fail closed on observed changes, and performs no filesystem mutation. The
  snapshot is time-bound evidence only: it grants no planner/executor authority,
  is absent from FFI and clients, and still requires the following protected-
  root registry and later executor-time revalidation. Windows inspection is
  still full-path based and remains non-actionable until handle-bound or
  equivalent executor revalidation closes its ancestor-reparse race. Native
  tests cover strict
  scope, normalization hazards, Unicode/lossy encoding, symlinks (including
  dangling/looped and symlinked roots), missing and special entries, hard links,
  root binding, identity chains, diagnostic path privacy, and non-mutation;
  this checkpoint is cross-target compiled for Windows and Linux.
- [x] Add protected-root registry with per-platform sets (macOS, Linux, Windows).
  A revisioned, crate-private textual registry now evaluates both requested and
  canonical scan-root and target locations with component boundaries and a
  deterministic hard-deny-over-guard precedence. The policy model separates
  exact structural anchors, hard-denied trees, and guarded trees whose children
  will require a future code-owned deterministic-rule grant; no arbitrary rule,
  caller boolean, scan selection, or mode can satisfy that grant today.

  The macOS table covers System and Darwin infrastructure, system applications
  and libraries, package roots, user/volume containers, `/private` backing
  paths, validated home-directory evidence, other profiles beneath conventional
  and configured profile containers, and guarded home Library content.
  Linux covers boot/device/process/runtime/system trees, usr-merge spellings,
  `/var`, root, Nix/Snap/package and service roots, home and mount containers,
  validated home-directory evidence, and other profiles beneath conventional
  and configured profile containers. Windows applies hard OS/recovery/recycle/
  volume-information denies on every drive, guards Program Files and ProgramData
  descendants, protects drive/profile roots and other profiles beneath
  conventional and configured profile containers, and guards current-profile
  AppData. Static ASCII system components compare conservatively without Unicode
  normalization; Linux remains case-sensitive, and every match is component-aware
  rather than a string prefix.

  The registry deliberately returns only `Denied`, `SpecificRuleRequired`, or
  `NoTextualMatch` textual dispositions. `NoTextualMatch` is not an allow or
  safety witness. Production registry construction is sealed until trusted OS
  account/known-folder discovery supplies the actual home and profile-container
  boundaries; accounts in unrelated undiscovered containers remain unknown, not
  safe. Complete filesystem-root-to-scan ancestry, authoritative mount-location
  identity (including Linux bind-mount detection), trusted selected-volume
  evidence, stable per-boundary deterministic-rule grant identifiers, and
  Windows handle-relative validation also remain mandatory before planner use.
  Focused tests exercise all platform tables on every host, a checked full-table
  revision fingerprint and duplicate checks, exhaustive exact/descendant target
  and scan coverage, invalid and relocated home boundaries, all four evidence
  forms, hard-over-guard precedence, custom homes, drive/case/component
  boundaries, unresolved Windows legacy aliases, unsupported-platform failure,
  diagnostic privacy, and live non-mutation.
- [x] Add dangerous-path corpus and fuzz/property tests.
  The deny-unknown v1 JSON corpus is independently authored rather than
  generated from implementation tables. It carries stable IDs and explicit
  stage outcomes for 37 host-native lexical cases, all 52 macOS/Linux/Windows
  static roots, 24 dynamic home/profile/guard cases, and six documented known
  gaps. Native companions exercise invalid Unix bytes and unpaired Windows
  UTF-16; deterministic properties cover every control character, dot segments,
  encoded-unit boundaries, lossless strict descendants, component-prefix
  lookalikes, exact/hard/guard matrices, and deny precedence across evidence
  forms. Corpus outcomes are deliberately `reject`, `denied`, `guarded`,
  `NoTextualMatch`, or non-authoritative evidence—never “safe” or “allowed.”

  This checkpoint also corrected dynamic guarded-root semantics: the exact
  `$HOME/Library` and `$HOME/AppData` roots are denied, while descendants and
  scan scopes require a future stable rule-boundary grant. The isolated
  `fuzz/` package pins `libfuzzer-sys` and its own audited lockfile, exposes a
  bounded `cfg(fuzzing)` invariant oracle only to fuzz builds, performs no
  filesystem I/O, and retains six seed classes. A 20,000-run fixed-seed smoke
  campaign passes locally; pinned nightly CI repeats it on changes and runs a
  five-minute weekly campaign, preserving crash artifacts for promotion into
  the reviewed corpus.

  These tests do not close the explicitly recorded Linux bind-mount, macOS
  firmlink, Windows alias/reparse, trusted-volume/home discovery, stable rule-
  grant, or executor-revalidation gaps. The corpus instead records every such
  unresolved gap as non-authoritative; executable alias and textual-miss cases
  separately ensure the current API produces no positive witness.
  `SECURITY_DESIGN.md` remains the next safety checkpoint.
- [x] Add `SECURITY_DESIGN.md`.
  The normative design now defines the threat model, assets, one-way authority
  graph, fresh-observation/candidate/plan/approval/executor chain, path and
  protected-root stages, deterministic rule trust, execution-mode semantics,
  scheduling gates, AI isolation, TCC/unsandboxed boundaries, sensitive local
  persistence, FFI/CLI/multi-process rules, release integrity, incident review,
  required verification, and a cleanup shipping gate. It separately labels the
  read-only app, non-authoritative Rust scaffolding, and the actual legacy CLI
  permanent-delete path—including current protected-root, lossy-name, recursive,
  hard-link, journal, capacity-reporting, and path-based race debt—so target controls are
  not misrepresented as runtime enforcement.

  Its code-grounded review also found and closed an immediate legacy escape:
  a forged cached terminal `.` or `..` component could previously alias the
  scan root or parent because deletion planning validated only the target's
  parent. Legacy admission now requires an absolute, control-free, valid-text,
  normal-component strict descendant and rejects target, ancestor, or marker
  evidence on another filesystem; regression tests preserve the boundary.

  The design resolves two ambiguous roadmap boundaries conservatively: the
  first production authority graph has no arbitrary-path permanent mode, and
  cloud contents/paths remain excluded from AI and direct deletion while a
  future supported local-copy eviction flow is metadata-only, fully-uploaded,
  non-destructive, and separately disclosed.
- [x] Add forbidden destructive-call CI lint. CI now combines Clippy's
  compiler-resolved `disallowed_methods` policy for Rust with a repository
  scanner for Rust, Swift, shell/workflow, PowerShell, and Python mutation and
  process escape hatches plus selected direct truncation APIs. Exceptions use a
  one-use registered ID bound to an exact path, detected primitive/rule, and
  where needed enclosing symbol or test context; each annotation is adjacent
  and reasoned. The ledger is restricted to the legacy executor, create-new
  cache files, repository-owned build/release outputs, fixed Finder reveal, or
  test-owned temporary paths; malformed, copied, overbroad, misplaced, unknown,
  and stale annotations fail. Platform-native Clippy runs on Linux, macOS, and
  Windows; the same scanner and self-tests run in PR and tagged-release gates.
  Executable/shebang sources and generated/untracked local sources are included,
  while an unclassified executable language fails closed. The
  XCFramework builder now rejects caller-selected output paths before running
  tools or mutation and rejects symlinked/non-physical output parents. The lint
  is a removal/relocation/truncation/process escape boundary, not a proof that
  arbitrary app persistence writes can never overwrite data; persistence still
  requires its separate ownership, permissions, and semantic validation gates.
- [x] Route existing CLI delete requests through a temporary centralized executor adapter.
  Single- and multi-item requests now prepare and consume an opaque,
  target-bound plan through `dux-core::cleanup::legacy_cli`; the CLI UI no
  longer contains or directly invokes `remove_file`/`remove_dir_all`. The
  adapter preserves the existing strict-descendant, same-volume, target,
  ancestor, and marker-identity rechecks without claiming to implement the
  future cleanup-plan/approval/executor authority graph. The destructive-call
  policy pins its three raw effects to this adapter and rejects adapter
  references or re-exports from FFI, Swift, and every non-CLI client.
- [x] Preserve CLI behavior and tests; clearly label current permanent deletion until replaced.
  The existing deletion regression suite moved intact to core and continues to
  cover replaced/missing targets, symlinks, ancestor redirection, marker
  replacement, invalid targets, and worker transfer. CLI confirmation,
  progress, footer, help, and deferred-quit copy now consistently says
  “permanent”; scan-derived item totals are labeled as estimates rather than
  measured freed capacity. Focused rendering tests enforce both disclosures.

Exit criteria:

- CLI and core tests pass on all existing platforms.
- Classification results are owned by core.
- No feature UI directly invokes recursive deletion.
- Planner can produce dry-run decisions from fixtures.
- Safety documentation and regression corpus are reviewed.

### Milestone 2: History and engine service

Goal: provide a durable orchestration layer shared by CLI and app.

Tasks:

- [x] Add engine handle and task registry.
  `dux-core::engine` now owns an application-scoped, cloneable handle with
  explicit database/snapshot/cache paths and one FIFO task registry per engine
  session, bounded to two workers, sixteen queued tasks, sixty-four retained
  terminal records, and sixty-four events per task. The application owns one
  engine session. Nonzero process-global task IDs never reuse or
  wrap; typed snapshots keep cancellation intent separate from the observed
  terminal outcome; event cursors and page sizes fail closed; worker panics
  become stable internal failures; and explicit nonblocking close cancels and
  removes queued work, requests running cancellation, rejects use after close,
  and supports a bounded quiescence wait. Immutable read-only formatting and
  durable full-scan tasks exercise production submission, typed progress,
  cancellation, result publication, and retention. Tests cover worker/queue bounds, FIFO,
  cancellation races, close/drop/clone lifecycle, panic containment and safe
  shutdown after mutex poisoning, cursor continuity, storage-role overlap,
  non-wrapping ID exhaustion, and
  cross-platform path construction. Scan lifecycle/persistence is now attached
  by the sub-checkpoint below. Priority, callback, cleanup, CLI, and FFI task
  integration remain separate; the Phase 0 UniFFI `DuxEngine` is still a
  smoke-only transport handle until a later coarse task DTO/event slice.
- [x] Add versioned SQLite migrations. Completed 2026-07-16: engine startup now
  provisions and opens a private SQLite store before publishing any worker,
  with one reusable coordinator per retained physical database identity and
  process, including case/normalization aliases on case-insensitive
  filesystems. The checksummed v1 migration creates every §11.1 table with
  SQLite `STRICT`, bounded text/blob fields, constrained self-describing
  semantic values, and a frozen lossless path codec: UTF-8 host bytes use tag
  1, little-endian UTF-16 host units use tag 2, and UTF-8 logical aggregate
  keys use tag 0. The 32,768-host-unit limit admits the corresponding
  65,536-byte Windows representation. A contiguous SHA-256 migration ledger,
  DUX application ID, and exact schema fingerprint detect partial migration or
  drift; upgrades run in one `BEGIN IMMEDIATE` transaction.

  A stable advisory lock plus SQLite WAL, five-second busy bounds, `FULL`
  synchronization, defensive connection settings, disabled attachment/schema
  trust, and runtime limits guard each writer. Full startup and migration
  integrity inspection use fixed VM-operation ceilings and deadlines sampled by
  a SQLite progress callback. Live, fallible status uses a smaller bounded
  ledger/version/fingerprint compatibility inspection instead of rescanning
  history or foreign-key rows on every render. It rechecks under the same
  lease, transitions a reused coordinator to read-only after an external
  upgrade, and never treats cached startup status as write authority.
  Marker-owned rollback/WAL recovery artifacts are opened RW only under the
  lease; a recovered newer schema is immediately reopened strictly read-only.
  Foreign, corrupt, partially migrated, drifted, over-budget, or unmarked
  stores fail with distinct path-free categories.

  First provisioning builds an exact private sibling stage containing a
  durably written fixed-length ownership marker and empty database, then publishes
  the directory atomically without replacement. The marker entry and identity
  remain permanent; its exact legacy-v1 content has one monotonic, writer-locked
  transition to cleanup-layout v2 after both cleanup controls are durable, and
  older binaries intentionally reject that one-way layout boundary. Successful current-schema
  setup durably adds a separate private initialization sentinel, so a later
  zero-length truncation cannot be mistaken for an interrupted first provision.
  A collision re-probes the
  winner and never overwrites it; unmarked empty directories/databases and a
  marker missing its database remain untouched. Unix stage creation and final
  no-replace rename are descriptor-relative beneath retained directories, with
  retained identities, no-follow opens, single-link checks, and exact
  0700/0600 repair. macOS permits deny-only ACLs on the publication parent (as
  used by normal `Application Support`) but rejects extended ACLs on final DUX
  objects; Linux relies on owner/mode checks. Windows uses owner-only protected
  DACLs at birth, retained file IDs, handle-relative stage children,
  handle-bound no-replace publication, a retained final-root rename guard, and
  immediate exact-DACL repair for inherited SQLite sidecars;
  reparse/device/multi-link and ambiguous DOS/ADS/alias syntax reject. The root
  admits only the allowed SQLite/control/sidecar and reserved app-support
  entries (on macOS, another spelling is tolerated only when it resolves to the
  same filesystem object reached by an allowed canonical name), plus at most
  64 lexically exact private
  `.dux-snapshot-stage-<32 lowercase hex>` directories through a complete walk
  with fixed total-entry and 256-KiB aggregate-name budgets plus sampled checks
  against a 250-ms elapsed-time budget. A current-user-owned Unix stage whose
  mode is a strict subset of 0700 is tolerated only as opaque interrupted-
  creation debt. Tolerating that namespace keeps a crashed
  snapshot provision reopenable; it is not ownership or deletion authority,
  which remains with the independent snapshot owner. Failed
  or losing provisions can leave tiny private `.dux-stage-*` siblings with only
  the marker/empty database; bounded identity-safe scavenging is deferred to
  retention maintenance rather than recursively deleting an unproven path.
  Unix has no supported source-handle-bound directory rename, so malicious
  same-user substitution of the unpredictable stage name remains outside the
  storage-isolation boundary and is rejected by post-publication identity
  validation; Windows binds publication directly to the retained source handle.

  Cross-platform tests cover exact migration/checksum/fingerprint/codec
  constraints, idempotent process reuse, transactional rollback, bounded
  inspection, live status skew, newer-version read-only behavior, shared-layout
  siblings, foreign metadata preservation, collisions, missing ownership
  evidence, schema/ledger drift, hard links, and bounded lease release. Unix
  subprocess tests add cross-platform lock/version races, while Unix adds real
  SIGKILL hot-journal and missing-shared-memory WAL recovery. macOS tests add
  deny/allow ACL and physical alias cases;
  Windows-native tests add exact DACL/sidecar repair, source-path replacement,
  no-replace publication, and final-root rename-guard cases. Unix special-file
  and sidecar-symlink tests remain platform-scoped. This foundation exposes
  status only: domain CRUD, history queries, retention, binary snapshots,
  CLI/FFI transport, and real cleanup authority remain subsequent tasks.
- [x] Add durable scan, candidate, and cleanup-history persistence foundation.
  - Scan-history sub-checkpoint completed 2026-07-16: crate-private typed
    operations durably insert a running scan, compare-and-set it exactly once
    to a terminal summary, and load one validated record by stable ID. Writes
    hold the coordinator connection and cross-process lease, re-inspect exact
    schema compatibility before `BEGIN IMMEDIATE`, preserve lossless accepted
    host-codec absolute path bytes and checked time/count values, and reject duplicates, missing
    scans, invalid time order, and terminal rewrites. Exact-ID reads have fixed
    SQLite VM/deadline limits, decode every field into typed values, treat
    malformed lifecycle rows as corrupt observations, and survive coordinator
    teardown/reopen. The durable engine checkpoint below now calls this layer;
    the broad item stays open because v1 candidate rows cannot preserve
    paths/evidence/blockers/action, and
    v1 cleanup rows cannot preserve multi-path plan items or a dry-run's
    proposed Trash/permanent/eviction effect. Those facts require a subsequent
    schema migration before candidate or cleanup history can round-trip safely.
  - Candidate/cleanup schema sub-checkpoint completed 2026-07-16: checksummed
    schema v2 transactionally rebuilds the three lossy v1 history tables while
    preserving every legacy summary as explicit format 1, never fabricating
    missing paths, policy, or evidence. Format 2 candidates can preserve exact
    ordered lossless accepted-host paths, all typed evidence variants and
    blockers, category, safety/action policy, scheduling policy, and nanosecond
    timestamps. Cleanup sessions freeze source scan and exact plan lifetime;
    item/path/evidence/warning journals preserve multi-path plans and each
    proposed Trash, permanent-safe, or cloud-eviction effect even for dry runs.
    Owner/generation, heartbeat, cancellation, `effect_started`, and
    `outcome_unknown` fields reserve fail-closed crash recovery without treating
    process death as success or interruption. Exhaustive SQL checks constrain
    enum shapes, evidence nullability, policy pairs, path encodings, journal
    timing, and format separation. Separate exact v1/v2 object inventories and
    fingerprints, clean-chain and populated-upgrade tests, malformed v2
    regressions, and a 128-character multibyte legacy error prove compatibility.
    These rows remain sensitive historical observations, never planner or
    executor authority. The broad item stays open until typed candidate and
    cleanup APIs reject missing/noncontiguous child records and integrate with
    scan/cleanup lifecycles.
  - Candidate-history API sub-checkpoint completed 2026-07-16: a crate-private
    insert freezes one validated domain candidate as format 2 with status
    `discovered`, after preparing and bounding every path, byte count,
    nanosecond timestamp, evidence payload, and blocker before locking. The
    atomic parent/ordered-child transaction requires an existing source scan,
    holds the coordinator mutex and cross-process lease through exact schema
    inspection, commit, and post-commit storage validation, and documents
    exact-ID reconciliation for ambiguous post-commit failures. The bounded
    exact-ID reader returns a dedicated non-executable complete observation or
    an explicit legacy summary; it never reconstructs a domain `Candidate`.
    It checks SQLite types and byte lengths before allocation, contiguous
    ordinals, child limits, source-scan presence, absolute accepted-host paths,
    every enum/evidence shape, policy compatibility, scheduling rules, and
    exact per-path cloud-upload facts. Nine focused tests cover process-style
    reopen, all evidence and blocker variants, exhaustive mappings, legacy
    pollution, missing scans, duplicate rollback, invalid inputs, malformed
    ordinals and evidence, newer-schema skew, maximum 256-path/512-evidence/
    64-blocker budgets, and oversized values before materialization. At this
    checkpoint, status transitions, cleanup journaling, and lifecycle
    integration remained open. The subsequent journal slice closed the
    execution-state portion; the broad checkbox stays unchecked for candidate
    status and scan/evaluator/planner lifecycle integration.
  - Candidate review-status sub-checkpoint completed 2026-07-16: the
    crate-private format-2 history boundary now exposes only typed review
    commands, never an arbitrary status setter. Exact source-state transitions
    support `discovered` ↔ `selected`, either source to `dismissed`, and an
    explicit dismissed-to-discovered restore; there is no direct dismissed-to-
    selected edge. An exact target retry is idempotent, while evaluator/planner/
    journal-owned states cannot enter the review API. Selection means cleanup-
    review intent and requires a complete cleanup-capable observation with no
    blockers, but grants no plan or execution authority. Dismissal hides this
    exact scan-bound observation and does not create a security exclusion or
    revoke a plan already frozen elsewhere.

    Each operation full-decodes the bounded candidate and succeeded source scan
    before one primary-key/status compare-and-set under the coordinator and
    cross-process writer lease. It changes no immutable parent fact or child;
    legacy summaries, malformed children, missing IDs, incompatible schemas,
    and wrong source states fail closed. Commit ambiguity is adopted only when
    retained storage and the current schema revalidate under the same lease and
    the fully decoded row has the exact target state; unsafe storage becomes
    `OutcomeUnknown`. Complete candidate insertion and loading now reject
    running, cancelled, failed, or interrupted source scans, closing the path
    by which non-success traversal observations could satisfy later plan-
    history dependency checks. Migrated format-1 summaries still require only
    their historical scan foreign key and remain non-actionable.

    Focused tests cover select/deselect, dismissal from either source, explicit
    restore, idempotence, lifecycle-owned-state refusal, every blocker and
    cleanup/non-cleanup policy pair,
    immutable-fact preservation after reopen, exact concurrent CAS races,
    injected post-commit reconciliation, unsafe-storage ambiguity, legacy,
    missing/corrupt/newer-schema rows, every non-success scan status, the shared
    candidate/source-scan query budget, and selected-versus-dismissed plan-
    history admission including post-plan dismissal. Status remains a current
    projection, not an event timeline. Evaluator-owned stale/unavailable,
    planner-atomic planned, journal-derived completed/failed, and engine/FFI/UI
    transport remain open, so the broad persistence checkbox stays unchecked.
  - Candidate evaluator-status sub-checkpoint completed 2026-07-16: a sealed
    persistence-internal command boundary now owns invalidation of one complete
    scan-bound observation. Exact source commands permit `discovered`,
    `selected`, or `dismissed` to become `unavailable`, and permit those three
    states plus `unavailable` to become conclusively `stale`. Exact target
    retries are idempotent. Neither terminal state can return to review or
    become planned. Stale cannot become unavailable; unavailable can be
    conclusively refined to stale with a fresh exact-source command. Concurrent
    unavailable/stale commands have one compare-and-set winner, after which a
    reloaded unavailable observation converges through that explicit refinement
    edge. A later successful evaluation must create a new candidate ID tied to
    its new succeeded scan rather than reviving old evidence.

    Every command full-decodes the bounded format-2 candidate and its succeeded
    source scan before an exact status compare-and-set under the coordinator and
    cross-process writer lease. Missing, legacy, corrupt, wrong status-source,
    planner-owned, journal-owned, and newer-schema records fail closed. Shared
    post-commit reconciliation adopts only the exact target while retained
    storage and the current schema remain valid. Focused tests cover every
    source/target edge, exact retries, stale refinement including a concurrent
    unavailable/stale race, wrong-status-source refusal, blocked and non-cleanup
    candidate invalidation, missing/legacy/corrupt/planner/journal rows,
    immutable-fact preservation, process-style reopen, injected commit
    ambiguity, unsafe-storage ambiguity, and review-versus-evaluator races.

    This boundary is deliberately `persistence`-private and has no engine, FFI,
    Swift, CLI, AI, or production evaluator caller yet, so stored status still
    grants no authority. At this sub-checkpoint planner/journal coupling was
    still open and could not directly overwrite schema-v2
    `discovered`/`selected` rows without losing prior review intent and
    stranding dry-run, cancelled, or expired plans. The following schema-v3
    sub-checkpoint closes that persistence gap; engine integration remains
    open.
  - Candidate-plan claim coupling sub-checkpoint completed 2026-07-16:
    checksummed schema v3 adds an explicit per-session coupling version and a
    strict one-candidate-to-one-plan-item claim table. Existing schema-v2
    sessions migrate as coupling version 1 with no fabricated claim or prior
    review state; new sessions use coupling version 2. Both candidate and item
    ownership edges are delete-restricted, and exact v1/v2/v3 object
    inventories, fingerprints, populated upgrades, and format-2 pristine,
    running, recovering, and terminal migration fixtures keep compatibility
    explicit.

    New plan insertion now full-validates every succeeded-scan-bound candidate
    before one immediate transaction freezes the session graph, compare-and-
    sets only `discovered` or `selected` candidates to `planned`, and records
    each exact prior review state. A second incompatible candidate, duplicate
    claim, or competing plan rolls back the complete graph and every earlier
    projection. Candidate reads require exactly one bounded, format-2 claim
    while status is `planned`, reject claims on every other state, and validate
    joined storage classes, byte bounds, parent coupling, active lifecycle, and
    item identity before materializing values. Legacy uncoupled pristine plans
    cannot enter the new generation-one claim path.

    Journal terminalization derives candidate projection per complete item in
    the same transaction that freezes the terminal session and removes its
    exact claim. Mode-correct real effects become `completed`; dry-run and a
    fully effect-free cancelled item restore its exact `discovered` or
    `selected` state; every rejected, changed, unavailable, interrupted,
    skipped, mixed-failure, or failed terminal item becomes `failed`.
    `effect_started`, `outcome_unknown`, running, and recovering work retains
    `planned` plus its claim and cannot counterfeit completion. Restored review
    state remains a live projection after settlement, so later dismissal or
    evaluator invalidation does not corrupt immutable terminal cleanup history.

    A cleanup-lock-owned expiry settlement closes the otherwise stranded
    pristine-plan case without granting effect authority. It accepts the exact
    nanosecond expiry boundary, records a millisecond ceiling that cannot appear
    earlier than that boundary, rejects every item/path with `plan_expired`,
    projects candidates to `failed`, deletes claims, and writes generation-one
    terminal provenance atomically. One instant before expiry is refused.
    Failure on a later candidate rolls the entire settlement back, while
    injected post-commit ambiguity is adopted only for the same owner, rounded
    time, rejected graph, and candidate/claim transaction. Existing active
    uncoupled sessions remain recoverable and terminalizable without retroactive
    candidate projection.

    Focused regressions cover exact D/S preservation, late review refusal,
    competing plans, all success/failure/dry-run/cancellation projections,
    partial completion, outcome-unknown retention, missing/oversized/wrong-
    version claims, exact expiry, multi-item rollback, ambiguous terminal and
    expiry commits, immutable-history readability after later candidate state
    changes, maximum bounded graphs, and v2 migration lifecycles. This layer
    remains crate-private and performs no filesystem effect. The broad
    persistence checkbox stays open for deterministic evaluator/planner engine
    lifecycle integration, history query surfaces, and retention.
  - Planned-cleanup journal sub-checkpoint completed 2026-07-16: a crate-private
    boundary prepares and bounds an immutable cleanup plan before locking,
    requires its scan and every format-2 candidate observation to exist and
    match all frozen facts, and atomically inserts the format-2 session,
    contiguous items/paths/evidence/warnings, and each proposed effect even in
    dry-run mode. The bounded exact-ID reader returns a distinct complete
    `planned` observation or explicit format-1 legacy summary; it checks SQLite
    types and byte lengths before allocation, preserves legacy relative path
    observations without treating them as actionable, and rejects child
    pollution, gaps, over-limit records, malformed host paths/evidence,
    dangling or mismatched dependencies, overlap, invalid plan lifetime,
    estimate/warning drift, and mode/policy incompatibility. Six focused tests
    cover reopen with all three dry-run proposed effects, missing/duplicate
    rollback, legacy pollution, oversized rows/newer-schema skew, and the shared
    query budget at 256 paths plus 512 evidence facts. It cannot reconstruct a
    plan, claim an execution owner, transition or recover work, or perform an
    effect. At this checkpoint, the broad checkbox remained open for
    execution-state journaling and scan/evaluator/planner lifecycle integration;
    the subsequent journal slice closed the execution-state portion.
  - Cleanup-lock storage sub-checkpoint completed 2026-07-16: every secured
    store now retains a distinct exact-marker `<database>.cleanup.lock` with a
    bounded, non-expiring OS lock and independent same-process exclusion.
    Legacy provisioning happens exclusively under the existing writer lock,
    flushes the lock before a `.cleanup.lock.ready` control, then durably
    advances the retained root-ownership marker from layout v1 to v2. That
    non-recreatable v2 anchor distinguishes legacy absence from a current store
    whose one or both cleanup controls were removed; v2 never recreates a
    missing or malformed control. Every open and acquisition rechecks the
    private root inventory, marker length/bytes, owner, mode or protected DACL,
    regular-file/link shape, retained identity, and no-follow pathname.
    Windows retains the controls without `FILE_SHARE_DELETE`, preventing a
    `LockFileEx` holder from being displaced while a second inode is locked.
    Fourteen focused regressions plus expanded symlink/FIFO coverage exercise
    legacy upgrade serialization, crash-state refusal, malformed controls,
    hard links, same- and cross-process contention/release including abrupt
    owner death, writer-lock independence, and native Windows replacement denial
    for all retained controls. The guard proves only
    exclusion and has no plan, journal, owner, target, recovery, or effect
    capability. At this checkpoint, liveness evidence was still required before
    any owner-generation-fenced transition could use this exclusion primitive.
  - Process-instance liveness sub-checkpoint completed 2026-07-16: a private,
    strict, versioned owner identity fits the existing 128-byte journal field
    and combines platform, PID, OS process-start token, a 128-bit random claim
    nonce, and a SHA-256-scoped macOS boot session or Linux boot/PID namespace.
    The nonce distinguishes claims but is never liveness evidence. The probe
    returns `Alive`, `DefinitelyGone`, or `Unknown` rather than a boolean;
    only matching reliable scope plus an absent PID or changed start token can
    prove the old instance gone. Scope changes, partial/malformed OS data,
    permission failures, unsupported platforms, and every other ambiguity stay
    `Unknown`, so a database observed from another host cannot be recovered by
    consulting that host's PID table. Windows retains a queried process handle
    across creation-time and zero-time wait checks and can prove an exact live
    match, but intentionally reports non-live observations as `Unknown` until
    DUX has a reliable Windows host/boot scope. Pure regressions cover canonical
    encoding, malformed and maximum-width fields, PID reuse, scope mismatch,
    and unscoped behavior. If a hardened macOS runtime denies the boot-session
    sysctl, DUX retains an unscoped exact PID/start observation so durable work
    can still identify its live owner; that value can never prove death or
    enter scoped recovery. Native macOS/Linux subprocess coverage proves live,
    graceful-death, and abrupt-death classification. The module is not exposed
    through engine, FFI, CLI, Swift, AI, or cleanup execution and performs no
    journal writes. Cleanup-lock-coupled owner/generation compare-and-set
    transitions are implemented by the following slice; heartbeat age alone
    still cannot authorize recovery while an old worker could mutate. Because
    the current scope folds
    boot and host evidence together, a reboot also yields `Unknown`; recovery
    across reboot needs separate stable local-host provenance and must not be
    claimed by that same-boot fencing slice.
  - Cleanup-journal state-machine sub-checkpoint completed 2026-07-16: a
    crate-private, non-cloneable and non-shareable journal lease now owns the
    retained store-wide cleanup lock and an internally generated
    process-instance identity before it can claim a pristine plan as
    generation one. Every read and short `BEGIN IMMEDIATE` write revalidates
    that lock in cleanup-before-connection-before-writer order; every session,
    item, and path transition compare-and-sets the exact owner and generation.
    Typed transitions keep heartbeat evidence separate from expiry,
    cancellation request separate from acknowledgement, and derive item and
    session results from the complete path graph. A durable `effect_started`
    receipt is millisecond-canonical, retains its finer ordering instant, and
    can be issued or ambiguity-reconciled only after commit. It must be
    revalidated with no cancellation request and the held cleanup control
    immediately before a future executor call; this checkpoint performs no
    filesystem effect.

    The bounded full-graph reader validates immutable plan facts plus every
    mutable lifecycle field, ordinal, generation, timestamp, effect provenance,
    mode-compatible outcome, error key, and derived parent result. Terminal
    rows retain their owner/generation provenance and are immutable. Recovery
    probes liveness outside all database locks and writes only for same-scope
    `DefinitelyGone`: one transaction exact-CASes the stale phase, owner,
    generation, heartbeat, and cancellation bit, increments the generation,
    resets in-progress `validating` work to `planned`, and converts every
    interrupted effect to `outcome_unknown` without guessing its result.
    `Alive` and `Unknown` leave mutable journal fields byte-for-byte unchanged;
    recovering claims prohibit new validation/effects until unknown outcomes
    are reconciled and the remaining plan is explicitly resumed or cancelled.
    A cancellation arriving after
    durable effect intent blocks final revalidation and can record a known
    no-call interruption without retaining false effect provenance. Reboot and
    Windows death recovery remain sealed because their current scope evidence
    is insufficient.

    Twenty-eight focused tests cover all validation/effect/reconciliation
    outcomes, exact plan expiry and generation fencing, cancellation settlement
    and late-cancellation races, sub-millisecond timestamps, malformed effect
    provenance, corrupt attempts and terminal times, cleanup-lock contention,
    live/foreign-scope recovery refusal, durable effect receipts, unknown-
    outcome sealing, and native same-scope `DefinitelyGone` generation-two
    recovery/resume. Claim failures retain the lease, state-changing retries
    reconcile exact post-state, and a fault-injected ambiguous effect-start
    commit proves retries cannot substitute a different fine-grained instant
    within the same persisted millisecond. Premature terminalization cannot
    abandon a live-owned session. The state machine remains absent from engine,
    FFI, CLI, Swift, AI, and every effect primitive. The broad persistence item
    stays open for candidate status and scan/evaluator/planner lifecycle
    integration.
  - Fresh-scan snapshot-preparation sub-checkpoint completed 2026-07-16: a
    private node-ID-aligned provenance witness now exists only on a genuinely
    completed traversal and carries logical bytes, optional physical allocation,
    modification/access times, object identity, link count, and scan flags from
    one fresh traversal, with a platform handle capture where required. Public
    or cache-rebuilt `DiskTree` values cannot mint it, and cancelled or failed
    outcomes cannot become durable snapshots.
    Logical bytes count every pathname while multiply linked regular-file
    allocation is counted once per stable identity; the exact lossless
    lexicographically smallest path is the deterministic representative.
    Conflicting size, allocation, link-count, or modification observations fail
    allocation closed and become bounded changed-during-scan coverage facts.
    Unknown allocation stays `None` in the application snapshot while the
    legacy CLI retains the checked sum of bytes that are actually known instead
    of substituting logical bytes or collapsing the whole subtree to zero.

    The completed-only converter validates the live arena graph, rebuilds
    lossless host components from exact paths rather than display names,
    assigns canonical depth-first snapshot IDs, enforces every v1 bound and
    aggregate invariant, and rejects followed file or directory symlinks until
    the wire can represent that provenance. Empty directories remain exact
    `Some(0)` allocations. Tests cover sparse files, deterministic hard links
    across walker thread counts, conflicting identity observations, unknown
    allocation, empty trees, lossless non-UTF-8 names on non-macOS Unix,
    encode/decode, invalid capture time, followed symlinks, and non-completed
    type state.
    Cache v7 invalidates older per-path hard-link totals. The durable engine
    checkpoint below now consumes this converter.
  - Durable engine scan-task sub-checkpoint completed 2026-07-16: public
    `start_scan` admission canonicalizes and snapshot-bounds the requested root,
    refreshes current-schema write authority, and reserves one session-local
    overlapping root scope under the registry mutex before enqueueing. Queued
    cancellation releases that scope and creates no scan ID or history row.
    Workers generate a cryptographically random scan ID only after dequeue,
    exact-reconcile the canonical millisecond start record, install a race-safe
    scanner cancellation token, and run an explicit no-follow, full-depth,
    same-filesystem scan while publishing bounded typed progress/finalization
    events. The scanner's terminal claim, not a later generic cancellation
    check, decides success versus cancellation.

    Completed traversal artifacts pass through the completed-only converter and
    atomic snapshot repository, producing one immutable checksummed file plus an
    exact durable succeeded summary. Cancelled and failed traversals publish no
    snapshot and conservatively store default counts with their measured
    coverage instead of presenting an unfinalized partial tree as exact.
    A completed traversal can leave an unreferenced immutable orphan when file
    publication wins but SQLite completion cannot be reconciled; it is never
    exposed as the failed task's snapshot, and later bounded maintenance owns
    that case.
    Cancelled/failed tasks retain a typed scan result and stable scan ID when
    terminal persistence succeeds. An unwind after durable start is best-effort
    CASed to Interrupted; persistence ambiguity is surfaced distinctly and can
    never rewrite a succeeded row. Engine close requests real scanner
    cancellation; workers do not exit until the bounded terminal persistence
    attempt has finished.

    Integration tests cover successful publication and process-style reopen,
    snapshot decode, running and queued cancellation, close-time cancellation,
    real scanner failure, panic interruption and worker recovery, exact
    post-commit start reconciliation, current-to-newer schema fencing,
    overlapping ancestor/descendant scopes, canonical symlink aliases, result
    kind checks, and scope release. Scope exclusion is intentionally
    engine-session-local; cross-process scan leases, last-complete/history query
    APIs, FFI/Swift/CLI transport, hard-process-death recovery of a running row,
    and retention remain later work. Candidate evaluation is attached by the
    following checkpoint.
  - Deterministic candidate-evaluation lifecycle sub-checkpoint completed
    2026-07-16: a build-time and engine-open-validated SHA-256-bound catalog
    now maps the existing marker-verified M0 developer-artifact projection into
    stable, scan-bound candidates. At this completed checkpoint, all eleven
    initial rules explicitly used the selected scan root scope, were
    Informational/RevealOnly, were never
    schedule-eligible, and every result carries unresolved `ProtectedPath`;
    incomplete traversal adds a separate coverage blocker. The catalog's exact
    matcher arrays, policy, scope, revision, provenance-bearing rule documents,
    and fixed byte digest are validated before an engine can publish workers.
    Production evaluation accepts only the fresh completed-scan type-state
    witness used to build the same immutable snapshot, not a public or
    cache-reconstructed `DiskTree`. Candidate IDs and ordering are deterministic
    across arena insertion order and bind scan ID, rule revision, and lossless
    native target path. A versioned context digest additionally binds evaluator
    revision, exact catalog, selected root, coverage facts, and the explicit
    unresolved protection policy. The 4,096-result limit stops at the first
    excess match without first materializing an unbounded projection.

    Checksummed schema v4 adds one strict evaluation record per succeeded scan,
    binding evaluator/catalog/context identity to the exact snapshot version
    and digest. It distinguishes pending, succeeded, and typed failed states;
    migration fabricates no evaluations for historical scans. Normal engine
    scans compute a terminal discovery result before publication, then commit
    the succeeded scan summary, evaluation identity, entire candidate batch or
    typed failure, and terminal state in one immediate SQLite transaction after
    the immutable file is durable. Thus no normal crash boundary can expose a
    succeeded scan without its evaluation result. A reserved pending-only API
    exists for a future recovery worker but is not claimed as operationally
    restartable until snapshot-to-evaluator reconstruction and bounded pending
    queries land.

    Exact retry and ambiguous-commit reconciliation compare the scan,
    snapshot, identity, normalized times, terminal state, and complete unordered
    candidate set. Batch insertion validates source scan and unique IDs before
    locking and rolls back every candidate on any later failure. Bounded reads
    use set-based parent/child/claim queries instead of N+1 loading, validate
    storage classes, format, terminal time order, snapshot binding, candidate
    count, and every complete child record. The dedicated fixed VM/time budget
    admits the 4,096-candidate cardinality maximum when its aggregate graph
    also fits the later 32 MiB decoded-materialization limit. Standalone legacy
    candidate insertion is refused for any scan that owns an evaluation row,
    preventing post-terminal batch drift. Cancellation observed after the
    scanner's Completed claim but before discovery's final cancellation
    checkpoint records discovery as Cancelled while the scan, snapshot, and
    overall scan task remain truthfully succeeded. Requests accepted after that
    point of no return remain visible as task intent but cannot rewrite the
    already-produced terminal batch. Public task results and
    ordered events expose NotRun, candidate count, or a path-free typed failure;
    non-successful traversals create no evaluation.

    Focused tests cover zero and marker-matched batches, process-style reopen,
    late cancellation, exact retry, sub-millisecond post-commit ambiguity,
    cross-scan and duplicate rejection, complete rollback, typed failure,
    hostile rows, pending reservation, standalone insertion sealing, and
    non-success traversal exclusion. The broad persistence checkbox remains
    open for planner/executor engine lifecycle integration, bounded history
    query surfaces, and retention maintenance.
  - Durable candidate-discovery read-bridge sub-checkpoint completed
    2026-07-16: `EngineHandle::candidate_history_for_scan` now loads one exact
    scan-bound discovery observation from the private store after an open-
    session preflight. The query distinguishes a missing scan from a real scan
    whose evaluator did not run and preserves Pending, Succeeded, or typed
    Failed evaluator state. Before any payload-bearing child query, it counts
    rows and encoded bytes against the same conservative 32 MiB aggregate
    materialization budget applied during evaluation write preparation. That
    preflight, the dedicated VM/time budget, and full graph decoding reject
    oversized batches, incompatible schema, hostile storage classes,
    malformed children, count/status/time drift, and cross-scan candidates
    before publishing any value. A valid evaluator result that exceeds the
    aggregate budget becomes a durable typed discovery `LimitExceeded` failure;
    it does not turn the successfully completed scan into a persistence
    failure.

    The public durable DTO survives task-record eviction and process-style
    reopen. Successful evaluations expose only candidate ID, rule, category,
    estimated bytes, newest mtime, safety/action policy, schedule eligibility,
    path count, evidence kinds, blockers, creation time, and historical status.
    Exact paths and evidence payloads were left sealed for the following paged
    detail boundary. A stored review, planned, or terminal status is an
    observation only: this bridge cannot reconstruct a domain `Candidate` or
    `CleanupPlan`,
    mutate review state, validate current evidence, approve cleanup, or reach
    an effect.

    Focused regressions cover missing/closed sessions, zero- and one-candidate
    success across reopen, evaluator cancellation, non-successful scans with
    NotRun, exhaustive candidate-status and failure mapping, newer-schema
    fencing, corrupt child rejection, exact aggregate-budget boundaries, and a
    schema-shaped over-budget graph rejected before payload decoding. The broad
    persistence checkbox remained open for the following detail/review slice,
    cleanup-history queries, planner/executor engine lifecycle, and
    FFI/Swift/UI/CLI transport. History must never become a planner witness.
  - Bounded candidate-review boundary sub-checkpoint completed 2026-07-17:
    the core engine now exposes separate exact path and evidence pagers keyed
    by both scan and candidate ID. Code-owned 1..=64 limits, strict immutable
    cursors, exact totals, full evaluation/candidate validation, and the shared
    SQLite VM/time plus 32 MiB decoded-materialization ceilings bound every
    call. The exact-candidate loader now performs its own scalar/type/count/
    encoded-byte preflight before any child payload is copied, including
    hostile large values hidden in scalar columns; standalone candidate writes
    apply the same charge so accepted observations remain reloadable.

    Path pages are an explicit user-requested local disclosure. Each item
    carries a display string plus the lossless accepted-host UTF-8 or
    little-endian UTF-16 representation without exposing SQLite tags or a
    `PathBuf` execution input. Evidence pages map all eight variants into a
    distinct durable presentation type, so even a lossless historical fact
    cannot satisfy planner validation. Candidate IDs remain path-derived
    pseudonyms and neither IDs nor detail payloads are remote/AI-safe by
    default.

    `Select`, `ClearSelection`, `Dismiss`, and `Restore` are the only public
    review commands. They accept no paths. The store validates the complete
    exact candidate and source-scan binding, resolves discovered-versus-
    selected dismissal inside the same immediate transaction, then preserves
    the existing writer lease, compare-and-set, exact retry, and ambiguous-
    commit reconciliation. Selection still requires a blocker-free cleanup
    policy. Every currently emitted candidate still has `ProtectedPath` and
    therefore rejects selection, including the later M5 Rust target rule that
    may propose a safe-regenerable cleanup policy; candidates may be dismissed
    and restored. Planned, evaluator-
    owned, and journal-owned states cannot enter review. Dismissal creates no
    exclusion, does not revoke an existing plan claim, and no command creates
    a plan or effect.

    Focused regressions cover paging/cursor/limit boundaries, lossless path
    transport, all evidence variants, reopen, missing and non-successful scans,
    source mismatch, closed/newer-schema sessions, corruption and over-budget
    rejection, exact preflight byte boundaries, hostile parent/child scalar
    blobs, select/clear/dismiss/restore idempotence, blocked selection,
    terminal-owner refusal, concurrent select/dismiss convergence, and exact
    post-commit adoption. Maximum-legal cleanup-history and active-journal
    contracts also prove that the exact preflight does not consume their
    enclosing aggregate query budget. At that checkpoint the broad persistence
    checkbox remained open for the following cleanup-history bridge,
    planner/executor engine lifecycle, FFI/Swift/UI/CLI transport, and retention.
  - Bounded cleanup-history read-bridge sub-checkpoint completed 2026-07-17:
    `EngineHandle::recent_cleanup_history` now returns a path-free 1..=64
    keyset page ordered by `started_at DESC, session_id ASC`; an opaque cursor
    and limit-plus-one sentinel preserve equal-time ordering without an
    unbounded offset. Summaries distinguish migrated format-1
    `LegacyIncomplete` from complete format-2 storage and expose only
    plan/source identity, lifecycle
    times, mode, trigger, status, cancellation observation, estimated bytes,
    separately verified capacity delta, bounded graph totals, and exhaustive
    item/path status counts. Exact lookup accepts only an opaque session token
    obtained from the feed and returns bounded path-free item policy/status
    summaries and warnings.

    Every selected recent row first passes parent and format-shape checks,
    checked estimate summation, graph-count and encoded-byte preflight,
    per-item nonempty/contiguous path and evidence relationships, warning
    continuity/uniqueness, source-scan existence, and active claim coupling.
    A journal-owned scalar validator then reuses the execution state machine's
    lifecycle, generation/time, path-shape, item derivation, mode/action success,
    active/recovering, and terminal-status rules without reading target paths,
    evidence payloads, or candidate payloads. Exact format-2 lookup additionally
    runs the complete bounded journal decoder before scrubbing its projection;
    exact legacy lookup preserves its incomplete summary without fabricating
    source, lifetime, policy, evidence, warning, or trusted error-category facts.

    Returned DTOs contain no target path, evidence payload, candidate ID,
    owner/generation/heartbeat, plan claim, prior review state, or execution
    fence. Neither historical policy nor an outcome can become current
    validation, planner input, approval, recovery permission, or an effect.
    Reads retain the normal compatibility/writer guard for a coherent view but
    intentionally acquire no cleanup OS lock. Separate fixed SQLite VM/deadline
    envelopes guard recent and exact calls, and a conservative 64 MiB per-graph
    read-materialization charge rejects oversized storage before payload decode.
    That read cap is intentionally not claimed as write-admission symmetry: a
    schema-valid historical graph can later return `QueryLimitExceeded`.

    Focused regressions cover empty/closed/missing and limit behavior,
    process-style reopen, same-time cursor order, 128-character multibyte legacy
    error scrubbing, all status/error mappings, a legal 64-record page plus
    sentinel, no-mutation/path-disclosure assertions, orphaned and gapped child
    graphs, scalar lifecycle drift, and oversized payload rejection before
    decode. History remains observation, never a planner witness.

  - M2 persistence scope completed 2026-07-17: schema versions 1 through 10,
    the durable scan/snapshot/evaluation lifecycle, candidate review and
    invalidation state, plan/journal coupling, and bounded scan, candidate, and
    cleanup-history read bridges provide the shared restart-safe engine
    foundation required by this milestone. The remaining product surfaces are
    assigned explicitly below rather than hidden behind this broad checkbox:
    snapshot/history drill-down and read-only detail transport are M4;
    evaluator recovery, reviewed planner/executor orchestration, cleanup-history
    presentation, and cross-reboot execution fencing are M5; final CLI
    transport, cross-process scan-scope leasing, storage controls, and release
    compatibility are M9. None of those deferred surfaces is implied complete
    here, and no historical row grants current plan or effect authority.
- [x] Keep binary snapshots atomic and checksummed. Completed 2026-07-16:
  `dux-core::persistence::snapshot` now owns an independent crate-private v1
  full-tree wire rather than extending the legacy CLI cache. Its frozen
  96-byte header, 112-byte depth-first node records, and trailing SHA-256
  preserve lossless host roots/components, stable node IDs, logical and
  allocated aggregates, file/directory counts, optional modification/access
  times, typed inaccessible/timeout/hard-link/mount flags, optional Unix
  device/inode observations, and exact node kinds. Fixed bounds cap files at
  2 GiB, nodes at five million, depth at 4,096, components at 1 KiB, and
  reconstructed encoded paths at 64 KiB. Decoding copies bounded host values,
  grows node storage only after records exist, rejects unknown/reserved bits,
  duplicate sibling paths, graph/child/depth drift, noncanonical optional
  fields, arithmetic overflow, inconsistent aggregates, checksum errors,
  incompatible versions, truncation, and trailing data. Platform-specific
  whole-wire golden digests prevent encoder/decoder drift without a version
  bump. These bytes remain sensitive non-authoritative observations.

  The exact `<database parent>/snapshots` owner independently provisions a
  marker-complete private directory. Corrected provisioning creates
  `<database parent>/.dux-snapshot-stage-<32 lowercase hex>` inside that same
  retained marker-owned root and atomically publishes it without replacement
  to the sibling `snapshots` entry. It validates a bounded exact inventory and
  uses exclusive PID-plus-random temps, collision winner validation, and
  read-only reopened final handles. Unix
  creation repairs exact 0700/0600 modes even beneath a restrictive umask;
  snapshot provisioning requires the exact private database root (and, on
  macOS, no extended ACL); the
  deny-only publication-parent exception applies only to initial SQLite-root
  provisioning on macOS. Windows uses protected owner-only DACLs, retained IDs, handle-relative
  operations, reparse/multi-link rejection, no-replace tests, and read-only
  winner reopening. Read-only newer-schema startup never provisions storage.

  SQLite stores the positive version, canonical relative file name, encoding,
  and 32-byte digest as an all-or-none tuple only on successful scans. The
  mutation order is SQLite connection → SQLite writer/compatibility lease →
  snapshot writer. A current-to-newer race is rechecked before provisioning,
  staging, and publication; publication retains snapshot exclusion through the
  exact scan-summary CAS and post-commit revalidation. The file is durable
  before SQLite can reference it; an ambiguous commit is reconciled by every
  frozen completion fact, and exact retries or existing-file collisions decode
  and compare the entire document plus digest. Focused tests cover corrupt,
  truncated, hostile-length and checksum-valid hostile inputs, missing/corrupt
  durable references, same-ID/different-document collisions, process-style
  reopen, file-first orphan adoption, post-commit ambiguity, lock contention,
  restrictive umask, macOS ACLs, version-skew races, and Windows storage
  compilation/regressions. The normative implementation reference is
  [`docs/SNAPSHOT_FORMAT.md`](docs/SNAPSHOT_FORMAT.md). Durable engine scan-task
  publication is now attached by the following checkpoint. Last-complete
  selection, latest-two/2 GiB retention, app/FFI review-lease binding, and
  exact-marker-owned root-local provisioning-stage maintenance were later
  tasks at this checkpoint and are recorded by the sub-checkpoints below.
  Pre-correction external snapshot stages are not root-bound, remain manual
  debt, and are not claimed by this checkpoint. Typed coverage/issues and the
  completed-only fresh-scan converter were attached by following checkpoints
  without changing the v1 snapshot wire.
- [x] Add capacity sample storage. Completed 2026-07-16: the existing SQLite
  schema-v2 `volumes` and `disk_samples` tables now have a typed Rust boundary
  for raw, non-authoritative capacity observations. Public opaque `VolumeId`
  and `DiskPressure` values constrain stable identity and semantic pressure
  labels without exposing persistence internals. Stable volume IDs are supplied
  evidence and are never synthesized from a mount path, display name, or
  filesystem label. Each write requires a positive SQLite-representable total,
  required ordinary available capacity, optional important-usage available
  capacity, an absolute losslessly encoded mount path, millisecond-canonical
  time, and bounded metadata; both availability values remain separate and
  cannot exceed total capacity. A valid UI sample that has important-usage
  capacity but no ordinary available capacity is deliberately not persisted;
  the storage layer never fabricates the missing ordinary value.

  One current-schema transaction under the coordinator connection and
  cross-process writer lease monotonically inserts or refreshes volume metadata
  and records the raw sample. Routine observations are suppressed after one per
  UTC hour, while a real change from the latest stored pressure is written
  immediately. Exact natural-key collisions and ambiguous post-commit failures
  succeed only after retained storage and the current schema revalidate, every
  stored sample fact matches, and the timestamp remains inside the volume's
  first/last-seen interval; a differing collision, stale timestamp, schema-
  version race, unsafe storage, or malformed value fails closed. Latest and
  cursor-based history reads are page-bounded, protected by
  SQLite VM/time budgets, deterministically ordered, and validate hostile
  storage classes, lengths, enums, paths, timestamps, and numeric ranges before
  returning typed observations. Process-style reopen, concurrent exact writers,
  cadence/transition boundaries, exact retry reconciliation, before-first and
  after-last metadata corruption, unsafe post-commit storage, hostile rows,
  cursor continuity, and current-to-newer version fencing have focused coverage.

  This checkpoint does not connect Swift, engine tasks, UniFFI, either CLI, or
  the pressure evaluator. It does not create daily rollups, enforce the 30-day
  raw-history policy, run retention, or persist pressure episodes. Those remain
  later roadmap work.
- [x] Add typed scan coverage/issues. Completed 2026-07-16: bounded public
  semantic values now distinguish Unknown, Complete, Limited access, and
  Partial without exposing constructors that let clients manufacture a false
  measured result. Thirteen explicitly mapped issue kinds carry canonical
  localization keys, optional accepted-host absolute paths, checked occurrence
  counts, and a stable ordering independent of Rust discriminants. The
  scanner's deterministic accumulator retains at most 256 records, reserves an
  overflow fact, and reports permission/metadata failures, policy exclusions,
  symlink omissions, depth boundaries, mount decisions, cancellation, probe
  timeouts, filesystem-boundary uncertainty, and queue/pool exhaustion.

  `ScanOutcome`, rather than progress messages, is the terminal authority and
  keeps the tree paired with its coverage and Completed/Cancelled/Failed state.
  The progress channel is bounded, cancellation uses a final one-way terminal
  claim, native max-depth traversal never opens the excluded boundary, and the
  fixed probe pool distinguishes queue starvation from a started syscall. A
  circuit fast-fails repeated probes while every worker is known unavailable
  after timeout or cancellation and recovers when workers return. Focused
  regressions cover clean, policy-skipped,
  depth-limited, no-follow symlink, missing-root, pre-cancelled, stalled-
  consumer, deterministic-overflow, timeout, and circuit-recovery behavior.

  Schema v2 now atomically commits terminal coverage, optional permille, the
  checked sum of issue occurrences, canonical message keys, and losslessly
  shortened root-relative issue paths with counts and an optional immutable
  snapshot reference. Bounded reads reconstruct and validate the complete
  report, reject hostile types/lengths/encodings/kinds/keys, duplicate or
  out-of-root facts, parent/child count mismatches, and terminal-status
  contradictions. Exact retries and ambiguous commits adopt stored facts only
  after retained-storage and current-schema revalidation. Process-style reopen,
  rollback, collision, post-commit ambiguity, unsafe storage, and hostile-row
  tests cover the boundary.

  Fresh CLI results retain and render the coverage qualifier; legacy cache
  trees, including cache v7 after the hard-link accounting change, deliberately
  reload as `coverage unknown` instead of being mislabeled Complete.
  Scanner-to-engine task wiring is now complete. Swift/FFI transport, paged
  Explorer issue details, permission onboarding, and legacy-cache migration
  remain in their later roadmap items. Full logical/allocated/hard-link
  accounting is attached to the completed-only fresh-scan snapshot converter
  described above.
- [x] Add JSON CLI status/history scaffolding. Completed 2026-07-16: the
  existing `dux [PATH]` TUI remains the default while explicit `status` and
  `history [--limit 1..=200]` commands dispatch before terminal raw mode. Both
  have human output and opt-in JSON schema v1; every serialized object is
  versioned, paths and storage locations are deliberately absent, unknown
  allocation/coverage remain null, non-successful SQLite count defaults are
  not presented as measurements, and a newer read-only database is distinct
  from an empty history. Runtime JSON errors are path-free stderr objects with
  stable categories and nonzero status; broken pipes exit cleanly. Reserved
  command-name directories remain reachable through `./name` or `-- name`.

  `EngineHandle::recent_scan_history` supplies the shared, non-authoritative
  DTO: one fixed-budget query selects at most 200 plus a `has_more` sentinel in
  start-descending/ID-ascending order backed by `scans_by_started`, passes each
  selected row through
  the strict full scan/coverage decoder, exposes counts only for success, never
  opens snapshot files or candidate batches, survives process-style reopen,
  and maps corruption, schema skew, contention, unsafe storage, and resource
  limits to typed path-free failures. Its dedicated fixed ceiling is sized for
  the legal maximum of 200 parents and 51,200 coverage children without
  weakening the smaller exact-record budget. Parser, boundary, hostile-row,
  maximum-child, reopen, null-semantics, newer-schema, and exact golden JSON
  tests cover the slice.
  [`docs/CLI_JSON.md`](docs/CLI_JSON.md) is the normative v1 contract.
- [x] Add bounded automatic history and DUX-owned snapshot retention
  maintenance.
  - SQLite capacity/AI-cache sub-checkpoint completed 2026-07-16: one
    current-schema, writer-leased immediate transaction creates deterministic
    UTC daily capacity rollups from the exact last raw tuple, retains raw
    samples for 30 exact days and daily rollups for 365 complete UTC days, and
    removes AI insights only at or after their stored expiration. Raw pruning
    inside the daily window first requires the exact rollup in the same
    transaction; a conflicting rollup or malformed target rolls the entire
    batch back. Fixed limits prune at most 128 raw rows, create at most the 128
    rollups required by that selected raw batch, prune 128 daily rows and 16 AI
    rows, and report `has_more` for later idle rescheduling. Every retained day
    is rolled up no later than its first raw sample aging out; any future earlier
    rollup pass requires a durable bounded cursor so dense covered history cannot
    starve pruning.
    A SQLite authorizer admits only capacity inserts/deletes and AI-table
    deletes; the private fixed AI statement additionally binds the selected ID
    and expiration cutoff. A fixed VM/deadline budget bounds every query and
    write.
    Exact post-commit reconciliation, newer-schema fencing, boundary,
    idempotence, batch-cap, corruption/rollback, and forbidden-table tests cover
    the private store operation. [`docs/RETENTION.md`](docs/RETENTION.md) is the
    normative policy.
  - Engine orchestration sub-checkpoint completed 2026-07-16: the shared engine
    now exposes one typed, path-free history-maintenance task admitted only at a
    session-local idle boundary. Closed, duplicate, and busy requests are
    resolved before SQLite access; eligibility and current-schema compatibility
    are rechecked before admission. Each task samples its clock on the worker,
    runs exactly one bounded store batch, reports typed counts and `has_more`,
    and never self-enqueues a drain loop. Duplicate starts return the exact
    active task ID, while foreground work returns `DeferredBusy` without
    allocating a task record.

    Cancellation/close and the `Applying` point of no return are ordered under
    one registry lock: cancellation that wins performs no mutation, while a
    request after `Applying` remains truthful intent and cannot rewrite a
    committed success. Stable failure categories cover clock, schema, budget,
    contention, storage, corruption, ambiguity, and internal state without
    exposing paths. Tests cover event order, explicit `has_more` rescheduling,
    two engine sessions sharing one store, corrupt-row rollback, schema races,
    cancellation on both sides of the commit, duplicate/idle admission, panic
    cleanup, and exclusive-marker release. At that checkpoint the broad item
    still awaited the snapshot-retention prerequisites and writer recorded
    below, plus app/FFI periodic scheduling, orphan/temp/stage maintenance, and
    explicit user clear-data actions.
  - Snapshot logical-availability prerequisite completed 2026-07-16: schema v5
    adds an append-only `snapshot_retention_tombstones` relation whose exact
    succeeded scan ID, completion time, snapshot version, losslessly encoded
    relative name, and digest are foreign-key-bound to the immutable terminal
    scan reference. Migration from v4 fabricates no tombstones; update/delete
    guards are activated only after the exact supported schema fingerprint and
    preserve the historical transition; untrusted/newer schemas retain the
    existing trigger-disabled read-only posture. The bounded loader validates
    storage classes and lengths, treats a mismatched row as corruption, and
    checks logical availability under a current-schema database guard before
    opening the retained file. Exact tombstones return a distinct unavailable
    result while the valid referenced bytes still exist, while the scan row
    remains immutable and explainable. The already-guarded publication
    retry path reuses its guard and cannot deadlock by reacquiring the
    connection mutex. Migration fingerprint/constraint, v4 upgrade,
    valid-load, hostile-row, tombstone-before-file, reopen, and guarded-retry
    regressions cover the boundary.

  - Snapshot active-review prerequisite completed 2026-07-16: schema v6 adds
    an explicit mutable `snapshot_review_pins` lease relation instead of
    inferring an open UI from candidate selection or cleanup-session state.
    Every row is exact-composite-FK-bound to one succeeded snapshot, uses a
    canonical random 128-bit ID, a strictly decoded process-instance owner,
    an `explorer` or `cleanup_review` purpose, and a fixed ten-minute expiry.
    Identity, owner, purpose, and creation are immutable; only monotonic exact
    renewal and idempotent exact release are allowed. V5 upgrades fabricate no pins.

    The sealed repository API uses one stable owner per repository, admits at
    most 64 local/owner leases and 1,024 rows per store, inspects every bounded
    row with explicit storage-class and relationship validation, prunes at most
    64 expired rows during acquisition, and treats expiry equality as inactive.
    It generates 16 collision-resistant IDs before locking, validates the
    exact parent and tombstone state, then retains the snapshot writer lock
    while inserting the pin under the existing database guard. The returned
    non-cloneable lease owns a retained read-only file handle, can load, renew,
    and explicitly release the exact row idempotently after expiry pruning,
    and reconciles only exact
    post-commit states. Drop never enters SQLite or inverts lock order; its row
    expires naturally. Independent store reopen, arbitrary Explorer acquisition,
    renewal/release/drop/equality including post-prune expiry, deterministic
    collision retry within one prune pass, bounded pruning and population
    limits, hostile rows, external newer-schema fencing,
    read-only/tombstone/missing/corrupt files, post-commit ambiguity including
    conflicting release state, and publication-lock contention have focused
    coverage. An expired lease still retains its file handle until the future
    app/FFI owner drops it; this is an explicit lifecycle gate before unlink.

    This checkpoint intentionally provides no production tombstone writer.

  - Snapshot retention-inventory prerequisite completed 2026-07-16: schema v7
    adds a partial exact lookup index over losslessly encoded snapshot names,
    and the sealed repository now reconciles one bounded physical inventory
    under the permanent database-before-snapshot lock order. One directory
    pass sequentially opens every final and recognized temp, captures its
    identity and usage, closes it, then sequentially reopens and revalidates
    each name before handoff. This avoids making the 2,048-entry bound a file-
    descriptor requirement while it enforces the
    2,048-entry/256-KiB-name/64-temp/250-ms bounds, and measures logical plus
    platform allocation bytes from validated handles on Unix and Windows. The
    cap charges `max(logical, allocated)` and separately reports controls,
    available/protected/eligible snapshots, tombstoned residuals, physical
    orphans, and unknown-liveness temps with checked totals.

    Each physical final receives at most one indexed, strictly typed history
    match; exact raw root bytes define groups without filesystem
    canonicalization. Physically present, logically available succeeded
    snapshots receive deterministic latest-two ranks per exact root, and
    unpinned older observations are ordered oldest-first. The complete bounded
    pin population is decoded without pruning; expiry equality is inactive,
    while active-pin/missing-file and active-pin/tombstone contradictions fail
    closed. Duplicate references, hostile rows, oversize files, accounting
    overflow, unsafe objects, and bounded lock/query failures likewise produce
    no eligibility evidence. Recognized temps are point-in-time accounting
    only because an active stage can continue growing outside the writer lock.
    The metadata report exposes no tombstone, unlink, or cleanup authority, and
    focused restart-style, corruption, no-mutation, cap, orphan/temp, ranking,
    pin, expiry, storage-bound, Unix accounting, and Windows handle-accounting
    regressions cover the slice.

    Immutable history can outgrow the bounded physical directory, so this
    physical-driven inventory intentionally cannot enumerate every missing old
    reference. Exact loads still report a requested missing snapshot; any full
    diagnostic history pager must remain bounded and non-authoritative.

  - Settings-backed snapshot-cap prerequisite completed 2026-07-16: the
    existing schema-v1 `settings` table now reserves only the exact
    `snapshot_retention` key for a deny-unknown value-schema-v1 object whose
    canonical JSON is `{"cap_bytes":<u64>}`. Absence means the versioned 2 GiB
    default without creating a row; explicit set and reset never touch unknown
    keys. Explicitly setting 2 GiB stores that override; reset removes it, so a
    future default change cannot erase user intent. Every load checks SQLite
    storage classes and small byte bounds before
    allocation, rejects malformed or noncanonical current values as corrupt,
    and reports a newer per-setting schema as incompatible instead of silently
    using the default or overwriting it. All `u64` caps, including zero, are
    policy-valid; latest-two and active pins remain protected independently.

    Typed, path-free core engine get/set/reset APIs expose default-versus-stored
    provenance and update time, reject closed or newer-schema sessions, and
    exact-reconcile post-commit failures. Multiple engine sessions observe the
    same durable value across process-style reopen. The sealed retention
    inventory reads the effective cap after acquiring its current-schema
    database guard and before the snapshot lock, then holds that guard through
    reconciliation. It never trusts a cached settings DTO. Focused default/no-
    write, unknown-key, canonical-boundary, exact-retry/reset, reopen,
    malformed/newer-value-schema, clock, engine-lifecycle, multi-session, and
    inventory-integration regressions cover the slice. This prerequisite itself
    added no FFI/Swift setting, tombstone writer, unlink, or cap-enforcement
    task; those boundaries are recorded by the later checkpoints below.

  - Durable snapshot-temp-lease prerequisite completed 2026-07-16: schema v8
    adds a bounded immutable `snapshot_temp_leases` relation for the exact
    running scan, deterministic final name, unique recognized temp name,
    process-instance observation, creation time, and random 128-bit lease ID.
    V7 upgrades fabricate no leases. Insert and update guards enforce the
    64-row bound and immutable creation facts, while a scan cannot transition
    to `succeeded` until its exact lease is consumed. Failed, cancelled, or
    interrupted scans may retain a row as explicit recovery debt; PID and
    process-instance data never establish liveness.

    Staging now acquires the current-schema database guard before the snapshot
    writer lock, reserves a unique name, commits the row, and only then creates
    the private file. The staged handle holds a nonblocking kernel-exclusive
    lock while encoding continues outside both store-wide locks. This
    row-before-file order means a crash may leave a row without a file or a
    row-bound file, but a compliant writer cannot create the file later after
    releasing the reservation. Drop and unwinding close only.

    The read-only inventory strictly reconciles the complete bounded lease
    population with physical temps. A contended kernel lock reports `active`;
    an acquired-and-released probe reports `quiescent_at_observation`; a
    recognized temp with no row reports `unleased`; and a row with no file is
    separate residual metadata. Active and unleased temps keep accounting
    unstable, while every class remains charged and non-evictable. The state is
    an observation, not scavenging authority.

    Before creating a replacement for the same scan, the repository may
    reconcile only that scan's exact residual while holding database then
    snapshot exclusion. Active returns busy. A quiescent row-bound temp is
    reopened by name, required to retain its observed identity, locked
    nonblockingly, revalidated, unlinked, and followed by a directory flush
    before the exact row is deleted. A row-only residual is deleted under the
    same locks. Unleased temps are never adopted or removed by this retry.
    Normal abort likewise performs physical removal and directory durability
    before exact row consumption. Publication first durably publishes or
    validates the immutable winner, then consumes the lease in the same SQLite
    transaction that commits the exact succeeded scan and optional evaluation,
    retaining the snapshot writer lock through commit and revalidation.
    A retained current-call staged handle remains narrow rollback authority
    over only its exact identity: missing/conflicting metadata forbids
    publication, removes that current temp, preserves conflicting rows, and
    returns corruption. Failure to establish the current-schema guard remains
    close-only; this is not unleased-temp scavenging.

    Migration/fingerprint, strict row decoding and constraints, row-before-file
    residual, active/quiescent/unleased inventory, exact retry, atomic terminal
    commit, and cross-process kernel-lock regressions cover the implemented
    boundary. The kernel path is implemented for Unix and Windows, but native
    Windows runtime verification remains outstanding. This checkpoint does not
    provide a general temp sweeper, terminal-scan residual maintenance,
    provisioning-stage scavenging, tombstone insertion, final-file unlink,
    app/FFI ownership, scheduling, or cap enforcement.

  - Core snapshot-cap enforcement sub-checkpoint completed 2026-07-16: one
    sealed repository batch now holds the current-schema database guard before
    one snapshot writer lease and rebuilds the complete physical/history,
    settings, temp-lease, and review-pin inventory inside that final mutation
    boundary. It removes at most one final per call. A pre-existing exact
    tombstoned residual is handled first even when the store is below cap;
    otherwise active or unleased temps defer new retirement, latest-two per
    exact lossless root and active pins remain protected, and the deterministic
    oldest eligible available snapshot is selected only while charged bytes
    exceed the freshly loaded cap.

    Before either path may unlink, the exact observed final is reopened
    read-only, required to retain its identity and logical/allocated usage, and
    fully decoded against the immutable scan ID and snapshot digest. New
    retirement prepares and validates the complete succeeded parent tuple,
    commits one append-only tombstone, exact-reconciles commit-adjacent failure,
    and only then reopens the same observed identity with deletion access while
    keeping the digest-validated read handle live. Retained/name identity and
    usage are rechecked before descriptor-relative name unlink on Unix or
    handle disposition on Windows, followed by a durable snapshot-directory
    flush. The inventory lease remains valid after removal. A commit/schema
    race therefore leaves a logically unavailable residual for the next batch;
    changed bytes or a post-validation replacement are not removed by that
    batch. Scan history and tombstones are never deleted or rewritten. As with
    publication, malicious same-user name substitution remains outside the
    private-store isolation guarantee on Unix.

    Focused latest-two, oldest-first, active-pin, active/unleased-temp deferral,
    quiescent-temp continuation, zero/max-cap, exact post-commit,
    future-schema residual, changed-content,
    post-validation replacement, retained-identity, directory-accounting, and
    tombstone-writer regressions cover the slice.
    Unix storage tests execute the unlink path and Windows MSVC compilation
    covers its delete-capable retained handle; native Windows runtime remains
    outstanding.

  - Engine snapshot-retention orchestration sub-checkpoint completed
    2026-07-17: `EngineHandle::start_snapshot_retention` now admits one typed,
    path-free `SnapshotRetention` task only at a session-local idle boundary.
    Closed, duplicate, foreground-busy, and other-maintenance-busy states
    already visible at initial preflight are resolved before storage access;
    after the compatibility refresh, lifecycle, duplicate, and idle admission
    are rechecked before the task record becomes active. Duplicate requests
    return the exact active task ID. Each admitted task samples its clock on the
    worker and calls the sealed repository cap batch exactly once; it accepts
    no cap, inventory, victim identity, or path and never self-enqueues another
    batch.

    The repository canonicalizes the observation time, rereads the cap, and
    rebuilds the complete inventory under the final database-to-snapshot lock
    order. The public result deliberately discards the selected scan identity
    and exposes only the aggregate cap, charged bytes before and after,
    `has_more`, and one of `UnderCap`, `DeferredUnstable`,
    `DeferredNoEligibleSnapshot`, `RemovedTombstonedResidual { bytes }`, or
    `TombstonedAndRemoved { bytes }`. Checked accounting is established before
    physical mutation. Stable path-free failures cover clock, schema,
    contention, unsafe storage, resource limits, corruption, snapshot-version
    incompatibility, unavailability, commit ambiguity, and internal state.

    Cancellation and close are ordered against
    `SnapshotRetentionBatchApplying` under the registry lock. If cancellation
    wins, the repository is not called; if Applying wins, later cancellation
    remains visible intent and cannot rewrite the exact repository success or
    failure. A task removes at most one final, publishes one finished event and
    immutable result, and leaves any later retry to an explicit future native
    scheduler. Focused tests cover typed under-cap and one-victim results,
    event order, exact rescheduling, idle/deduplicated and cross-maintenance
    admission, cancellation on both sides of Applying, schema races, stable
    failure mapping, exclusive-marker release, and independently opened engine
    sessions sharing one store.

  - Physical-orphan reconciliation sub-checkpoint completed 2026-07-17: a
    separate sealed repository batch now removes at most one immutable DUX
    snapshot final whose absence from the exact snapshot-path catalog is
    re-proven under a current-schema database guard followed by the snapshot
    writer lease. The classifier is bounded independently of cap, pin,
    tombstone, and temp policy, fully validates every referenced physical-final
    catalog row, and selects only the deterministic first typed zero-reference
    name. A prior inventory or caller-supplied path never grants authority.

    Before mutation, the repository retains the observed identity and exact
    logical/allocation usage, fully decodes the bounded checksum-valid body,
    requires the decoded scan ID to derive the exact filename, and loads the
    exact parent. Its lossless root must match, its snapshot reference must be
    absent, and its status must be `running`, `failed`, `cancelled`, or
    `interrupted`; missing, queued, succeeded, referenced, duplicate,
    malformed, or root-conflicting evidence fails closed. A live publication
    cannot be in its file-first/database-commit interval while reconciliation
    owns the same writer lease. Active or unleased temps do not authorize or
    defer this distinct final, and no scan, temp-lease, tombstone, pin, or other
    history row is changed.

    The digest-validated read handle stays live while a separate delete-capable
    handle repeats name, identity, and usage checks. Count and charged-byte
    postconditions are checked before unlink; success requires snapshot-
    directory durability. The storage boundary now distinguishes a guaranteed
    pre-effect failure from post-unlink durability uncertainty, which maps to
    `OutcomeUnknown`. No tombstone is fabricated for an unreferenced final, and
    a later bounded inventory converges from physical state.

    `EngineHandle::start_snapshot_orphan_maintenance` exposes this as a
    separate typed, path-free, idle-only `SnapshotOrphanMaintenance` task. It
    accepts no filename, scan ID, root, path, cap, inventory, or victim; invokes
    exactly one repository batch; redacts the private scan identity; reports
    canonical time, bounded orphan counts and charged bytes, removed bytes, and
    `has_more`; linearizes cancellation/close with an Applying point of no
    return; and never self-enqueues. Focused tests cover deterministic
    one-at-a-time and below-cap removal, active-temp independence, every allowed
    terminal parent, invalid parent/body/name/root/reference evidence,
    duplicate/malformed catalog rows, exact accounting, pre/post-effect
    failures, event/result redaction, idle/deduplicated cross-maintenance
    admission, cancellation/close, schema/clock/panic failures, and two engine
    sessions racing the same orphan. Unix executes the unlink path. The Windows
    implementation consumes and closes the POSIX-disposition handle before
    directory sync; local MSVC cross-checking is blocked before Rust compilation
    by the bundled SQLite C build's missing Windows sysroot, so native Windows
    compile/runtime verification remains open.

  - Terminal snapshot-temp reconciliation sub-checkpoint completed 2026-07-17:
    `SnapshotRepository::reconcile_terminal_snapshot_temp_residual` now owns a
    separate one-row maintenance boundary for durable temp leases. Under the
    permanent database-before-snapshot lock order, it classifies the complete
    at-most-64-row immutable lease population through joined parent status and
    matches one bounded physical inventory. Those aggregate counts are
    observations only. Before any effect, the selected actionable row's exact
    fully decoded parent must be `failed`, `cancelled`, or `interrupted` with no
    snapshot reference. Active entries are skipped so they cannot starve later
    actionable debt, and at most the first deterministic row-only or quiescent
    residual is selected. Missing, queued, succeeded, running, malformed,
    snapshot-bearing, or conflicting selected parent/lease evidence grants no
    mutation. PID, owner, age, and a prior quiescent observation are never
    liveness or recovery authority.

    Row-only debt first durably confirms the locked snapshot-directory state,
    then consumes only the exact immutable lease row. Quiescent debt is reopened
    no-follow, identity- and exact-usage-matched, kernel-locked nonblockingly,
    and name/private-object revalidated. Count, active-count, and charged-byte
    postconditions are checked before a physical-first unlink; the delete handle
    is consumed and closed before directory sync, and only then may the exact
    row be deleted. Guaranteed pre-effect errors remain distinct from
    post-unlink `OutcomeUnknown`, and a later batch safely converges through a
    row-only residual. Running rows, unleased temps, provisioning stages, scans,
    finals, tombstones, pins, and unrelated history remain untouched.

    `EngineHandle::start_snapshot_terminal_temp_maintenance` exposes exactly one
    sealed repository call through a typed idle-only
    `SnapshotTerminalTempMaintenance` task. The public
    `SnapshotTerminalTempMaintenanceBatchApplying` and
    `SnapshotTerminalTempMaintenanceBatchFinished` events plus
    `SnapshotTerminalTempMaintenanceResult` retrieval discard scan, lease,
    owner, name, root, and path identity while reporting
    `NoTerminalResidual`, `DeferredActive`, `ReconciledRowOnly`, or
    `RemovedTemp { bytes }`, canonical time, bounded terminal/active counts,
    charged bytes, and `has_more`. Applying linearizes cancellation and close;
    core never loops or self-enqueues. Focused tests cover all three admissible
    terminal states, row-only and physical removal, active deferral without
    starvation, running and unleased exclusion, deterministic one-at-a-time
    accounting, identity revalidation and usage races, pre/post-effect failures,
    exact row-delete reconciliation, hostile rows/parents, schema and clock
    failures, redaction, every maintenance admission pair, cancellation/close,
    panic release, and two engine sessions. Structural module boundaries keep
    scans, finals, pins, candidates, and unrelated history outside this sealed
    authority. Unix exercises the unlink path. The MSVC Rust typecheck passes
    only when `libsqlite3-sys` is redirected through the host `pkg-config`
    bypass; the ordinary bundled-SQLite cross-build remains blocked by the
    missing Windows C sysroot, and native Windows mutation-path runtime
    verification remains open.

  - Unleased snapshot-temp reconciliation sub-checkpoint completed 2026-07-17:
    `SnapshotRepository::reconcile_unleased_snapshot_temp` now owns a separate
    physical-only, one-item maintenance boundary for recognized snapshot temps
    whose exact names are absent from the complete durable lease population.
    Authority is the full conjunction of the retained marker-owned private
    snapshot store, a current-schema database guard acquired before the
    snapshot writer lease, the complete at-most-64-row immutable lease
    population, one bounded physical inventory, the exact generated
    `.snapshot-<64 lowercase hex>.<canonical nonzero u32 PID>.<32 lowercase
    hex>.tmp` grammar, absence of that exact name from every lease row, and a
    fresh no-follow retained/name identity, exact logical/allocation usage,
    private-file, one-link, and second nonblocking kernel-lock proof. No
    individual fact is sufficient. In particular, prefix, embedded PID,
    process identity, age, mtime, and a prior quiescent observation never grant
    removal authority.

    Entries are ordered by exact ASCII name. One call skips active observations
    without letting them starve a later quiescent entry, then removes at most
    the first lexicographic quiescent item; an empty inventory returns
    `NoUnleasedTemp`, while active-only debt returns `DeferredActive` for an
    explicitly backoff-controlled later request. Counts and charged-byte
    postconditions are checked before the physical effect. The locked
    delete-capable handle is consumed and closed before snapshot-directory
    sync, including Windows POSIX disposition, so a known pre-effect failure is
    distinct from post-unlink durability `OutcomeUnknown`. Success updates the
    retained physical inventory only. The operation never adopts the file,
    inserts or deletes SQLite data, maps it to a scan, consumes a row-bound
    lease, settles a `running` parent, or touches finals or provisioning stages.

    Pre-v8/version-skew behavior is deliberately narrower than v8's retained
    kernel-lock protocol. A current-schema fence prevents an older writer from
    committing its publication after v8 has won. On Windows, pre-v8 writable
    temp handles were opened without delete sharing, so an actually live older
    writer also prevents the delete-capable reopen. On Unix, unlink can detach
    an older writer's still-open inode because advisory locking was not part of
    the pre-v8 contract; that older publication subsequently fails its
    name/current-schema revalidation and cannot create a final or history
    reference. Avoiding that same-user availability race depends on not
    concurrently running old and new DUX binaries against the same private
    store. The store's 0700/0600 or protected owner-only DACL boundary excludes
    other users, but does not claim protection from a malicious or
    incompatible process running as the same user.

    `EngineHandle::start_snapshot_unleased_temp_maintenance` exposes one sealed
    repository call through the typed idle-only
    `SnapshotUnleasedTempMaintenance` task. Its Applying/Finished events and
    result getter expose only canonical time, bounded unleased/active counts,
    charged bytes, `has_more`, and `NoUnleasedTemp`, `DeferredActive`, or
    `Removed { bytes }`; names, PIDs, identities, roots, paths, and any inferred
    scan mapping remain private. Applying is the cancellation point of no
    return, the task is mutually exclusive with foreground and every other
    maintenance kind within its engine session, and core never loops or
    self-enqueues. Independently opened sessions serialize at the retained
    repository locks.

    Focused repository/storage tests cover empty and deterministic
    one-at-a-time removal, exact accounting, active skip/deferral and later
    convergence, exclusion of row-bound and provisioning-stage debt, invalid
    clocks, effect uncertainty, exact-temp-only removal, and inventory
    selection. Typed engine tests cover path-free no-op/removal results,
    explicit one-item rescheduling, active deferral, idle and cross-maintenance
    admission, cancellation/close ordering, stable clock/schema/panic failure
    mapping, redaction, and independently opened sessions. Unix exercises the
    unlink path. Native Windows compile/runtime mutation-path verification
    remains open.

  - Snapshot provisioning topology correction completed 2026-07-17: new
    `.dux-snapshot-stage-<32 lowercase hex>` directories are created inside the
    retained marker-owned database root, beside `snapshots`, and published by
    same-parent atomic no-replace rename. The SQLite root validator tolerates
    only this exact private-directory grammar. The walk uses a 64-stage cap,
    fixed total-entry and 256-KiB aggregate-name budgets, and sampled elapsed-
    time checks against a 250-ms budget; a malformed name, unsafe
    type/permission/DACL/reparse shape, or 65th stage fails closed. Synchronized
    two-root contention coverage verifies each root retains only its own
    collision loser and leaves outer legacy stages untouched, while crash-
    reopen tests cover empty and marker-complete root-local remnants, including
    a current-user-owned Unix stage left at mode 000 between `mkdirat` and its
    exact-mode repair. Such a stricter-mode entry is tolerated only as opaque debt. This
    checkpoint adds no removal authority. Legacy external sibling stages have
    a globally fixed marker with no database identity and are never adopted or
    automatically removed. Future maintenance may consider only a canonical root-local stage
    with an exact ownership marker and bounded known child set; an empty,
    partial, malformed, linked, or extra-entry stage remains untouched, and no
    stage cleanup may recurse.

  - Root-local snapshot provisioning-stage reconciliation completed
    2026-07-17: `SnapshotRepository::reconcile_snapshot_provisioning_stage`
    holds the current-schema database guard across one complete bounded raw/
    native root inventory and accepts no root, name, path, identity, or victim
    from its caller. Only an exact canonical private stage whose complete child
    set is the 16-byte store marker alone or that marker plus the exact writer
    marker is marker-owned. Empty and stricter-than-0700 Unix crash remnants are
    unproven observations; they do not starve later proven debt. Wrong/partial
    markers, writer-only or extra children, links/reparse points, broader
    permissions/DACLs, and the 65th stage fail the whole batch before effect.
    Legacy outer stages are outside the retained root and remain manual debt.

    The storage boundary removes at most the first lexical proven stage and
    never recurses. It freezes checked before/after counts plus exact charged
    marker/writer bytes before mutation, then orders writer removal, stage sync,
    marker removal, stage sync, empty-directory removal, and root sync. Known
    pre-effect failures remain retryable; every failure after the first
    namespace effect is `OutcomeUnknown`. A marker-removal crash may leave an
    empty unproven directory that requires manual handling or a future durable
    deletion journal. The separate typed idle-admitted
    `SnapshotProvisioningStageMaintenance` engine task runs exactly one batch,
    exposes only canonical time, aggregate counts/bytes, `has_more`, and
    `NoStage`, `DeferredUnproven`, `RemovedMarkerOnly`, or
    `RemovedMarkerComplete`, and never self-enqueues. Applying linearizes
    cancellation and close. Focused hostile-shape, cap, accounting,
    effect-boundary, redaction, admission, cancellation/close, schema-race,
    panic-release, and two-engine convergence tests cover the slice. Unix runs
    the mutation path; Windows native tests are compiled for CI but have not run
    on this host.

  - Native review-lease ownership and idle-maintenance scheduling completed
    2026-07-17: FFI contract v3 replaces the smoke-only adapter with one real
    application-scoped `EngineHandle` opened from input-only data/cache roots.
    Opaque review sessions can be acquired only by a validated scan ID for the
    fixed Explorer purpose; no path, filename, digest, file handle, candidate,
    inventory, or cleanup authority crosses UniFFI. Core repeats snapshot,
    tombstone, history, identity, and body validation during acquisition and
    now linearizes the final publication against engine close: if close wins,
    exact pin release is attempted before `Closed` is returned, and any
    unresolved durable row expires naturally.

    Swift's actor-owned review controller retains leases outside render state,
    renews them every five minutes and on wake/significant time change, and
    explicitly releases them on selection close and app shutdown. Per-scan
    generations reject acquisition completions that arrive after release,
    prevent an old renewal failure from removing a replacement lease, and
    serialize/coalesce overlapping renewals. FFI additionally tracks weak
    issued-session registrations: engine close first rejects new renewal,
    attempts exact release for every still-live registered review, then closes
    core; a failed durable release expires naturally. Release remains
    idempotently available after close. Concurrent FFI close callers wait for
    and receive the same bounded five-second quiescence result.

    The same contract exposes six opaque, non-reconstructable maintenance-task
    kinds: history, retention, physical orphan, provisioning stage, terminal
    temp, and unleased temp. Start returns only Started, AlreadyActive, or
    DeferredBusy plus an opaque task; poll/cancel return versioned phases,
    stable path-free failure classes, and compact kind-tagged aggregates. Byte
    fields contain only bytes; the four history row counters separately report
    created daily rollups and pruned raw, daily, and AI rows. No FFI input can
    select a victim or broaden any sealed core maintenance authority.

    The menu-bar runtime lazily opens the real Rust engine on a dedicated
    utility queue so migration/lock waits cannot block launch. One actor
    scheduler starts after a 60-second grace, admits exactly one batch at a
    time, rechecks Low Power Mode and serious/critical thermal state before
    each admission, and fairly completes at most one batch of every eligible
    kind before the normal six-hour cycle delay. Kinds are separated by one
    minute; `has_more`, deterministic deferrals, busy admission, retryable
    failure, and energy denial use distinct bounded delays/backoffs. A blocked
    kind remains excluded until app restart. Activation, wake, and time-change
    signals coalesce but cannot shorten startup grace or resource/failure
    backoff; an energy-policy transition may advance only a deadline created by
    the energy gate itself.

    App termination is asynchronous and ordered: cancel the active maintenance
    task, release all review leases, then close the engine. Every concurrent
    shutdown caller awaits the same task, and duplicate AppKit termination
    requests cannot approve exit early. Scheduler stop also cancels and awaits
    its driver, so a suspended energy check, admission, or task poll cannot
    publish or enqueue cancellation after engine shutdown. Deterministic Swift
    clock/energy/task tests cover full-cycle cadence, wake coalescing, startup
    grace, every backoff class, forced suspension at both reentrant boundaries,
    cancellation, pending-acquire release, stale-renewal replacement,
    overlapping renewals, and the termination gate. Rust tests
    cover missing/running/succeeded/tombstoned/newer-schema review states,
    expiry/release, close-acquire linearization, FFI use after close, all six
    maintenance kinds, concurrent close, and exact pin drainage.

    This FFI-v3 checkpoint was superseded by the schema-v9/FFI-v4 running-scan
    recovery checkpoint below. Explicit clear-data actions, cross-reboot and
    legacy-v8 running-row policy, native Windows mutation-path qualification,
    and diagnostics for unattributable legacy snapshot stages are assigned to
    the M9 production-storage gates below; they are intentionally not automatic
    retention authority.

  - Same-scope hard-process-death running-scan recovery completed 2026-07-17:
    schema v9 adds a strict immutable `scan_process_claims` relation. Every new
    typed scan start commits its pristine `running` row and one exact
    process-instance claim atomically; every normal terminal path consumes only
    the same coordinator owner's validated format/scope/start tuple. Existing
    v8 running rows migrate without fabricated ownership and remain explicit
    legacy debt. Claims are capped at 64 per exact owner, with separate owner
    and reliable-scope indexes, so old boot/foreign owners neither make start
    admission unbounded nor create a global lockout.

    Recovery reads and completely validates one 64-row keyset page for the
    current reliable macOS/Linux boot/namespace scope, drops every SQLite and
    writer guard, and performs OS liveness probes. Only `DefinitelyGone` is a
    permit. The current-schema writer boundary is reacquired and one exact
    pristine claim/scan tuple is compare-and-set to `interrupted`; normal
    completion or another recoverer winning the race is reported without
    overwrite. `Alive` and `Unknown` are durable byte-for-byte no-ops. A
    malformed owner/scope/parent fails closed, newer-schema races prevent the
    write, commit ambiguity reconciles only the exact terminal facts, and page
    sentinels expose honest `has_more`. Reboot/foreign scope remains `Unknown`;
    Windows still cannot prove death without reliable host/boot scope.

    This transition changes history only and takes no cleanup lock. It never
    opens, selects, or removes a path, snapshot, temp, candidate, plan, or user
    data. Any exact snapshot-temp lease remains after the parent becomes
    `interrupted`, and the independently sealed terminal-temp batch owns later
    physical-first reconciliation. The engine exposes one idle-only sealed
    `ScanRecoveryMaintenance` batch with Applying cancellation linearization,
    path-free counts/outcomes, deduplication, cross-maintenance exclusion, and
    no self-enqueue behavior.

    FFI contract v4 adds that task without exposing process identities, paths,
    scan IDs, or authority-bearing inputs. Swift expands the native rotation to
    seven kinds and deliberately requests scan recovery before terminal-temp
    reconciliation while retaining the existing startup grace, energy gates,
    cadence, backoffs, and shutdown ordering. Migration/checksum/fingerprint,
    exact owner completion, hostile scope, index-plan/keyset page fairness,
    live/unknown no-op, scope-replacement, normal-completion and two-recoverer
    races, schema race, post-commit
    reconciliation, real graceful/SIGKILL child death, and temp-debt convergence
    regressions cover the boundary. Verification passed the 690-test core
    library (plus one ignored platform fixture), full locked workspace, FFI,
    linked 35-test Swift suite, formatting, workspace/fuzz/MSRV lint and check,
    destructive-call policy, and script-policy suites. Fresh generated bindings
    were byte-identical across Debug and Release; unsigned universal arm64 and
    x86_64 Debug/Release apps built and both deployment-target checks reported
    macOS 14.0. Windows and Linux Rust cross-checks were attempted but this
    macOS host lacks the C cross-compilers required by bundled SQLite
    (`stdlib.h` for MSVC and `x86_64-linux-gnu-gcc` respectively); host tests and
    the existing target-specific compile fixtures cover the changed liveness
    branches without claiming those environment-blocked checks passed.

  - M2 automatic-retention scope completed 2026-07-17: bounded raw/daily
    capacity and expired-AI pruning, latest-two/pin/cap snapshot policy,
    one-item cap enforcement, orphan/temp/proven-stage reconciliation,
    same-scope abandoned-scan recovery, opaque FFI tasks, and the native
    seven-kind scheduler are implemented. User-directed data clearing remains
    a separately confirmed M9 settings boundary. Cross-reboot, legacy-v8,
    unattributable-stage, and Windows qualification work remains explicit M9
    release debt and cannot be used to infer liveness or broaden removal.

  - M2 closure verification completed 2026-07-17: 92 consecutive isolated-HOME
    durable scan/reopen runs passed after the conservative macOS unscoped-owner
    fallback. The full workspace passed with all 734 runnable core tests plus
    one intentionally ignored subprocess helper, all 15 FFI tests, and all CLI
    integration tests. The destructive-boundary checker scanned 163 source
    files and its 20 tests passed; the linked macOS suite passed all 105 tests.
    Debug and Release bindings were byte-identical, and unsigned universal
    arm64/x86_64 Debug and Release apps both target macOS 14.0.
- [x] Add engine integration tests with temporary HOME and database. Completed
  2026-07-16: an actual `dux-core` engine scans a fixture into an isolated
  platform-correct application-support/cache layout, closes to full worker
  quiescence, and separately launched `dux status --json` and
  `dux history --json --limit 1` processes reopen that database and report the
  same succeeded scan and snapshot reference. A separate completely fresh-HOME
  process proves the adapter prepares missing standard data/cache parents while
  the core still owns private-store validation and publication. The subprocess
  regressions also prove every emitted object is versioned, forbidden
  scan/storage keys remain absent recursively, stdout has no terminal controls,
  stderr stays empty on success, and invalid limits exit 2 without entering the
  TUI or writing stdout. Existing engine tests independently cover restart,
  corrupt selected rows, the full 200-by-256 coverage bound, newer-schema
  read-only behavior, and deterministic same-time order.

Exit criteria:

- A scan creates a durable summary and snapshot.
- History queries survive process restart.
- Corrupt/incompatible snapshots fail safely.
- CLI can print versioned JSON status and history.

### Milestone 3: Native menu bar MVP

Goal: ship a useful read-only disk-pressure companion.

Tasks:

- [x] Build `AppModel`, `EngineService`, and `VolumeMonitor`. Completed
  2026-07-17: the native runtime now owns one `@MainActor` render model, a
  retryable utility-queue engine service, and a Foundation startup-volume
  monitor independently of menu/popover/window lifetime. Engine smoke state was
  replaced by a versioned production status and UniFFI-v5 startup-volume call.
  Volume refreshes are single-flight and generation-fenced: uncached work shows
  loading, cached work remains visible while refreshing, failures preserve the
  last good sample as explicitly stale, cancellation restores the prior state,
  and late/superseded completions cannot publish. Engine construction remains
  lazy, failures are not permanently cached, all blocking Foundation/Rust work
  stays off the main actor, and shutdown invalidates capacity publication before
  stopping maintenance, releasing reviews, and closing the engine. Ordinary
  filesystem availability remains optional in the UI; important-only capacity
  never fabricates used bytes or a usage percentage. Launch/wake/mount routing,
  retry, coalescing, stale-state, cancellation, real-FFI, important-only, and
  ordered-shutdown regressions cover the composition. The callback-to-
  `AsyncStream` part of the general engine architecture is deliberately deferred
  to Milestone 4's first progressive scan event source; capacity is a bounded
  request/response operation and maintenance already has a checked typed polling
  abstraction, so this slice introduces no synthetic callback stream.
- [x] Implement capacity sampling and pressure hysteresis. Completed 2026-07-17:
  a dedicated actor samples immediately at app launch, every five minutes, on
  wake, and after mount/unmount/rename notifications. Exactly one sample may run
  at a time; event bursts during a sample become one immediate follow-up, idle
  events advance the cadence deadline, and stop cancels the driver plus
  generation-invalidates an uninterruptible Foundation resource query. Wake
  routing signals this priority status path before review/maintenance renewal.

  Rust owns the deterministic policy: important-use capacity is preferred,
  ordinary availability is the explicit fallback, Critical enters at or below
  `min(10 GiB, 5%)`, Warning enters at or below `min(30 GiB, 10%)`, escalation is
  immediate, and recovery must clear the applicable boundary by both 2 GiB and
  one percentage point. Integer/u128 arithmetic makes boundaries and maximum-
  capacity behavior deterministic without floating point.

  One direct engine call serializes a single bounded, timestamped session
  baseline, then holds the existing connection/current-schema/cross-process
  writer boundary and one immediate SQLite transaction while it loads the
  latest durable pressure, selects the newest valid session/durable baseline,
  evaluates hysteresis, selects routine versus transition admission against
  durable history, writes, commits, revalidates, and reconciles an ambiguous
  commit. Stale or same-time-conflicting session observations fail before state
  changes; a newer cross-process durable sample wins inside the final guard.
  Routine history remains capped at one raw sample per UTC hour while real
  Healthy/Warning/Critical transitions persist immediately. The versioned input
  and returned v1 status are path-free; status exposes the headline source,
  effective boundaries, prior durable pressure, and an explicit history
  disposition. A missing UUID, incomplete optional metadata, or important-only
  observation is still classified with session hysteresis for display but cannot
  create or advance either persistence table; no stable identity is synthesized
  from `/` or mutable labels.

  Boundary, hysteresis, monotonicity, maximum-integer, reopen, hourly cadence,
  transition, exact retry/conflict, stale observation, corrupt-state,
  cross-writer, important-only no-write, schema fencing, and post-commit
  reconciliation tests cover the core/store/FFI path. A separate-engine
  regression also proves that durable `last_seen` rejects differing or
  unverifiable equal-time retries after hourly suppression and supersedes older
  ephemeral observations rather than losing their ordering across restart.
  Fifty-seven linked Swift
  tests cover the real universal FFI call, retryable engine open, lifecycle event
  routing, wake priority, full shutdown ordering, scheduler
  coalescing/cancellation, version rejection, history-evidence validation,
  honest important-only and localized pressure accessibility presentation, and
  the normal 500 ms median capacity-sample budget. The UI shows effective
  available bytes and percent; there is no one-second timer or directory scan in
  the idle capacity path.
- [x] Implement persistent user-configurable pressure thresholds as a separate
  schema-v10 / FFI-v6 checkpoint. Completed 2026-07-17: `DiskPressureConfig`
  remains the sole
  validator and evaluator; expose typed versioned get/set/reset engine methods
  rather than raw settings keys, JSON, UserDefaults, or a policy supplied with
  each observation. Store exact canonical schema-v1 JSON at
  `disk_pressure_policy`; absence means the current defaults at revision 0,
  while every real set/reset advances a checked monotonic revision. An explicit
  custom value equal to the defaults must preserve Stored provenance, and reset
  must retain a new Default epoch rather than deleting the row. Add the policy
  revision to raw samples and daily representatives, treating migrated rows as
  default revision 0. A revision change resets old-policy hysteresis and forces
  one `PolicyBaseline` sample even when pressure and UTC hour are unchanged;
  historical classifications remain immutable.

  Load the effective policy and select the revision-matched durable/session
  baseline inside the capacity observation's final guarded transaction so a
  concurrent process cannot change policy between evaluation and persistence.
  Never acquire the capacity-session mutex from a settings write (the observation
  order is session → store). FFI v6 must add strict record-v1 policy input,
  policy/update DTOs, source/revision, and typed invalid-policy failures. Keep
  the existing startup observation/status records at v1 and map a forced policy
  baseline to the existing Stored disposition unless a separately named v2
  status is genuinely required; do not silently add fields to a v1 record.
  Swift Settings must edit exact decimal GiB and percentage values without
  silently rounding arbitrary stored policy, explain that each boundary is the
  smaller of bytes and percent, show validation/save/reset state accessibly, and trigger exactly
  one generation-safe resample after a successful change. The status UI must
  always show effective available bytes and percentage beside the label.

  The completed regression set covers missing-row defaults without a write, explicit-default
  provenance, reopen, invalid/malformed/newer schemas, idempotency, revision
  overflow, ambiguous commit, cross-process serialization, v9→v10 backfill,
  policy-change baseline insertion and subsequent cadence, revision-matched
  ephemeral hysteresis, rollup retention, FFI v6 version/error/get-set-reset,
  linked off-main Swift round trips, save-failure last-good state, one resample,
  and stable Settings accessibility identifiers/hints. A deterministic
  child-process test commits a new
  policy while retaining the cross-process writer lease, proves a concurrent
  observation cannot finish, then verifies its stored Warning/revision-1 tuple.
  Linked Swift tests exhaust all 1...10,000 basis-point round trips, exercise
  exact GiB conversion through `UInt64.max`, use the real Rust get/set/reset
  boundary off-main, preserve last-good state and attempted input on failure,
  and prove changed-only, cancellation-safe resampling. Universal Debug/Release
  verification is recorded with this checkpoint's commit. This preference
  changes classification only and grants no cleanup, notification, scan, or
  scheduling authority.
- [x] Implement menu bar label modes. Completed 2026-07-17: the status item now
  supports Icon only, Icon and free space (GiB), and Icon and free space (%)
  modes, with free GiB as the useful first-run default. One validated,
  versioned Swift-owned UserDefaults preference lives in the shared `AppModel`,
  updates immediately from Settings, preserves every stable raw mode across
  launch, and treats missing, corrupt, or future values as the default without
  rewriting them. This presentation-only preference never enters Rust, SQLite,
  FFI, sampling, scanning, or cleanup authority.

  The label consumes only the existing cached startup-volume state. It uses
  important-use availability when Foundation supplies it and the already
  disclosed filesystem fallback otherwise; it starts no work. Exact integer
  formatting floors to tenths of binary GiB or percent so the compact value
  cannot round free capacity upward, handles `UInt64.max` without overflow, and
  localizes the decimal separator. Loading/failure states invent no zero value,
  while refreshing and stale states retain the last measurement. Monochrome,
  shape-distinct Healthy/Warning/Critical/Unknown symbols remain legible across
  menu-bar appearances without relying on color. Every mode, including Icon
  only, exposes a stable VoiceOver identifier and a localized spoken summary of
  pressure, GiB, percent, availability basis, and freshness. Focused linked
  tests cover all modes and render states, exact boundaries/locales/integer
  extremes, system-symbol availability, preference default/round-trip/future
  fallback, one-write AppModel propagation, and accessibility contracts. The
  full 82-test linked Swift suite passes, and unsigned universal Debug and
  Release apps contain both arm64 and x86_64 at the macOS 14 deployment target.
- [x] Implement conditional menu bar visibility. Completed 2026-07-17: Settings
  keeps **Always visible** as the first-run default and offers the opt-in
  **Only when free space is low** mode with an exact whole-percentage threshold
  from 1 through 100, defaulting to 10%. One versioned Swift-owned UserDefaults
  value stores the validated mode and threshold together. Missing, malformed,
  out-of-range, and future values fail back to the default in memory without
  rewriting unknown data. No derived insertion, pressure, or reveal state is
  persisted.

  The shared `AppModel` derives `MenuBarExtra` insertion only from that
  preference and the existing cached effective startup-volume capacity. It
  starts neither a capacity query nor a directory scan and never crosses FFI,
  SQLite, or Rust pressure-policy boundaries. Entry is immediate and inclusive
  at the configured percentage. Recovery hides only at one full percentage
  point above it, using overflow-safe basis-point integer arithmetic; a 100%
  threshold remains visible. Refreshing and stale states use their last cached
  measurement, while missing, loading, failed, and otherwise unknown capacity
  remain visible so uncertainty cannot remove the user's control surface.
  Launch at Login uses the same policy without opening Explorer.

  Reopening the running app from Finder, Spotlight, or `open` reveals the item
  for the remainder of that process session without changing the stored
  preference. This provides a tested escape hatch if a healthy disk hides the
  item. Settings explains the cache-only behavior and exposes stable VoiceOver
  identifiers for both controls. Fifteen focused linked tests cover preference
  compatibility, exact arithmetic and boundaries, threshold entry, hysteresis,
  cached refresh/stale behavior, unknown fail-open behavior, no-work preference
  changes, the session reveal override, and delegate routing. The complete
  170-test linked Swift suite, full Rust format/lint/workspace-test gates,
  20 destructive-boundary checker tests, and 175-source authority scan pass.
  XcodeGen is deterministic, Debug/Release Swift bindings are byte-identical,
  and unsigned universal arm64/x86_64 Debug and Release apps target macOS 14.
  This remains presentation-only and grants no sampling, scan, notification,
  scheduling, AI, plan, or cleanup authority.
- [x] Implement popover layout with cached status and scan progress. Completed
  2026-07-17: the menu-bar popover renders the cached startup-volume identity,
  Rust-owned pressure classification, effective available and total capacity,
  percentage, availability basis, and freshness without starting a scan or
  fabricating missing data. Refresh keeps the last confirmed measurement and
  marks it stale on failure. Scan now explicitly starts one shared Home scan;
  reopening the popover does not start or duplicate work, cancellation remains
  visible until Rust reports a terminal outcome, late success is represented
  honestly, and the last successful result remains available through retries,
  cancellation, and failures. Stable keyboard shortcuts and VoiceOver
  identifiers cover every action and status region.

  UniFFI contract v7 exposes a bounded read-only discovery request and an opaque
  task whose polls/results contain only aggregate path-free progress and counts.
  Only the exact same canonical root can coalesce; non-equivalent overlapping
  roots return typed Busy rather than attaching a broader request to narrower
  work. The boundary grants no path detail, candidate, plan, approval, AI, or
  cleanup authority. Blocking FFI calls stay on the engine utility queue, Swift
  validates monotonic snapshots and generation-fences callbacks, and shutdown
  requests scan cancellation before closing maintenance, review, and engine
  state. The complete 105-test linked Swift suite, the full Rust workspace test
  and lint gates (including 735 core tests), the 20-test destructive-boundary
  checker suite, and unsigned universal arm64/x86_64 Debug and Release builds at
  the macOS 14 deployment target pass.

  Corrected 2026-07-18: the application now holds an explicit AppKit automatic-
  termination lease from launch through ordered shutdown. Closing the popover
  therefore cannot make this `LSUIElement` menu-bar process eligible for
  retirement merely because it has no visible ordinary windows; explicit Quit
  remains the only normal termination path. A follow-up lifecycle hardening
  slice reasserts the lease after SwiftUI restores the transient scene and at
  AppKit's last-window/termination callbacks, because scene restoration can
  otherwise re-enable automatic termination after the initial launch lease.
  Corrected 2026-07-19: the lifetime guard enables AppKit automatic-termination
  support before taking one process-lifetime opt-out, because Apple's contract
  makes the counter ineffective when support is disabled. Close/resignation
  callbacks only restore the support flag across the AppKit scene teardown
  turn; they never acquire additional counter leases. This prevents the
  process from becoming TAL-eligible after the popover closes while keeping
  explicit Quit as the only normal termination path. Delegate tests cover
  launch, scene restoration, last-window close, incidental termination, and
  repeated acquire/reassert balancing.
  Corrected 2026-07-19 (bundle hardening): both Debug and Release targets now
  use an explicit app plist with `NSSupportsAutomaticTermination=false`, so
  closing the transient menu-bar scene cannot retire the agent app before the
  delegate's runtime lease is installed. The generated plist and source-level
  lifecycle tests must remain part of every release verification pass.
- [x] Implement Explorer window shell and Overview. Completed 2026-07-17: one
  reusable `Window(id: "explorer")` opens after the menu-bar scene, shares the
  process-wide `AppModel`, activates only after the open request, and does not
  auto-scan or create another engine. Closing it leaves the `LSUIElement` app
  running. Its `NavigationSplitView` exposes the honest Overview surface and a
  working Settings shortcut without advertising unimplemented destinations.

  Overview separates startup-disk capacity from Home-scan allocation. The
  accessible segmented chart derives its used/filesystem-free composition only
  when ordinary filesystem availability exists; important-use availability is
  reported separately and unknown values are never rendered as zero. Loading,
  refresh, stale, failure, retry, pressure, basis, and freshness states remain
  explicit. The Home card reports only the current in-session path-free
  aggregate, scopes every coverage label to Home, renders a percentage only
  when Rust supplied one, and retains the last confirmed coverage while a new
  scan is active, cancelled, or failed. Scan and cancel are mutually exclusive,
  with Command-R, Command-period, and Command-comma shortcuts and stable
  VoiceOver identifiers.

  This slice requires no FFI revision and grants no path, durable-history,
  snapshot-review, candidate, plan, AI, or cleanup capability. Review-lease
  paging and drill-down remain Milestone 4; recommendations and reclaimable
  groups remain Milestone 5; trends remain Milestone 6; permission guidance is
  the separate Milestone 3 onboarding task. Pure presentation tests cover
  missing/basis-sensitive capacity, stale retention, scoped coverage, retained
  results, action exclusivity, accessibility, shortcuts, and singleton window
  activation ordering. The complete 123-test linked Swift suite, full Rust
  format/lint/test gates, the 20-test destructive-boundary checker and
  167-source scan, byte-identical Debug/Release bindings, and unsigned universal
  arm64/x86_64 Debug and Release builds at the macOS 14 deployment target pass.
- [x] Implement launch-at-login setting. Completed 2026-07-17: Settings now
  exposes one opt-in `SMAppService.mainApp` control whose source of truth is the
  current macOS Login Items status, never UserDefaults. The shared `AppModel`
  retains the last confirmed state while loading or changing, serializes
  operations, re-reads status after every register/unregister attempt, and
  normalizes already-settled races without claiming an unconfirmed outcome.
  Enabled, not registered, approval required, service not found, and unknown
  future statuses have distinct accessible text. Approval-required remains
  visibly registered but blocked and links to the exact System Settings pane;
  unavailable/unknown states remain disabled and retryable. Settings refreshes
  on appearance and after returning active, but the app never registers during
  startup or first run.

  The production actor privately owns `SMAppService.mainApp`; render state and
  errors are app-owned, bounded, and path-free. Launching remains an
  `LSUIElement` menu-bar startup and adds no helper, daemon, plist, entitlement,
  privilege, TCC access, engine/FFI call, scan, AI, or cleanup capability. The
  17-test focused suite covers all known Apple statuses, typed errors, honest
  presentation, postcondition mismatch, approval/duplicate error races,
  external changes, single-flight operations, caller cancellation,
  accessibility identifiers, the System Settings action seam, and a production
  read-only status query without ever registering the temporary bundle
  identifier. The complete 140-test linked Swift suite, full Rust
  format/lint/test gates, 20 destructive-boundary checker tests and 170-source
  boundary scan pass. Debug and Release bindings are byte-identical, and
  unsigned universal arm64/x86_64 Debug and Release apps target macOS 14.
  Production identity was subsequently frozen on 2026-08-08; signed stable-
  install and sign-in-cycle validation remains an explicit Milestone 9 gate.
- [x] Implement notification permission UI but do not notify repeatedly.
  Completed 2026-07-17: Settings now reads and presents macOS's authoritative
  notification authorization state, distinguishes not requested, denied,
  allowed, provisional/quiet, and unknown future states, and refreshes after
  returning active. Authorization is requested only from an explicit **Allow
  notifications…** button while the last confirmed state is Not Determined;
  opening Settings, app activation, startup, and ordinary refreshes never show
  the system prompt. Denied and unknown states remain truthfully retryable, and
  determined states provide honest guidance to System Settings without relying
  on an undocumented per-app deep link.

  The actor-owned service rechecks Not Determined before requesting only Alert
  and Sound authorization. Its deliberately narrow protocol can read status or
  request permission but cannot construct, add, deliver, remove, or respond to
  a notification. `AppModel` retains the last confirmed status during work,
  coalesces duplicate calls, survives caller cancellation, re-reads macOS after
  every request or error, treats Denied as a settled user choice, and never
  persists competing permission truth. No notification-center delegate,
  notification content, pressure episode, cooldown, deep link, scan, AI, plan,
  scheduling, or cleanup authority was added. Transition-based delivery, the
  24-hour per-level cooldown, and Recommendations deep links remain explicitly
  owned by Milestone 6.

  Nineteen focused linked tests cover every known authorization status, typed
  domain-gated errors, exact request options, bounded/localized presentation,
  stable accessibility actions, explicit-only requests, settled/error races,
  external changes, duplicate requests, and caller cancellation. One production
  integration test reads the current status only and never requests permission.
  The complete 159-test linked Swift suite, full Rust format/lint/workspace-test
  gates, 20 destructive-boundary checker tests, and 173-source authority scan
  pass. XcodeGen is deterministic, Debug/Release Swift bindings are
  byte-identical, and unsigned universal arm64/x86_64 Debug and Release apps
  target macOS 14.
- [x] Implement permission/coverage onboarding. Completed 2026-07-17: the
  menu-bar popover gives a truthful, one-time local-analysis introduction that
  persists only the acknowledgement and starts no scan, probe, capacity sample,
  FFI call, or permission prompt. Explorer continues to report the actual Home
  scan's typed coverage. Only a measured incomplete or uncertain result offers
  **Understand broader access…**; the app remains useful with the files it can
  already read and never presents Full Disk Access as a first-run requirement.

  Broader-access discovery is explicit and bounded to directory-open checks for
  three fixed user-library locations. It never enumerates names, reads file
  contents, returns paths to presentation state, or claims that Full Disk Access
  is enabled. Because macOS exposes no authoritative public status query, the UI
  reports only observed readable, unreadable, and unobserved counts. Optional
  System Settings guidance uses a stable generic app-opening action rather than
  an undocumented deep link. A return-to-app check runs exactly once only after
  that action was used; updating scan coverage still requires an explicit new
  Home scan.

  Probe work is off-main, single-flight, survives caller cancellation, retains
  the last evidence through refreshes and failures, and is generation-fenced at
  shutdown. Eleven focused onboarding tests plus activation integration bring
  the complete linked Swift suite to 183 tests. Full Rust and repository gates,
  source-authority counts, deterministic project generation, binding parity,
  and universal Debug/Release build results are recorded in the changelog for
  this checkpoint.
- [x] Add signed/notarized local release script. Completed 2026-07-17: the
  fail-closed local workflow requires a clean exact `vX.Y.Z` tag, matching DUX
  workspace versions, a non-placeholder production bundle identifier, positive
  build number, exact Developer ID Application identity and Team ID, and a
  Keychain-backed `notarytool` profile. It accepts no password, Apple ID, or API
  private-key path and refuses an existing versioned output directory.

  The script runs the Rust, linked Swift, destructive-boundary, generated-file,
  matching Debug/Release layout, and universal deployment gates before signing.
  It permits only the reviewed empty release-entitlement dictionary, rejects
  unreviewed nested code bundles,
  signs known nested Mach-O code before the outer app without recursive signing,
  and verifies exact arm64/x86_64 architecture, macOS 14, bundle identity, Team
  ID, Developer ID authority, secure timestamp, Hardened Runtime, and absent App
  Sandbox/debug entitlements. It notarizes the app ZIP, retains and validates
  Apple's log, staples and Gatekeeper-assesses the app, then creates a DMG with
  an exact `/Applications` link and repeats signing, notarization, stapling,
  integrity, mount-layout, signature, and Gatekeeper verification for the final
  image.

  Same-filesystem private staging guarantees successful output is atomically
  published under a new immutable `target/dux-macos-release/vX.Y.Z` directory
  with sanitized submission records,
  complete Apple logs, a manifest, and SHA-256 sidecar. Failed work is retained
  at the reported private staging path for diagnosis. This slice deliberately
  did not invent a production identity; that identity was subsequently frozen
  on 2026-08-08, while the first full Developer ID/notary app run remains a
  Milestone 9 release prerequisite.

Exit criteria:

- App starts at login when enabled.
- Menu bar displays accurate important available capacity without a scan.
- Popover opens Explorer reliably.
- Read-only app is useful with no AI and no Full Disk Access.
- Idle resource usage meets the initial budget.

### Milestone 4: Visual Explorer

Goal: reach feature parity with CLI navigation and materially improve clarity.

Tasks:

- [x] Expose last-complete/recent snapshot selection, paged nodes, candidate
  paths/evidence, and treemap-budget APIs over FFI under review leases.
  - [x] 2026-07-17 slice: expose a bounded newest-first recent-scan selection
    page through FFI contract v8 and the off-main Swift adapter. Records are
    path-free and only advertise whether a durable snapshot reference was
    recorded; exact availability remains gated by review-lease acquisition.
    App-owned models identify the newest candidate in the returned page and
    reject malformed or authority-shaped responses.
  - [x] 2026-07-17 slice: acquire the exact newest complete, non-tombstoned
    snapshot through FFI contract v9 without trusting the history hint. The
    core chooses a deterministic scan, repeats exact repository validation
    while pinning its review lease, and falls back to the next available
    snapshot after retention tombstones. The native controller validates the
    returned scan identity, retains and renews the lease outside SwiftUI render
    state, generation-fences concurrent latest requests, and releases stale
    handles. Paged nodes, candidate details, and treemap budgets remain in this
    task.
  - [x] 2026-07-17 slice: expose the retained snapshot root and deterministic
    direct-child pages through FFI contract v10 and app-owned Swift models.
    Pages are capped at 200, support name/logical/allocated/modified ordering,
    retain lossless historical host bytes plus display text, and omit live
    filesystem identity and every cleanup capability. Each request revalidates
    the exact durable pin and retained immutable file; decoding is cached only
    while the review remains live, uses a compact child index, and is separately
    capped at two trees within a conservative 1 GiB decoded-memory admission
    estimate per engine. Direct-child sorting is budget-gated above 100,000
    entries until the measured million-node latency work lands. The native
    controller generation-fences results by
    scan ID and immediately evicts expired leases.
    Candidate paths/evidence remain in this task.
  - [x] 2026-07-17 slice: add FFI contract v11's lease-bound logical-size
    treemap budget. Rust returns no more than 64 positive-size direct-child
    cells in the same deterministic logical ordering as node pages, with
    contiguous ranks and exact Other counts/bytes including zero-size children;
    the app requests 48. Invalid budgets, oversized fan-out, expiry, missing
    nodes, and non-directory parents stay typed. Swift validates hostile
    versions, ranks, identities, counts, and overflow before publishing an
    app-owned projection. No live path, category, candidate, plan, AI input, or
    cleanup capability crosses.
  - [x] 2026-07-18 slice: expose bounded candidate summary, path, and typed
    evidence pages through FFI contract v17 on the exact retained snapshot
    review. The summary page lets Explorer enumerate candidates before
    drilling into a selected candidate.
    Rust maps durable candidate observations without granting planning or
    cleanup authority; Swift validates record versions, candidate identity,
    paging cursors, counts, optional-field shapes, timestamps, path-byte
    bounds, and hostile enum values before publishing display-only models.
    Expiry, invalid limits, missing evaluations, missing candidates, query
    budgets, and closed engines remain typed failures. Candidate detail is
    historical disclosure only and is never accepted as a planner or executor
    input.
- [x] Add progressive scan events.
  - [x] 2026-07-18 slice: expose the engine's bounded, sequenced task-event
    pages in FFI contract v18 alongside the existing aggregate scan poll. Each
    event is path-free and typed (queue/start/progress/finalizing/candidate
    evaluation/cancellation/terminal), while internal maintenance activity is
    explicitly coalesced into a non-authoritative maintenance event. The poll
    carries the last-delivered event cursor, oldest retained sequence, and
    sticky truncation bit; Swift rejects version, ordering, cursor, terminal,
    and truncation contradictions before publishing an app-owned model. The
    macOS scan driver retains only the latest 64 observations per generation,
    resets them for a new scan, and keeps them visible through terminal results;
    no UI string, path, plan, AI input, or cleanup authority crosses this
    boundary.
- [x] Implement treemap, synchronized list, breadcrumbs, local snapshot/history
  drill-down, history navigation, and inspector.
  - [x] 2026-07-17 slice: add the first lease-backed Latest Snapshot Browser to
    the macOS Explorer. Opening the destination atomically acquires the
    core-selected newest review, reads its root, and publishes one bounded
    100-row direct-child page. The table exposes truthful logical/allocated
    sizes, item counts, modification time, scan warnings, and proportional
    share bars; server-side sorting, previous/next page replacement, folder
    drill-down, and ID-backed breadcrumbs stay generation-fenced. Leaving the
    destination and app shutdown release the exact review, including late
    acquisitions. Historical display names never become live paths or cleanup
    authority. History selection and measured large-snapshot budgets remain in
    this task.
  - [x] 2026-07-17 slice: render the bounded projection as a deterministic
    logical-size treemap synchronized with the existing table by node ID and
    logical rank. Selecting an off-page cell loads its exact 100-row logical
    page; selecting an omitted table row highlights Other without treating the
    aggregate as a node. A historical-only inspector exposes name, kind, sizes,
    counts, timestamps, and scan warnings, while the table remains the textual
    and accessibility fallback. Treemap failure preserves a confirmed list and
    review expiry invalidates both. Finder, Quick Look, copy-path, category,
    candidate, reclaimability, AI, and cleanup actions remain deferred.
  - [x] 2026-07-17 slice: add a bounded Recent Scans chooser to the Snapshot
    Explorer. Presentation loads at most the newest 50 path-free history rows
    independently from the latest review, shows every lifecycle state, and
    discloses when older rows are omitted. Only succeeded rows with recorded
    snapshot evidence can be requested, and that evidence remains advisory:
    selection acquires the exact scan-bound review before loading its root,
    first page, and treemap. The prior confirmed snapshot stays visible until
    the replacement is fully validated, then its lease is released. Missing or
    expired historical snapshots are marked unavailable only for the session;
    refresh clears that hint, and failed history refreshes preserve both the
    active review and last confirmed list. Close and late-acquisition races
    release both old and target ownership without publishing stale content.
- [x] Implement Large Files.
  - [x] 2026-07-17 slice: add FFI contract v12's lease-backed, path-free Large
    Files projection and native segmented view. The app lazily requests the 100
    largest matching file observations, defaults to a configurable 1 GiB size
    threshold, supports strict modification-age presets, renders bounded
    historical parent context and exact aggregate/truncation disclosure, and
    keeps the existing historical inspector. Rust uses O(k) memory with a hard
    cap of 200 and post-query lease validation; Swift strictly validates record
    shapes, ordering, totals, contexts, filter semantics, and snapshot/mode
    races. Type/root/candidate filters and deterministic semantic labels remain
    follow-up work. No live path, preselection, AI, plan, or cleanup authority
    is introduced.
- [x] Implement scan coverage details.
  - [x] 2026-07-17 slice: add FFI contract v13's exact, paged durable-history
    coverage endpoint and a native Coverage view. The endpoint remains useful
    for failed, cancelled, interrupted, legacy, and snapshot-pruned scans; it
    does not acquire or depend on a snapshot lease. Rust fully validates the
    immutable scan record before returning at most 64 of the canonical 256
    issue records, with exact record/occurrence totals, zero-based ordinals,
    all 13 semantic issue kinds, and no absolute root. Locations are historical
    display observations only: Global, Scan root, or at most the nearest eight
    root-relative components with explicit truncation. Swift loads all pages
    off-main, rejects contradictory versions, totals, coverage states,
    ordinals, scopes, locations, and pagination, and generation-fences scan and
    mode changes. The UI distinguishes Unknown from measured zero, renders an
    accessible measured-coverage bar only when a measurement exists, explains
    exact limitations and historical context, and grants no live path, access
    state, reclaimability, AI, planning, or cleanup authority.
- [x] Implement reveal, copy path, and Quick Look.
  - [x] 2026-07-18 slice: add FFI contract v14's purpose-bound current-item
    resolution under the exact retained snapshot review. Callers supply only a
    node ID and Reveal, Copy Path, or Quick Look purpose—never a path. Rust
    reconstructs the lossless path from the validated immutable graph, then
    uses descriptor-relative no-follow traversal to match the recorded Unix
    device/inode and kind for the root, every ancestor, and the target before
    revalidating the lease. Missing identities, replacements, moves, symlinks,
    special files, cross-volume paths, access failures, and unsupported action
    shapes fail closed with path-free typed errors. Swift validates all echoes
    and absolute-path bytes, generation-fences selection/mode/snapshot/close
    races, and invokes injected Finder, pasteboard, or app-global Quick Look
    adapters only after a fresh match. Non-Unicode paths are rejected because
    Swift Foundation cannot round-trip them through a path-based URL without
    changing the bytes. These are ephemeral,
    read-only conveniences: path-based macOS APIs leave a documented
    post-validation same-user TOCTOU window and no result can enter candidate,
    AI, planning, or cleanup authority.
- [x] Add category colors and accessible text alternatives.
  - [x] 2026-07-18 slice: add FFI contract v15's display-only historical
    storage classification to every bounded snapshot-node projection. Rust
    uses only the exact scan's immutable validated candidate evaluation,
    applies the nearest classified ancestor, and fails closed to Unclassified
    for absent, conflicting, non-Unicode, or over-budget evidence. The optional
    join is capped at 4,096 roots and 1 MiB of exact path payload, indexed for
    ancestor lookup, and cleared with the review lifecycle. No candidate ID, path, evidence,
    status, safety, action, reclaimability, plan, AI result, or cleanup witness
    crosses this boundary. The native treemap colors represented cells by a
    stable app-owned palette while retaining kind icons, category glyphs,
    visible legend text, a non-color selection mark, category columns in both
    tables, inspector disclosure, and complete VoiceOver summaries. Other is
    neutral and truthfully states that category details are not summarized for
    the omitted items; their individual categories remain available in the
    complete table.
- [x] Add scan cancellation and subtree refresh.
  - [x] 2026-07-18 slice: add FFI contract v16's path-free, review-bound
    selected-folder scan. Swift supplies only the exact retained review and a
    versioned snapshot node ID; Rust verifies the review belongs to the engine,
    resolves a directory from immutable snapshot evidence, and matches its
    current device/inode identity before durable start, before traversal,
    against the completed artifact, and immediately before publication.
    Replacement, symlink, cross-volume, missing, file, released, foreign,
    read-only, overlapping, and resource failures remain typed and fail closed.
    The result is an ordinary standalone immutable snapshot rooted at that
    folder—not a splice into or rewrite of its historical parent—and the live
    path never crosses FFI. The existing bounded scan task provides polling and
    idempotent cancellation; queued cancellation creates no scan row or
    snapshot. The native app distinguishes Home and selected-folder scope so a
    subtree result cannot replace Home coverage evidence, keeps the old review
    and content visible through scanning and exact result validation, then
    atomically switches leases only if the source presentation generation is
    still current. Failure, cancellation, navigation, history selection,
    reload, close, and shutdown suppress stale publication. Explorer exposes
    Rescan This Folder on Command-R, Load Latest Snapshot as a separate action,
    accessible progress/cancel status, and explicit standalone-root copy.
- [x] Add performance fixtures for million-node snapshots.
  - [x] 2026-07-18 slice: add generated balanced and worst-case wide immutable
    snapshot fixtures over the real durable publication, exact review lease,
    retained decode/index, paging, treemap, Large Files, release, and shutdown
    path. Normal CI runs both shapes at 10,000 nodes; isolated Release lanes run
    exact balanced/wide 1,000,000-node and balanced 5,000,000-node scenarios,
    emit versioned JSON, capture macOS peak RSS, and are available through a
    weekly/manual artifact-producing workflow. The checked baseline records
    roughly 124 MB wire and 423–460 MB peak RSS for 1M, with first review at
    1.51–1.65 seconds off-main; the wide first page measured 7 ms, cached page
    1 ms, and 48-cell treemap 15 ms. That evidence raises the direct-child sort
    ceiling from 100,000 to the measured 999,999, while retaining one compact
    index and revalidating the exact lease after potentially long page/treemap
    work.
    A repeated native 64-cell layout gate remains below one 60 Hz frame.

    The exact 5M lane measured a 619,996,236-byte wire and approximately 2.01 GB
    publication-process peak RSS, then proved the existing conservative
    `wire × 3`/1 GiB decoded-review budget refuses the review before decode.
    DUX deliberately keeps that typed resource refusal: raising the UI-process
    budget without a streaming/indexed format is not justified by the evidence.
    No fixture creates live paths, FFI node floods, AI input, plans, or cleanup
    authority. Commands, topology, measurements, and interpretation live in
    `docs/SNAPSHOT_PERFORMANCE.md`.

Exit criteria:

- Users can scan, drill down, go back, and inspect large files.
- UI stays responsive during scans.
- Unknown/unscanned storage remains visible.
- No destructive action exists outside plan review.

### Milestone 5: Deterministic recommendations and reviewed cleanup

Goal: provide trustworthy recovery actions.

Tasks:

- [x] Ship the first independently researched safe-regenerable rules. The
  revisioned Rust `target` and Python `__pycache__` catalog entries are bundled
  and independently documented; they remain unschedulable until the trusted
  planning and executor gates below are complete.
  - [x] Stage the independently researched Rust Cargo `target` rule as revision
    2 (2026-07-18). Discovery now requires a direct regular `Cargo.toml`
    sibling and direct regular `CACHEDIR.TAG` child before proposing
    SafeRegenerable/RemoveKnownRegenerableContents. The exact catalog policy is
    build-time and load-time allowlisted, adversarial fixtures cover missing,
    misplaced, wrong-kind, case-mismatched, nested, and symlink evidence, and
    the rule stays unschedulable. Every result still carries `ProtectedPath`,
    so selection and even test-only plan construction fail closed. The tag
    filename is supporting immutable-snapshot evidence only, not proof of its
    contents or current Cargo ownership. See
    `docs/rules/developer-rust-target.md`.
  - [x] Stage a sealed Unix live default-layout witness (2026-07-18). A private
    planner module accepts only the exact revision-2 policy/evidence shape while
    `ProtectedPath` remains its sole blocker, reconstructs no-follow root,
    target, sibling manifest, and child tag identities, requires single-link
    regular markers on the same volume, and reads Cargo's exact 43-byte tag
    prefix through a nonblocking retained descriptor with before/open/after
    identity checks. The ephemeral witness cannot clone, serialize, construct a
    plan, clear a blocker, cross FFI, or execute. It intentionally proves only
    the current direct default layout; it does not resolve Cargo
    workspace/config, authoritative mount/protected-root scope, process
    inactivity, target descendants, or executor-time state. Windows remains
    unsupported at this planner boundary until ancestry is handle-relative.
  - [x] Stage a bounded Cargo workspace/target resolution witness (2026-07-18).
    A second sealed Unix type consumes the live witness, accepts no caller
    manifest/cwd/arguments/expected paths, and invokes only an observed
    canonical regular file named `cargo`; symlink launchers, including the usual
    rustup proxy, fail closed. Observation and every resolution bind the file's
    full bounded SHA-256, identity, single-link shape, and exact reviewed Cargo
    1.96.0 verbose version; the returned witness preserves that executable and
    canonical environment evidence. The subprocess runs from the manifest
    parent with a cleared, fixed environment and exact
    offline/locked/no-dependency format-version-1
    arguments; nonblocking stdout/stderr, time, and JSON are bounded, while
    timeout and overflow terminate the original process group. Only
    `resolve: null`, the exact project workspace root, and the
    exact already witnessed target directory are accepted. A new retained-file
    full SHA-256 snapshot detects same-inode manifest changes before and after
    Cargo. The combined witness still cannot clone, serialize, clear
    `ProtectedPath`, construct a plan, cross FFI, schedule, or execute.
  - [x] Bind the Rust-target planner checkpoints to their exact durable
    discovery source (2026-07-18). Production acquisition now accepts only a
    succeeded, complete-coverage scan with an available immutable snapshot and
    one `Discovered` candidate from the current evaluator revision, catalog
    schema/SHA-256, context format/digest, and exact snapshot version/digest.
    It recomputes the current deterministic Rust candidate ID, decodes the
    retained snapshot through the charged review-memory budget, locates the
    exact root/ancestor/target/manifest/tag graph, and carries its Unix
    device/inode observations into live validation. A
    non-cloneable source owns an exact `CleanupReview` lease and repeats the
    complete history read around snapshot acquisition; fresh lease and history
    checks bracket every live and Cargo revalidation, while consuming release
    and best-effort drop cleanup prevent ordinary failure/success loops from
    exhausting the pin cap. Different observed device/inode objects,
    candidate review-state changes, forged IDs, stale evaluator identities,
    repository mismatch, pin expiry, and source drift fail closed. The source
    remains a private locator/provenance witness: it exposes no public or FFI
    path surface and no plan, blocker-removal, scheduling, or effect capability.
    Device/inode reuse is still possible, so retained/generation evidence
    remains required before authority.
  - [x] Replay the complete deterministic candidate evaluation from the exact
    retained snapshot before admitting the Rust-target source (2026-07-18).
    The snapshot-native replay shares the evaluator's catalog pattern and
    candidate-finishing policy, but independently enumerates the immutable
    depth-first graph without reconstructing a second full-path DiskTree.
    Per-directory marker summaries keep enumeration O(nodes + edges);
    materializing the bounded result adds only its candidate/evidence path
    bytes. O(depth + accepted candidate payload) working state is charged
    incrementally against the same conservative 32 MiB durable-batch budget
    before retention. The replay reproduces exact regular-marker selection,
    native-byte evidence ordering,
    any-classified-ancestor suppression across all eleven catalog bindings,
    known allocated-byte estimates, directory/file descendant mtimes, partial-
    coverage blockers, IDs, policies, and the global 4,096/4,097 fail-closed
    boundary. Admission compares the ID-keyed full batch against every durable
    immutable candidate field; missing, injected, or modified candidates and
    reordered ordered fields fail before live validation. Replay runs inside
    the charged CleanupReview lease and the existing source/lease revalidation
    sandwich;
    failure releases its decoded-memory slot and pin. It returns only unit or a
    path-free error and still cannot expose candidates, remove ProtectedPath,
    construct a plan, cross FFI, schedule, or execute.
  - [x] 2026-07-18 slice: independently research and stage the Python
    `__pycache__` rule as revision 2. The immutable classifier now requires a
    direct regular, non-symlink `.py` sibling and a symlink-free candidate
    ancestry before proposing `SafeRegenerable` /
    `RemoveKnownRegenerableContents`; wrong-case, missing, and symlink markers
    fail closed. The catalog digest/build gate allowlists exactly two safe
    rules, both remain unschedulable, and every result retains the unresolved
    `ProtectedPath` blocker. Python's import reference, FAQ, and PEP 3147 are
    recorded in `docs/rules/developer-python-pycache.md`; no plan, executor,
    AI, FFI cleanup authority, or live Python-writer witness was added.
  - [x] Promote the bounded Cargo observation into trusted planning authority,
    then add authoritative volume and protected-root grants,
    change/process/descendant guards, and executor-time revalidation before
    removing `ProtectedPath` or enabling scheduling.
    The sole admitted Rust-target path now crosses the complete typed
    enrolled-Cargo, current-account volume, protected-root, process,
    descendant, reviewed-plan, journal, and descriptor-relative revalidation
    chain. This closes the prerequisite work without treating it as permission
    to weaken policy: durable candidate history still records `ProtectedPath`,
    generic planning still rejects blocked candidates, scheduling remains
    disabled, and Release cannot start permanent-safe cleanup.
    - [x] 2026-07-18 slice: add explicit, revisioned enrollment for one exact
      direct Cargo executable. The macOS core exposes inspect-then-commit,
      status, and revoke operations; inspection accepts no `PATH` or rustup
      selection, executes no selected bytes, and binds the canonical
      single-link file, full SHA-256, canonical scrubbed resolution
      environment, and strict bounded Security.framework static-code evidence.
      CMS and ad-hoc signatures are distinguished; ad-hoc evidence is integrity
      only and becomes locally trusted solely through the explicit user commit.
      A non-cloneable preview belongs to one engine/store and one exact prior
      settings revision. Consuming commit rechecks the static evidence, then
      explicitly authorizes execution of the selected bytes to bind the exact
      reviewed Cargo 1.96.0 verbose-version digest before a conditional write.
      Exact retries are no-ops;
      replacements advance the monotonic revision; revocation retains a
      tombstone; and stale pre-change or pre-revocation previews cannot restore
      old trust. The sealed metadata entry derives Cargo only from the same
      durable store, statically matches stored bytes/signature and rereads the
      exact enrollment before any automatic execution, then retains the
      enrollment guard before and after Cargo.
      Malformed, oversized, newer-schema, changed, unsigned, foreign-engine,
      and ambiguous-write cases fail closed. This core-only checkpoint added
      discovery provenance only: it added no FFI/Swift surface, blocker
      removal, plan, schedule, or effect.
    - [x] 2026-07-28 slice: expose explicit direct-Cargo discovery enrollment
      through UniFFI contract v27 and native Settings without opening cleanup.
      A versioned input carries only one user-selected, canonical, direct
      `cargo` path as bounded control-free UTF-8 Unix bytes; no `PATH`, rustup
      selection, command, arguments, environment, version, signature, digest,
      candidate, or cleanup value can be supplied. Static inspection executes
      no selected bytes and returns one engine-bound, non-cloneable opaque
      preview. Each engine admits at most one live preview, exposes only
      independently bounded display evidence, releases it explicitly or on
      close, checks engine affinity before commit, and consumes it permanently
      before core version validation. Status and revoke are observational
      settings operations; revocation retains the core tombstone.
      `EngineService` performs the synchronous bounded work off the main actor;
      AppModel generation-fences late presentation, releases superseded
      previews, binds confirmation to the exact evidence shown, releases an
      in-flight inspection that arrives after Settings closes, and treats
      confirmed enrollment as finishing rather than falsely cancellable.
      Returned mutation records are correlated to the preview/operation; any
      malformed or mismatched response becomes outcome uncertainty, triggers
      one observation-only authoritative reload, and visibly blocks another
      mutation until that reload succeeds. Projection failures after core
      success are outcome-unknown, while pre-mutation core failures keep their
      typed mapping. Runtime shutdown memoizes one shared task before
      suspension so concurrent callers wait for the same confirmed mutation
      and engine close. Settings separates static inspection from the final
      confirmation that permits one fixed Cargo 1.96.0 version execution,
      distinguishes ad-hoc integrity from CMS evidence, and states that this
      enables deterministic Rust discovery only. Revocation has a separate
      confirmation and deletes no files. No planner, approval, journal,
      executor, scheduler, CLI, AI, or cleanup route was added. Nine focused
      FFI lifecycle/race tests, 16 focused native tests, the complete 340-test
      linked Swift suite, and deterministic binding/project regeneration pass.
    - [x] 2026-07-18 slice: close Cargo 1.96 file-based configuration to one
      bounded negative case and retain the metadata working directory.
      Production now accepts metadata only when both `config` and
      `config.toml` are absent from every manifest-parent ancestor's `.cargo`
      directory and from the exact Cargo home at every validation checkpoint.
      Canonical directory identities, a revisioned
      policy-2 closure digest (including watch semantics) and
      64-ancestor/132-watch/64-KiB bounds bracket the fixed
      metadata process. macOS vnode fences make entry changes terminal in the
      project root and existing non-Cargo-home `.cargo` lookups; exact
      before/after absence checks cover higher absent lookups and Cargo home,
      where broad directory-write events include unrelated system/cache work.
      Configured projects fail closed before metadata execution. The child
      enters the exact retained project
      directory with `fchdir`, all retained/watch descriptors are close-on-exec,
      and path plus descriptor identities are rechecked around launch and
      output collection. The witness remains non-authoritative and still
      carries `ProtectedPath`. Non-local and non-APFS lookup directories reject;
      kqueue supplies strong reviewed-filesystem change inference, not proof of
      which inode Cargo opened. Positive configuration,
      workspace-member manifest provenance, and a pathname-independent Cargo
      executable launch remain open.
    - [x] 2026-07-18 slice: close the admitted macOS Cargo executable
      swap/restore race with selected-running-code continuity. Production now
      arms local-APFS vnode fences on the exact enrolled executable and every
      canonical ancestor, then calls the exact path with direct
      `posix_spawn`, `START_SUSPENDED`, a new process group, fixed signal state,
      and `CLOEXEC_DEFAULT`. The retained cwd is installed by descriptor file
      action. Before `SIGCONT`, DUX requires the direct child to remain stopped
      with its expected PID/parent/process-group, credentials, start instant,
      and cwd identity, asks Security.framework to validate that kernel guest,
      and requires its selected Code Directory hash to belong to the enrolled
      all-architecture static record. Full executable digest/identity is
      rechecked before resume and after exit, while the vnode fence is polled
      throughout bounded output. Resolution policy 3 records launch-policy
      revision 1 and a SHA-256 of the running Code Directory hash. Adversarial
      coverage proves that, absent external signaling, no helper user-space
      runs before DUX's own resume, plus wrong-image rejection,
      retained cwd, descriptor non-leakage, exact-child termination/reaping,
      in-place executable writes, and higher-ancestor renames. macOS still has
      no supported fd-based exec: this is swap/restore-resistant selected-code
      continuity on reviewed local APFS, not pathname-independent execution or
      confinement. A same-UID external actor can signal the stopped child, and
      kqueue remains event inference. The witness remains private,
      non-authoritative, and blocked by `ProtectedPath`.
    - [x] 2026-07-18 slice: bind the exact Cargo workspace-member manifest
      closure with a guarded two-pass protocol. The first bounded, fixed
      `cargo metadata --no-deps --locked --offline` result is discovery only.
      DUX requires a non-empty one-to-one relation between at most 256 opaque
      workspace-member IDs and local packages, rejects duplicate/unknown
      members and defaults, non-null package sources, manifest aliases, and
      malformed root/target data, and always includes the virtual or package
      root `Cargo.toml`. It then captures at most 257 canonical descendant,
      single-link regular manifests with 4-MiB/file, 64-MiB aggregate, and
      256-KiB native-path bounds. A domain-separated closure binds root/member
      role, opaque ID, native path, identity, length, and full SHA-256. On
      macOS, exact retained local-APFS manifest and deduplicated
      ancestry-through-root vnode watches bracket an identical second metadata
      pass. A preflight
      counts currently open descriptors and reserves 128 launch/app slots;
      insufficient process limits fail closed instead of reducing coverage.
      The combined config/workspace guard is checked before spawn/resume,
      throughout bounded
      output, after reaping, and before witness extraction. Resolution policy 4
      records manifest policy 1, member/manifest counts, and the closure digest.
      Tests cover a real virtual two-member workspace, malformed relations,
      symlink/hard-link aliases, the 256/257 bound, second-pass drift, and
      same-inode write/restore. This attests reported root/member manifests, not
      Cargo's complete read set, workspace glob namespace generations,
      lockfiles, excluded/path-dependency manifests, or source/build files;
      kqueue remains event inference and post-witness mutation still requires
      executor-time revalidation. No blocker, plan, FFI, schedule, or effect
      authority was added.
    - [x] 2026-07-18 slice: attest a bounded positive Cargo 1.96
      configuration/include closure and exact ordered path intent. DUX now
      accepts unambiguous project/ancestor config roots, independently parses
      only their top-level includes with Cargo's exact TOML 1.1.2 generation,
      and reads each canonical UTF-8, single-link regular file from one retained
      descriptor. Policy 3 binds full bytes, identities, root/read order,
      include edges, lookup selection, and watch semantics under 64-file,
      128-edge, 16-depth, 1-MiB/file, 16-MiB aggregate, and path bounds. Both
      fixed metadata passes require exact reviewed commit `30a34c6821b57de0aaec83a901aca39f88f6778c`,
      enable only its pinned Cargo context trace, and
      require every exact pre-read path record to equal the independent closure
      in count and order; missing, extra, reordered, malformed, duplicated, and
      newline-spoofed records fail closed. Exact config-file and complete-
      ancestry local-APFS vnode fences bracket both passes and detect
      write/restore; resolution policy 5 records file/edge/byte, closure, and
      intent evidence. Cargo-home config, dual names, aliases, cycles, missing
      optional includes, and unsupported paths reject. Real Cargo tests cover
      an included config and pin the reviewed fact that 1.96
      `metadata --no-deps` neither reads malformed `Cargo.lock` nor creates a
      missing one. This is path-intent plus reviewed-filesystem stability, not
      kernel proof of the descriptor Cargo opened. Higher absent lookup
      create/remove, excluded ancestor workspace and external path-dependency
      manifests, workspace/target/glob namespaces, source/build inputs, and
      post-witness changes remain outside the proof. No blocker, plan, FFI,
      scheduling, or effect authority was added.
    - [x] 2026-07-18 slice: bind Cargo 1.96's bounded potential ancestor-
      manifest probe namespace before the first metadata pass. DUX mirrors the
      pinned `find_root_iter` order from the selected manifest's parent toward
      the filesystem root, including its exact `target/package` and one-item
      Cargo-home look-behind stops. At most 64 canonical UTF-8 candidates are
      admitted; each directory identity and present direct `Cargo.toml` is
      bound, and every present manifest must be a single-link regular file
      captured by full SHA-256 under 4-MiB/file, 64-MiB aggregate, and native-
      path bounds. The independent policy-1 digest binds ordinal, native path,
      presence, directory/file identities, byte length, and content digest.
      On macOS all retained objects must be local APFS. Exact manifest vnode
      events are terminal, including write/restore, as are directory
      delete/rename/revoke events. Candidate-directory entry writes trigger
      complete observation replay, rejecting persistent create/remove while
      tolerating unrelated high-ancestor activity whose relevant entry state
      is unchanged. The guard brackets both metadata
      passes and resolution policy 6 records its counts, bytes, and closure.
      Real Cargo coverage includes a standalone package below an excluding
      ancestor workspace; adversarial tests cover stop rules, aliases, bounds,
      persistent namespace changes, and same-inode write/restore. This is a
      conservative potential-probe closure, not proof of which manifest Cargo
      read. Transient absent-candidate create/remove, external path-dependency
      manifests and their ancestor probes, glob/target/source/build
      namespaces, and post-witness changes remain open. No blocker, plan, FFI,
      scheduling, or effect authority was added.
    - [x] 2026-07-18 slice: close the accepted Cargo 1.96 local path-
      dependency graph over already reported workspace packages. Every
      serialized package must include its full dependency list. Across at most
      4,096 declarations and 256 KiB of local-path text, policy 1 requires
      `source: null` exactly for local paths, rejects relative, non-normalized,
      control-bearing, or mismatched values, and maps every local directory's
      exact `Cargo.toml` to a reported package manifest. Duplicate aliases and
      dependency kinds remain valid but are retained in total/local counts;
      unique targets are counted separately. A domain-separated sorted edge
      digest binds owner IDs and target manifest bytes, and resolution policy
      7 requires identical graph evidence across both metadata passes. The
      existing workspace guard therefore fences every admitted target during
      the accepted pass. Exact-Cargo regressions accept an internal implicit
      member and reject an external false-target optional build dependency
      that Cargo exposes but omits from the workspace package set. This is
      deliberately sufficient rather than necessary: discovery can read a
      rejected external manifest before no witness is produced, while
      standalone and excluded dependencies can be rejected even if Cargo did
      not read their manifests. Attesting safe unreported dependencies,
      transient absent ancestor create/remove, workspace glob/target/source/
      build namespaces, kernel-level read identity, and post-witness changes
      remain open. No blocker, plan, FFI, scheduling, or effect authority was
      added.
    - [x] 2026-07-18 slice: close Cargo 1.96's finite reported-package
      target/source/build discovery namespace. The strict metadata document
      now requires every package's complete `targets` array and policy 1 binds
      at most 256 packages, 4,096 targets, 16 kind labels per target, 512 KiB
      of target text, 2 MiB of native paths, and 16,384 namespace records.
      Every reported `src_path` must be an absolute normalized canonical
      single-link regular descendant. DUX independently snapshots each package
      root's `src`, `src/lib.rs`, `src/main.rs`, edition-2015
      `src/<target-name>.rs` and `src/bench.rs` fallbacks, implicit `build.rs`,
      complete direct `src/bin`, `examples`, `tests`, and `benches` entries,
      and `main.rs` below their direct child directories. On macOS all present
      entries and required ancestry must be local APFS. Directory writes are
      terminal under vnode fences, including create/remove restoration; exact
      file identity/link/rename/delete/revoke changes also reject, and complete
      observation replay brackets the accepted pass. Resolution policy 8
      retains package/target/namespace counts and the closure digest. Unit and
      exact-Cargo regressions cover inferred library/build targets, transient
      directory writes, source replacement, external/missing/symlink sources,
      and bounded evidence. Workspace globs, README/license metadata probes,
      transient absent ancestor create/remove, kernel-level read identity, and
      post-witness changes remain open. No blocker, plan, FFI, scheduling, or
      effect authority was added.
    - [x] 2026-07-18 slice: close Cargo 1.96's workspace-member glob
      generation before metadata discovery. Workspace-glob policy 1 parses the
      exact single-link root manifest with the pinned TOML 1.1.2 generation,
      preserves absent-versus-empty `members` and `default-members`, and uses
      Cargo's pinned glob 0.3.3 component semantics for `*`, `?`, classes, and
      recursive `**`. Cargo's non-glob `exclude` declarations remain literal
      normalized prefixes, raw file matches are filtered without triggering
      the no-match fallback, and leading-dot names retain Cargo's ordinary
      match behavior. The positive profile admits at most 256 declarations in
      each array and 768 total, 4 KiB per declaration, 256 KiB aggregate text,
      64 components/depth, 4,096 observed directories, 65,536 namespace
      entries and raw matches, 8 MiB across retained native-path copies,
      262,144 traversal states, and 2,097,152 entry comparisons. Enumeration
      is streaming and fails before storing the first over-bound entry;
      symlinks, special or
      unreadable selected entries, escapes, workspace-root metacharacters, and
      malformed patterns reject whenever a non-empty declaration invokes the
      glob engine; no-pattern packages remain supported. Literal components
      use Cargo's targeted native lookup (including case-insensitive APFS
      resolution), observe the
      selected present/missing state, and fence the parent generation without
      enumerating or rejecting unrelated siblings. Explicit default-member
      rows preserve ordered duplicates exactly, including multiple derivations
      from one recursive pattern.
      The guard is armed before the first metadata pass, owns the exact root
      manifest fence, and retains every conservatively consulted local-APFS
      directory behind terminal write/delete/attribute/link/rename/revoke
      events. Capture-arm-replay closes construction races, and directory
      `NOTE_WRITE` makes create/remove restoration terminal through both
      passes. Reported-membership-consistency policy 1 requires non-excluded
      expanded members plus an eligible root package to reach every reported
      package through Cargo's already validated serialized local path graph;
      it also reproduces Cargo's raw-member override of literal excludes and
      exact explicit/implicit default-member result. This rejects disconnected
      output but does not independently authenticate dependency declarations
      from manifest bytes. Resolution policy 9 records
      declaration, namespace, match, seed, excluded, reachability, default,
      root-manifest, and domain-separated closure evidence. Tests cover exact
      Cargo glob/exclude/default output, disconnected fake packages,
      fallback-versus-file behavior, duplicate defaults, literal sibling and
      native-case behavior, aggregate retained-path exhaustion, recursive
      multiplicity/collapse and bounds, selected symlinks, active versus
      inactive root metacharacters, persistent
      namespace changes, and an APFS create/remove during the discovery pass.
      Independent dependency-declaration provenance,
      package README/license metadata probes, transient absent ancestor-
      manifest create/remove, safe unreported dependency manifests,
      kernel-level read identity, and post-witness changes remain open. No
      blocker, plan, FFI, scheduling, or effect authority was added.
    - [x] 2026-07-18 slice: independently authenticate the admitted local
      path-dependency edges from the exact retained workspace-manifest bytes.
      Dependency-manifest policy 1 uses the pinned TOML 1.1.2 generation to
      enumerate direct and workspace-inherited `path` values in normal,
      development, build, and target-specific dependency tables. Relative
      paths are normalized against the declaring package or workspace root,
      every result must equal one of the canonical single-link manifests
      already retained by the workspace guard, and the duplicate-preserving
      owner-to-target multiset must exactly equal Cargo's bounded reported
      local edges. Invented, omitted, escaping, malformed, unreported, and
      unsupported inherited declarations fail closed under the existing
      4,096-declaration and 256-KiB local-path limits. The guard revalidates
      before and after independent parsing and retains policy/count/unique-
      target/domain-separated closure evidence in resolution policy 10. Tests
      cover duplicate direct/dev declarations, fabricated and omitted reported
      edges, and pinned real-Cargo workspace inheritance. This authenticates
      only local path relations, not remote dependency attributes or Cargo's
      kernel-level reads. Package README/license probes, transient absent ancestor-manifest
      create/remove, safe unreported dependency manifests, kernel-level read
      identity, and post-witness changes remain open. No blocker, plan, FFI,
      scheduling, or effect authority was added.
    - [x] 2026-07-18 slice: close Cargo 1.96's package README and
      `license-file` metadata generation. Package-metadata policy 1 parses the
      exact retained root/member manifests with pinned TOML 1.1.2 and
      independently reproduces direct strings, `readme = true/false`, and
      `[workspace.package]` inheritance with exact member-relative rebasing.
      An absent direct `readme` binds Cargo's ordered `README.md`, `README.txt`,
      then `README` lookup across every reported package root, preserving
      missing, directory, and single-link regular-file observations. Selected
      symlinks/hard links, special entries, malformed values, absolute paths,
      escapes, and targets outside the witnessed workspace fail closed.
      Explicit README/license targets are path declarations only: pinned Cargo
      reports them without requiring the files to exist, so DUX does not claim
      their bytes or current existence. The profile admits at most 256
      packages, 4 KiB per string, 256 KiB aggregate text, 2 MiB native paths,
      2,048 namespace records, 64 path components, and 4,096 watched objects.
      Exact manifests and package-root generations are local-APFS fenced;
      directory writes make implicit README create/remove restoration terminal
      through the accepted pass. Resolution policy 11 records package,
      manifest, declaration/probe/selection counts, identities, full manifest
      digests, and a domain-separated closure. Unit and pinned real-Cargo tests
      cover priority, suppression, explicit true, direct/inherited paths,
      fabricated/omitted output, symlink rejection, and transient namespace
      restoration. Transient absent ancestor-manifest create/remove, safe
      unreported dependency manifests, kernel-level read identity, and
      post-witness changes remain open. No blocker, plan, FFI, scheduling, or
      effect authority was added.
    - [x] 2026-07-18 slice: close the transient absent-ancestor manifest
      namespace race on macOS with exact, bounded FSEvents replay. Policy 2
      captures the event cursor before observation, arms the farthest
      absent-candidate directory, and replays a short-lived file-events
      history stream after the kqueue fence is ready. DUX requires
      history-done, monotonic event IDs, and volume-UUID continuity before
      advancing the cursor; dropped/coalesced, wrapped, unknown, or incomplete
      coverage fails closed. Exact `Cargo.toml` candidates use the mounted
      volume's case-sensitivity semantics. Candidate-file writes and
      directory delete/rename/revoke events are terminal; directory writes
      request replay, absent-candidate create/remove is terminal, and
      unrelated sibling activity is ignored. kqueue `NOTE_WRITE` is therefore
      a replay hint rather than proof of a complete observation. Resolution
      policy 12 binds the revision, cursor/fence result, counts, bytes, and
      ordered closure. Unit tests cover event classification, case variants,
      dropped/unknown coverage, and history completion; macOS high-level
      regressions cover create/remove during discovery and unrelated sibling
      activity. This remains path-based stability evidence rather than
      kernel-level read identity and adds no cleanup or `ProtectedPath`
      authority. Safe unreported dependency manifests, kernel-level read
      identity, and post-witness changes remain open.
    Kernel-level Cargo read identity, complete remaining Cargo manifest and
    namespace provenance, and all remaining
    protected-root, volume, process, descendant, plan, and executor grants are
    still open.
    - [x] 2026-07-19 slice: bind the repeated filesystem-boundary observation
      into exact-path review. The planner now captures the complete no-follow
      scan-root ancestry and platform mount identity before selected-path
      validation, retains it in the non-cloneable review evidence, and
      revalidates the boundary before publication. Boundary capture or drift
      fails closed; this remains observational and does not mint a trusted
      volume/location grant, clear `ProtectedPath`, construct a plan, cross
      FFI, schedule, or mutate.
    - [x] 2026-07-19 slice: add the crate-private
      `TrustedVolumeLocationWitness` proof boundary. It can be constructed
      only from the code-owned `CanonicalScanRoot`, retains repeated ancestry
      plus kernel filesystem and mount-location evidence, and exposes only
      proof revision, boundary matching, and revalidation. macOS rejects
      requested/canonical alias or firmlink-like mismatches; Linux requires a
      non-zero kernel mount ID rather than a device number alone; unsupported
      platforms fail closed. This remains observation-only: it grants no
      volume/location or protected-root rule, does not clear `ProtectedPath`,
      and cannot construct a plan, cross FFI, schedule, or perform an effect.
    - [x] 2026-07-19 slice: close the current-account home/mount observation
      gap without granting authority. Unix root ancestry, final-directory
      ownership, and platform mount identity now come from one retained
      no-follow descriptor during boundary capture; repeated observations
      include owner identity. `TrustedHomeMountWitness` is macOS-only,
      non-cloneable, and built only from the OS account database plus a
      canonical scan root. It requires the scan root to equal or descend from
      the exact current home, retain the home identity in ancestry, and share
      the exact mount; it rereads account/home evidence and both boundaries on
      revalidation. Linux/Windows fail closed for this positive profile. The
      witness remains location evidence only: no protected-root rule grant,
      blocker removal, plan, FFI, schedule, or effect was added.
    - [x] 2026-08-09 qualification slice: add a real, nondestructive macOS
      APFS mount/firmlink matrix for that witness boundary. The
      [repository-owned harness](docs/testing/macos-apfs-boundary-qualification.md)
      derives the current home from the OS account database,
      creates one disposable APFS image below it, and runs only one exact
      ignored Rust test. The positive control acquires and revalidates a
      same-mount descendant witness. A no-follow symlink alias and the live
      `/System/Volumes/Data/...` spelling of the same home object fail closed;
      after APFS is mounted over the reviewed empty directory, the retained
      witness reports `Changed`, fresh home authority reports `DifferentMount`,
      and descriptor-relative target capture reports `CrossVolume`. The test
      cannot skip a missing fixture and never invokes a planner, journal,
      executor, FFI, Swift, application process, or cleanup effect.

      The exact harness passed on arm64 macOS 26.5 (25F71) with Rust/Cargo
      1.96.0. It detached the image and removed only its random fixture. This
      closes the local APFS evidence gap without widening authority; the exact
      signed Intel/newest-supported Apple Silicon application protocol and
      Windows handle-relative/reparse evidence remain separate release gates.
    - [x] 2026-07-19 slice: retain Cargo's complete read-set and enrollment
      fences after metadata publication. The non-cloneable Cargo witness now
      owns configuration, ancestor-manifest, workspace-glob, workspace,
      package-metadata, and target-namespace guards, the retained metadata
      working directory, the exact executable/version observation, the
      optional direct-Cargo enrollment guard, and the filesystem boundary.
      Its private revalidation repeats all fences, the enrolled Cargo version,
      live Rust-target evidence, and boundary identity. A post-publication
      manifest mutation regression fails closed. This is still provenance-only:
      no trusted volume/protected-root/rule grant, blocker removal, plan,
      approval, FFI, schedule, or effect was added.
    - [x] 2026-07-19 slice: add a consumed, path-private Cargo planning-
      provenance token. Construction revalidates the retained witness and
      binds the exact source-scan ID, candidate ID, witness revision,
      resolution-policy revision, and unresolved `ProtectedPath` marker.
      Retained-token revalidation repeats those bindings and every owned
      filesystem/Cargo fence. The token has no path getter, plan conversion,
      approval, FFI, schedule, or effect method; it only supports further
      revalidation and explicit lease release.
    - [x] 2026-07-19 slice: consume Cargo provenance into a sealed
      `RustTargetRuleBoundaryEvidence` join with the macOS current-account
      home-mount witness. The join requires exact retained scan-boundary
      equality, revalidates both Cargo/read-set and account/home/mount fences,
      preserves the unresolved `ProtectedPath` marker, and exposes only
      revalidation and lease release. It has no path getter, blocker-removal,
      plan, approval, FFI, scheduling, or effect operation; Linux/Windows
      location authority and process/descendant/executor guards remain open.
    - [x] 2026-07-19 slice: stage the private process-activity witness seam.
      macOS uses the read-only libproc table directly (never a shell), bounds
      the PID list and executable-path/name records, and retains PID,
      start-time, and executable identity without crossing persistence or FFI.
      Exact process-name guards require a complete, duplicate-free observation;
      active, malformed, truncated, inaccessible, or unsupported observations
      fail closed. Bundle identifiers do not downgrade to process names and
      remain unsupported until a signed bundle-identity provider exists. A
      fresh revalidation rejects PID/image replacement and any newly active
      guard. The rule-boundary evidence can consume this witness and repeats
      it, but the current catalog has no activity guards, so no blocker is
      removed and no plan, schedule, approval, or effect authority is added.
    - [x] 2026-07-19 slice: stage bounded exact descendant-policy evidence.
      The private non-cloneable witness binds validated rule-relative
      protected/excluded selectors to no-follow target identities, entry kinds,
      ancestors, and regular-file link counts. Missing, malformed, overlapping,
      multiply-linked, symlinked, or changed entries fail closed; selector
      matching is component-aware (`.git` cannot match `.github`). The sealed
      rule-boundary join can retain and revalidate this witness, but the
      current catalog has no descendant selectors. Recursive enumeration,
      retained descriptor-relative child effects, blocker removal, planning,
      approval, FFI, scheduling, and cleanup effects remain explicitly open.
    - [x] 2026-07-19 slice: add the first trusted deterministic-rule scope
      authorization. A private non-cloneable token allowlists only the
      revision-2 Rust-target and Python-`__pycache__` rules, binds the exact
      macOS current-account home/mount witness and protected-root registry,
      and authorizes one fresh no-follow target only when requested and
      canonical policy both produce `NoTextualMatch`. It repeats account,
      mount, ancestry, boundary, policy, and target identity checks before
      release. The token has no path getter, plan conversion, approval,
      persistence, FFI, schedule, or effect method, so existing candidates
      still retain `ProtectedPath`; process/descendant coverage and the plan
      and executor joins remain open.
    - [x] 2026-07-19 slice: bind the exact Cargo/read-set witness into a
      private Rust-target planning grant. The grant consumes retained Cargo
      boundary evidence, exact source-scan/candidate identity, current
      home/mount boundary, target snapshot, and requested/canonical
      `ProtectedRootRegistry::NoTextualMatch` policy evidence. Revalidation
      repeats the Cargo read-set, executable, account, mount, boundary,
      policy, and target checks; foreign candidates, target replacement,
      policy drift, and manifest/read-set changes fail closed with path-free
      errors. The grant remains non-Clone and exposes only revalidation and
      release: it cannot clear `ProtectedPath`, construct a plan, approve,
      schedule, cross FFI, or perform an effect. Generic no-Cargo fixtures
      are explicitly test-only; production Rust-target authorization cannot
      bypass the Cargo join.
    - [x] 2026-07-19 slice: promote home/mount and textual policy observations
      into private authoritative grants without widening cleanup authority.
      A revisioned home-volume grant consumes the exact current-account
      `TrustedHomeMountWitness`; a code-owned protected-rule grant binds the
      allowlisted rule/revision, stable boundary key, exact scan root/target,
      and both requested and canonical policy revisions. Each grant repeats
      its evidence and fails closed on boundary-key, policy, volume, or
      target drift. Linux/Windows remain unsupported for this positive
      profile, and candidates retain `ProtectedPath`; no plan, approval,
      scheduling, FFI, or effect path was added.
    - [x] 2026-07-19 slice: make Rust-target process quiescence a mandatory
      private boundary input. The code-owned process witness uses the bounded
      macOS libproc provider (never a shell) and requires exact inactive
      `cargo` and `rustc` process-name guards. Incomplete, malformed, active,
      PID-replaced, or wrong-guard observations fail closed; revalidation is
      repeated at every Cargo boundary reuse. Test fixtures use an explicit
      empty provider so the test runner's own Cargo process cannot weaken the
      production rule. Candidates remain blocked and no plan, approval, FFI,
      schedule, or effect authority was added.
    - [x] 2026-07-19 slice: make descendant coverage explicit at the same
      boundary. Rust-target admission now always carries a revalidated
      `DescendantPolicyWitness`; when no selectors are declared, the witness
      is an explicit code-owned empty selector set rather than absent evidence.
      Non-empty selectors are rejected for this catalog revision until a rule
      declares and binds them. Missing, overlapping, symlinked, multiply-linked,
      changed, or malformed selector evidence remains fail closed; no blocker,
      plan, approval, FFI, schedule, or effect authority was added.
    - [x] 2026-07-27 slice: add a private Rust-target promotion checkpoint
      without weakening discovery or exact review. The non-Clone
      `RustTargetPromotion` token admits only the exact revision-2,
      unscheduled, sole-`ProtectedPath` candidate with the canonical three
      marker facts, deterministic ID, exact live target, and a full immutable
      body match against the retained durable candidate record. It consumes
      the candidate-bound Cargo/home-volume/protected-rule authorization and
      revalidates that grant before admission. The token retains the blocker
      and has only private revalidation/release methods: it cannot clear a
      blocker, construct a plan, approve, persist, cross FFI, schedule, or
      mutate. A typed `RustTargetPlanFacts` capability now consumes this token
      only after immediate grant revalidation and retains the canonical
      scan-root/target witnesses for the next boundary. It still exposes only
      private revalidation/release; generic exact review and `CleanupPlan`
      continue to reject blocked candidates. A private facts-to-plan join now
      constructs a permanent-safe domain plan only from that typed capability,
      re-running mode, duplicate, source-scan, overlap, and byte validation
      while returning the authorization alongside the plan. It still performs
      no journal, approval, FFI, scheduling, or effect operation. A private
      Rust-target handoff now pairs this plan with its authorization inside
      the existing `TrustedReviewedCleanupPlan` wrapper after exact item/path
      matching and another grant revalidation. It is still not exposed to
      arbitrary callers, exact-review UI, journal, FFI, scheduling, or effects;
      a private approval handoff now consumes the wrapper through the existing
      expiry-checked approval capability. The next slice must connect that
      approved capability to the existing journal claim lifecycle under the
      same generation fence. A private Rust-target handoff now feeds the
      approved capability into the existing planned-session persistence and
      owner-fenced journal claim path with the same canonical timestamps and
      lock timeout. It still exposes no executor effect; the next slice must
      connect only the claimed session to the already private executor bridge
      after final effect admission.
- [x] Implement candidate groups and overlap resolution. Completed 2026-07-19:
  deterministic grouping and conservative overlap resolution are now consumed
  by exact-path review. Equivalent observations coalesce by stable candidate
  identity, same-rule parent ownership is component-aware, and blocked,
  conflicting, mixed-policy, mixed-rule, and internally overlapping findings
  remain unresolved with zero actionable bytes. Malformed paths, duplicate
  IDs, mixed scans, and byte overflow fail closed. The review retains the
  immutable grouping witness, so downstream callers cannot reconstruct a
  selection from reordered candidate input. Presentation/FFI projections and
  cleanup execution remain covered by their later roadmap boundaries.
  - [x] 2026-07-19 slice: bind the deterministic grouping result to the
    planner-owned exact review. The review now retains the immutable group and
    overlap witness that selected its items, so later callers cannot recreate
    selections from a reordered candidate slice. This promotes grouping to the
    reviewed planning boundary without granting approval, persistence, FFI,
    scheduling, or mutation authority; unresolved overlaps still fail closed.
  - [x] 2026-07-18 slice: add the sealed, non-authoritative core grouping
    result. Candidates are grouped deterministically by category, safety, and
    proposed action. Exact duplicate observations coalesce only when every
    immutable fact and rule revision matches; same-rule parent/child findings
    coalesce only when the parent owns every child path. Candidate IDs provide
    the stable equivalent-duplicate tie-break, never input order. Blocked,
    mixed-rule, mixed-policy, conflicting-fact, and internally overlapping
    candidates remain explicit unresolved decisions with zero actionable bytes;
    component-aware sibling prefixes remain disjoint. Relative, traversal,
    duplicate-ID, and mixed-scan inputs fail closed. The result retains member
    and selected indices for a future planner but cannot construct a plan,
    persist state, cross FFI, approve, schedule, or mutate anything.
- [x] Implement exact-path plan review. Completed 2026-07-19: the planner
  captures bounded no-follow live evidence, binds deterministic grouping and
  protected-policy witnesses, constructs only compatible domain plans with
  matching rule-scope authorizations, and issues an expiry-bound approval
  capability that revalidates every retained grant. The review and approval
  types remain non-Clone, path-private, and cannot reach persistence, FFI,
  scheduling, or filesystem mutation; those are separate executor boundaries.
  - [x] 2026-07-19 slice: add the sealed permanent-safe plan construction
    checkpoint. Exact review now retains the selected candidate facts and can
    consume exactly one matching trusted rule-scope authorization per live
    target into a crate-private domain `CleanupPlan`. Dry-run and Trash modes,
    missing/duplicate/mismatched authorizations, incompatible candidates, and
    all existing domain validation failures remain fail-closed. The resulting
    plan retains its authorization tokens for future executor-time
    revalidation; it has no journal, FFI, scheduling, or effect path.
  - [x] 2026-07-19 slice: add a crate-private explicit approval capability for
    trusted permanent-safe plans. Approval is rejected at or after the frozen
    plan expiry and revalidates every retained rule-scope authorization before
    issuing a non-cloneable approved wrapper. The wrapper can revalidate its
    expiry and grants, but cannot create journal rows, cross FFI, schedule, or
    invoke a filesystem effect.
  - [x] 2026-07-18 slice: add a sealed planner-owned evidence boundary that
    requires a code-owned canonical scan-root witness and runs every selected
    target through lossless lexical validation plus no-follow live identity
    capture. The bounded result retains requested/canonical/relative paths,
    target kind, volume/object identity, ordered ancestor identities, hard-link
    count, rule/category/action facts, evidence, estimates, and warnings. It
    deterministically consumes candidate-group selections, rejects blockers,
    non-cleanup candidates, incompatible modes, unresolved overlap, invalid or
    out-of-scope paths, missing/symlink/special targets, multiply-linked
    permanent files, and byte overflow. The result is intentionally
    non-Clone/non-serializable and always non-actionable while trusted
    protected-root and volume grants are absent; no plan, approval, persistence,
    FFI, schedule, or mutation path was added. Full protected-root grants,
    approval binding, executor revalidation, and plan/journal integration remain
    open.
  - [x] 2026-07-18 slice: add a private repeated filesystem-boundary
    observation for the canonical scan root. Unix captures the complete
    no-follow root-to-scan ancestor identity chain, while macOS retains the
    `fstatfs` filesystem identity and mount location and Linux requires
    descriptor-relative `statx` mount identity plus filesystem statistics.
    The bounded ancestry (64 entries) is captured twice and can be explicitly
    revalidated; mismatches, malformed mount evidence, and unsupported Windows
    platforms fail closed. This remains observation only: it does not issue a
    trusted volume/location grant, change `ProtectedPath`, construct a plan,
    cross FFI, or authorize an effect. APFS firmlink semantics, trusted home
    discovery, and rule-boundary grants remain open.
  - [x] 2026-07-18 slice: add code-owned current-account home discovery for
    the textual registry on Unix/macOS. The resolver uses the OS account
    database for the real/effective UID (never HOME/USERPROFILE), rejects
    setuid ambiguity and lossy/relative homes, captures the no-follow live
    directory twice, and requires the final home directory to be owned by the
    current account. This supplies only protected-policy input; it does not
    grant a rule, authorize a plan, cross FFI, or enable cleanup. Windows
    known-folder/reparse evidence, firmlink semantics, profile-container and
    volume grants remain open.
  - [x] 2026-07-18 slice: bind the code-owned protected-root classifier into
    exact-path review. Requested and canonical forms are both assessed; hard
    denies fail the review closed, specific-rule requirements remain explicit
    non-actionable protection metadata, and no-textual-match retains the policy
    revision without becoming an allow. Production review fails closed if
    account-home discovery or policy assessment is unavailable. The review
    still has no trusted volume/rule grant, approval, plan, FFI, or executor
    capability.
- [x] Implement Trash executor for Explorer selections.
  - [x] 2026-07-18 slice: add a crate-private Unix/macOS no-follow final-link
    witness for future Trash admission. It keeps the requested and validated
    lexical object paths, ordered no-follow ancestor identities, volume/object
    identity, target kind, and link count without canonicalizing or reading a
    symlink target. Regular files and directories use the same witness shape;
    final dangling and loop links are retained as link objects, while root or
    intermediate links and special entries fail closed. Windows remains
    `UnsupportedPlatform` until handle-relative reparse-tag validation exists.
    The witness is non-actionable and adds no plan, approval, journal, FFI,
    `FileManager.trashItem`, or mutation authority.
  - [x] 2026-07-19 slice: add a separate core-owned Trash review witness from
    an Explorer snapshot node. It resolves the historical chain, revalidates
    the current scan root and every no-follow ancestor, accepts a final
    symlink as the selected link object, and rejects roots, special entries,
    missing/replaced identities, and symlinked intermediate ancestors. The
    witness is non-Clone/non-serializable, never crosses the read-only live
    target FFI record, and cannot approve, construct a plan, or invoke an
    effect. The one-shot platform executor, journal integration, approval
    binding, and macOS `FileManager.trashItem` adapter remain open.
  - [x] 2026-07-19 slice: add journal-fenced one-shot Trash admission before
    any platform effect. Admission claims the store cleanup lease, binds the
    opaque review witness to the exact frozen journal path, revalidates the
    no-follow root/ancestor/target identities, records `effect_started`, and
    rechecks the owner/generation receipt immediately before a future adapter
    call. Changed, missing, unsupported, and unbound targets become typed
    rejected/changed/unavailable outcomes; a pre-effect cancellation path
    settles the receipt without touching the filesystem. The capability is
    non-Clone and path-free at its error boundary. No macOS `FileManager`,
    Swift/FFI surface, approval binding, or real Trash mutation exists yet.
  - [x] 2026-07-19 slice: close the core effect lifecycle with a private,
    synchronous one-shot platform-driver seam. The consuming admission repeats
    target and receipt validation immediately before the driver call, records
    `Trashed`, `Failed`, or conservative `OutcomeUnknown` while the journal
    claim remains held, and returns only bounded path-free errors. Recording
    adapters verify exact-path delivery, no mutation, single-call behavior,
    and no retry after an unknown outcome. The driver is crate-private and no
    Swift/FFI caller or real platform primitive can invoke it yet.
  - [x] 2026-07-19 slice: add the internal macOS Trash adapter contract. A
    Foundation-backed `TrashFileManaging` seam maps a successful synchronous
    `FileManager.trashItem(at:resultingItemURL:)` call to success and every
    thrown Foundation result to path-free `OutcomeUnknown`. Injected fake tests
    verify exact URL delivery, no mutation, one call, and no retry. The adapter
    is not wired to UI or FFI; only the future core-owned callback may invoke it.
  - [x] 2026-07-19 slice: stage the UniFFI v19 core-issued callback contract.
    `TrashEffectRequest` has no public constructor, carries only bounded
    target-kind/encoding metadata plus the exact ephemeral path bytes, and
    consumes those bytes once. `TrashPlatformDriver` is synchronous and returns
    only `Completed`, `Unsupported`, `Failed`, or `OutcomeUnknown`. There is
    still no callback registration or mutation entry point: a future reviewed
    plan/approval capability must issue the request while the journal claim is
    held, and the Swift adapter must consume it immediately without retaining
    or retrying it.
  - [x] 2026-07-19 slice: add the inert Swift callback-side request adapter.
    The generated request is validated for record version, Unix encoding,
    bounded absolute bytes, and target kind before Foundation URL creation;
    byte-round-trip failure returns `Failed` without entering `FileManager`,
    while a Foundation throw remains `OutcomeUnknown`. A fake request seam
    verifies exact URL delivery, one-shot consumption, malformed-path refusal,
    and no retry. `MacOSTrashPlatformDriver` is compiled but not registered.
  - [x] 2026-07-19 slice: wire the explicit Explorer action through the full
    core/FFI/Swift path. Core creates a fixed, review-required single-item
    Trash plan from only a live no-follow Explorer witness, persists a bounded
    planned journal row, holds the cleanup claim across the synchronous
    one-shot callback, and returns only bounded platform outcomes. UniFFI v21
    rejects foreign/closed reviews and maps storage, journal, review, and
    unknown-outcome failures without exposing paths. Swift routes the retained
    review lease to `MacOSTrashPlatformDriver`, validates the core-issued
    bytes before `FileManager.trashItem`, and presents an explicit destructive
    confirmation in Explorer. The UI explains that Trash does not reclaim
    space until emptied; no AI, CLI, scheduler, arbitrary path, or permanent
    delete path can invoke this action.
- [x] Implement permanent-safe executor for approved rules. Completed locally
  2026-08-09 for the sole authorized `developer.rust.target` rule. The complete
  deterministic core → FFI → native path, final effect admission, outcome
  quarantine, history, consent/exclusion gates, CI fixture, and nondestructive
  APFS boundary qualification exist. Ordinary Release still compiles out the
  action; the signed two-host disposable-data protocol remains a §17.3 release
  gate, not missing executor implementation.
  - [x] 2026-07-19 slice: add the crate-private multi-path session
    orchestrator. It consumes only the non-cloneable approved session,
    iterates item/path order under the same owner-generation fence, continues
    after durably settled failures, stops on outcome-unknown recovery, and
    terminalizes only after every path is settled. Cancellation requests and
    interrupts only the remaining work; the bounded result reports removed
    entries/bytes and the journal terminal status. Prefix-terminal journal
    validation now permits settled earlier paths while requiring the exact
    current path to be `validating` and all later paths to remain `planned`.
    The bridge remains crate-private and unreachable from FFI, Swift, CLI, AI,
    or production cleanup UI.
  - [x] 2026-07-19 slice: add the first crate-private engine execution bridge.
    `EngineHandle` now accepts only the non-cloneable `ApprovedCleanupSession`,
    enforces an open engine lifecycle, and selects the concrete
    descriptor-relative Rust driver without accepting caller paths, callbacks,
    AI output, CLI requests, or FFI values. The bridge is proven end to end in
    a project-local Cargo fixture: it removes only the approved target
    contents, preserves `CACHEDIR.TAG`, settles the journal item, and exposes
    completed cleanup history. It remains unreachable from production UI/FFI
    until protected-root/volume grants and the centralized orchestration are
    complete.
  - [x] 2026-07-19 slice: fence journal validation before rebuilding the
    live Rust-target witness. Changed targets, stale approvals, and journal
    revalidation failures now settle the path out of `Planned` with bounded
    validation outcomes instead of leaving an apparently executable row for
    recovery; no effect starts on those paths.
  - [x] 2026-07-19 slice: add the private descriptor-relative Rust-target
    contents executor boundary. It inventories descendants before mutation,
    preserves the direct `CACHEDIR.TAG` marker, rejects symlinks, special files,
    multiply-linked regular files, changed identities, and boundedness
    violations, and reopens each parent descriptor-relative while checking
    ancestry before every unlink. Cancellation and partial effects map to
    conservative journal outcomes. The driver is crate-private, has no FFI or
    Swift caller, and is not yet reachable from production cleanup UI.
  - [x] 2026-07-19 slice: retain trusted rule authorizations in exact
    plan-item/path order and add a private Rust-target effect witness. The
    witness revalidates the target directory, Cargo manifest identity/content,
    standard cache-tag signature, hard-link policy, and no-follow ancestry
    immediately before a future executor boundary. It performs no deletion,
    does not clear `ProtectedPath`, and is not exposed to FFI or Swift; the
    descriptor-relative contents executor and journal effect admission remain
    open.
  - [x] 2026-07-28 slice: add the engine-owned, consume-once
    `PermanentSafeCleanup` task for the exact opaque Rust-target review. Start
    admission rejects a foreign engine, closed/full registry, competing Trash
    or permanent operation, and unresolved prior outcome without consuming the
    review. Acceptance synchronously consumes and approves the child before it
    returns; the consuming transition rechecks parent liveness and both
    deadlines after its final authorization revalidation, detaching queued
    authority from the parent-review lifetime without a release/expiry gap. The
    task closure retains no `EngineInner`. Only the worker can mint random
    session identity/manual trigger, claim the journal, bind a volume sampler,
    or reach the descriptor-relative driver. Queued cancellation creates no
    journal; durably settled cancellation retains its path-free result. Known
    completed, partial, failed, rejected, and cancelled journal outcomes remain
    task results, while unproven state is typed `OutcomeUnknown`, includes the
    exact path-free session correlation, and cannot retry automatically. One
    engine reservation serializes this task with synchronous Explorer Trash.
    The claim handoff retains an ambiguous generation-one lease and retries
    only that exact owner/session/timestamp. A still-unproven claim,
    post-claim comparison, Trash admission transition, or effect settlement is
    retained in a physical-store quarantine for the remainder of the process;
    known pre-effect Trash refusal terminalizes before releasing its owner.
    Same-process engine close/reopen therefore cannot outrun durable restart
    recovery. The permanent driver and Explorer Trash callback are both caught
    inside their receipt scopes: panic records `outcome_unknown`; owned
    settlement retry has no callback and cannot repeat the filesystem call.
    End-to-end macOS tests cover completion-time parent release and exact
    expiry, no task/engine self-cycle, exact-review success,
    foreign/full/busy review retention, queued cancellation/no journal, blocked
    Trash contention, pre-journal drift, single and continued claim ambiguity,
    continued settlement ambiguity, Trash panic, same-store reopen,
    marker/source preservation, and path-free unknown correlation. This is a
    core orchestration checkpoint only: UniFFI still exposes the v28 review as
    observation/release, so explicit native confirmation and app execution
    remain open and the parent item is intentionally unchecked.
  - [x] 2026-07-29 slice: expose the exact core task through UniFFI contract
    v31 without adding a caller-shaped execution request. The sole start input
    is the engine-bound opaque `RustTargetPlanReviewSession`; callers cannot
    submit a path, candidate or plan ID, timestamp, approval Boolean, callback,
    AI result, command, or retry token. A foreign engine is rejected before
    touching the child. A correct-engine attempt consumes the review exactly
    once even when core admission later refuses it, while an overlapping
    information read is changed to release-pending so it cannot restore
    authority after losing the start race. Start participates in the existing
    bounded operation tracker, clones the open core handle without retaining
    the FFI state mutex across revalidation, and lets close wait for the
    admitted operation. The returned opaque task exposes only explicit
    cancellation and path-free polling: phase, cancellation state, revision,
    bounded failure taxonomy, durable session correlation, removed aggregates,
    and optional verified capacity delta. FFI independently rejects the wrong
    core task kind, malformed phase/failure/result combinations, invalid
    cleanup session IDs, and recovering results that imply a known effect.
    Dropping the observer does not cancel or retry work.
    The production macOS process-quiescence proof is revision 2: it still
    completely bounds and reads the libproc PID/name table, but requests
    start-time/executable identity only for exact guarded names (`cargo` and
    `rustc`). An unreadable guarded identity remains a fail-closed refusal;
    unrelated applications no longer make cleanup unavailable merely because
    macOS withholds their executable path. A zero-length or completely filled
    PID buffer is treated as incomplete/possible truncation. A failed PID
    inspection is skipped only
    when libproc reports `ESRCH` or a second independently complete PID table
    proves that exact PID disappeared; a still-listed or unprovable identity
    failure refuses.
    The generated Swift interface and universal XCFramework carry the v31
    types, but `EngineService`, AppModel, and Explorer intentionally do not
    consume them yet. Explicit native confirmation, changed-since-plan
    presentation, and release gates remain open, so the parent item stays
    unchecked.
  - [x] 2026-07-29 slice: connect v31 to an internal native execution
    checkpoint without opening the public cleanup gate. `EngineService`
    independently validates record version, strict phase/failure/result shape,
    monotonic revision, sticky cancellation, terminal stability, exact
    `cleanup:rust-target:<32 lowercase hex>` correlation, and outcome-unknown
    aggregates. Its review wrapper serializes info/release/start, marks every
    owning-engine start consume-once before calling FFI, and retains no caller
    path, reconstructed plan, approval Boolean, callback, AI output, command,
    or retry token. The authority-owning review controller stores the complete
    immutable `ExplorerRustTargetPlanReviewInfo`, requires exact equality at
    start, verifies the exact parent generation, and removes the child before
    suspension. A same-UUID/same-candidate handle with altered plan or target
    display is rejected without consuming the real child.
    Explorer binds confirmation to a separate generation, exact child UUID,
    and complete displayed plan; clears the preview before transfer; and
    presents queued, running, explicit cancellation, changed-since-plan,
    partial, failed, outcome-unknown, cancelled, and terminal path-free states
    in a global accessible banner. It never retries. Closing Explorer preserves
    observation; ordered app shutdown requests cancellation and awaits the
    driver. The confirmation/action compile only under
    `DUX_INTERNAL_PERMANENT_SAFE_CLEANUP`, which XcodeGen assigns to Debug.
    Release shows execution unavailable, and notarized-release automation
    rejects resolved Release settings containing that condition. Focused
    adapter/controller/browser tests cover consume-once mapping, malformed and
    regressing polls, forged display information, stale/repeated confirmation,
    close survival, and shutdown cancellation. Public §17.3 gates remain open,
    so the parent item intentionally stays unchecked.
  - [x] 2026-07-29 slice: retire the legacy CLI arbitrary-descendant
    permanent-delete authority instead of treating its temporary adapter as a
    compliant migration. The CLI remains a supported companion for scanning,
    navigation, selection, computed views, history, and reveal, while its
    `d` action, confirmation/progress modes, background delete workers,
    cache/tree post-delete mutation, and session delete accounting are absent.
    `dux-core::cleanup::legacy_cli`, its raw recursive-delete effects, and the
    three `legacy-adapter-delete-*` lint exceptions are removed. Repository
    policy rejects reintroducing the retired module/symbol family, and a
    fixture-backed input regression proves the former shortcut is inert and
    cannot alter the filesystem. Any future CLI cleanup must consume the same
    current, unexpired, reviewed-plan executor without accepting a caller path
    or restoring client-owned effects. This removes one noncompliant authority
    edge; it does not enable permanent-safe cleanup in Release or close the
    remaining §17.3 gates, so the parent item stays unchecked.

    Verification covers 34 CLI unit tests and 3 process-boundary CLI
    integration tests, including the fixture-backed inert-key regression;
    locked all-target workspace check and warnings-denied Clippy; all 29
    repository policy tests; and an authority scan of 242 repository source
    files. The full locked all-target workspace lane passed 1,074 tests with 2
    intentional ignores before reporting two unrelated concurrency-sensitive
    Rust-target safety failures; both exact regressions passed when rerun in
    isolation (including the 292.60-second quiescence case). Formatting and
    diff hygiene pass.
  - [x] 2026-07-29 slice: raise `developer.rust.target` to revision 3 and make
    inclusive seven-day inactivity part of deterministic cleanup authority.
    Evaluator revision 3 and context format 2 bind the exact persisted scheduled
    instant into the context digest, require complete file/directory
    modification-time coverage, emit an exact fourth `MinimumAge` fact, and
    reject recent, future, or missing required timestamps. Snapshot replay uses
    the retained scheduled instant, preserving exact-boundary and restart
    determinism. Planning, promotion, candidate
    coupling, plan review, and effect handoff accept only the current
    marker/age shape. Historical sealed revision-2 sessions remain decodable
    solely so recovery can terminalize them; current-only planner and effect
    gates prevent that compatibility from reviving authority.
    Before preview publication and again before `effect_started`, a shared
    descriptor-relative no-follow validator compares two complete bounded
    inventories and requires the target root, cache tag, and every descendant
    to remain at or before the fresh seven-day cutoff. The concrete driver
    repeats the inventory after effect admission and retains per-file
    identity/type/link-count/logical-size/mtime checks before unlink. Recent,
    future, missing, linked, special, symlinked, added, removed, or changed
    entries fail closed; pre-preview and pre-effect failures perform zero
    unlinks.
    UniFFI contract v33 transports every plan timestamp losslessly as
    seconds/nanoseconds plus the exact duration; Swift independently validates
    revision, duration, candidate evidence, and age before publishing the
    opaque child. Final materialization reruns the live subtree check after
    parent validation, and the concrete driver performs its final inventory
    after the last Cargo/read-set revalidation. Explorer adds the newest-change
    and required-inactivity facts
    to the plan card, accessibility summary, and internal destructive
    confirmation. Release execution remains disabled and the rule remains
    unschedulable.

    Verification: the focused Rust-target lane passed 89 tests, including the
    exact nanosecond boundary, replay, final-materialization leaf drift,
    pre-effect drift, historical revision-2 recovery fencing, and final driver
    inventory. The serialized all-target workspace run passed 1,057 tests with
    2 intentional ignores and exposed one discovery fixture that still created
    a fresh target; after the fixture was made explicitly eight days old and
    required to carry `MinimumAge`, its exact persistence/reopen regression
    passed. No non-test implementation changed after that full run. All 13
    projection integration tests and 64 ordinary FFI tests pass; the two
    intentionally ignored quiescent FFI cleanup regressions were also run
    through their exact process-boundary harness and passed independently.
    Locked all-target workspace check, warnings-denied Clippy, formatting, all
    29 repository policy tests, and the destructive-boundary scan of 242 source
    files pass. Universal Debug FFI/XCFramework generation produced arm64 and
    x86_64 slices, and the complete native Xcode scheme passed 401 tests with
    zero failures.
  - [x] 2026-07-29 slice: add the first production-core dry run through the
    exact Rust-target evaluator → reviewed-plan → validator → history chain.
    The engine accepts only the same consume-once opaque child used by the
    proposed permanent operation; no path, display DTO, candidate/plan ID,
    timestamp, approval Boolean, callback, AI result, command, or retry token
    can start it. Core consumes and projects the exact frozen plan to
    `DryRun`, preserving its item/action/estimate/expiry while recomputing the
    ordered dry-run warnings. The resulting non-cloneable capability has no
    approval, target export, capacity, journal-claim, driver, or effect-witness
    method.
    Permanent and dry-run paths share one inert internal observation validator
    for current plan shape, Cargo/read-set and process quiescence,
    account-home/mount/protected scope, target identity, seven-day complete
    subtree recency, manifest digest, and cache-tag evidence. Effect-witness
    conversion remains private to the permanent branch. A distinct serialized
    `RustTargetDryRun` task ignores the permanent-cleanup opt-in and closes a
    precise cancellation boundary before durable recording, so late requests
    are never reported as accepted. Under the cleanup lock, current user
    exclusions convert only an otherwise successful observation to durable
    rejection; cancellation, drift, and unavailable evidence remain distinct.
    Persistence atomically inserts an uncoupled, ownerless terminal graph; it
    never creates a candidate/trusted-rule claim, execution owner, heartbeat,
    effect-start receipt, capacity sample, removed-byte result, or platform
    call. Ambiguous writes retry only exact graph reconciliation, then release
    the non-authoritative lease and report history uncertainty without
    quarantining filesystem cleanup or inventing an unknown filesystem
    outcome.
    Focused domain/planner tests cover projection fidelity, warnings, exact
    coupling, expiry, marker and recency parity, and validation non-mutation.
    Focused journal regressions cover success/refusal terminal graphs,
    exclusion precedence, timestamp ordering, candidate immutability, zero
    effect/capacity fields, exact ambiguous-commit adoption, and unresolved
    metadata lease release. A lightweight engine regression covers the
    cancellation terminalization race. One serialized real stale-Cargo fixture
    ran the successful,
    exclusion-refused, cancellation-during-validation, and manifest-drift task
    paths in 797.72 seconds with the target unchanged; the committed regression
    retains the complete successful mutation detector while focused layers
    keep the ordinary suite bounded. At this core-only checkpoint the API had
    no UniFFI or Swift exposure and did not enable Release cleanup.
  - [x] 2026-07-29 slice: expose the production Rust-target dry run as a
    complete non-destructive native vertical through UniFFI contract v35.
    `DuxEngine.start_rust_target_dry_run` accepts only the exact engine-bound,
    consume-once opaque review and returns a distinct task; it has no path,
    candidate/plan/session identifier, mode flag, approval, driver, callback,
    capacity sampler, or permanent-policy input. Wrong-engine refusal occurs
    before consumption, while every owning-engine start attempt is one-shot
    and mutually exclusive with permanent cleanup. The path-free result
    contains only record version, a strict
    `cleanup:rust-target-dry-run:<32 lowercase hex>` correlation identifier,
    and a bounded terminal status. Separate start/task/failure/cancellation
    taxonomies preserve `HistoryUnresolved` as uncertain read-only metadata
    rather than permanent cleanup's unknown-effect state. Both Rust and Swift
    reject wrong task kinds, invalid identifiers/statuses, impossible
    phase/failure/result envelopes, regressing revisions, and cancellation
    rollback.

    Swift owns distinct dry-run DTOs, exact-handle controller consumption,
    generation-fenced observation, explicit cancellation, dismissal, and
    shutdown quiescence. The Explorer Release UI offers **Run dry check** from
    the exact ready preview, describes it as point-in-time validation, and
    always reports **No files changed · 0 B freed**. A terminal result explains
    that the preview was consumed and a fresh preview is required before any
    later dry run or cleanup. Permanent cleanup remains separately typed,
    confirmation-gated, and absent from Release behind
    `DUX_INTERNAL_PERMANENT_SAFE_CLEANUP`; the dry-run surface cannot chain or
    upgrade itself into an effect.

    Verification includes five focused UniFFI mapping/shape/consume-once
    regressions; the complete FFI package passed 69 tests with only the two
    intentional real-process cleanup fixtures ignored. Native boundary,
    controller replay, browser lifecycle, and exact-once history-refresh tests
    pass; the complete Debug Xcode scheme passed after an unrelated close
    timing test failed once and immediately passed in isolation and on the
    complete rerun. Locked all-target workspace check, warnings-denied Clippy,
    formatting, generated arm64/x86_64 bindings, and an unsigned universal
    Release build pass. The exact commit-stamped Release artifact is launched
    after this checkpoint is committed.
  - [x] 2026-08-09 slice: promote both real-process Rust-target cleanup FFI
    fixtures into a required, serialized macOS CI qualification lane instead
    of relying on ad hoc local execution. The bounded harness asks Cargo for
    the one exact `dux-ffi` test binary, waits for Cargo to exit, and directly
    invokes only the two fully qualified ignored tests once each. It rejects a
    missing, duplicated, filtered, ignored, or failed result. Disposable
    fixtures prove foreign/same-store rejection, owning-engine consume-once
    transfer, path-free task/history correlation, marker/manifest/lock/source
    preservation, terminal no-retry behavior, busy-refusal non-restoration,
    and close-time operation draining across the production Rust API layer of
    `dux-ffi`. Generated Swift/C ABI marshalling remains covered separately and
    is not claimed by this lane. A focused repository contract test keeps the
    CI step required, bounded to 15 minutes, stable under CI's forced terminal
    color, single-invocation, and absent from the ordinary parallel lane. The
    complete protocol and limits are recorded in
    [`docs/testing/permanent-safe-cleanup-qualification.md`](docs/testing/permanent-safe-cleanup-qualification.md).
    This closes one qualification gap only. Public Release cleanup remains
    disabled until prior-boot diagnostics, a real private vulnerability-
    reporting channel plus `SECURITY.md`, and the remaining §17.3 evidence are
    complete. Local qualification on macOS 26.5 passed both exact fixtures
    (2/2) through the same committed harness before this checkpoint.
  - [x] 2026-08-09 slice: add the bounded read-only prior-boot cleanup
    diagnostic required before Release permanent cleanup can be considered.
    ADR 0010 deliberately separates observation from the unresolved
    reconciliation policy. One synchronous core/UniFFI v57 call inspects at
    most 64 active `running`/`recovering` journal rows plus one fully validated
    lookahead through the existing recovery index. It independently partitions
    phase and stored execution provenance into exact aggregate counts, treating
    legitimate legacy/all-null ownership as stored-unproven and complete
    evidence without current OS context as unavailable-to-compare. Every row
    must pass the strict path-free scalar journal graph and provenance checks;
    malformed selected or lookahead data fails the whole response. A dedicated
    200-million-VM-instruction/10-second conjunctive SQLite budget covers the
    legal maximum 65 fully populated scalar graphs; exact maximum-shape and
    forced-exhaustion regressions prove valid availability, typed refusal, and
    progress-handler removal.

    The public record has no identity, owner, PID, time, digest, path, bytes,
    selector, liveness result, cursor, or opaque handle. The call performs no
    process probe, cleanup-lock/lease acquisition, claim, recovery, target
    validation, filesystem enumeration, planner, task, executor, or write, and
    a regression compares the complete SQLite mutable graph before and after.
    Swift repeats record-version, bound, truncation, phase, provenance,
    overflow, and current-context arithmetic before publication. Settings →
    Storage & Privacy presents lazy, generation-fenced **Unfinished cleanup
    bookkeeping** phase/provenance charts with redundant textual and VoiceOver
    labels, exact observation time, explicit read-only refresh, and retained
    earlier results after refresh failure. Copy states that this is neither a
    user-file inspection nor disk usage, reclaimable space, liveness, or
    cleanup permission, and the surface has no mutation action.

    This closes visibility only. Prior-boot, foreign-host, migrated, malformed,
    and Windows-unproven rows remain non-executable byte-for-byte no-ops; even
    same-host/current-boot is not a liveness fact. No reconciliation capability
    exists, the CLI/AI/pressure/schedule paths cannot consume the census, and
    `DUX_INTERNAL_PERMANENT_SAFE_CLEANUP` remains absent from public Release.

    Verification completed 2026-08-09 with the serialized locked all-target
    workspace (1,795 passed, five intentional ignores), the 65-session
    maximum-shape regression after its full warning population, warnings-denied
    workspace Clippy, workspace check and formatting, 126 UniFFI tests (two
    intentional ignores), all 707 native tests, all 85 repository contract
    tests, and the destructive-call boundary across 357 source files. Debug and
    Release binding generation is byte-identical. Unsigned universal Debug and
    Release app builds both contain `arm64` and `x86_64`; direct inspection of
    the Release executable finds the non-executable fallback and no permanent-
    cleanup action label. Two independent reviews found no remaining material
    correctness, lifecycle, privacy, accessibility, or query-budget issue.
  - [x] 2026-08-09 slice: resolve ADR 0010's release-policy fork by accepting
    diagnostic-only durable non-executability for v1 in ADR 0011. Prior-boot,
    foreign-host, migrated, malformed, stored-unproven, and unavailable-
    current-context cleanup rows retain their complete journal/candidate-claim
    graph indefinitely. No selector, liveness probe, claim, resume,
    reconciliation, terminalization, history clearing, retention, AI,
    pressure, notification, schedule, CLI, or filesystem edge may consume the
    diagnostic or mutate that debt. Same-host/current-boot remains only a
    provenance observation; the existing private recovery state machine still
    requires an exact OS `DefinitelyGone` result.

    The decision is availability-honest. Old debt does not populate the new
    process's cleanup quarantine and does not block a distinct, freshly
    reviewed cleanup session; even an overlapping current path must derive all
    authority and execution-time evidence again. The old candidate claim
    remains stranded and cannot be reused. Terminal cleanup-history clearing
    preserves the active graph. Whole-app-data reset deliberately remains
    fail-closed while any cleanup session is active/recovering, because reset
    cannot erase an unresolved effect journal. That future UX limitation is an
    explicit ADR reconsideration trigger, not permission to weaken the rule.

    A focused Rust composition regression leaves a prior-boot active graph and
    its candidate claim byte-for-byte unchanged, then creates and claims a
    distinct fresh candidate for the exact same target path. Repository policy
    tests freeze the accepted ADR/index/retention/security contract, absence of
    public reconciliation identifier families across UniFFI/native/CLI, and
    Debug-only permanent-cleanup compilation condition. Existing byte-for-byte
    journal regressions remain the executable evidence for foreign-host,
    missing/migrated provenance, and Windows-unproven no-ops. This policy
    decision adds no production code and does not enable public Release
    cleanup; private vulnerability reporting, signed-app destructive
    qualification, exposed-platform evidence, and the final separately
    reviewed Release gate remain open.

    Validation completed 2026-08-09 with all 73 cleanup-journal tests in the
    unrestricted macOS provenance lane, warnings-denied all-target core
    Clippy, all 88 repository policy tests, and the clean 357-source
    destructive-call audit. The first sandboxed journal run correctly lacked
    boot-scope evidence and failed the six provenance-dependent cases (five
    pre-existing plus the new composition regression); the canonical
    unrestricted rerun passed all 73. No production, FFI, Swift, binding,
    package, or build-setting input changed, so the immediately preceding
    universal Debug/Release qualification remains the applicable app artifact
    evidence.
  - [x] 2026-08-09 slice: prepare the repository-owned vulnerability-reporting
    policy without falsely claiming that a private channel already exists.
    Root [`SECURITY.md`](SECURITY.md) now defines the supported CLI/app states,
    examples of security impact, minimized and sanitized report contents,
    three-business-day acknowledgement and seven-business-day triage targets,
    fourteen-day active-report updates, coordinated disclosure, good-faith
    research boundaries, and the cleanup/update incident workflow. It rejects
    public exploit details, real user files, complete databases/snapshots,
    personal paths, credentials, and signing material by default.

    GitHub Private Vulnerability Reporting for the public repository was
    observed disabled during this checkpoint. The policy therefore labels the
    channel inactive and records the exact advisory URL, maintainer activation
    command, verification response, UI checks, and follow-up documentation
    update. A repository contract test freezes that fail-closed wording and
    prevents the accepted ADR 0011 decision from remaining listed as an open
    reconciliation choice. This local documentation checkpoint neither changes
    GitHub settings nor sends, publishes, or accepts a vulnerability report.
    The release gate remains open until a maintainer explicitly authorizes the
    external setting change, `"enabled": true` is verified, the private draft
    advisory flow is exercised, and the policy/roadmap status is updated.
  - [x] 2026-08-09 slice: add the repository-owned signed-app destructive
    qualification lane without exposing permanent cleanup in public Release.
    Accepted ADR 0012 defines a third, Release-optimized
    `CleanupQualification` configuration using the frozen production bundle
    and Team identity, reviewed empty entitlements, ordinary DUX product name,
    and the same native/Rust authority graph. It alone compiles both the
    existing internal permanent-safe action and a distinct non-shipping marker.
    Its display name is **DUX Cleanup Qualification**; signed Info.plist
    metadata binds protocol version 1 and the exact clean source commit; and an
    unavoidable warning appears in both the menu popover and Explorer.
    Debug remains the default Xcode configuration. Ordinary Debug and Release
    carry protocol 0/source `none`; Release contains neither compilation
    condition, retains display name **DUX**, and its byte-sealed packaging
    script rejects any qualification marker before and after signing.

    A clean-source builder creates one unsigned arm64/x86_64 app in a new
    canonical DerivedData directory, keeps Cargo output out of the repository,
    regenerates bindings/project files, rejects tracked or untracked drift,
    validates the bundled CLI metadata, and verifies the exact seven-Mach-O/
    Sparkle 2.9.5 shape. A separate read-only verifier accepts
    only `/Applications/DUX Cleanup Qualification.app` and checks exact
    metadata, source/version/build, universal inventory, macOS 14 deployment,
    Developer ID/Team/runtime/timestamps, signed CLI metadata, empty
    entitlements on every code object, production designated requirement,
    dormant signed-feed policy, notarization staple, and Gatekeeper before
    emitting nine path-free identity lines including the independently checked
    final private-archive SHA-256. Neither
    script signs, notarizes, installs, launches, uploads, or invokes cleanup.

    The supervised protocol requires one immutable final archive of the exact
    same signed/notarized/stapled bytes on
    an Intel Mac and Apple Silicon Mac, each under a fresh disposable account
    with AI and Full Disk Access disabled. It separately proves real Explorer
    Trash plus Finder Put Back, the zero-effect dry check, and one exact
    permanent-safe Rust-target removal with marker/project preservation,
    terminal history correlation, and no retry. No test-only path, effect API,
    approval Boolean, command, or UI driver was added. The local universal
    qualification build proves the repository configuration and unsigned app
    shape only. Developer ID/notary credential use, installation, real
    disposable effects, two-host evidence, private vulnerability-reporting
    activation, exposed-platform evidence, and the final Release enablement
    review remain open §17.3 gates; qualification bytes can never be a public
    artifact or update enclosure.
- [x] Implement execution-time revalidation. Completed 2026-08-09: retained
  rule, Cargo, process, home/mount, plan, journal, policy, exclusion, approval,
  and path evidence is rechecked before effect admission and within the driver.
  The real nested-APFS qualification additionally proves that replacing a
  reviewed same-mount directory with a different mounted filesystem invalidates
  the witness before an effect receipt can exist.
  - [x] 2026-07-19 slice: make the approved-session handoff use one canonical
    millisecond start time for persistence and journal claiming, and recheck
    the journal-owned `validating` path after durable validation while every
    other path remains `planned`. This closes the sub-millisecond claim gap
    and prevents target switching between the validation write and live
    witness capture; broader production orchestration remains open.
  - [x] 2026-07-19 slice: add a private pre-effect revalidation witness on
    the approved journal session. It rechecks approval expiry, every retained
    trusted rule-scope grant, and the exact frozen journal plan immediately
    before a future permanent-safe driver boundary. It returns no target or
    effect capability; per-path identity admission and the executor remain
    open.
- [x] Implement cleanup session/item history. Completed by the schema-v12/v14
  claim lifecycle and the later path-free summary, exact-session, outcome,
  capacity, annotation, clearing, and native drill-down slices below.
  - [x] 2026-07-19 slice: join the approved trusted-plan boundary to the
    existing bounded planned-session persistence API. The capability
    revalidates expiry and every retained rule-scope grant immediately before
    writing the session, then persists only frozen history/journal data. The
    stored row does not retain approval authority and cannot invoke an effect;
    engine orchestration, terminal outcomes, capacity verification, and UI
    history remain open.
  - [x] 2026-07-19 slice: add the private approved-plan → journal handoff.
    The handoff revalidates the expiring approval, persists one bounded
    planned session, holds the cleanup lock, compares the frozen plan before
    claiming and again under the owner/generation fence, then returns a
    non-cloneable session containing the approved capability and journal claim.
    It exposes no path, callback, FFI, schedule, or filesystem effect; the
    permanent-safe executor and terminal transitions remain open.
  - [x] 2026-07-19 slice: bridge the typed Rust-target facts and journal
    request into the existing private permanent-safe session executor. The
    engine performs the final open-lifecycle check, claims the planned session
    through the canonical journal handoff, and passes only that non-cloneable
    claimed session to the descriptor-relative executor. No caller path,
    callback, AI output, CLI request, FFI value, or UI route can enter this
    bridge; production evaluator acquisition and orchestration remain next.
  - [x] 2026-07-27 slice: add macOS end-to-end regression coverage for the
    private facts-to-executor bridge. A temporary, test-owned trusted-rule
    fixture now traverses durable candidate acquisition, a planner-owned
    `RustTargetPlanFacts` capability, approval, planned-session persistence,
    owner/generation-fenced claim, and descriptor-relative execution; the
    payload is removed, `CACHEDIR.TAG` is retained, and completed history is
    observed. At this checkpoint the production
    Cargo/home/protected-root/process/descendant grant chain remained covered
    by dedicated planner tests; the 2026-07-28 integration slice below replaces
    that shortcut with one real production-chain regression. A second fixture
    mutates the target contents after facts acquisition and proves the handoff
    rejects the path read-set drift before writing a planned journal row or
    touching the target. Cargo manifest revalidation was still separate at this
    checkpoint. This remains private test evidence: no FFI, Swift, CLI,
    scheduler, AI, or production cleanup route is enabled.
  - [x] 2026-07-28 slice: close the durable candidate-claim mismatch for the
    trusted Rust-target path without clearing or deleting its discovery
    blocker. The private reviewed-plan handoff now mints an insertion-only
    coupling variant for exactly one revision-2 `developer.rust.target`
    permanent-safe item. Persistence still requires the complete immutable
    source-scan, candidate, rule, path, byte, modification-time, evidence,
    safety, action, schedule, and review-state body to match, then moves the
    candidate and claim atomically while retaining `ProtectedPath` in candidate
    history. This checkpoint initially stored the variant as the existing
    plan-claims format; the immediately following schema-v12 slice makes its
    active reopen identity explicit. Ordinary plan claims cannot mint the
    variant, every other blocked candidate remains rejected, and a focused
    rollback regression proves no session or claim is left behind. The macOS
    facts-to-executor fixture now exercises this exact retained-blocker route
    rather than deleting the blocker from its database. No FFI, Swift, CLI,
    scheduler, AI, or production cleanup route is enabled.
  - [x] 2026-07-28 slice: preserve that private trusted coupling across an
    active journal reopen without inferring authority from the rule-shaped
    candidate alone. Checksummed schema v12 adds a revisioned seal bound to the
    exact candidate, session, and item ordinal. Planned, running, and
    recovering decoders reconstruct the retained-blocker coupling only when
    the seal, candidate claim, frozen one-item/one-path permanent-safe plan,
    complete immutable candidate body, and sole `ProtectedPath` blocker all
    still agree. Ordinary Rust-shaped claims must retain an empty blocker set;
    forged or moved seals, missing seals, extra blockers, and unsealed pre-v12
    trusted recovery fail closed. Migration creates an empty seal relation and
    fabricates no trust. The existing atomic candidate-claim settlement
    cascades the seal away for terminal history. Reopen, active-recovery,
    tamper, migration, and completed-execution regressions pass; no public
    cleanup caller or new filesystem effect was added.
- [x] Implement pre/post capacity verification.
  - [x] 2026-07-19 slice: add a private bounded verifier for cleanup capacity
    witnesses. It requires a stable volume identity, ordered effect window,
    pre/post samples inside a 15-minute skew bound, unchanged total capacity,
    unchanged headline source, and unchanged ordinary/important availability
    shape. It records only a signed headline-available delta (positive means
    more available space); missing, stale, mismatched, or unrepresentable
    evidence remains unknown rather than becoming zero or an estimate. A
    private journal adapter accepts only this verified value. No executor, UI,
    FFI, or production cleanup session calls the boundary yet.
  - [x] 2026-07-19 slice: integrate the verifier with the crate-private
    permanent-safe session boundary. A core-owned sampler supplies one
    pre-effect and one post-settlement observation; only stable, bounded,
    matching evidence persists a signed available-space delta in cleanup
    history. Missing, stale, changed-volume, source-shape, and unknown-outcome
    cases retain a null delta, while outcome-unknown sessions remain in
    recovery and are never terminalized. The sampler and capacity-aware bridge
    remain private until trusted volume grants and production observation
    wiring exist.
  - [x] 2026-07-28 slice: bind production sampling to the exact authorized
    target volume and real effect window. Every trusted rule-scope
    authorization can now yield only its revalidated, crate-private kernel
    filesystem ID, mount location, platform mount discriminator (mount path on
    macOS; mount ID where available), and filesystem type; an approved
    multi-path session requires every authorization to resolve to that same
    scope. The macOS sampler is constructed inside the engine from that scope
    and accepts a `statfs` result only when filesystem ID, mount path, and type
    still match. No caller path, volume label, stable-ID DTO, or capacity value
    enters the production bridge. Pre-sampling completes before a fresh
    effect-start timestamp and post-sampling begins after a fresh
    effect-completion timestamp, so the journal's canonical transition time
    cannot fabricate a verification window. The executor also reads the
    authority clock again after pre-sampling, immediately before every live
    effect witness is rebuilt; an approval expiring during telemetry
    terminalizes as rejected without mutation. Missing samples, scope drift,
    total/source/shape changes, skew, and signed overflow persist `NULL`, never
    zero or an estimate. The real enrolled-Cargo → trusted plan → journal →
    descriptor-relative executor regression now proves that terminal history
    receives a signed sample from the exact target volume. The separately
    exposed v28 plan review remains observation/release only; no FFI, Swift,
    CLI, scheduler, AI, approval, or app execution route was added. Focused
    scope, wrong-filesystem-ID, missing-sample, synthetic-clock,
    expiry-during-sampling, and real production-chain regressions pass. The
    full serial core lane passed 1,014 tests; four pre-existing Cargo
    manifest-probe availability/drift cases failed under the 26-minute
    accumulated load, and each exact failure passed immediately when rerun
    alone (0.87 s, 0.50 s, 145.92 s, and 103.41 s respectively). The
    destructive boundary's 28 unit tests and 241-file repository scan also
    pass, as do all 376 native tests and fresh universal arm64/x86_64 Debug and
    Release builds targeting macOS 14.
- [x] Implement exclusions and global permanent-cleanup disable setting.
  Completed 2026-07-29: Settings exposes the bounded path-bearing deny list and
  explicit opt-in policy, while dry-run and final pre-effect admission reread
  the authoritative state. V1 intentionally treats exclusions as deny-only;
  they need not become planner authority or an earlier positive recommendation.
  - [x] 2026-07-29 slice: make the global permanent-cleanup gate an explicit
    opt-in before any Release exposure. Rowless state and reset are now
    core-owned `Default(false)`; only durable `Stored(true)` consent can admit
    a permanent-safe effect. Value schema v2 decodes schema-v1 `Stored(true)`
    and `Stored(false)` as prior explicit choices, but strengthens the legacy
    `Default(true)` epoch to disabled and rewrites it canonically on reset.
    Malformed and newer values still fail closed. The claimed executor observes
    the gate under the settings cleanup exclusion, records a disabled path as
    durably Rejected before any effect receipt, and the final journal transition
    rechecks the gate; an end-to-end reviewed Rust-target regression proves zero
    unlinks and zero removed bytes. UniFFI contract v34 rejects impossible
    `Default(true)` projections. EngineService accepts only disabled Default
    shapes, AppModel requires a loaded authoritative disabled policy plus the
    exact `ENABLE PERMANENT CLEANUP` phrase before the sole product enable
    call, and reset immediately restores the safe disabled default without
    confirmation. Settings copy, neutral safe status, VoiceOver identifiers,
    and an exhaustive Swift/Rust production setter-call-graph regression
    reflect those semantics. The Debug-only permanent-safe action and Release
    gate remain unchanged. Verification includes workspace format/check/clippy,
    the focused migration and journal suites, isolated real reviewed-task deny
    and explicitly enabled success paths, all 64 ordinary UniFFI tests plus both
    dedicated macOS cleanup harness cases, 31 repository policy tests, the
    243-file destructive-call scan, all 403 native tests, and universal
    arm64/x86_64 Debug bindings and Release app builds targeting macOS 14. The
    independent adversarial review found no remaining blocker.
  - [x] Original 2026-07-19 slice (default semantics superseded above): add the
    revisioned global permanent-cleanup kill switch to the typed settings store.
    Missing state initially defaulted to enabled;
    explicit disable/enable/reset operations preserve provenance and monotonic
    revisions, while malformed or newer values fail closed. Setting writes
    take the store-wide cleanup exclusion, and the journal checks the effective
    value while holding that same exclusion immediately before `effect_started`.
    Existing rule-relative protected/excluded descendant evidence remains
    private, bounded, identity-revalidated, and non-authoritative. A bounded
    lossless user exclusion set now stores exact lexical path prefixes behind a
    typed settings key; the journal rechecks it under the cleanup lock before
    `effect_started`, so it can only deny an effect. The Swift settings surface
    now loads and presents this path-free state through EngineService/AppModel,
    applies disable immediately, and requires the exact phrase
    `ENABLE PERMANENT CLEANUP` before re-enable or reset could restore the
    then-enabled default. Path-bearing exclusion presentation remained separate.
  - [x] Original 2026-07-19 slice (UI semantics superseded above): expose the
    path-free global permanent-cleanup kill
    switch through UniFFI contract v22. Versioned get/set/reset records carry
    only enabled state, Default/Stored provenance, monotonic revision, and
    optional update time; Rust remains the semantic validator and the switch
    can only deny effects. Closed engines, malformed state, storage failures,
    and write uncertainty map to typed errors. The generated Swift bindings
    were regenerated from the universal Debug XCFramework. Swift EngineService
    maps every typed error and rejects malformed status shapes; AppModel
    generation-fences load/mutation/reset work and invalidates it during ordered
    shutdown. Settings exposes the deny-only gate with VoiceOver identifiers and
    an exact typed confirmation for the then-current re-enable/reset flow.
    Exclusions remain a
    separate path-bearing presentation boundary.
  - [x] 2026-07-19 slice: expose the bounded lexical exclusion set through
    UniFFI contract v23. Get/set/reset carry exact path bytes with Unix or
    UTF-16 encoding, source/revision/timestamp, and a changed flag. FFI
    rejects wrong record versions, unsupported encodings, relative/control/
    parent paths, oversized paths, oversized sets, malformed core shapes, and
    non-canonical ordering. The native Settings surface presents the paths as
    lossless observations, lets users add a local file/folder prefix, and
    requires explicit confirmation before removing one or resetting all.
    Exclusions remain deny-only and never become plan or executor authority.
- [x] Add partial failure, retry, cancellation, and changed-since-plan UI.
  Completed 2026-07-29 with path-free phase/progress, explicit cancellation,
  changed-since-plan refusal, terminal partial/failed/unknown outcomes, and
  history routing. “Retry” is resolved as a safety requirement: no effect retry
  exists after a consume-once capability or unknown outcome; the user must
  start from a fresh scan and review. Live per-item progress remains optional
  presentation work and is not authority or a Milestone 5 exit gate.
  - [x] 2026-07-19 slice: preserve changed-since-plan as a distinct typed
    outcome for the reviewed Explorer Trash path. Core journal validation now
    maps a target identity change to `ChangedSincePlan`; UniFFI contract v24
    carries that bounded error without paths; Swift presents an explicit
    rescan-before-retry message. Existing one-shot/unknown-outcome behavior
    remains conservative: no automatic retry or effect is attempted.
  - [x] 2026-07-19 slice: expose a bounded read-only cleanup-history summary
    feed through UniFFI contract v25 and `EngineService`. The newest-first
    keyset cursor, session lifecycle, mode/trigger, estimates, optional
    verified capacity delta, cancellation observation, and bounded item/path
    status counts are mapped into strict Swift presentation models. Record,
    token, time, ordering, count, and legacy/complete-shape validation fails
    closed; no paths, evidence, plans, approvals, or executor authority cross
    the boundary. Exact-session detail, live partial-progress controls, and
    cleanup UI remain open.
  - [x] 2026-07-27 slice: expose scan-bound candidate review intent through
    UniFFI contract v26 and the native EngineService boundary. The four
    commands (`Select`, `ClearSelection`, `Dismiss`, `Restore`) accept only a
    candidate ID already held by an exact review lease; Swift validates record
    version, scan ID, and candidate ID before publishing the returned status.
    Not-reviewable candidates remain typed failures. No path, plan, approval,
    schedule, AI input, or executor capability crosses this endpoint; the
    candidate presentation controls and plan lifecycle remain the next UI
    slice.
  - [x] 2026-07-29 slice: add internal permanent-safe changed-since-plan,
    progress, partial/failed/unknown result, explicit cancellation, and
    terminal presentation through the v31 path-free task. There is deliberately
    no retry control; the consumed capability cannot be reconstructed, and an
    unknown outcome directs the user to durable Cleanup History. Ordinary
    Explorer dismissal does not imply cancellation, while app shutdown
    explicitly cancels and waits. The action remains Debug-only until §17.3 is
    complete, so broader release UI keeps this parent open.
- [x] Add bounded pending-evaluation discovery and snapshot-backed restart
  recovery; malformed or incompatible state fails closed.
  - [x] 2026-07-19 slice: add a bounded oldest-first pending-evaluation
    discovery sentinel and a core recovery seam that replays only the exact
    retained immutable snapshot. The request rechecks scan/snapshot identity,
    evaluator/catalog/context digests, snapshot metadata, replay identity, and
    candidate materialization limits before atomically completing the pending
    row. Catalog/context drift becomes a typed terminal discovery failure;
    malformed, missing, or unavailable snapshot state never becomes replay or
    cleanup authority. The result is path-free and reports only recovered count
    plus a bounded `has_more` hint; startup scheduling and FFI/Swift wiring
    remain the next orchestration slice.
  - [x] 2026-07-29 slice: schedule that seam as the eighth idle-only native
    maintenance task through UniFFI contract v32. Production admission accepts
    no caller path, scan/snapshot/candidate identity, clock, evaluator/catalog
    input, AI output, plan, approval, or command; Rust samples time inside the
    worker and processes at most one oldest row from the exact retained
    immutable snapshot. The former direct caller-clock recovery method and its
    public replay result/error types are removed, leaving the admitted task as
    the only production entry point. Applying linearizes cancellation, late
    cancellation or close preserves the exact result, invalid clocks are typed,
    malformed or missing state fails closed, and `has_more` remains only a
    bounded hint because core never self-enqueues. FFI exposes only `None`,
    `Recovered`, or `Incompatible`, recovered candidate count, timestamp, and
    `has_more`, and validates task kind plus phase/failure/result shape. Swift
    independently validates all eight maintenance result families and requests
    candidate recovery immediately after abandoned-scan recovery while
    preserving the 60-second startup grace, energy gates, fair cadence,
    backoffs, and ordered
    shutdown cancellation. Evidence includes 10 focused core recovery tests,
    all 65 FFI tests (63 passed, two intentionally isolated), all 398 linked
    native tests, all 29 destructive-policy tests, and a clean 242-file
    destructive-call scan. Workspace format, check, and warning-denied Clippy
    gates pass. The broad parallel Rust lane passed the CLI suites but reported
    three existing load-sensitive core timing/availability failures; each exact
    failed regression passed independently (one 220-second registry case and
    two subsecond planner cases). Universal arm64/x86_64 Debug and Release app
    builds and the generated Release FFI archive pass with macOS 14.0 minimum;
    Release contains neither the internal permanent-cleanup compilation
    condition nor its action string.
- [x] Wire deterministic evaluator → reviewed plan → journal → executor through
  the core engine and FFI/Swift while preserving generation, cancellation,
  recovery, and outcome-unknown fencing. Completed locally 2026-08-09; public
  Release exposure remains intentionally gated by §17.3 qualification.
  - [x] 2026-07-19 slice: add the first production-core evaluator → planner
    acquisition boundary for the staged Rust-target rule. It loads only the
    exact succeeded scan/evaluation/source record, rehydrates the domain
    candidate from the current bundled catalog, compares every immutable body
    field back to durable history, and acquires a fresh lease-backed live
    witness. The EngineHandle entry point is crate-private and stops before
    Cargo/protected-root grants, plans, approval, journal, FFI, UI, scheduling,
    AI, or effects.
  - [x] 2026-07-19 slice: consume that live input through the enrolled-Cargo
    metadata, current-account home/mount, protected-root, process-quiescence,
    and empty-descendant-policy boundaries. The private EngineHandle join
    returns only the non-cloneable Rust-target promotion token, which retains
    the unresolved `ProtectedPath` blocker and still cannot construct a plan,
    approve, claim a journal, cross FFI, schedule, invoke AI, or mutate.
  - [x] 2026-07-19 slice: consume the private promotion with its retained
    canonical scan-root/target witnesses into `RustTargetPlanFacts`. The
    facts capability repeats grant validation and remains path-private and
    non-cloneable; plan IDs, review, approval, journal, FFI, UI, scheduling,
    AI, and filesystem effects are still handled only by later boundaries.
  - [x] 2026-07-28 slice: join the real production Rust-target acquisition
    chain to the reviewed-plan, schema-v12 journal claim, and deterministic
    executor in one macOS regression. The fixture explicitly enrolls the exact
    direct Cargo executable, scans a real one-package workspace, and calls
    `EngineHandle::prepare_rust_target_plan_facts`; the former test-only facts
    constructor and Cargo-revalidation bypass have been removed. The first
    full run exposed that the journal's legitimate atomic `Discovered` →
    `Planned` candidate transition invalidated the retained discovery witness.
    The source is now rebound after sealed planned-session insertion but before
    the owner/generation claim, and only when the exact
    session/item/candidate, trusted coupling seal, complete frozen plan,
    immutable candidate body, sole `ProtectedPath` blocker, scan, evaluation,
    and snapshot reference still agree. A binding failure therefore cannot
    abandon an active owner. Every later revalidation repeats that joined
    check against the now-active claim. The success path removes only target
    contents and preserves
    `CACHEDIR.TAG`, `Cargo.toml`, `Cargo.lock`, and source, settles
    candidate/history state, and leaves no active claim or seal. A companion
    manifest-drift run fails before journal insertion and leaves the target
    untouched. No FFI, Swift, CLI, scheduler, AI, or app cleanup route is
    enabled; generation-fenced product orchestration remains open.
  - [x] 2026-07-28 slice: expose the first production-chain reviewed-plan
    observation without exposing cleanup authority. An exact active Explorer
    snapshot review plus one candidate ID can enter the deterministic
    Rust-target acquisition chain; Rust alone derives the source scan, current
    target, permanent-safe mode, plan identity, warnings, estimate, and frozen
    effective expiry. The non-cloneable core review retains the real reviewed
    plan but publicly supports only bounded observation and consuming release.
    Admission, expensive acquisition, exact-parent post-validation, plan
    materialization, and final publication are split so neither the parent
    review mutex nor the FFI engine-state mutex is held across live Cargo,
    filesystem, or reviewed-plan revalidation. A per-parent session identity
    and liveness token reject same-scan substitution, explicit release, drop,
    and expiry. This endpoint creates no approval, durable plan/session row,
    candidate transition, journal claim, schedule, callback, AI request, or
    filesystem effect; the broader evaluator → approval → journal → executor
    product orchestration remains open.
  - [x] 2026-07-29 slice: complete the first internal Swift orchestration edge
    from the exact immutable reviewed child to the core-owned v31 task. The
    controller, not a display DTO, owns the child; full-record equality and
    parent generation are rechecked before consume-once transfer. Browser
    confirmation and observation are separately generation-fenced,
    cancellation is explicit, unknown outcomes never retry, and app shutdown
    quiesces the observer. This is compiled out of the public Release action
    surface, so §17.3 and later production orchestration keep the parent
    unchecked.
- [x] Expose review intent, plan lifecycle, and path-free cleanup history through
  FFI/Swift UI without turning history into planner authority.
  Completed 2026-08-09 through the bounded candidate review, exact drill-down,
  consume-once plan preview/task, global task card, summary and exact-session
  history, terminal correlation, annotations, and clearing surfaces below.
  - [x] 2026-07-19 slice: add the first app-facing history boundary as a
    bounded summary page and cursor. `EngineService` reads only path-free
    durable outcome metadata and returns typed errors; Swift rejects malformed
    records before they reach presentation. This is intentionally not an
    executor, planner, approval, or exact-session-detail API.
  - [x] 2026-07-19 slice: consume that boundary in the native Explorer.
    AppModel loads and generation-fences the history page, supports bounded
    cursor pagination, and invalidates reads during shutdown. Explorer adds an
    accessible Cleanup History destination with empty/error/loading states,
    outcome rows, and a compact color-coded item-status distribution bar.
    The screen is explicitly read-only: it offers no retry, approval, plan,
    path, or cleanup action; exact-session detail remains a later slice.
  - [x] 2026-07-27 slice: add the native Explorer Candidates view. The view
    loads one bounded page through the retained exact review lease, shows
    deterministic rule/category/observed-size/safety/status facts without
    paths, and generation-fences reloads and review transitions. Select,
    clear-selection, dismiss, and restore are explicit review intent only;
    blocked candidates remain typed failures and every confirmation states
    that no cleanup action was performed. Plan lifecycle, exact candidate
    detail, and effect controls remain separate gates.
  - [x] 2026-07-28 slice: add exact candidate drill-down to the native
    Explorer without widening cleanup authority. Selecting one confirmed
    summary loads its first bounded historical-path and deterministic-evidence
    pages concurrently through the same retained scan review; independent
    previous/next controls replace one 64-row page at a time. The model checks
    scan ID, candidate ID, every immutable candidate field, cursor, total,
    count, and next cursor again above the strict FFI adapter, generation-fences
    snapshot/mode/selection/page changes, discards late results, and releases
    the complete review when detail reports expiry. Direct task cancellation
    clears current loading/paging latches without accepting a reply. FFI and
    Swift both enforce the 64-KiB encoded-path, 256-KiB display-path, and
    24-MiB aggregate-page ceilings before projection. The accessible inspector
    shows category, observed size/time, safety, proposed action, status,
    schedule eligibility, blockers, lossless display paths, precise typed
    evidence, visible page status, and named row inspection, with explicit
    historical/read-only/no-AI/no-plan copy. UniFFI remains v26; Swift now
    expects that current contract, exhaustively maps its candidate review
    error, and returns typed cleanup-history closed-state errors. Focused
    malformed-response, cancellation, candidate-switch, expiry, and 65-row
    pagination regressions, the full macOS test suite, and universal
    arm64/x86_64 Debug and Release builds pass.
  - [x] 2026-07-28 slice: add a review-only Rust-target plan preview through
    UniFFI contract v28 and the native Explorer. The FFI request contains only
    the candidate ID and an opaque exact parent review; one engine admits at
    most one preparation or published child review. The child exposes only
    `info` and idempotent `release`, is frozen to the shorter of its reviewed
    authority and exact parent deadlines, and is reaped on child expiry,
    parent release/drop/expiry, engine close, or terminal evidence drift.
    Expensive preparation and information reads are operation-tracked without
    holding registry/session locks, publication rechecks close and exact-parent
    state, and shutdown remains bounded even when an operation or lease release
    is still completing. Exact current paths cross only as bounded lossless
    bytes plus a byte-derived display; controls, Unicode-16 format/default-
    ignorable scalars, non-UTF-8 bytes, and backslashes are escaped, while
    Swift compares the display as UTF-8 bytes to prevent normalization
    substitution. App-owned adapters independently require the exact rule,
    revision, candidate body, one-item/one-path shape, warning order, current
    short lifetime, normalized terminal `target`, and scan/candidate
    correlation. The controller owns every opaque child, releases children
    before parents, and generation-fences preparation, refresh, candidate,
    mode, snapshot, expiry, and shutdown races. Explorer presents the current
    target, estimate, warnings, and expiry in an accessible
    **Permanent-safe plan preview** with only prepare/check-again/close
    controls and explicit “not approved; no files changed” copy. It performs
    deterministic discovery and explanation only: the preview child exposes
    no approval, permanent cleanup, journal, schedule, callback, CLI, AI, or
    executor authority and cannot be supplied to the existing separately
    confirmed Explorer Trash route. No plan, approval, schedule, AI input, or
    executor capability is minted from the preview.
    Verification covers 5 focused core lifecycle/path tests, all 49 FFI unit
    tests, 129 focused native adapter/model/browser/controller tests, 21
    destructive-boundary tests plus the 241-file source scan, deterministic
    Debug/Release binding generation, and universal arm64/x86_64 Debug and
    Release app builds targeting macOS 14.
  - [x] 2026-07-29 slice: add the internal native confirmation and path-free
    observation layer for the exact v31 task. The accessible destructive
    confirmation repeats the exact target, unverified estimate, preserved
    marker/target directory, and irreversibility; a persistent global banner
    survives candidate/window transitions and exposes cancellation plus honest
    aggregate/capacity/history correlation. Release retains an unavailable
    execution label because the explicit Debug-only compilation condition is
    checked again by release automation. History and display values remain
    observations and cannot mint or retry cleanup authority. Verification
    covers 122 focused and 394 full native tests, Rust formatting/check/clippy,
    all 62 active FFI unit tests plus the isolated exact-success and
    refusal/close-drain regressions, 29 Python repository tests, and the
    246-source destructive-call scan. Debug and Release binding generation is
    deterministic; the Debug and Release app executables and FFI archive are
    universal arm64/x86_64 and target macOS 14. Resolved Release settings omit
    the internal condition, and the Release executable contains no destructive
    action label.
  - [x] 2026-08-09 native readiness correction: make the durable global
    permanent-cleanup policy part of shared AppModel startup instead of a
    Settings-view side effect, coalescing concurrent scene loads and treating
    missing, failed, mutating, or impossible Default-enabled policy as
    unavailable. The Debug-only action now routes disabled/unavailable state
    to Settings, clears an open confirmation if consent changes, and rechecks
    authoritative Stored-enabled presentation state immediately before handing
    the already opaque review to the controller; Rust remains the sole effect
    authority and repeats its durable gate under the cleanup exclusion. The
    path-free cleanup task card is lifted above `NavigationSplitView`, so
    starting/progress/cancellation/terminal state survives every Explorer
    destination without lifting the target or review handle. Cancelled terminal
    polls retain honest removed-byte/count/capacity aggregates and their
    session correlation. Correlated history navigation forces and joins the
    current bounded first-page refresh, refuses stale cached rows after a
    refresh failure, validates the exact session before selection, and
    generation-fences rapid routes as last-writer-wins; missing correlation
    opens only the read-only list. Terminal dismissal is withheld while the
    runtime-owned history refresh is still finalizing. Focused Swift adapter,
    AppModel, browser, accessibility, concurrency, and repository-boundary
    regressions cover these rules. Verification passes all 698 linked native
    tests, all 80 repository policy tests, Rust formatting, locked workspace
    check, warning-denied Clippy, and the clean 354-source destructive-call
    audit. Unsigned Debug and Release apps build universal arm64/x86_64;
    resolved Release settings omit the internal condition and its executable
    contains no destructive action label. Public Release permanent cleanup
    remains disabled pending §17.3 and the milestone exit gates.
- [x] Expand production cleanup session/item history with exact-session detail,
  verified capacity outcomes, and separately confirmed history clearing.
  - [x] 2026-07-28 slice: expose the existing core exact-session observation
    through UniFFI contract v29 without widening cleanup authority. The request
    contains only a versioned, bounded, path-free session ID copied from recent
    summary history. Rust loads and fully validates the bounded durable graph;
    FFI returns one independently validated session summary, ordered item
    summaries, and ordered warnings while preserving verified capacity delta,
    all typed session/item statuses, stable error categories, complete-versus-
    legacy policy shape, and exact aggregate counts. Paths, evidence payloads,
    candidate IDs, execution owner/generation, claims, receipts, and all clear,
    retry, recovery, approval, scheduler, AI, and executor operations remain
    absent. Swift independently repeats the complete graph, lifecycle,
    duplicate-ID, checked estimate-sum, aggregate, and derived-warning
    validation; AppModel generation-fences selection, refresh, read retry,
    close, and shutdown; and Cleanup History now drills
    into lifecycle timing, mode/trigger, distinct estimate versus signed
    verified-capacity outcome, textual/accessibility-backed item and path
    charts, warnings, and ordered rule/item cards. The only retry control reads
    history again and cannot repeat cleanup. Verification covers all 51 FFI
    tests plus the full 376-test native suite. The workspace-wide Rust lane
    passed 1,015 core tests before one load-sensitive probe-pool case observed
    `QueueSaturated`; that exact case passed immediately in isolation.
  - [x] 2026-07-28 slice: add separately confirmed terminal cleanup-history
    clearing through UniFFI contract v30 and native Settings. The public
    boundary accepts no row, session, candidate, path, plan, approval, AI
    result, cleanup instruction, or effect; it returns one two-minute,
    monotonic-expiring, engine/store-bound, consume-once preview containing
    only the exact terminal-session count and oldest/newest start times. Rust
    validates and SHA-256 fingerprints raw rows across the five cleanup
    history tables in 64-session keyset pages with fixed per-page/per-session
    budgets. Active/recovering and outcome-unknown evidence plus its claims is
    preserved; a terminal graph with unfinished items or live claims fails
    closed. Commit consumes the preview, recomputes the witness under cleanup
    exclusion and an immediate transaction, and installs an authorizer that
    permits only child-first deletion from those five tables. Changed history
    rejects before mutation; post-commit observation returns success only for
    proven applied, retains the original error only for proven not-applied,
    and maps every ambiguous or failed reconciliation to outcome-unknown
    without retry.
    Native Settings shows the exact count/date range, requires destructive
    confirmation, releases unconfirmed authority on dismissal, fences
    concurrent history reads and stale replies, waits through confirmed
    clearing during shutdown, and performs exactly one observation-only
    refresh after every terminal response. Its copy states that the action
    deletes only DUX activity metadata, performs no file cleanup, database
    compaction, or capacity resample, deletes no snapshots/scans/candidates/
    settings/exclusions/capacity samples/AI insights, and promises no free
    space. Explorer Trash now terminalizes known outcomes only after the OS
    callback returns, records capacity delta as unknown, and leaves
    outcome-unknown effects recoverable. Focused verification covers 12 core
    clear-history tests, the post-effect timestamp regression, 7 FFI lifecycle/
    race/projection tests, 11 strict-concurrency native tests, the full
    387-test native suite, and universal arm64/x86_64 Debug and Release app
    builds targeting macOS 14. The locked all-target Rust workspace lane
    passed 1,168 tests with 2 intentional ignores and no failures; formatting
    and warnings-denied Clippy pass, both fuzz adapters compile, all 28
    destructive-policy tests pass, and the authority scan accepts all 244
    repository source files.
- [x] Define and test cross-reboot and Windows-unproven cleanup-journal recovery;
  unknown ownership must remain non-executable.
  - [x] 2026-07-28 slice: define recovery across a changed boot and on
    Windows without qualified host/boot evidence as refusal, not resumable
    cleanup. The existing version-1 owner identity deliberately hashes macOS
    boot-session or Linux boot/PID-namespace scope as one opaque value, so a
    changed boot is indistinguishable from observing a copied database on a
    foreign host. Both remain `Unknown`. Windows can prove only an exact live
    PID/start-token match; process exit, PID reuse, absence, access failure,
    and changed start remain `Unknown` because its owner is unscoped. PID,
    heartbeat age, cleanup-lock availability, and plan timestamps never
    substitute for that proof.

    `CleanupJournalLease::try_recover` returns `LivenessUnknown` for those
    cases, issues no `CleanupJournalClaim`, increments no generation, and
    changes no session, item, path, ordinary candidate claim, or trusted
    Rust-target claim byte. Therefore no validation, resume, effect receipt,
    callback, or filesystem mutation is reachable. Only same-reliable-scope
    `DefinitelyGone` retains the existing exact-CAS generation-two recovery;
    interrupted validation resets to planned, interrupted effect intent
    becomes outcome-unknown, and new effects remain blocked until explicit
    reconciliation and resume.

    Deterministic journal regressions cover a changed combined scope and a
    Windows unscoped changed-start owner while validating and effect-adjacent
    rows exist, then compare the complete mutable cleanup/candidate-claim graph
    byte-for-byte. Native macOS/Linux subprocess coverage proves graceful and
    abrupt same-scope death; the same test is compiled for native Windows and
    requires both deaths to remain `Unknown`. All 49 journal tests and the
    process-liveness tests pass on macOS. The installed Windows Rust target
    reaches bundled SQLite compilation, where this macOS host lacks the MSVC
    C headers (`stdlib.h`); native Windows execution remains an explicit M9
    release gate. Schema v14 now provides separate stable-host and boot-scope
    provenance for new claims and a typed non-executable distinction between
    prior boot, foreign host, and unproven state. Migrated rows stay unproven.
    Reconciliation remains unimplemented; any future capability must be
    non-resumable and unable to validate, resume, or start effects.

Exit criteria:

- Every mutation originates from a current, reviewed plan.
- Trash actions do not claim immediate freed space.
- Permanent actions are limited to safe deterministic rules.
- All actions, failures, rejections, and skips appear in History.
- Safety integration and fuzz tests pass.

### Milestone 6: Low-disk response and trends

Goal: make DUX proactive and explain recurring disk pressure.

Tasks:

- [x] Persist disk samples and pressure episodes.
  - [x] 2026-07-19 slice: add schema v11 durable pressure episodes. Warning
    and Critical entries, Healthy recovery, Warning→Critical escalation, and
    policy-revision boundaries are updated in the same immediate transaction
    as the raw capacity sample. Unknown pressure never opens or claims
    recovery; exact retries are idempotent; bounded readers reject malformed
    or overlapping history. `StoredPressureEpisode` remains path-free,
    telemetry-only, and is not exposed through FFI yet.
  - [x] 2026-07-29 slice: expose durable pressure episodes as bounded,
    anchored, path-free telemetry through the engine and UniFFI contract v36.
    The persistence reader validates the complete returned page plus one
    lookahead row, the referenced volume lifetime, strict newest-first order,
    non-overlap, and the single-newest-open invariant before truncation. The
    public contract accepts only one stable startup-volume identity, exact
    observation anchor, and a limit of 1–64; it distinguishes Warning from
    Critical and returns no path, candidate, plan, recommendation, approval,
    or mutation command. Swift independently validates the full envelope,
    identity, anchor, record versions, ordering, intervals, open-state shape,
    and truncation claim off the main actor. `AppModel` coalesces identical
    scene requests and generation-fences volume/anchor changes while retaining
    an explicitly stale last-known history on failure. Final review tightened
    accepted-anchor proof to an exact durable raw sample or the exact current
    `last_seen` observation when hourly cadence suppressed its raw row;
    arbitrary between-observation timestamps fail closed.
- [x] 2026-07-19 slice: add the bounded core trend contract. `CapacityTrend`
  anchors on the newest durable raw sample at or before the requested time,
  reports signed total/available changes over exact 24-hour and 7-day cutoffs
  using at-or-before baselines, and emits at most one validated point per UTC
  day across the 30-day window. Completed-day points prefer exact daily
  rollups; the current day prefers the newest raw anchor. Missing baselines
  remain `None`, important-usage deltas remain unknown unless both endpoints
  are present, and malformed volume/sample facts fail closed. The contract is
  path-free telemetry and remains outside FFI/Swift until the chart adapter
  slice.
- [x] Add 24-hour/7-day changes and 30-day chart.
  - [x] 2026-07-19 slice: expose the bounded trend contract through the
    engine and FFI contract v20. `get_capacity_trend` accepts only a validated
    stable macOS volume ID and nonnegative anchor time, returns signed
    24-hour/7-day deltas, optional important-usage deltas, and source-labeled
    UTC-day points. Swift validates record versions, identity, ordering,
    byte relationships, and the 31-point bound off the main actor. The
    generated bindings and real Rust FFI round-trip tests pass; chart UI
    composition remains the next presentation slice.
  - [x] 2026-07-19 slice: refresh the trend from the long-lived `AppModel`
    after each accepted capacity snapshot, with generation fencing, identity
    fencing, single-flight loading, and cancellation-safe task cleanup. The
    menu-bar popover now shows signed 24-hour/7-day available-space changes
    and a bounded 30-day sparkline when enough points exist. Missing history
    stays visibly warming up; charts clamp only validated fractions and expose
    stable accessibility identifiers/summary text. This is presentation-only
    telemetry and grants no scan, plan, approval, notification, or cleanup
    authority. The app target's universal Debug build is green; the linked
    test target remains blocked by pre-existing generated/test-source symbol
    mismatches and Observation macro-server failures.
  - [x] 2026-07-29 slice: complete the Explorer history surface with signed
    24-hour/7-day metrics, a full-width 30-day capacity graph, pressure-colored
    sample points, Warning/Critical episode ribbons, and a textual recent
    low-space-period list. Loading, warming, stale, truncated, unavailable, and
    not-enough-history states remain explicit; ongoing durations are frozen to
    the accepted sample anchor rather than wall-clock time. The menu popover
    adds only a matching current Warning/Critical period as compact context.
    Stable accessibility identifiers and summaries cover the chart, timeline,
    list, and menu context. Focused native integration/presentation coverage is
    green with the generated v36 bindings. The app tracks the requesting
    snapshot anchor separately from the trend's older at-or-before durable raw
    timestamp, preventing cadence-suppressed refreshes from discarding or
    repeatedly reloading a valid chart. Checkpoint verification includes all
    413 native tests, focused core/FFI accepted-anchor regressions, workspace
    clippy with warnings denied, the 244-file destructive-call boundary, and a
    universal arm64/x86_64 Debug app.
- [x] Add transition-based notifications and cooldown.
  - [x] 2026-07-19 slice: preserve the Rust-owned previous durable pressure
    through the Swift capacity snapshot, then gate notifications only on newly
    stored Warning/Critical transitions. A versioned, bounded Recommendations
    payload carries stable volume identity and urgency without paths; separate
    per-volume/per-urgency UserDefaults cooldowns suppress repeats for 24 hours
    and are written only after the notification center accepts delivery.
    Delivery failures leave capacity state and cooldown unchanged. The app now
    submits a concise safe-recovery suggestion through the existing long-lived
    notification service; no cleanup, approval, or executor authority is
    carried. Notification response routing to the Explorer Recommendations
    surface remains the next deep-link slice.
- [x] Deep-link notifications to urgent Recommendations.
  - [x] 2026-07-19 slice: validate notification responses with the exact bounded
    versioned payload before dispatching. Valid responses now route the
    long-lived runtime to Explorer, select a Recommendations destination, and
    open/focus the window through the SwiftUI scene action; malformed,
    unknown-route, oversized, or non-string user-info values are ignored.
    Recommendations is intentionally review-only: it explains that future
    groups require a completed read-only scan and carries no deletion or AI
    execution authority. The affected-volume identity is retained in the
    validated payload but is never used as a filesystem path.
- [x] Add targeted reclaim scan on Warning/Critical.
  - [x] 2026-07-29 prerequisite slice: add the durable configured-project-root
    registry required to scope focused project discovery before pressure
    orchestration exists. Core stores at most 16 absolute, normalized,
    non-root host paths losslessly in deterministic order, rejects duplicates,
    nested/overlapping roots, control bytes, oversized input, corrupt rows, and
    newer value schemas, and preserves explicit empty Stored state separately
    from the rowless empty Default. Revisioned replace/reset operations
    reconcile ambiguous writes and remain synchronous settings changes only.
    UniFFI contract v37 and Swift independently validate record/source/time,
    ordering, bounds, encoding, and overlap. Native Settings provides a
    directory-only, non-symlink picker plus accessible add/remove/reset/error
    states; uncertain writes block further edits until an authoritative reload
    succeeds. A configured root is discovery scope only: adding it starts no
    scan, grants no access, and cannot become a candidate, plan, approval, AI
    input, or cleanup effect. The next slice must add the bounded multi-root
    Warning/Critical scan runner and its priority/cancellation policy; this
    parent task intentionally remains open.
  - [x] 2026-07-29 configured-root runner slice: add one path-free,
    ordinal-selected pressure runner over the durable project-root registry.
    Rust requires the exact latest accepted capacity anchor and current open
    Warning/Critical episode, rereads the registry at every admission, scans
    one root at a time with a 50,000-node per-root and 200,000-node aggregate
    ceiling, and performs a final path-free pressure/anchor/revision/count
    checkpoint before Swift may call the batch complete. Roots retain
    no-follow identity through publication, must belong to the pressured
    startup volume, and treat macOS's sealed `/` plus
    `/System/Volumes/Data` devices as one startup-volume pair without admitting
    external devices. Exact successful targeted evidence is restart-safe and
    reusable only for the current pressure level/episode with a retained
    snapshot and terminal candidate evaluation; targeted snapshots are
    excluded from generic Home latest/history queries. Interactive work
    preempts queued targeted work, cancellation of owned running work is
    observed through terminal scope release, and an existing task is exposed
    only when it is the exact targeted root. UniFFI contract v38 carries no
    caller path or cleanup/AI authority and Swift independently validates the
    full response graph. Explorer/menu presentation shows timestamped
    allocated/logical observations, coverage, issues, deterministic candidate
    counts, partial failures, cancel/retry state, and an exact-scan
    **Review findings** action. The parent remains open for bounded known
    user-cache roots and the resulting Critical recommendation ordering.
  - [x] 2026-07-29 known user-cache targeting foundation: extend the focused
    pass with one Rust-derived `<OS account home>/Library/Caches` root ahead of
    every configured project root. Discovery never accepts `HOME`, a Swift
    path, or an FFI path; it retains and revalidates the current account's
    no-follow home identity and mount witness before traversal, after
    traversal, and before candidate publication. DUX's own
    `~/Library/Caches/Dux` subtree is excluded by an exact component-aware
    scanner policy that fails closed on malformed, overlapping, or unbounded
    exclusions. Lexical or canonical overlap removes later configured roots.
    A versioned SHA-256 catalog binds root kind/order, configured revision,
    known-root policy revision, lossless and canonical paths, live identity,
    unavailable reason, per-root budget, and exclusions; Swift must echo that
    digest for every ordinal after zero and again at the final checkpoint.
    Missing later-ordinal digests fail as invalid catalog input. An in-process
    active task is reusable only when its root kind and ordinal, live identity,
    catalog digest, node budget, exclusions, and exact pressure episode all
    match; catalog edits, same-path root replacement, and a new episode return
    `Busy` rather than joining stale work.
    UniFFI contract v39 exposes only the typed catalog and root kind. It still
    accepts no caller path, cleanup plan, approval, AI input, or effect
    authority. One global 200,000-node pass reserves up to 100,000 nodes for
    user caches and keeps 10,000–50,000 nodes for each remaining configured
    root. Durable reuse additionally requires the exact root source, retained
    snapshot root identity, and terminal candidate evaluation. Automatic
    user-cache observations deliberately use a separate evaluator scope and
    currently publish an honest zero-candidate result rather than borrowing
    user-selected Rust-target rules. Native progress labels the first row
    **User caches**, propagates the exact catalog digest, and keeps all
    findings read-only. Focused core/FFI and native adapter/runner tests cover
    budget bounds, cache-first order, exclusion behavior, catalog echo, and
    the evaluator authority boundary. The parent remains open until
    deterministic cache-specific rules can turn this evidence into reviewable
    safe findings; emergency ordering remains the next separate slice.
  - [x] 2026-07-29 deterministic cache-findings slice: add independently
    reviewed `developer.homebrew.cache` and
    `developer.python.pip_cache` revision-1 rules for exact direct children of
    the Rust-owned current-account `Library/Caches` scan. Evaluator revision 4
    and the updated catalog digest bind the separate user-cache scope, exact
    component, seven-day inclusive newest-mtime fact, candidate-local coverage,
    aggregate observation, and symlink boundary. Neither age nor a cache name
    proves provider inactivity or ownership, so every positive finding remains
    unschedulable and blocked by `MissingOrIncompleteEvidence` plus
    `ProtectedPath`; a subtree symlink, recent timestamp, incomplete timestamp,
    or local coverage issue adds or withholds evidence fail-closed. Snapshot
    replay reproduces the exact result, rejects scope/scan-ID mismatches, and
    pending known-cache evaluations are not recovered without a fresh
    OS-account root witness. Durable targeted reuse now rejects an older
    evaluator or catalog revision during the same pressure episode.
    Recommendations' **Review findings** action queues the exact targeted scan
    directly into Explorer's Candidates view. The native view uses
    revision-bound friendly names, page-local category/safety/action cards
    without summing potentially overlapping estimates, exact raw rule IDs in
    detail, and bounded previous/next candidate paging. The rule reviews in
    `docs/rules/developer-homebrew-cache.md` and
    `docs/rules/developer-python-pip-cache.md` record official provider
    evidence and the missing authority needed for any future promotion.
    Focused core and native regression gates cover exact matching, age,
    symlink, candidate-local coverage, replay, stale reuse, deep-link intent,
    grouping, and pagination. Final checkpoint verification passes formatting,
    workspace Clippy with warnings denied, the serialized Rust suites (34 CLI
    unit, 3 CLI integration, 1,119 core with 2 performance tests ignored, 13
    projection, and 77 FFI with the 2 effect regressions run separately and
    passing), all 31 policy-checker tests, the 250-source destructive-call
    boundary, all 444 native tests, byte-identical generated Debug/Release
    Swift bindings, and unsigned universal arm64/x86_64 Debug and Release apps
    targeting macOS 14. Critical recommendation ordering remains the next
    separate slice.
- [x] Add emergency recovery ordering.
  - [x] 2026-07-30 slice: add Rust-owned, revision-1 Critical recovery
    ordering through UniFFI contract v40. The engine atomically finalizes only
    against the exact current Critical pressure proof and targeted-root catalog,
    privately reselects retained exact-root scans and current terminal candidate
    evaluations, and revalidates the pressure/catalog checkpoint after the
    bounded projection. A one-hour core freshness policy excludes older scans;
    a scan completed exactly one hour before the capacity anchor remains
    eligible, and a newly triggered scan may complete after its admitting
    capacity sample. Warning, changed episode/catalog/identity, stale evaluator,
    missing snapshot, malformed record, or unsupported policy data fail closed.

    The complete §13.3 lane priority is explicit and stable in Rust:
    cloud eviction, stale safe-regenerable observations, Trash information,
    installers/archives, large files, guided exploration, then coverage and
    permission gaps. Revision 1 emits only evidence-backed lanes 2, 6, and 7;
    unsupported lanes are omitted rather than shown as zero or invented
    actions. Stale-regenerable membership requires discovered/selected status,
    the exact safe-regenerable action pair, `MinimumAge`, and no contradictory
    recent/missing-mtime blocker. Existing Homebrew and pip findings remain
    review-only behind their mandatory blockers. Root-level unavailable
    observations appear as a typed count in the permission lane without fake
    scan IDs or hidden-byte estimates. Complete-zero, incomplete, stale,
    updating, and unavailable states remain distinct.

    The path-free ordering contains only policy/pressure/catalog proofs,
    bounded completeness counts, deterministic rule/category groups, exact scan
    navigation IDs, observation times, candidate counts, blocker counts, and
    permission-issue counts. It carries no path, candidate ID, byte forecast,
    combined reclaim total, cleanup mode, plan, approval, AI input, schedule,
    callback, provider command, filesystem handle, or executor capability.
    Core, FFI, and Swift independently validate bounds, strict ranks, fixed lane
    order, semantic uniqueness, lane-specific field shape, root ordinals,
    timestamps, completeness arithmetic, and the unavailable-root permission
    group. Malformed Swift responses do not consume the exact retained proof.

    Explorer Recommendations now leads with a pressure-aware recovery hero and
    accessible vertically ordered review cards. Supported cards open only the
    exact Candidates, snapshot Browse, or Coverage views. The menu bar mirrors
    at most the first three core-ranked cards without reordering, labels the
    actual §13.3 step numbers (including omitted-lane gaps), and always exposes
    Current, Updating, or Earlier evidence plus freshness. Copy says
    “review only,” reports observation age and incompleteness, never sums
    overlapping estimates, and never promises bytes freed. Focused verification
    covers the core freshness/order/shape boundary, the generated FFI
    projection, all typed adapter failures, post-anchor scan completion,
    proof-consumption fencing, native AppModel integration, navigation,
    stale-state rendering, accessibility, and malformed/duplicate response
    rejection. Final checkpoint verification passes formatting, workspace
    Clippy with warnings denied, the serialized Rust suites (34 CLI unit, 3 CLI
    integration, 1,130 core with 2 performance tests ignored, 13 projection,
    and 79 FFI with the 2 effect regressions run separately and passing), all
    31 policy-checker tests, the 253-source destructive-call boundary, and all
    460 native tests. Generated Debug/Release bindings are byte-identical, and
    unsigned universal arm64/x86_64 Debug and Release apps target macOS 14.
- [x] Add rule outcome/regrowth measurement.
  - [x] 2026-07-30 prerequisite slice: checksummed SQLite schema v13 persists a
    domain-separated SHA-256 digest of the code-owned filesystem identity for
    every newly admitted scan root and adds a bounded exact-root/start index.
    General, subtree, and targeted scans capture and revalidate the same
    identity before durable start and after traversal. Migrated scans retain
    an explicit unknown identity and cannot become comparable outcome
    evidence. The digest is path-free history only and grants no scan,
    candidate, cleanup, schedule, AI, or filesystem authority. A later slice
    must still prove a complete post-cleanup zero observation before labeling
    a subsequent compatible nonzero observation as regrowth. Focused and
    cross-crate verification covers the complete checksummed migration chain,
    populated v1/v12 upgrades, exact digest persistence and root lookup,
    general/targeted/subtree drift, 34 CLI unit plus 3 integration tests, 13
    projection tests, 79 FFI tests with 2 isolated effect regressions, and
    workspace Clippy with warnings denied.
  - [x] 2026-07-30 Rust-core derivation slice: evaluator revision 5 subtracts
    only the preserved direct regular non-symlink `CACHEDIR.TAG` allocation
    from Rust-target estimates and refuses an explicit zero when any
    reclaimable descendant has unknown allocation. A bounded, read-only
    exact-session engine query derives one path-free typed state per cleanup
    item from terminal permanent-safe v2 journal evidence and compatible v13
    scans. Candidate absence is not zero; `Regrown` requires an explicit
    zero followed by a nonzero observation. Source evaluation must precede
    plan creation, root identity/evaluator/catalog/context/scope must match,
    observation completion order is deterministic, overlapping scan intervals
    are ignored, and a later overlapping removed path supersedes attribution.
    Full follow-up evaluations and intervening journals are validated one at a
    time, compacted to fixed scalar slots, and dropped; row, materialization,
    elapsed-time, and pure-Rust work limits return no partial result. The
    legacy `rule_outcomes` table is deliberately ignored. Focused regressions
    cover marker-only zero, unknown allocation, absence, zero-to-nonzero
    regrowth, first-observed nonzero, source chronology and mismatch,
    reversed/equal scan intervals, native-component overlap, active later
    cleanup supersession, legacy poison data, and a byte-identical database
    after the query.
  - [x] 2026-07-30 FFI/native presentation slice: UniFFI contract v41 exposes
    the exact-session outcome query as a separate bounded path-free batch with
    one outcome per immutable cleanup-history item. The transport preserves all
    six typed ineligibility reasons and all six observation states, validates
    exact rule/ordinal/revision binding, and rejects malformed chronology,
    bytes, versions, overflow, pre-epoch values, and timestamps whose strict
    order would collapse at millisecond projection. Swift repeats those checks
    before constructing app-owned values. Dynamic outcome loading has its own
    task, generation, error, and legacy-unavailable state; immutable cleanup
    detail remains visible if the derived query fails, and selection, refresh,
    close, clear, shutdown, and stale replies are fenced independently.
    Successful home, subtree, and low-disk targeted scan observations re-read
    the selected outcome once, while the loaded view also shows its read time
    and offers an explicit observation-only refresh. Cleanup History presents
    a compact accessible six-state distribution and per-item explanations with
    exact relevant timestamps. Copy distinguishes waiting, first later size,
    explicit zero, confirmed zero-to-nonzero regrowth, supersession, and typed
    non-comparability; it never describes an estimate as verified capacity,
    treats absence as zero, or grants repeat-cleanup authority. VoiceOver
    distinguishes loading, failed, migrated, mismatched, and loaded states.
    Focused coverage exercises every transport state/reason, exact binding and
    malformed graph, presentation/accessibility copy, selection/reselection,
    refresh/close/stale-reply fencing, and ordinary plus targeted scan
    invalidation. The full checkpoint verifies 82 FFI tests plus both isolated
    effect regressions through their dedicated runner, all 472 native tests,
    workspace Clippy with warnings denied, destructive-call policy, generated
    Debug/Release binding parity, and universal arm64/x86_64 Debug and Release
    apps targeting macOS 14.
- [x] Add recurring “storage thief” ranking.
  - [x] 2026-07-30 slice: add one bounded, read-only deterministic ranking
    over the newest 32 terminal schema-v2 permanent-safe cleanup sessions.
    Complete journals are grouped by stable rule ID across recorded revisions;
    one rule counts as a successful cleanup session only when every matching
    item has a complete successful regenerable-removal effect. Only the exact
    rule-outcome derivation's explicit zero-to-compatible-nonzero `Regrown`
    state contributes to observed bytes or time. Candidate absence, a first
    later nonzero size, an explicit zero without later growth, supersession,
    and non-comparable evidence never become a zero rate or a growth claim.
    Rankings use the exact aggregate bytes/duration fraction before successful
    cleanup count, confirmed cycle count, recency, and rule ID; the displayed
    bytes/day value is checked, floored, and explicitly capped on overflow.
    One lookahead discloses omitted older qualifying sessions, results are
    limited to the first 12 rules while preserving the total ranked count, and
    the legacy `rule_outcomes` table remains outside the query. Shared fixed
    SQLite VM/deadline, journal/scan/materialization, and pure-Rust work budgets
    fail the whole read without partial publication or durable mutation.
    Manual cleanup and manual regrowth evidence are counted separately. The
    latest recorded revision reaches only a history threshold after at least
    two successful manual sessions of that same revision and one manual
    confirmed regrowth cycle; this is not current automation eligibility,
    never enables a schedule, and grants no planner or executor authority.
    UniFFI contract v42 transports the path-free bounded ranking through its
    own typed error domain. Core revalidates the derived display rate and cap;
    the FFI adapter separately checks versions, limits, unique rule IDs,
    counts, exact fraction order, duration shape, timestamps, and threshold
    prerequisites. Swift repeats the bounded response-shape, uniqueness,
    duration, timestamp, exact overflow-free fraction order, derived
    bytes-per-day/cap pair, and threshold checks before publishing app-owned
    values. Cleanup History
    lazily loads an accessible ranked bar presentation with observation time,
    explicit window/group truncation, empty/loading/failure/earlier-result
    states, and a read-only refresh. Successful scan observations, history
    refresh/clear, close, shutdown, cancellation, and newer generations fence
    stale replies. Copy states that the ranking is deterministic rather than
    AI, reports observed estimates rather than verified capacity change, and
    keeps scheduling off pending the later Milestone 8 policy and fresh
    candidate gates. Focused core regressions cover exact rate arithmetic,
    session-level de-duplication, manual versus CLI evidence, qualifying-window
    bounds, same-revision threshold isolation, legacy poison data, and a
    byte-identical database after reading; FFI coverage pins the empty/closed
    boundary, fraction ordering, and every typed error mapping. Final
    checkpoint verification passes formatting, workspace Clippy with warnings
    denied, Rust 1.88 workspace/fuzz compatibility, all 31 policy-checker tests,
    and the 257-source destructive-call boundary. The serialized Rust lanes
    pass 1,148 core tests plus four race-sensitive macOS regressions in their
    dedicated isolated lane (with two performance tests ignored), all 13
    projection tests, and all 86 FFI tests including the two isolated effect
    regressions. All 481 native tests pass. Generated Debug/Release Swift
    bindings are byte-identical, and unsigned universal arm64/x86_64 Debug and
    Release apps target macOS 14 with the Release app retaining menu-bar-only
    `LSUIElement` packaging.
- [ ] Add iCloud evictable candidates and an eviction executor (non-destructive; disclosed as re-download-on-demand). MAY ship after the first beta.
  - [x] 2026-07-30 read-only eligibility foundation: accept ADR 0006 and
    add a fail-closed selected-file metadata probe before creating any
    candidate or effect. A retained Explorer review chooses one exact
    non-root, regular, single-link file with known nonzero allocation and
    revalidates its no-follow identity and ancestors. UniFFI contract v43
    gives the macOS adapter only a consume-once exact path; Foundation performs
    one fresh ubiquitous-item resource-value read and returns only bounded
    tri-state upload, download, conflict, exclusion, and local-copy facts.
    Rust fixes the provider, kind, allocation, and observation time, then
    returns a path-free deterministic assessment whose unknown values all fail
    closed. The passing state is discovery metadata only: it cannot prove
    absence of a concurrent writer and creates no rule, candidate, plan,
    approval, journal row, history result, AI input, emergency-recovery group,
    button, schedule, provider command, or eviction effect. Focused tests cover
    every policy fact, root/type/link/allocation and replacement rejection,
    callback error/panic containment, consume-once transport, malformed
    records, and Foundation status/error mapping. The parent remains open for
    bounded candidate discovery, durable provider/account/version evidence,
    final live re-probing, the journal-fenced one-shot Foundation eviction
    executor, truthful UI/accessibility, and isolated real-iCloud race tests.
    Final verification passes formatting, workspace Clippy with warnings
    denied, all 31 policy-checker tests, and the 265-source destructive-call
    boundary with no production eviction primitive. The serialized Rust lanes
    pass 1,160 core tests plus three timing-sensitive isolated regressions
    (with two performance helpers ignored), all 37 CLI tests, all 90 ordinary
    FFI tests plus both isolated effect regressions, and all 495 linked native
    tests. Debug/Release Swift bindings are byte-identical; the Rust archive
    and both unsigned apps are universal arm64/x86_64, target macOS 14, retain
    identical three-file layouts and `LSUIElement=true`, and exclude the
    internal permanent-cleanup condition from Release.
  - [x] 2026-07-30 selected-file observation UI: expose the existing v43
    read-only probe as one explicit, manual Explorer inspector check for the
    exact selected regular file. The app does not probe automatically,
    enumerate iCloud Drive, infer provider identity from a path, aggregate
    cloud allocation, or append Foundation metadata to the pure replayable
    candidate-evaluation batch. The retained review controller forwards the
    exact scan/node request through its owned lease and rechecks the lease
    generation before returning. Explorer publishes only path-free app-owned
    facts, ordered blockers, observation time, and the scan's historical
    allocated-byte value. Selection, navigation, paging/query, content-mode,
    snapshot, close, and presentation generations clear the observation and
    reject late replies. The accessible inspector distinguishes idle,
    loading, favorable, blocked, changed, unsupported, unavailable, failed,
    and malformed states; labels favorable metadata only as “Currently
    supports review”; says “Remove local copy,” “Stays in iCloud,” and
    “Requires a network connection to download again”; and states that no
    cleanup action is available yet. It creates no `Candidate`, persistence
    row, review selection, recovery group, approval, plan, journal/history
    record, AI input, CLI edge, notification, schedule, provider command,
    retry, or effect. Existing weak `CloudUploadComplete` evidence remains
    outside this flow because it cannot represent current local state, idle
    transfers, errors, conflicts, sync inclusion, provider/account/container,
    or item version. The parent remains open for a bounded multi-item
    observation source, separately versioned durable provider evidence,
    purpose-built candidate admission, final live re-probing, the
    journal-fenced one-shot Foundation executor, and isolated real-iCloud race
    tests. Verification: the full linked native suite passes 504 tests, the
    Python policy suite passes 31 tests, destructive-call inspection covers
    265 source files with no violation, and clean universal arm64/x86_64
    Debug and Release apps build for macOS 14 with `LSUIElement=true`.
  - [x] 2026-07-30 bounded directory observation UI: add a dedicated,
    Rust-owned, path-free source for explicit multi-item iCloud metadata
    review. UniFFI contract v44 accepts only the exact retained snapshot
    directory node ID, walks at most 200,000 descendants, and fails the whole
    query instead of silently truncating traversal. It ranks at most 32
    complete regular files with known nonzero historical allocation and no
    scan warning by allocated bytes, logical bytes, then stable node ID. Exact
    ranked/omitted/visited counts and bounded historical parent context cross
    FFI; no current path or provider conclusion does. Swift independently
    validates versions, scan/scope/request echoes, accounting, limits,
    uniqueness, contiguous ranks, node shape, scan flags, context depth, and
    ordering before publishing app-owned rows. Explorer's fifth **iCloud
    Status** mode loads only that historical source and never probes
    automatically. The user's manual check sends rows serially through the
    existing v43 before/after witness and classification; item-local changed,
    invalid, or metadata failures may continue, while systemic unavailable,
    unsupported, or malformed states stop the remainder. Code-owned
    single-flight state prevents overlapping cancel/restart work. Because the
    Foundation resource-value read is synchronous, cancellation is honestly
    labeled **Stop after current check**: it suppresses the in-flight result
    and starts no later call but does not claim a mid-call timeout. Navigation,
    snapshot, mode, close, and review generations fence late results; selection
    alone does not cancel the directory-scoped batch. Accessible table,
    progress, omission, favorable/blocked/failure, observation-time, and
    non-destructive disclosure states are visible without any aggregate byte
    or reclaim claim. The inspector exposes no Trash or eviction action and
    states that a future local-copy removal would stay in iCloud and require a
    network download. Results remain memory-only discovery: no rule,
    `Candidate`, durable evidence, plan, approval, journal/history row, AI
    input, CLI edge, notification, schedule, provider command, retry, or
    effect exists. The parent remains open for separately versioned durable
    provider/account/container/item evidence, purpose-built candidate
    admission, final live proof, the journal-fenced one-shot Foundation
    executor, and isolated real-iCloud race verification. Final verification
    passes formatting, workspace and fuzz Clippy with warnings denied, Rust
    1.88 workspace/fuzz compatibility, all 31 policy-checker tests, and the
    265-source destructive-call boundary. Serialized Rust lanes pass 1,163
    ordinary core tests plus three timing-sensitive regressions in their
    dedicated exact lane (with two helper/performance tests ignored), all 37
    CLI tests, all 13 projection tests, and all 90 ordinary FFI tests plus both
    isolated effect regressions. All 524 linked native tests pass, including
    83 Explorer browser state-machine tests. Generated Debug and Release Swift
    bindings are byte-identical; the Rust archive and both unsigned apps are
    universal arm64/x86_64, target macOS 14, retain identical three-file
    layouts and `LSUIElement=true`, and exclude the internal permanent-cleanup
    condition from Release.
  - [x] 2026-07-30 identity-capability probe: UniFFI contract v45 makes the
    next safety blocker explicit without introducing durable evidence or
    eviction authority. Each manual item check observes the current account,
    two complete Foundation resource-value samples, the current file version
    beside each sample, and the current account again. Account identity, item
    generation, and file version are classified independently as stable,
    unavailable, changed during read, or unsupported; bounded comparison
    archives are neither decoded nor persisted. Shared-item and sync-paused
    values remain separate tri-state policy facts. Production container
    identity is explicitly unsupported: `ubiquityIdentityToken` identifies
    the current account but does not connect the app to containers, while
    `url(forUbiquityContainerIdentifier:)` addresses only containers declared
    for the app and cannot identify the arbitrary user-selected iCloud Drive
    container containing an Explorer item. DUX does not substitute paths,
    display names, metadata-query scope, Finder state, or private provider
    metadata.

    Rust reports sync eligibility and identity readiness independently in a
    fixed fail-closed order. Favorable upload/download/conflict/exclusion
    metadata may still support a read-only review, but any unavailable,
    changed, or unsupported identity fact blocks identity readiness. The
    production unsupported container fact therefore prevents v45 observations
    from becoming durable provider evidence or candidate input. Explorer
    exposes the distinction accessibly and does not imply that current sync
    metadata authorizes removal. This slice adds no persistence schema, rule,
    `Candidate`, plan, approval, journal/history row, emergency group, provider
    command, cleanup button, retry, effect, AI input, CLI edge, notification,
    or schedule.

    The isolated real-device protocol in
    `docs/testing/icloud-local-copy-real-device.md` covers a dedicated
    disposable account, macOS 14 plus the newest supported macOS, restart and
    reboot stability, no-op/edit/rename/evict-redownload transitions, and
    characterization of any future public File Provider item/domain witness.
    The present probe is read-only; future tests that invoke eviction require
    a separate explicit destructive opt-in and disposable files. Focused
    and final verification passes formatting, workspace and fuzz Clippy with
    warnings denied, Rust 1.88 workspace/fuzz compatibility, all 31 policy
    tests, and the 265-source destructive-call boundary. The serialized direct
    core lane covers all 1,168 runnable tests across its complete run and one
    exact retry after a transient FSEvents probe failure; two
    helper/performance tests remain intentionally ignored. All 37 CLI tests,
    all 13 projection tests, all 91 ordinary FFI tests plus both isolated
    effect regressions, and all 535 linked native tests pass. Debug and Release
    Swift bindings are byte-identical; their Rust archive and unsigned apps
    are universal arm64/x86_64, target macOS 14, and retain
    `LSUIElement=true`. Release has the expected three-file app layout and
    excludes the internal permanent-cleanup condition; Xcode 26.5 adds its two
    expected dynamic-replacement dylibs only to Debug. The parent remains open
    for a supported stable container witness, separately versioned durable
    provider/account/container/item evidence, purpose-built candidate
    admission, fresh final proof, the journal-fenced one-shot Foundation
    executor, and isolated destructive race verification.
  - [x] 2026-08-09 public File Provider identity observation: UniFFI contract
    v58 replaces v45's hard-coded production container blocker with a bounded,
    read-only use of
    `NSFileProviderManager.getIdentifierForUserVisibleFile(at:)`. The native
    reader observes the exact Rust-issued user-visible URL twice around the two
    complete Foundation samples, in the fixed sequence account A, File
    Provider A, resource A, version A, resource B, version B, File Provider B,
    account B. It compares the public domain/container and provider-owned item
    identifiers independently. API error, missing response, five-second
    timeout, empty value, or more than 4 KiB of UTF-8 becomes unavailable;
    changed values remain changed; a non-ubiquitous file remains unsupported.
    The opaque raw identifiers never leave the Swift reader, enter logs or
    persistence, cross FFI, reach AI, or become a path/provider display
    inference.

    Core policy adds a distinct provider-item fact and three fixed blockers
    rather than confusing provider identity with resource generation or file
    version. Rust owns the exact account → domain/container → provider item →
    generation → version → shared → sync-paused blocker order. The v58 raw and
    assessment records carry only four-state classifications; Swift repeats
    the ordering and readiness derivation before showing a seventh accessible
    identity row. A fully stable observation can now report in-memory identity
    readiness, but that remains one-read capability evidence only. This slice
    adds no durable identifier or schema, rule, `Candidate`, plan, approval,
    journal/history row, emergency group, cleanup button, provider command,
    retry, effect, AI input, CLI edge, notification, or schedule.

    Focused native coverage passes all 34 reader/driver/strict-adapter tests,
    including bracket order, independent domain/item drift, missing and
    oversized identifiers, non-ubiquitous refusal, path consume-once, blocker
    projection, and malformed-response rejection; the complete linked native
    suite passes all 709 tests. Focused core and FFI suites, the v58 handshake,
    formatting, workspace and fuzz Clippy with warnings denied, Rust 1.88
    workspace/fuzz compatibility, all 105 policy tests, and the 364-source
    destructive-call boundary pass. The serialized core process passes 1,599
    tests with four helpers ignored; its four timing/environment-sensitive
    failures pass in exact isolated lanes, as does the complete 45-test Cargo
    attestation module. Generated Debug/Release Swift bindings are
    byte-identical, and signed Debug plus unsigned Release apps and their
    bundled CLIs are universal arm64/x86_64. The parent remains open for macOS
    14 plus newest-supported real-device qualification, a separately
    versioned durable opaque evidence design, candidate admission, fresh final
    proof, a journal-fenced one-shot supported eviction executor, and isolated
    destructive race verification.
  - [x] 2026-08-09 read-only v58 real-device qualification harness: add a
    non-shipping, unhosted XCTest target that compiles the exact production
    Foundation reader behind a qualification-only condition. Ordinary test and
    app builds never enable the condition, and the live test skips before
    fixture or provider access unless the operator supplies every explicit
    read-only opt-in. The reader brackets the same v58 account, File Provider,
    resource, and file-version observations, but exports only stable/changed/
    unavailable/unsupported continuity plus domain-separated HMAC-SHA256 tags.
    Raw account, domain, provider-item, generation, and file-version values
    remain inside the reader and never enter evidence, logs, XCTest output, or
    production code paths. Qualification state uses an operator-provided exact
    32-byte key and create-only private 0600 files below an existing owned 0700
    directory outside iCloud, DUX application storage, and the repository.

    The operator wrapper requires a clean, unchanged commit; an exact physical
    iCloud regular-file fixture; an opaque account label, fixture label, and
    external content reference; and one of the fixed protocol phases. Each row
    performs exactly three complete observations, records only classifications,
    HMAC tag continuity, fixture allocation/stat invariance, and path-free
    expected-versus-observed results, and validates a closed versioned JSON
    schema before publication. Package resolution is locked and disabled, the
    fixture contents are never read, and no delete, move, trash, eviction,
    download, metadata mutation, retry, cleanup, candidate, approval, journal,
    history, AI, notification, schedule, or provider command is available.
    The paired protocol document defines the still-required macOS 14 and newest
    supported macOS rows across baseline, same-process repeat, process restart,
    reboot, and controlled identity/version transition cases. This harness
    creates no row by itself and does not claim that matrix complete.

    Verification passes 16 qualification tests with the live case skipped by
    default, all 709 ordinary linked native tests, all 113 policy tests, the
    370-source destructive-call boundary, formatting, workspace and fuzz Clippy
    with warnings denied, and universal arm64/x86_64 qualification, Debug, and
    Release builds targeting macOS 14. The ordinary app and bundled CLI contain
    no qualification source, symbol, compile condition, environment key, or
    protocol marker. The monolithic core stress process passes 1,589 tests with
    four helpers ignored; all 14 resource/timing-sensitive failures pass in
    exact isolated lanes. The parent remains open until both real-device matrix
    endpoints produce reviewed opaque evidence and that evidence supports a
    separately versioned durable design, candidate admission, fresh final
    proof, a journal-fenced one-shot supported eviction executor, and isolated
    destructive race verification.
- [x] Add snapshot diff mode in Explorer: tree/treemap colored by growth between the last two snapshots. MAY ship after the first beta.
  - [x] Land the Rust-owned comparison foundation and UniFFI contract v46
    (2026-07-30). One exact active Explorer review can prepare its immediately
    preceding succeeded, non-targeted, non-tombstoned snapshot only when
    lossless root bytes and the non-null code-owned root-identity digest match.
    Ordering matches per-root latest-two retention. The parent-scoped child
    retains exactly one baseline lease under the existing two-document/1 GiB
    decoded budget, matches hierarchical host bytes rather than snapshot-local
    IDs or lossy names, and exposes bounded union pages plus
    magnitude-descending treemaps. Added, removed, grown, shrunk, unchanged,
    and replaced observations are explicit; removed-only directories remain
    drillable. Delta is direction plus `u64` magnitude, while represented and
    omitted growth/shrinkage are accounted separately. Both coverage reports
    cross as historical context. The contract has no live-target resolver,
    candidate, reclaimability, planner, approval, AI, provider, schedule, or
    effect edge.
  - [x] Add a distinct accessible **Changes** Explorer mode over v47. Entering
    lazily prepares the child; leaving, snapshot replacement, subtree
    replacement, close, and shutdown release it. Fence every prepare, page,
    navigation, sort, treemap, and selection result by parent-review and diff
    generations. Preserve ordinary Browse if comparison is unavailable or
    expires.
  - [x] Present `older → newer` timestamps and both coverage states; summary
    current/previous logical size, total observed growth, and total observed
    shrinkage; a table with Name, Change, Current, Previous, State, and
    Category; and a dedicated inspector with no Trash/cleanup/live-path
    actions. Persistently disclose: “Logical-size observations, not verified
    capacity change or reclaimable space.”
  - [x] Render change magnitude as treemap area and direction redundantly by
    color, symbol, pattern, text, and VoiceOver: orange `↑ Grew`, blue
    `↓ Shrunk`, purple `+ First observed`, gray `− No longer observed`, and an
    explicit replacement treatment. Keep separate Other Growth and Other
    Shrinkage cells; never net them. Zero-change rows remain in the table.
    Return drills into matched or one-sided directories, Backspace navigates
    back, and table/treemap selection stays synchronized.
  - [x] Complete the native v47 checkpoint (2026-07-30). Contract v47 adds
    independently optional current/baseline node kinds so Swift can prove
    one-sided presence, replacements, and descent instead of trusting the
    display kind. The native controller owns and renews the exact child,
    releases child before parent on mode/snapshot/subtree/close/expiry/shutdown,
    and generation-fences late prepare/root/page/treemap/navigation/sort
    results. The Changes view keeps Browse intact, persistently discloses the
    logical-size limitation, renders the exact six-column table and
    older-to-newer coverage/summary, and uses color, symbol, border pattern,
    text, and VoiceOver with separate Other Growth/Shrinkage cells and a
    no-authority inspector. Focused Rust comparison and 92 ordinary FFI tests
    pass (two isolated effect tests remain intentionally ignored); native
    hostile-adapter, controller, and browser suites pass 68, 38, and 92 tests,
    and the complete serial native run passes all 561 tests. Workspace checks
    and warnings-as-errors Clippy pass. Debug/Release binding identity,
    universal app architecture, macOS 14 deployment, menu-agent metadata, and
    Release-condition absence are verified for the checkpoint artifact.

Exit criteria:

- A simulated threshold transition produces one correct notification.
- Repeated samples do not spam.
- Critical flow presents current safe candidates quickly.
- History distinguishes estimated size from verified capacity change.

### Milestone 7: AI explanations

Goal: help users understand unknown storage without expanding deletion authority.

Tasks:

- [x] Define versioned AI input/output schemas.
  - [x] 2026-08-09 provider-neutral v1 contract: checked Draft 2020-12 input
    and output schemas pair with a crate-private Rust validator that imports no
    DUX module and has no crate-root, engine, persistence, FFI, Swift, CLI, or
    provider surface. The input is capped at 256 KiB and one direct level of
    128 request-local nodes; the output is capped at 64 KiB and can return only
    bounded summaries, labels, disjoint presentation groups, questions,
    uncertainties, and human rule-research suggestions. Every field is
    required, every object denies unknown fields, integers stay in the exact
    53-bit cross-language range, root/child/omitted logical and age accounting
    is exact, and `protected`/`content_included` are false-only. The v1 surface
    deliberately omits allocated bytes until they can be reconciled as exactly
    as logical observations. A frozen tagged typed SHA-256 encoding binds every
    response to the exact input metadata independently of JSON formatting,
    escaping, or Rust field order. Input labels pass a conservative path-shaped
    grammar; output text also passes a conservative cleanup/execution
    vocabulary and remains non-linkified inert presentation data. Unknown,
    duplicate, or overlapping group references
    reject the complete output. Valid/schema-invalid/semantic-invalid fixtures
    plus 18 focused tests cover hostile names as inert JSON, malformed/trailing/
    duplicate/invalid-UTF-8 documents, authority-shaped fields, privacy flags,
    every N/N+1 collection/text/identifier/integer boundary, classifications,
    exact bucket accounting, Unicode code-point-versus-byte narrowing, digest
    formatting independence, and the full private AI source-tree import/public-
    surface guard.
    `docs/AI_CONTRACT.md` records the full wire contract and makes explicit
    that shape validation is not privacy authorization. No AI provider,
    subprocess, network, cache write, UI, plan, or cleanup edge was added.
- [x] Add privacy redaction and sensitive-path exclusion tests.
  - [x] 2026-08-09 dormant core-owned privacy boundary: the contract-private
    shaper accepts only one selected directory from a validated immutable
    snapshot plus complete typed scan coverage, inspects at most 200,000 nodes,
    and fails closed on incomplete, ambiguous, unrepresentable, mount-boundary,
    error, or over-budget observations. Independent revision-1 deny policy
    covers protected system and exact per-user Library/AppData roots,
    credentials/tokens, keychains, browser profiles,
    Messages/Mail/Notes, password managers, security/management state,
    VM/container disks, and every recognized cloud-document state. A sensitive
    descendant excludes its entire direct-child aggregate before totals are
    formed. Only fixed generic labels and request-local IDs survive; known
    classifications remain empty. Eligible logical bytes and non-directory-
    leaf age buckets are recomputed exactly before the frozen digest and JSON
    encoding, while local disclosure contains only bounded aggregate counts.
    The non-cloneable proof and constructor stay private with no engine, FFI,
    Swift, CLI, provider, process, network, cache, persistence, planner, or
    executor consumer. A revisioned policy corpus and 13 focused privacy tests
    cover protected-root selections plus every sensitive-data category at
    selection/direct/nested boundaries, cloud variants, prompt/path injection,
    non-UTF-8 and incomplete observations, exact
    128/129 child and 200,000-node limits, deterministic path-free IDs/labels,
    exact filtered accounting/ages/digest, path-free diagnostics, and source-
    tree authority isolation. Future orchestration must bind complete coverage
    to the exact retained succeeded snapshot under one reviewed lease.
- [x] Run the adversarial macOS security/TCC spike and record whether local AI
  subprocesses can be confined when DUX has broad access.
  - [x] 2026-08-09 negative direct-subprocess result: the non-shipping v1
    adversarial harness gives a hostile child only a minimal environment, an
    empty working directory, standard descriptors, and a known absolute path
    to a disposable 0600 same-user canary. On macOS 26.5 arm64 the child still
    reads the out-of-scope canary, which fails the gate at the ordinary
    filesystem layer before any favorable TCC assumption can matter. A
    deprecated `sandbox-exec` comparison denies the same read but is explicitly
    ineligible for production. The path-free frozen evidence, schema, protocol,
    and accepted [ADR 0009](docs/adr/0009-reject-direct-local-ai-subprocesses.md)
    reject direct local commands without probing personal data. A future
    metadata-only remote transport, separately sandboxed component, or VM still
    needed its own accepted architecture. ADR 0013 below now supplies only the
    remote decision; the other architectures remain unapproved.
- [x] Implement the Claude CLI probe/invocation adapter only if that spike
  approves its authority boundary. Closed 2026-08-09 without implementation:
  the gate returned no-go, so direct Claude invocation is prohibited by ADR
  0009 and no probe, process, provider, FFI, engine, Swift, or CLI edge exists.
- [x] Implement the Codex CLI probe/invocation adapter only if that spike
  approves its authority boundary. Closed 2026-08-09 without implementation:
  the gate returned no-go, so direct Codex invocation is prohibited by ADR
  0009 and no probe, process, provider, FFI, engine, Swift, or CLI edge exists.
- [x] Select and approve a metadata-only remote transport or a separately
  sandboxed/virtualized provider architecture. ADR 0013 selects fixed direct-
  vendor HTTPS; disabled/no-provider remains the only runtime state until a
  separately reviewed adapter is implemented.
  - [x] 2026-08-09 fixed direct-vendor HTTPS architecture: accept
    [ADR 0013](docs/adr/0013-metadata-only-remote-ai-transport.md) without
    adding a provider or network consumer. V1 uses reviewed built-in
    adapters and exact HTTPS origins/paths only; arbitrary URLs, redirects,
    caller-supplied headers, commands, SDKs, local subprocesses, XPC providers,
    VMs, proxies implemented by DUX, uploads, cookies, caches, telemetry, streaming,
    background work, and automatic retry are outside the boundary. Provider
    selection is explicit and each transmission requires a user-invoked
    explanation plus inspection of the exact path-free disclosure. Only a
    core-minted privacy proof bound to one retained succeeded-snapshot lease may
    become input; parsed or Swift-authored JSON can never be upgraded into that
    proof. DUX-managed API credentials use the exact data-protection Keychain
    class/service/account/synchronizability/accessibility tuple and never enter
    settings, payloads, persistence, logs, or errors; credential verification
    is local-only. A future native transport must use an ephemeral session, default
    platform TLS validation, redirect refusal, fixed request/deadline and
    64-KiB response caps, cancellation/task teardown, no retry, and no tool,
    function, MCP, web, file, image, URL-fetch, computer, code-execution, or
    optional server-storage capability. Mandatory provider caching/retention is
    disclosed and never described as disabled merely by `store: false`.
    Adapter-owned provider/model identity is
    attached only after the existing all-or-error Rust output validator accepts
    the exact input digest. Failure leaves deterministic Explorer state
    unchanged, and accepted prose remains inert, non-linkified presentation
    with no candidate, rule, safety, plan, approval, schedule, or executor edge.
    Each concrete provider still requires its own fixed-envelope tests and data-
    retention disclosure before it can be enabled. The private shaper remains
    dormant with no engine, FFI, Swift, provider, or network consumer in this
    checkpoint, so disabled/no-provider is still the sole runtime state.
    Seven focused architecture-policy tests, all 120 repository policy tests,
    the clean 371-source destructive-call boundary, workspace formatting, and
    warning-denied workspace Clippy pass. Independent boundary review also
    tightened the exact data-protection Keychain selector/tuple, made
    provider-mandated retention and caching explicit, removed a stale raw-
    insight FFI suggestion, and required migration of the legacy 16-MiB cache
    row before any future AI cache can admit validated output.
- [x] Bind the privacy proof to one exact retained Explorer review and expose a
  preview-only disclosure boundary before adding transport code.
  - [x] 2026-08-09 retained-review metadata preview and UniFFI v59: the only
    engine consumer of the private shaper obtains both the immutable snapshot
    document and complete typed coverage from the same live Explorer pin. The
    repository reloads the exact succeeded scan under the current history
    guard, requires its identical retained snapshot reference, and revalidates
    the retained object; the engine repeats the coverage and lease proof after
    bounded shaping. A non-cloneable core preview freezes a two-minute
    monotonic/wall deadline capped by its parent lease, retains the exact parent
    owner/session/scan identity, and keeps the request-local input-ID to
    snapshot-node mapping private for future overlay validation. The exact
    canonical path-free JSON shares its bounded backing storage with the sealed
    proof rather than duplicating the 256-KiB input cap. FFI v59 accepts only a
    record version and selected snapshot node ID, admits at most one available
    preview per engine, strongly retains the exact parent review, registers the
    child weakly, and drains it before parent reviews on close/reset. Its
    read-only info contains the exact JSON bytes and digest, explicit
    content/path/name false flags, aggregate privacy disclosure, generic
    labels, byte/age accounting, omission facts, and expiry. The native service
    adapter independently validates that projection and owns explicit release;
    no controller, AppModel, view, provider/model selection, credential,
    endpoint, network task/callback, output parser, cache, candidate, planner,
    approval, scheduler, CLI, or filesystem-effect edge exists. Focused
    ownership, redaction, wrong-engine/review, expiry/release, singleton,
    close/reset, FFI, and native adapter tests plus source-policy guards prove
    the boundary. Provider-disabled remains the only runtime state.
- [ ] Implement the approved remote request deadline, response-byte limit,
  cancellation/task teardown, redirect refusal, and no-retry lifecycle.
  Process-tree cleanup is inapplicable because ADR 0009 still prohibits local
  provider processes.
  - [x] 2026-08-09 dormant native lifecycle and credential-store prerequisite:
    an isolated Security-framework boundary uses only the exact generic-
    password data-protection Keychain tuple, a closed adapter-account enum,
    non-synchronizing device-only values, no access group, a fresh
    authentication context that refuses UI on every operation, and redacted
    1–512-byte visible-ASCII secret objects. Settings capability can observe
    only absent/present/failed and explicitly replace/delete; a separate
    request capability is the only secret reader. An independent Foundation
    lifecycle admits only a sealed attempt, creates one ephemeral cookie/cache/
    credential-free data session and one data task, freezes one original 60-
    second monotonic deadline before injected credential preparation, enforces
    the 384-KiB request cap and the 64-KiB delivered/decompressed response cap
    incrementally regardless of declared length, accepts status 200 only,
    refuses the first redirect and every non-server-trust credential challenge,
    and owns exactly-once cancellation, session invalidation, terminal
    publication, late-callback fencing, and no retry through an injected
    validation-handoff seam. Synchronous `SecItem*` work cannot be physically
    cancelled; a timed-out generation discards its late result and cannot start
    networking. Adversarial tests use only injected Keychain, clock,
    preparation, validation, session, task, response, redirect, and challenge
    fakes. Production still has no endpoint, authentication header, provider
    envelope/model, constructible request, preview/FFI/EngineService/AppModel/
    UI/CLI consumer, cache, candidate, plan, approval, schedule, cleanup, or
    executor edge. The parent task remains open until a separately reviewed
    fixed adapter and core-owned orchestration prove the same deadline across
    the real credential-to-Rust-validation handoff.
  - [x] 2026-08-09 fixed Anthropic Messages v1 adapter review: revision 1 owns
    only `POST https://api.anthropic.com/v1/messages`, API version
    `2023-06-01`, the exact `claude-sonnet-4-6` model, a one-request
    `x-api-key`, JSON media types, 8,192 output tokens, one constant system
    instruction, one exact metadata text block, and one constant
    provider-compatible structural output schema. The canonical encoded body
    is capped at 384 KiB and the metadata value at 256 KiB. It sends no tools,
    functions, MCP, web/file/image/URL capability, cache control, beta,
    thinking, streaming, background, conversation, metadata, storage, or
    fallback option. The all-or-error extractor admits only bounded duplicate-
    free `application/json` with the exact model/assistant/message/end-turn
    echo and one non-empty text block; tool/server-tool/thinking/file/image/
    search/refusal blocks, alternate stop reasons, unknown structure, duplicate
    keys, trailing data, depth abuse, and over-limit envelopes fail closed.
    Extracted bytes remain untrusted and are not displayed until the later Rust
    validation handoff. The reviewed disclosure conservatively records normal
    30-day provider deletion plus longer safety/legal exceptions, a possible
    24-hour constant grammar-schema cache, possible billing, and never infers
    ZDR from a key. Production compiles the private adapter but has no caller;
    only a DEBUG harness supplies inert input/key fixtures and no test contacts
    Keychain or the network. The complete review and suspension conditions are
    frozen in
    [docs/provider-reviews/anthropic-messages-v1.md](docs/provider-reviews/anthropic-messages-v1.md).
    The parent lifecycle task remains open because no core proof, credential
    lookup, native task, extractor, and Rust validator yet share one single-use
    60-second orchestration.
- [x] Validate tools-disabled behavior for each approved remote adapter as
  defense in depth: send no tool/function/server-tool declaration, reject every
  tool-shaped response, and reject an adapter whose API cannot guarantee that
  boundary.
  - [x] Anthropic Messages v1 revision 1 omits the capability fields entirely,
    rejects every non-text/tool-shaped block and non-terminal stop reason, and
    has no retry, continuation, or provider fallback. The reserved OpenAI
    identity is not an implemented or approved concrete adapter; any future
    provider or revision must pass this gate independently.
- [ ] Implement “Explain selection” and group overlays.
- [ ] Add “View metadata sent”; add cache and clear-cache controls only after
  the reserved SQLite row is migrated to the 64-KiB, fully revision-bound
  validated contract.
- [ ] Prove through type/module boundaries that AI cannot create plans.

Exit criteria:

- AI is entirely optional.
- Malformed or malicious output cannot reference unknown nodes or trigger actions.
- No file content can be sent in v1.
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

Goal: make the macOS app primary without abandoning CLI users, complete the
signed direct-distribution pipeline, and deliver reliable in-app updates with
Sparkle 2.

Frozen updater decision: use Sparkle 2 as DUX's sole in-app updater for the
direct Developer ID distribution. Do not build a custom updater or defer the
choice to implementation; the remaining work is the gated Sparkle 2
integration and release qualification specified below and in ADR 0002.
Sparkle 2 is a required Milestone 9 exit capability, not an optional follow-up;
no second updater framework or App Store update path may be added alongside it.

Tasks:

- [x] Add Settings CLI installer/upgrader/remover.
  - The app packages a reproducible universal arm64/x86_64 `dux` built for
    macOS 14 and ad-hoc signed with Hardened Runtime for local/CI builds. A
    hidden, argument-free CLI command emits only the bounded version,
    database-schema, and snapshot-format tuple. A strict build-time finalizer
    rejects duplicate, missing, unknown, malformed, non-canonical, or
    mismatched fields and binds the signed executable SHA-256 plus the exact
    architecture list. The Xcode post-compile phase verifies the signature,
    identity, architectures, deployment target, executable output, hash, and
    manifest before and after embedding; Debug and Release must embed
    byte-identical pairs. The production release lane re-signs the inner CLI
    with `<bundle-id>.cli`, rebinds the manifest to those signed bytes, and
    validates it again before signing the outer app.
  - UniFFI v50 exposes the engine's exact database-schema and immutable
    snapshot-format versions beside the library/contract versions, allowing
    native Settings and the packaged CLI manifest to report the same
    compatibility tuple without opening storage.
  - Settings lazily inspects only the fixed `~/.local/bin/dux` destination and
    offers Install, Upgrade, Reinstall, and Uninstall only for the exact
    observed state. Every action uses a one-shot confirmation containing the
    reviewed version and fixed display path. A changed observation cancels the
    operation; an unexpected mutation failure clears stale status and requires
    an authoritative reload. Closing Settings discards an unaccepted
    confirmation, while app shutdown generation-fences pending reads and waits
    for any already-confirmed synchronous mutation before closing the service.
  - The installer derives HOME from the current account rather than the
    environment, traverses HOME/`.local`/`bin` descriptor-relatively with
    `O_NOFOLLOW`, creates only missing per-user directories, rejects unsafe
    owners/modes/symlinks/non-regular files/hard links/oversized binaries, and
    never executes the installed destination. It verifies full bytes,
    universal Mach-O slices, the DUX marker, and static code-signature identity
    before treating a target as managed. It never overwrites or removes an
    unmanaged target, never downgrades a newer managed CLI, never escalates
    privileges, never writes a system directory, and never reads or edits shell
    startup files; Settings provides copyable PATH guidance only.
  - Install uses a private create-new sibling, writes and `fsync`s the complete
    source, adds a bounded DUX-owned xattr marker, reopens and revalidates the
    staged bytes/signature, then publishes with `RENAME_EXCL`. Upgrade uses
    `RENAME_SWAP`, validates the exact displaced managed inode before unlinking
    it, and swaps back on any mismatch. Uninstall first swaps the reviewed
    managed target with a random, fully observed sentinel; it removes the
    displaced inode and final sentinel only after exact revalidation, restoring
    the original target on a race. Directory `fsync` and typed unknown-outcome
    handling prevent a UI success claim without a proven final state.
  - The installed CLI remains a separate Terminal process and does not inherit
    app TCC access. Sparkle and app removal cannot mutate it; a later app
    version merely reports mismatch and offers the explicit atomic upgrader.
    Standalone Homebrew/crates.io installs remain external and are never
    adopted as app-managed.
  - Verified 2026-07-30 with formatting, workspace check, and workspace Clippy
    with warnings denied; all 41 CLI tests; all 97 runnable UniFFI tests with
    the two quiescence-only cleanup cases intentionally ignored; all 597 native
    tests, including 26 focused installer/model cases; all 39 repository script
    tests; and the 284-file destructive-call audit. The 1,206-case core
    aggregate passed 1,197, ignored three intentional host/performance helpers,
    and exposed six pre-existing load-sensitive FSEvents/revalidation timing
    cases while native tests and the source audit were concurrent; every one
    of those exact six cases then passed alone. Debug/Release Swift bindings
    are byte-identical. The final ad-hoc-signed Release app and bundled CLI are
    both exactly universal arm64/x86_64, target macOS 14.0, and pass strict
    all-architecture code-signature validation; the CLI is reproducible at
    SHA-256
    `34a7d6608efe85eef5bdd5a8db54364ce4255b52c9d39b350b0a7ef9d49ec1f9`.
    The embedded canonical manifest reports CLI 0.5.0, database schema 16, and
    snapshot format 1. The app retains `LSUIElement=true`, resolved Release
    settings omit the internal permanent-cleanup condition, and the exact
    verified Release executable was launched for manual review.
- [x] Add final JSON scan-detail, candidate, review-state, and cleanup-history
  commands with golden schema tests. Completed 2026-07-30:
  - `dux scan-detail` exposes one bounded immutable coverage-issue page without
    stored relative components; it reports only global, scan-root, or
    descendant scope plus explicit truncation. `dux candidates` exposes the
    complete validated evaluation state and bounded ordinal pages of path-free
    candidate summaries. Both retain exact scan identity, stable ordering,
    null semantics, and explicit continuation fields.
  - `dux cleanup-history list/show` exposes bounded newest-first keyset pages
    and one exact path-free session observation. The schema preserves complete
    versus legacy record shape, lifecycle and capacity observations, exhaustive
    item/path status counts, item summaries, and warnings while omitting target
    paths, evidence payloads, candidate IDs, execution owners, generations,
    claims, receipts, and effect fences. Cursor time and session ID are an
    indivisible pair.
  - `dux review-state` accepts only `select`, `clear-selection`, `dismiss`, or
    `restore` for one exact scan/candidate pair. It retains and revalidates the
    same snapshot-review lease used by Explorer, returns
    `cleanup_performed: false`, and cannot construct a plan, approve, schedule,
    recover, retry, or execute cleanup. A post-write uncertainty returns the
    non-retryable `outcome_unknown` result and requires a fresh candidates read.
  - Every JSON object is independently schema-versioned. Stable path-free
    errors distinguish missing scans/candidates/sessions, non-reviewable
    candidates, invalid immutable cursors, storage failures, and ambiguous
    outcomes. The normative command, pagination, privacy, compatibility, null,
    and error contract is frozen in `docs/CLI_JSON.md`; README and the security
    design describe the same boundary. Golden tests cover empty and populated
    shapes, paging/null semantics, review non-authority, newer-schema behavior,
    and structured errors, with process-boundary tests proving no terminal
    control bytes or forbidden storage/path keys escape.
  - Verified 2026-07-30 with formatting, workspace check, and workspace Clippy
    with warnings denied; all 51 CLI tests; all 97 ordinary UniFFI tests plus
    both quiescence-only cleanup cases in their dedicated lane; all 39
    repository script tests; and the clean 285-file destructive-call audit.
    The 1,206-case core aggregate passed 1,196, ignored three intentional
    host/helper cases, and exposed seven pre-existing load-sensitive
    FSEvents/revalidation deadline cases; every exact case passed in a fresh
    isolated process, with two transient stream/provenance failures requiring
    one additional fresh-process attempt. Native XCTest passes all 597 tests.
    The generated CLI and both unsigned Debug/Release apps are exactly
    universal arm64/x86_64, target macOS 14.0, embed the byte-identical
    Hardened Runtime ad-hoc-signed CLI and canonical manifest, and retain
    `LSUIElement=true`; the bundled CLI SHA-256 is
    `f175881db8361336521ff367be269d098a8aa045109dca7744dd3405419879a6`.
    Resolved Release settings omit the internal permanent-cleanup condition.
- [x] Add cross-process scan-scope leasing and version-skew tests so app and CLI
  cannot run conflicting overlapping scans.
  - Completed 2026-07-30 with checksummed SQLite schema v17. One bounded,
    immutable, random 128-bit lease names a losslessly encoded canonical root,
    exact process instance, acquisition time, and optional stable-host/boot
    provenance. Exact, ancestor, and descendant roots conflict across
    processes; component-distinct siblings remain independent. Every legacy
    `running` scan root, including an unclaimed row, is a transitional blocker.
    The registry retains at most 64 rows and fails closed when full.
  - User full/subtree scans and targeted recommendation scans acquire before
    engine queue publication. Queued cancellation, preemption, close, worker
    panic, and normal completion retain the move-only lease until work has
    quiesced, then exact-reconcile its release outside the task-registry lock.
    The progressive CLI acquires before terminal takeover, holds through scanner
    join, and preserves its existing tree if rescan admission is refused.
    Cache-only browsing takes no lease. The public standalone handle grants
    observation exclusion only: it cannot create scan history, snapshots,
    candidates, AI records, plans, cleanup authority, or filesystem effects.
  - Stale rows are reclaimed only from complete same-host prior-boot evidence
    or reliable same-scope proof that the exact process instance is definitely
    gone. Age and PID alone never suffice. Commit uncertainty is resolved only
    by exact-token post-state; failed storage revalidation leaves a durable
    process-lifetime availability quarantine. A newer schema stays read-only,
    and simultaneous helper processes prove exactly one overlapping acquisition
    wins before the winner's exact release allows a replacement.
  - Verification includes formatting, workspace Clippy with warnings denied,
    ten focused scope tests plus the exact two-process race, newer-schema,
    unclaimed-row, and populated-v16 migration cases; all 54 CLI tests; all 97
    ordinary UniFFI tests with two intentional quiescence-only ignores; all 39
    repository script tests; and the clean 286-file destructive-call audit.
    The full core host lane exercised 1,221 cases: 1,215 passed, three were
    intentionally ignored, two compatibility fixtures exposed by v17 were
    corrected and passed exactly, and one unrelated load-sensitive queue test
    passed exactly in a fresh process. The added two-process race also passes
    in its focused lane. Native XCTest passes all 597 tests.
  - Debug and Release generation produce byte-identical Swift bindings,
    universal Hardened Runtime ad-hoc-signed CLI binaries, and schema-v17
    manifests; the CLI SHA-256 is
    `afd17c9fda533b64ef56e23f082193d03634dc31034f77d7bc07e3d9baf123b9`.
    Both unsigned apps are universal arm64/x86_64, target macOS 14.0, and retain
    `LSUIElement=true`; the final Release app is signed, verified, and launched.
- [ ] Add Storage & Privacy settings: expose snapshot-cap get/set/reset through
  FFI/Swift, report DUX-owned SQLite/snapshot/AI footprint, and provide
  separately confirmed cache, history, snapshot, and reset-app-data operations
  through narrow marker-validated core boundaries that never touch user data.
  - [x] 2026-07-30 snapshot-cap Settings transport: UniFFI v51 projects the
    existing exact-key core policy as versioned, path-free get/set/reset
    records with default/stored provenance, optional update time, idempotence,
    and a typed error taxonomy. Zero and `u64::MAX` cross the boundary exactly.
    The adapter rejects unsupported record versions and inconsistent
    source/time shapes; Swift independently validates every response before
    publishing it.
  - Native Settings now presents the effective cap, provenance, exact GiB
    editor, Save, and Restore DUX default controls. The isolated observable
    model serializes operations off the main thread, generation-fences stale
    completions, preserves its last confirmed value on failure, and requires
    an explicit authoritative reload after an unknown or malformed write
    result. Shutdown cancels and joins the model before closing the engine.
    Copy states plainly that changing or zeroing the cap does not run
    retention, that protected or uncertain storage may exceed it, and that the
    setting cannot select a snapshot or touch user data.
  - Verification covers the exact core policy boundary, all 98 ordinary
    UniFFI tests with two intentional quiescence-only ignores, workspace Clippy
    with warnings denied, all 606 native tests, all 39 repository script tests,
    and the clean 290-file destructive-call audit. Debug and Release generation
    produce byte-identical Swift bindings, bundled CLI, and schema-v17 metadata;
    the Swift binding SHA-256 is
    `1271adec2b7b3afe2e3a783888771cb39827dc4e92644f04ecc3eb6eb4288e36`
    and the CLI SHA-256 is
    `afd17c9fda533b64ef56e23f082193d03634dc31034f77d7bc07e3d9baf123b9`.
    The final Debug and Release apps plus their embedded CLIs are universal
    arm64/x86_64, target macOS 14.0, and preserve `LSUIElement=true`; Release
    omits the internal permanent-cleanup condition and passes strict Hardened
    Runtime ad-hoc signature verification.
  - [x] 2026-07-30 DUX-owned storage footprint: UniFFI v52 now exposes one
    input-free, versioned, path-free observation of the active private database
    and snapshot stores. The core reports exact logical, handle-derived
    allocated, and conservative per-file charged usage for the retained SQLite
    main/sidecar/control files and the complete bounded snapshot inventory. It
    separates protected and retention-eligible available snapshots,
    tombstoned residuals, physical orphans, and active, quiescent, and unleased
    recognized temporary files; revalidates both stores under the permanent
    database-before-snapshot lock order; and rejects drift, unsafe storage,
    incompatible schemas, budget exhaustion, or arithmetic inconsistency
    without a partial result.
  - Embedded AI accounting sums strictly validated insight IDs, input digests,
    provider/adapter/model labels, and output payloads through an
    allocation-constant SQLite pager with VM/time limits and no arbitrary row
    ceiling. Its total and expired subset are logical content already inside
    SQLite, never added to the physical total or presented as reclaimable
    space. Directory metadata, unattributable interrupted provisioning stages,
    and the unmarked caller-selected legacy CLI cache remain explicitly
    excluded.
  - Native **Storage & Privacy** performs the synchronous observation off the
    main thread, coalesces one lazy load, refreshes only on request,
    generation-fences shutdown, and preserves the last complete observation
    after failure. It shows a full-range `UInt64` summary and non-color-only
    database/snapshot stacked chart, keeps embedded AI content separate, and
    states that the measurement does not inspect ordinary user files and is
    neither free space, reclaimable space, nor cleanup authority. Swift repeats
    the complete accounting algebra and accepts more than 4,096 valid AI rows.
  - Verification covers the exact core AI/physical/snapshot accounting tests,
    all 101 ordinary UniFFI tests with two intentional quiescence-only ignores,
    all 614 native tests, all 39 repository script tests, and the clean
    296-source destructive-call audit. Formatting, workspace check, and
    warning-denied workspace Clippy pass. The serialized 1,227-case core
    aggregate passed 1,205 tests and ignored three intentional host/helper
    cases; its 19 failures were existing isolation-sensitive cleanup/review
    cases, while every footprint case and representative `OutsideHome` and
    `ChangedDuringReview` failures passed in fresh exact processes.
  - Debug and Release generation produce byte-identical Swift bindings and
    bundled CLI payloads. The Swift binding SHA-256 is
    `1ebd5f5ccae60d529da34bb1a5c289fd4d36baa501f8dfff8e812f6aa9bee0ef`
    and the CLI SHA-256 is
    `e52f967d11c36ed42b23e3ff35cc195cd78ee4ee111b3e59297ad13a563dfcd5`.
    The final Debug and Release apps plus their embedded CLIs are universal
    arm64/x86_64, target macOS 14.0, and preserve `LSUIElement=true`; resolved
    Release settings omit the internal permanent-cleanup condition. The exact
    Release app passes strict Hardened Runtime ad-hoc signature verification
    and is the launched menu-bar process.
  - [x] 2026-07-31 marker-owned managed scan cache: the progressive CLI has
    migrated from caller-selected legacy cache files to the engine-owned fixed
    `Dux/scan-cache-v1` child. The conventional outer cache directory remains
    unmarked and outside DUX ownership, so existing legacy and unknown siblings
    are never adopted, inventoried, attributed, migrated, or cleared. Cache
    load/write failures are non-fatal presentation failures; the CLI scans
    fresh, retains its exact standalone scan-scope lease through completed
    cache publication, joins an older writer before rescan, and joins the final
    writer before engine close. `--no-cache` skips reading but still refreshes
    the managed cache after a successful scan.
  - The independent SHA-256-bound managed wire format never falls back to the
    legacy decoder. It preflights a fixed header before body allocation and
    validates root/config binding, metadata/tree equality, contiguous graph,
    unique sibling components, depths, aggregates, timestamps, paths, and
    checksum. Hard caps admit at most a 64 MiB file, 200,000 nodes, depth 512,
    8 KiB metadata, 24 MiB of names, 64 MiB of reconstructed paths, and 192 MiB
    of modeled decode residency. The Unix store is descriptor-relative and
    no-follow, requires current-user ownership, exact 0700/0600 permissions,
    single-link files and acceptable ACLs, uses a retained cross-process writer
    lock, fixed controls, create-new temporaries, atomic publication, bounded
    inventories, and typed before-publication versus outcome-unknown results.
    Normal save admission is 2,048 objects/64 temporaries; bounded recovery can
    observe and clear one additional object and temporary, then fails closed.
    Windows deliberately reports unsupported until equivalent handle/DACL
    evidence exists.
  - UniFFI v53 extends the path-free DUX-owned footprint with exact managed
    cache control, published-entry, temporary-remnant, and total usage. The
    checked additive physical total is now database/history + snapshots +
    managed cache; embedded AI remains a non-additive SQLite subset. Native
    **Storage & Privacy** presents all three physical shares with redundant
    labels, symbols, colors, and patterns plus exact cache counts and charged
    usage.
  - Clearing uses one engine-bound, consume-once preview over the exact current
    entries and recognized temporaries, expires after two monotonic minutes,
    accepts no path/key/name/selector, and requires a separate native
    destructive confirmation spelling out counts, accounting, exclusions, and
    the absence of a free-space promise. The final locked inventory must match
    before effect. Controls, outer/legacy cache siblings, embedded AI,
    database/history, snapshots, settings, and user files are unreachable.
    Changed state is rejected before effect; a possibly committed result is
    reported as outcome unknown, remeasured once, and never retried.
  - Verification passes formatting, workspace check, warning-denied Clippy,
    the full 1,263-case serialized core lane, all 48 CLI unit and six
    process-boundary tests, all ordinary UniFFI tests, all 39 repository script
    tests, the clean 299-source destructive-call audit, and all 622 linked
    native tests. Debug and Release generation produce byte-identical Swift
    bindings with SHA-256
    `fea84f8edf0693aa407c9e1cf401134161b7de1e17902a59c3fbcdffbe06177c`;
    the generated and embedded CLI SHA-256 is
    `8e2838b6cbf8184a2da4031c59454c237679b3fc9f4e7539c5c888105d9a440a`.
    Both unsigned apps and embedded CLIs are universal arm64/x86_64, target
    macOS 14.0, retain `LSUIElement=true`, and have identical three-file app
    layouts.
  - [x] 2026-07-31 separately confirmed older-snapshot clearing: UniFFI v54
    adds one path-free, engine-bound, consume-once preview over the exact
    retention-eligible available snapshot finals and physically present
    tombstoned residuals. The two-minute monotonic preview includes protected
    and excluded scalar accounting but exposes no path, scan identity, file
    name, digest, token, selector, or mutation authority. The complete private
    witness binds every final and body digest, exact-root latest-two rank,
    active review pin, orphan, recognized temporary, residual lease, control,
    identity, and charged-usage fact. Unstable temporary accounting reports
    busy instead of creating a partial confirmation.
  - Final consumption repeats current-schema database-before-snapshot locking,
    requires the complete witness to remain identical, predecodes every
    selected immutable body, atomically appends all required tombstones in one
    transaction, and then removes only the exact retained-handle finals.
    Latest-two snapshots for every exact encoded root and every active review
    remain protected. Orphans, all temporary and provisioning categories,
    controls, history, scan cache, AI content, settings, legacy cache data, and
    user files are unreachable. Pre-effect drift is
    `changed_since_preview`; any possible post-effect ambiguity is
    `outcome_unknown`.
  - Native **Storage & Privacy** offers **Clear older snapshots…** separately
    from managed-cache clearing. Its accessible confirmation names exact
    counts and charged accounting, protected/excluded classes, and the absence
    of a free-space promise. The isolated model synchronously claims one
    dialog, serializes both clear operations, invalidates stale footprint data
    before effect, remeasures exactly once after every terminal effect attempt,
    and never retries deletion. Shutdown releases unaccepted previews and
    waits for a confirmed synchronous effect while suppressing late UI
    publication.
  - Verification passes formatting, workspace check, warning-denied Clippy,
    all 1,268 serialized core cases (1,265 passed and three intentional
    host/performance helpers ignored), all 108 ordinary UniFFI tests plus both
    quiescence-only effect cases in dedicated processes, all 48 CLI unit and
    six process-boundary tests, all 39 repository script tests, the clean
    300-source destructive-call audit, and all 631 linked native tests. Debug
    and Release generation produces byte-identical Swift bindings at SHA-256
    `b6b71a89c20c480ec1e72c574640dc4c92c5d248c095d8aeb8034f8eefe7edd5`
    and byte-identical bundled CLIs at SHA-256
    `b2dc54608572e2aab10de9674be8e935a6e19ffde850ef72523677ba0575a972`.
    Clean Debug and Release apps and their CLIs are universal arm64/x86_64,
    target macOS 14.0, retain `LSUIElement=true`, and have matching
    five-regular-file bundle layouts: the app executable, bundled CLI,
    canonical CLI manifest, `Info.plist`, and `PkgInfo`. Resolved Release
    settings omit the internal permanent-cleanup condition; the exact Release
    app and CLI pass strict all-architecture Hardened Runtime ad-hoc signature
    verification, and that exact Release app is the launched menu-bar process.
  - [x] 2026-07-31 dormant reset-journal foundation: the accepted contract is
    frozen in `docs/APP_DATA_RESET.md`. Reset is a terminal app lifecycle, not
    SQL deletion or another live clear action. Logical reset will atomically
    detach only the exact marker-owned Application Support namespace and fixed
    managed-cache child; physical reclamation remains a separately observed,
    bounded detached-stage drain. User files, the unowned outer cache,
    unknown/legacy siblings, the installed CLI, Launch at Login, TCC and
    notification authorization, the app bundle, and Sparkle state are
    explicitly excluded.
  - The private Unix/macOS coordinator owns no reset target and is deliberately
    unreachable from the engine, FFI, CLI, and app in this checkpoint. Its
    independently marker-owned sibling is provisioned through a random private
    marker-complete stage and atomic no-replace publication. It uses exact
    0700/0600 current-user, one-link, ACL, retained-identity, component-wise
    no-follow, bounded-inventory, permanent-lock, and directory-durability
    checks. Marker-complete abandoned stages are reconciled only while their
    exact writer lock remains held; partial or hostile stages remain
    non-authoritative and untouched.
  - The canonical 4 KiB maximum journal is versioned, deny-unknown,
    domain-separated SHA-256 checked, and byte-canonical. It seals one random
    transaction, exact data/cache filesystem identities, derived stage names,
    and the strict forward-only phases `prepared`, `cache_detached`,
    `data_detached`, `fresh_namespace_ready`, `draining`, and `complete`.
    Begin refuses a second incomplete transaction; every advance is exact
    compare-and-swap under the permanent lock. Atomic replacement, directory
    sync, and read-back distinguish pre-publication refusal from
    outcome-unknown durability. This foundation performs no namespace detach,
    recursion, or user-data effect.
  - Verified 2026-07-31 with all 13 focused coordinator/journal cases; locked
    workspace check and warnings-as-errors Clippy; 1,275 serialized core
    passes with three ignored cases and three unrelated loaded Cargo/FSEvents
    timing cases each passing in a fresh exact rerun; all 108 ordinary FFI
    cases plus both isolated consume-once effect regressions; all 54 CLI
    unit/process-boundary cases; all 39 repository script-policy cases; the
    302-source destructive-call audit; and all 631 linked native tests. Clean
    Debug and Release app layouts and embedded CLI bytes match. Both apps and
    the bundled CLI are exactly arm64/x86_64, target macOS 14.0, and retain
    `LSUIElement=true`; the preserved Release app passes strict and deep
    all-architecture Hardened Runtime ad-hoc signature verification.
  - [x] 2026-07-31 terminal engine arbitration: the task registry now commits
    one private terminal intent under the same mutex that closes task
    admission. Ordinary close and app-data reset race atomically; only the
    reset winner receives a move-only shutdown capability, and no later close
    can upgrade to reset or second reset can duplicate it. Winning reset
    admission cancels queued and running work while preserving the existing
    rule that queued closures are dropped only after releasing the registry
    mutex. A separate move-only quiescence capability is returned only after
    lifecycle `Closed` and every worker handle has been joined, including
    poison recovery. Timeout consumes reset authority and leaves the old
    engine terminal.
  - This checkpoint creates no durable reset intent, opens no reset
    coordinator, performs no filesystem effect, and has no FFI, CLI, or
    native caller. Six focused cases cover cancellation, closed admission,
    ordinary-close precedence, simultaneous close/reset, simultaneous
    reset/reset, bounded timeout, and poisoned worker-handle recovery.
  - Verified 2026-07-31 with all 29 reset-focused core cases; locked workspace
    check and warnings-as-errors Clippy; 1,297 serialized core passes with
    three intentional ignores; all 108 active FFI cases with two intentional
    ignores plus both isolated Rust-target cleanup regressions; all 54 CLI
    unit/process-boundary cases; all 39 repository script-policy cases; the
    303-source destructive-call audit; and all 631 linked native tests.
    Regenerated Swift bindings remained byte-identical
    (`b6b71a89c20c480ec1e72c574640dc4c92c5d248c095d8aeb8034f8eefe7edd5`)
    and the bundled CLI retained its canonical hash
    (`fb1b5ef39858bc1e1414f122b07f6090fb8bb65514c2fd1cb219d615afaf64fb`).
    Clean Debug and Release apps and both embedded binaries are universal
    arm64/x86_64, target macOS 14.0, and retain `LSUIElement=true`; the
    preserved exact Release app passes strict and deep all-architecture
    Hardened Runtime ad-hoc signature verification.
  - [x] 2026-07-31 retained coordinator and database-side reset admission:
    one callback-scoped session holds the coordinator writer lock across typed
    recovery, provisioning-debt inspection, begin, and exact journal
    transitions. An atomic same-instance fence makes nested and concurrent
    attempts immediately busy without re-entering or releasing the outer OS
    lock. Checksummed schema v18 adds two no-row-rewrite partial indexes for
    bounded unresolved cleanup item/path probes.
  - Store admission is reachable only inside the retained coordinator session
    and its higher-ranked callback prevents the inner move-only guard from
    escaping or reversing the required coordinator → cleanup → database order.
    It reports path-free scalar blockers for cleanup-lock contention,
    running/recovering cleanup, durable `effect_started`/`outcome_unknown`
    item/path evidence, running scan/process-claim evidence, and scan-scope
    leases. Admission retains cleanup exclusion plus the database
    writer/connection; blocked results retain no guard. Cross-process tests
    prove cleanup contention blocks and an admitted writer prevents a new
    scan-scope commit until release.
  - This checkpoint creates no reset intent composition, owns no reset target,
    performs no namespace or user-data effect, and has no engine, FFI, CLI, or
    native caller. The parent remains open for integration with terminal
    quiescence and every in-memory worker/review/preview/maintenance/confirmed
    CLI-mutation blocker, snapshot/cache locks, exact namespace
    witnesses/detachment, pre-open roll-forward recovery, bounded
    detached-stage draining, path-free consume-once FFI/Swift transport, the
    separately confirmed native **Reset DUX** sheet, exact native-preference
    allowlist, relaunch, accessibility, and macOS Debug/Release race evidence.
    No button is admitted before all of those gates pass.
  - Verified 2026-07-31 with all 30 reset-focused core cases; locked workspace
    check and warnings-as-errors Clippy; 1,303 serialized core passes with
    three intentional ignores; all 108 active FFI cases with two intentional
    ignores plus both isolated Rust-target cleanup regressions; all 54 CLI
    unit/process-boundary cases; all 39 repository script-policy cases; the
    clean 304-source destructive-call audit; and all 631 linked native tests.
    Regenerated Swift bindings remained byte-identical
    (`b6b71a89c20c480ec1e72c574640dc4c92c5d248c095d8aeb8034f8eefe7edd5`)
    and the bundled CLI retained its canonical hash
    (`9409940b76b0fb4a18026095f4f7d8de0dc2a4c61a0aba31c9a2ee9b21958d6b`).
    Clean Debug and Release apps and both embedded binaries are universal
    arm64/x86_64, target macOS 14.0, retain `LSUIElement=true`, and report
    database schema 18; Debug and Release embed byte-identical CLI/metadata
    pairs. Resolved Release settings omit the internal permanent-cleanup
    condition, and the preserved exact Release app passes strict and deep
    all-architecture Hardened Runtime ad-hoc signature verification.
  - [x] 2026-07-31 coordinator-first terminal/store reset preflight:
    one private core-only boundary derives the coordinator root solely from
    the store's revalidated canonical database identity. Under one retained
    coordinator session it rejects incomplete journal state as
    recovery-required and refuses unproven coordinator provisioning debt
    before terminal arbitration. Those pre-terminal outcomes leave the prior
    engine lifecycle unchanged by the attempt; they do not falsely claim an
    already-terminal engine is open.
  - Only after preflight clears does the boundary atomically claim terminal
    reset, cancel queued/running work, verify the complete registry
    worker/maintenance inventory, and join every worker. It then checks the
    currently modeled process-local active-cleanup and process-quarantine bits
    before and after retained cleanup/database admission. Active
    snapshot-review pins join the path-free durable blocker set; expiry
    equality is inactive, while every expired row is still strictly decoded
    and relationship-validated.
  - The admitted higher-ranked callback runs only while coordinator ownership,
    engine quiescence, cleanup exclusion, and database writer/connection
    exclusion coexist. It repeats coordinator and store revalidation and
    cannot return any retained proof. Typed outcomes distinguish pre-terminal
    store/coordinator/journal/debt refusal, terminal ownership already held by
    ordinary close/reset, and post-terminal shutdown/runtime/store/coordinator
    refusal. Panic releases store exclusion before the coordinator and never
    reopens the old engine.
  - This composition may provision or reconcile only the independent
    coordinator namespace. It writes no reset-journal phase and exposes no
    path, target identity, namespace witness, reset-target operation, FFI,
    CLI, or native caller. The immediately following checkpoint adds
    snapshot/present-cache writer admission; all remaining
    FFI-child/review/preview/confirmed-CLI-mutation handoff, absent-cache
    fencing, exact namespace witnesses, `Prepared` intent,
    detach/recovery/fresh provisioning/drain, path-free transport, native
    confirmation/relaunch, and Windows evidence remain later gates; **Reset
    DUX** remains absent.
  - Verified 2026-07-31 with all 39 reset-focused core cases; formatting,
    locked workspace check, and warnings-as-errors Clippy; 1,329 serialized
    core and projection passes with three intentional ignores; all 108 active
    FFI cases with two intentional ignores plus both isolated Rust-target
    cleanup regressions; all 54 CLI unit/process-boundary cases; all 39
    repository script-policy cases; the clean 304-source destructive-call
    audit; and all 631 linked native tests. Regenerated Debug and Release Swift
    bindings remained byte-identical
    (`b6b71a89c20c480ec1e72c574640dc4c92c5d248c095d8aeb8034f8eefe7edd5`).
    Clean Debug and Release apps plus both embedded CLI binaries are universal
    arm64/x86_64, target macOS 14.0, retain `LSUIElement=true`, and embed
    byte-identical schema-v18 CLI/metadata pairs; the CLI SHA-256 is
    `e96fc93e96d5f29d4606d34da1d85428cbafc2d598d160be1957d42a284af1fd`.
    Resolved Release settings omit the internal permanent-cleanup condition,
    and the preserved exact Release app passes strict and deep
    all-architecture Hardened Runtime ad-hoc signature verification.
  - [x] 2026-07-31 snapshot and present-cache reset admission: extend the
    private core composition through the final two current storage locks.
    Snapshot acquisition requires the matching retained database guard,
    retains the canonical writer plus complete bounded inventory, repeats that
    inventory before handoff, and treats any active staged snapshot writer as
    typed busy. A present managed cache is re-probed read-only, then retains
    its canonical cross-process writer lock and complete inventory. A missing
    lazy cache is never provisioned and remains an explicit
    `ManagedCacheAbsenceUnfenced` refusal until an outer container witness can
    fence absence.
  - The complete acquisition sequence now consumes one monotonic deadline
    across coordinator, core-worker quiescence, cleanup/database, snapshot, and
    cache waits. Terminal/runtime mutex observation consumes that same budget;
    queued closure destruction is charged to tracked worker quiescence; and a
    typed final expiry check prevents callback admission after slow complete
    revalidation. Higher-ranked callback wrappers borrow every owned proof;
    normal return, panic, and deliberately forgotten wrappers all release
    cache → snapshot → database → cleanup → coordinator in reverse order.
    Revalidation rejects canonical snapshot/cache detachment and new unknown
    children even when an actor ignores an advisory lock. Real subprocess
    regressions prove database and managed-cache writer exclusion. A
    contended coordinator-provisioning stage is marker-complete before its
    lock wait and is safely reconciled by a later successful open.
  - This remains a dormant core-only admission checkpoint. It writes no reset
    journal phase and exposes no path, target identity, namespace witness,
    reset-target operation, FFI, CLI, or native caller. It does not fence an
    absent cache or prove release of UniFFI/native reviews, previews, tasks, or
    confirmed CLI mutations. Exact namespace witnesses, `Prepared` intent,
    detach/recovery/fresh provisioning/drain, path-free transport, native
    confirmation/relaunch, and Windows evidence remain later gates; **Reset
    DUX** remains absent.
  - Verified 2026-07-31 with all 48 app-data-reset-focused core cases;
    formatting, locked workspace check, and warnings-as-errors Clippy; 1,340
    serialized core passes with three intentional ignores plus all 13
    projection cases; all 108 active FFI cases with two intentional ignores
    plus both isolated Rust-target cleanup regressions; all 54 CLI
    unit/process-boundary cases; all 39 repository script-policy cases; the
    clean 304-source destructive-call audit; and all 631 linked native tests.
    Regenerated Debug and Release Swift bindings remained byte-identical
    (`b6b71a89c20c480ec1e72c574640dc4c92c5d248c095d8aeb8034f8eefe7edd5`).
    The exact Release app and its embedded CLI are universal arm64/x86_64,
    target macOS 14.0, retain `LSUIElement=true`, and embed the canonical
    schema-v18 CLI/metadata pair; the CLI SHA-256 is
    `4fc197dde4f143b173086e8f14a44aae8b887df48d30120a3306f692e48fe0d8`.
    Resolved Release settings omit the internal permanent-cleanup condition,
    and the preserved exact Release app passes strict and deep
    all-architecture Hardened Runtime ad-hoc signature verification.
  - [x] 2026-07-31 exact reset namespace publication admission: one private,
    neutral transaction now generates 128 random bits and derives distinct
    typed `.dux-reset-data-<id>` and `.dux-reset-cache-<id>` components.
    Journal construction and both namespace witnesses consume that sealed
    transaction, so callers cannot inject a path or swap the data/cache
    destinations. No `Prepared` journal phase is written in this checkpoint.
  - Normal Unix/macOS data-root probing and publication now use the same
    retained parent-directory fence as reset admission. The validation-only
    reset witness binds the canonical root name and retained root/parent
    identities, exact spelling, private ownership, same-filesystem detach
    boundary, and absent typed destination. It additionally refuses the
    unimplemented `ai` and `logs` reserved children, and revalidation accepts
    only the database guard issued by that exact store. Same-process and real
    subprocess regressions prove that a cooperating root publisher cannot
    cross the retained fence; collision, expiry, mount-device drift, canonical
    replacement, panic, and forgotten borrowed wrappers fail closed.
  - Managed-cache opening and provisioning now share two descriptor-backed
    publication fences: the conventional cache parent and the `Dux` container
    when present. Reset can therefore prove either the absent outer container,
    absent fixed child, or a present marker-owned `scan-cache-v1` child without
    provisioning anything. Present admission additionally requires the child
    to share the container filesystem, retains its writer and complete
    inventory, and proves the typed cache destination absent. Unknown outer
    siblings remain unowned and untouched. Same-process and real-process
    races cover absent publication and present writer exclusion.
  - The complete private order is coordinator → data publication → cleanup →
    database → snapshot → cache-parent publication → optional `Dux`
    publication → optional cache writer, all under the original absolute
    deadline. Runtime blockers are inspected after quiescence, after the data
    fence wait, after database admission, and during final revalidation.
    Higher-ranked validation-only wrappers cannot retain an owned lock; normal
    return, panic, and deliberate forgetting release everything in reverse.
    This remains effect-dormant: it exposes no path, target operation, journal
    transition, detach, FFI, CLI, Swift, or user-visible **Reset DUX** action.
    Strong FFI/native child and confirmed-CLI-mutation quiescence, durable
    intent, detach/recovery/fresh provisioning/drain, transport, confirmation,
    relaunch, preference handling, and Windows evidence remain later gates.
  - Verified 2026-07-31 with all 54 app-data-reset-focused and all 61
    reset-named core cases, all 13 projection cases, locked workspace check,
    warnings-as-errors Clippy, and formatting. The serialized full-core lane
    produced 1,351 passes and three intentional ignores; seven
    host-load-sensitive FSEvents fail-closed refusals each passed unchanged
    when rerun as its exact isolated case. All 108 active FFI cases and both
    isolated Rust-target cleanup regressions passed, with two intentional FFI
    ignores; all 54 CLI cases, all 39 repository script-policy cases, the
    clean 305-source destructive-call audit, and all 631 linked native tests
    passed. Debug and Release generated Swift bindings remained byte-identical
    (`b6b71a89c20c480ec1e72c574640dc4c92c5d248c095d8aeb8034f8eefe7edd5`).
    Exact universal Debug and Release builds contain only arm64/x86_64, target
    macOS 14.0, retain `LSUIElement=true`, and embed the canonical universal
    CLI (`c18d8d73173b813039f965c0304721117ec8eb563b803429e856624c9fdb0db3`).
    Resolved Release settings omit the internal permanent-cleanup condition;
    the preserved Release app has empty entitlements and passes strict and
    deep all-architecture Hardened Runtime ad-hoc signature verification.
  - [x] 2026-07-31 confirmed CLI-mutation terminal quiescence: the native CLI
    installation model now fences every later load/prepare/confirm admission
    before the first terminal suspension, cancels only unconfirmed work, joins
    any accepted install, upgrade, reinstall, or uninstall, discards an
    unconsumed confirmation exactly once, and closes its installer service
    before issuing a narrowly typed `ConfirmedCLIMutationQuiescence` marker.
    Its fileprivate initializer prevents another native model from fabricating
    the proof. One retained terminal task coalesces concurrent and reentrant
    callers through service close; cancelling the initiating caller cannot
    abandon or overtake the accepted mutation.
  - Ordinary `AppRuntime` shutdown consumes that exact marker before engine
    close through a marker-requiring helper. Focused races cover all four
    mutation actions, terminal-before-confirm ordering, late preparation,
    success, typed failure, outcome-unknown, unexpected failure, caller
    cancellation, concurrent terminal callers, single close, and the complete
    `perform end → installer close → engine close` runtime order.
    This is deliberately not a generic native-quiescence claim: startup,
    settings operations, Explorer work, review controllers, capacity,
    maintenance, and the complete FFI child/task surface still need one
    reset-specific terminal gate and joined drain before durable `Prepared`.
    No reset FFI/API, generated binding, journal transition, namespace effect,
    UI, preference clearing, relaunch, CLI command, or Windows claim is added.
  - [x] 2026-07-31 FFI reset terminal and child quiescence: replace the former
    one-bit closed observation with one session-wide gate carrying `Open`,
    typed ordinary-close/reset `Closing`, terminal `Closed`, and the exact count
    of admitted child operations. DUX engine calls retain the state lock across
    their complete callback; every review/preview method and scan, cleanup,
    dry-run, or maintenance poll/cancel carries the same gate. A terminal claim
    rejects all later admission, and losing close/reset contenders rendezvous
    on the winning result rather than returning an unproven snapshot.
  - A winning private reset validation drains and checks every live child in
    all seven FFI registries, including asynchronous plan-review and direct
    Cargo release, then joins session callbacks and plan operations before
    entering core exactly once. One original absolute deadline flows through
    FFI and core. Release error, poison, or timeout skips reset validation,
    closes the old engine, and permanently publishes unquiesced state; a
    background close may reclaim resources but cannot upgrade that failed
    proof. Live-child and blocking-iCloud-callback races prove the boundary.
  - The cross-crate core adapter is callback-free and accepts no payload. Its
    non-exhaustive result preserves only bounded terminal/recovery
    classification; it carries no path, transaction, witness, journal
    transition, target operation, or effect authority. The handoff is private
    Rust code outside every UniFFI export. It adds no generated binding,
    `Prepared` journal phase, namespace effect, reset API, Swift/UI/CLI caller,
    or public **Reset DUX** action.
  - Verified 2026-07-31 with all 120 active FFI cases and two intentional
    ignores plus both isolated Rust-target cleanup regressions; all 54 CLI
    unit/process-boundary cases; locked workspace check, warning-denied Clippy,
    formatting, all 39 repository script-policy cases, and the clean
    307-source destructive-call audit. The serialized full-core lane produced
    1,360 passes, four load-sensitive fail-closed refusals, and three
    intentional ignores; every one of the four exact cases subsequently passed
    unchanged in a fresh isolated process. All 641 linked native tests pass.
    Generated Swift remains byte-identical at SHA-256
    `b6b71a89c20c480ec1e72c574640dc4c92c5d248c095d8aeb8034f8eefe7edd5`.
    Clean Debug and Release app layouts match and embed byte-identical Sparkle
    2.9.2 and CLI payloads; DUX, its CLI, the Sparkle framework, and all four
    reviewed Sparkle helpers are exactly arm64/x86_64. Both app configurations
    and their CLIs target macOS 14.0 and retain `LSUIElement=true`; the CLI
    SHA-256 is
    `38c1696b344d37be698f341c1109eb79121fb2cf707f02fcaa64f3cea1af4342`.
    Resolved Release settings omit the internal permanent-cleanup condition.
    The exact Hardened Runtime ad-hoc-signed Release app, including explicitly
    signed nested Sparkle code, passes strict deep all-architecture verification
    and is preserved at
    `/private/tmp/dux-reset-ffi-release.CLy0Q1/DUX.app`.
  - [x] 2026-07-31 native runtime terminal and child quiescence: `AppRuntime`
    now owns one `Open` → typed `Closing` → `Closed` arbiter for ordinary quit
    and effect-dormant app-data reset. The first intent is immutable;
    concurrent and repeated callers share one retained completion even when an
    awaiter is cancelled. Before its first suspension the winner fences
    AppModel, CLI, and Explorer admission, then joins accepted startup,
    owned-storage Settings, CLI mutations and discards, AppModel operations,
    scans, Explorer work, reviews, capacity sampling, and maintenance. Scan
    cancellation is requested before retained polling drivers are joined, and
    preview/lease/discard releases remain independently retained after their
    presentation slots disappear.
  - The aggregate `NativeRuntimeResetQuiescence` marker is minted only after
    every native owner drains and incorporates the narrower confirmed-CLI
    marker. Ordinary quit closes the engine exactly once after that proof;
    reset never ordinary-closes it. Startup/drain recursion returns an explicit
    no-proof result. A repository source-layering test confines terminal
    ingress to top-level app/runtime files so a joined child cannot call back
    into the terminal task that is awaiting it. This slice remains
    effect-dormant: it adds no generated Swift/UniFFI change, public reset
    method, durable `Prepared` intent, namespace mutation, preferences,
    relaunch, Reset UI, CLI command, AI edge, or Windows claim.
  - Verified with all 680 linked native tests, including adversarial scan,
    cleared-slot release, concurrent caller, cancellation, ordering, and
    reentrancy races; all 40 repository script-policy tests; and the clean
    309-source destructive-call audit. The serialized core lane passes 1,364
    tests with three intentional ignores. The ordinary FFI lane passes all 120
    runnable tests with two intentional Rust-target cleanup cases ignored; each
    exact ignored case passes in the isolated cleanup lane. The CLI passes all
    54 unit and process tests. Generated Swift remains byte-identical at SHA-256
    `b6b71a89c20c480ec1e72c574640dc4c92c5d248c095d8aeb8034f8eefe7edd5`,
    and XcodeGen regenerates the committed project deterministically at
    SHA-256
    `49fcca83a7ec0765fb114be7de50c9e8d63bcc6691670dd8e70b8d4c43677a35`.
    The bundled universal CLI is unchanged at SHA-256
    `38c1696b344d37be698f341c1109eb79121fb2cf707f02fcaa64f3cea1af4342`.
    Unsigned Debug and Release apps are exact arm64/x86_64 universals, target
    macOS 14.0, retain `LSUIElement=true`, embed reviewed Sparkle 2.9.2, and
    have identical payload layouts. Release omits the Debug-only permanent
    cleanup condition. The exact Hardened Runtime ad-hoc-signed Release app,
    including explicitly signed nested Sparkle code, passes strict deep
    all-architecture verification and is preserved at
    `/private/tmp/dux-native-runtime-release.oH979Y/DerivedData/Build/Products/Release/DUX.app`.
  - [x] 2026-07-31 durable `Prepared` reset intent: the retained core admission
    can now be consumed exactly once to project data and optional managed-cache
    device/inode identities only from descriptor-backed witnesses and publish
    the exact singleton `Prepared` coordinator journal. Before the write it
    repeats all namespace/runtime/store/snapshot validation, then rechecks the
    empty journal, absent provisioning debt, and original monotonic deadline
    after identity projection. A pre-publication refusal proves no intent; any
    rename attempt or later durability/read-back failure returns payload-free
    recovery required and must never be retried.
  - The committed higher-ranked continuation retains the exact journal,
    coordinator session, quiescence, publication fences, database/cleanup and
    snapshot guards, optional cache writer, transaction, runtime inspector, and
    original deadline. It cannot escape the callback and exposes only complete
    revalidation, including final exact-journal and coordinator-debt checks.
    This checkpoint adds no cache/data detach, fresh namespace, draining,
    pre-open recovery, public FFI/API, generated binding, Swift/UI/CLI caller,
    preference mutation, relaunch, cleanup authority, or Windows claim.
  - Verified with all 47 app-data-reset-focused core cases and the four-point
    journal-publication certainty regression; locked workspace check,
    warnings-as-errors Clippy, and formatting; all 120 active FFI cases with two
    intentional direct-Cargo cleanup ignores; all 54 CLI cases; all 40
    repository policy cases; and the clean 309-source destructive-call audit.
    The loaded serialized full-core lane produced 1,353 passes, 24 historical
    host-budget/FSEvents refusals, and three intentional ignores; every new
    reset-intent case passed. The saturated walker case passed unchanged once
    host load ended, while a representative historical Cargo-review case and
    the isolated FFI cleanup success case still fail closed as
    `ChangedDuringReview`; this pre-existing host-sensitive debt is not counted
    as reset-intent completion evidence.
  - Debug and Release generated Swift remain byte-identical at SHA-256
    `b6b71a89c20c480ec1e72c574640dc4c92c5d248c095d8aeb8034f8eefe7edd5`,
    and XcodeGen remains deterministic at
    `49fcca83a7ec0765fb114be7de50c9e8d63bcc6691670dd8e70b8d4c43677a35`.
    Both app configurations and their bundled CLI are exact arm64/x86_64
    universals targeting macOS 14.0, retain `LSUIElement=true`, have identical
    payload layouts, and embed the same CLI at SHA-256
    `1faed8943bdefa42157e2a033eac3b0c3bb1569742d3628795f3dc9156b436b4`.
    The Release app embeds reviewed Sparkle 2.9.2 with all five executables
    universal and omits the Debug-only permanent-cleanup condition. Its main
    app and nested Sparkle code are signed inside-out with ad-hoc Hardened
    Runtime signatures and pass strict deep all-architecture verification at
    `/private/tmp/dux-prepared-intent-release.b3MfD9/DerivedData/Build/Products/Release/DUX.app`.
  - [x] 2026-07-31 exact managed-cache detachment: the committed `Prepared`
    continuation now consumes its cache witness to perform the first reset
    namespace effect and exact journal transition. A present marker-owned
    `scan-cache-v1` is revalidated against its descriptor-derived journal
    identity, controls, complete inventory, typed destination, publication
    fences, and original deadline, then renamed descriptor-relatively with
    no-follow/no-replace semantics to its transaction-derived cache stage.
    The retained `Caches/Dux` directory is synchronized before canonical
    absence, exact staged identity, controls, and inventory are proven. A
    prepared-absent cache provisions and renames nothing.
  - Every detach layer consumes its witness, so an ambiguous rename cannot be
    retried. Once `Prepared` exists, pre-effect drift or expiry, destination
    collision, every rename/sync/read-back failure, and every journal-advance
    failure return only recovery required. Exact compare-and-advance publishes
    `CacheDetached` only after the namespace proof succeeds. The committed
    higher-ranked continuation retains all locks and changes cache validation
    to the exact stage name; stale canonical handles fail closed. Unknown
    outer-cache siblings and the canonical data namespace remain untouched.
    Fault regressions cover before/after rename, after directory durability,
    real detached-name/inventory read-back, all four journal publication
    boundaries, post-`Prepared` inventory drift, stage collision, deadline
    expiry, a panic between effect and journal transition, and second-launch
    observation of both `Prepared`/detached and `CacheDetached`/detached crash
    shapes. This checkpoint adds no data detach, fresh namespace, drain,
    pre-open recovery, public FFI/API, Swift/UI/CLI caller, preference mutation,
    relaunch, reclaimed-byte claim, user-data cleanup, or Windows support.
  - Verification: all 57 app-data-reset-focused core cases pass. The serialized
    full-core lane produced 1,386 passes, one historical host-load-sensitive
    `ChangedDuringReview`, and three intentional ignores; the exact failing
    case passed immediately in isolation. All 120 active FFI cases (with two
    intentional direct-Cargo cleanup ignores), all 54 CLI cases, all 40
    repository policy cases, the clean 309-source destructive-call audit,
    locked workspace check, warnings-as-errors Clippy, and formatting pass.
    The linked native suite passes all 680 tests.
  - XcodeGen is deterministic at SHA-256
    `49fcca83a7ec0765fb114be7de50c9e8d63bcc6691670dd8e70b8d4c43677a35`,
    and generated Swift remains unchanged at SHA-256
    `b6b71a89c20c480ec1e72c574640dc4c92c5d248c095d8aeb8034f8eefe7edd5`.
    Clean Debug and Release app layouts match; DUX, the byte-identical bundled
    CLI, Sparkle 2.9.2 framework, and all four reviewed Sparkle helpers are
    exact arm64/x86_64 universals. DUX and its CLI target macOS 14.0,
    `LSUIElement=true`, Release omits the internal permanent-cleanup condition,
    and the embedded CLI SHA-256 is
    `333e0e42430f413c553219ff27eb66e4989957c769e1cc98f6730be2253b69a9`.
    The exact Release app and reviewed nested Sparkle code are signed
    inside-out with ad-hoc Hardened Runtime signatures and pass strict deep
    all-architecture verification at
    `/private/tmp/dux-cache-detach.SmGYJA/Qualified/DUX.app`.
  - [x] 2026-07-31 exact data-root detachment and ordinary-engine reset gate:
    the committed `CacheDetached` continuation now consumes its data witness
    exactly once, repeats the complete journal/cache/data/store/snapshot and
    deadline proof, then renames the canonical data root descriptor-relatively
    with no-follow/no-replace semantics to the transaction-derived stage. It
    synchronizes the retained parent, proves canonical absence plus the exact
    staged identity and retained database/control/sidecar/snapshot layout, and
    exact-compare-and-advances only then to `DataDetached`. Reserved `ai` and
    `logs`, unproven snapshot-provisioning stages, canonical-root replacement,
    detached-cache replacement, cache appearance after an absence-bound
    detach, and last-moment destination collision all fail closed without
    moving the disputed object. Cache fingerprints prove contents remain exact
    across every data and journal fault boundary.
  - Ordinary engine open now opens or provisions the fixed coordinator and
    takes a shared cross-process reset lease before database, snapshot, cache,
    or worker publication. It retains that lease through worker quiescence and
    `Closed`; a second live engine therefore excludes reset even after the
    initiating engine terminates. Every decoded incomplete phase refuses open
    before canonical-store publication, `Complete` admits normally, and
    corrupt, unsafe, or unavailable coordinator state has a distinct
    coordinator-unavailable classification. Ordinary open performs no
    coordinator reconciliation or phase advance. Present journal-stage debt is
    accepted only as an exact private single-link regular file and is never
    removed by the shared reader.
  - Rename-attempt, sync, read-back, panic-gap, and journal uncertainty consume
    the witness and return only recovery required; they are never retried.
    All 92 app-data-reset-focused core cases pass. A host-loaded serialized
    full-core lane passed 1,382 cases with three intentional ignores and 18
    conservative query/FSEvents/Cargo-probe budget trips; every exact failure
    passed in a fresh isolated replay, with the already-built binary and quiet
    windows used for home-bound FSEvents cases. All 120 active FFI cases pass;
    the two intentional direct-Cargo cleanup ignores also pass when run from
    the already-built binary outside the sandbox. All 54 CLI cases, all 40
    repository policy cases, the clean 309-source destructive-call audit,
    locked workspace check, warnings-as-errors Clippy, and formatting pass.
    The linked native suite passes all 680 tests.
  - XcodeGen remains deterministic at SHA-256
    `49fcca83a7ec0765fb114be7de50c9e8d63bcc6691670dd8e70b8d4c43677a35`,
    and Debug/Release UniFFI generation leaves committed Swift unchanged at
    SHA-256
    `b6b71a89c20c480ec1e72c574640dc4c92c5d248c095d8aeb8034f8eefe7edd5`.
    Clean Debug and Release payload inventories differ only by Debug's two
    expected Swift debug binaries. DUX, the byte-identical bundled CLI,
    Sparkle 2.9.2 framework, and all four reviewed Sparkle helpers are exact
    arm64/x86_64 universals. DUX and its CLI target macOS 14.0,
    `LSUIElement=true`, and the embedded CLI SHA-256 is
    `475597aaadc65a936cc77baa4eef8dbc531c92d188be2b81c168f712cec3d702`.
    The exact Release app and reviewed nested Sparkle code are signed
    inside-out with ad-hoc Hardened Runtime signatures and pass strict deep
    all-architecture verification at
    `/private/tmp/dux-data-detach.VOvau4/Qualified/DUX.app`.
    This checkpoint still has no roll-forward recovery runner, canonical fresh
    namespace, `FreshNamespaceReady`, `Draining`, `Complete` transition or
    detached-stage deletion, public reset API/UniFFI/Swift/UI/CLI caller,
    preference mutation, relaunch, reclaimed-byte claim, user-file effect, or
    Windows support.
  - [x] 2026-07-31 pre-open detach-phase roll-forward recovery: ordinary engine
    acquisition now turns an incomplete shared-lease observation into a
    move-only intent retaining the original coordinator storage descriptors,
    lease, and exact journal. Recovery transfers to the exclusive writer lock
    through that same storage and re-reads the exact journal before any
    namespace operation, so a missing, replaced, or changed handoff cannot be
    re-opened, re-provisioned, or downgraded into ordinary engine admission.
    Transaction reconstruction accepts only the canonical 32-character
    lower-hex ID and its exact role-separated data/cache stage names.
  - One five-second admission deadline covers the coordinator handoff,
    descriptor-only namespace admission, and every pre-effect validation.
    Cache detach/read-back remains inside it; once the data-root rename begins,
    its fixed 250 ms post-effect durability/read-back proof may finish so an
    already moved namespace cannot be accepted without inspection. Descriptor-
    only data/cache recovery admissions retain the normal publication fences,
    database/cleanup/snapshot and cache-writer locks, accept exactly one
    canonical or transaction-derived detached namespace (or proven cache
    absence), and never provision controls, open SQLite, migrate, repair, or
    delete. `Prepared` may reconcile cache and then data through durable
    `CacheDetached` and `DataDetached`; `CacheDetached` may reconcile only the
    data detach and rejects a canonical cache; `DataDetached` validates the
    exact detached shapes without another effect. Later phases are untouched.
    Busy, drift, invalid transition, unavailable, or unknown outcome after an
    incomplete observation remain recovery-required; corrupt/unsafe storage
    remains coordinator-unavailable. Every outcome still refuses ordinary
    engine open because fresh canonical provisioning is not implemented.
  - Focused coverage includes effect-gap adoption from `Prepared`, both
    canonical detaches in one pass, `CacheDetached` data reconciliation,
    already-detached data, impossible phase/namespace combinations, exact
    journal handoff deletion/change races, malformed and role-swapped stage
    names, absent/present cache shapes, publication-lock contention, and
    consume-once no-replace behavior. This checkpoint adds no fresh namespace,
    `FreshNamespaceReady`, drain, completed-state physical proof, public
    reset/UniFFI/Swift/UI/CLI caller, preferences, relaunch, reclaimed-byte
    claim, user-file effect, or Windows support.
  - Verified with all 97 app-data-reset-focused core cases; the serialized
    full-core lane produced 1,417 passes, three historical host-load-sensitive
    failures, and three intentional ignores, and all three failures passed on
    exact quiet-host replay. All 120 active FFI cases (with two intentional
    direct-Cargo cleanup ignores), all 54 CLI cases, all 40 repository policy
    cases, the clean 310-source destructive-call audit, locked workspace check,
    warnings-as-errors Clippy, formatting, and all 680 linked native tests pass.
  - XcodeGen remains deterministic at SHA-256
    `49fcca83a7ec0765fb114be7de50c9e8d63bcc6691670dd8e70b8d4c43677a35`,
    and Debug/Release UniFFI generation leaves committed Swift unchanged at
    SHA-256
    `b6b71a89c20c480ec1e72c574640dc4c92c5d248c095d8aeb8034f8eefe7edd5`.
    Clean Debug and Release payload inventories differ only by Debug's two
    expected Swift debug binaries. DUX, the byte-identical bundled CLI,
    Sparkle 2.9.2 framework, and all four reviewed Sparkle helpers are exact
    arm64/x86_64 universals. DUX and its CLI target macOS 14.0,
    `LSUIElement=true`, Release omits the internal permanent-cleanup condition,
    and the embedded CLI SHA-256 is
    `f1b4384f329892490b0597bd051de34ea80002fc7bfafe0f63d71f1f2791a00c`.
    The exact Release app and reviewed nested Sparkle code are signed
    inside-out with ad-hoc Hardened Runtime signatures and pass strict deep
    all-architecture verification at
    `/private/tmp/dux-preopen-recovery.LWV7cK/Qualified/DUX.app`.
  - [x] 2026-08-01 transaction-bound fresh canonical namespace: the pre-open
    runner now continues an exact `DataDetached` journal through one
    callback-scoped fresh-root publisher and durably advances only to
    `FreshNamespaceReady`. A third typed transaction component,
    `.dux-reset-fresh-<32 lowercase hex>`, cannot be swapped with the cache or
    old-data stages. Journal V2 records that exact stage, the distinct fresh
    device/inode, and the lossless raw-byte canonical root component in
    lowercase hex minted only by the retained root witness. V1 `Prepared` and
    `CacheDetached` records upgrade only after the exact canonical old-root
    identity proves that binding; V1 `DataDetached`, `FreshNamespaceReady`, and
    `Draining` fail as incompatible rather than reconstructing a destroyed
    name or identity. An exact V1 `Complete` tombstone remains ordinary-open
    compatible.
  - Recovery retains coordinator → data-parent → old cleanup/database/snapshot
    → cache publication/writer order. It creates a random 0700 work directory
    containing exactly a zero-length database, V2 writer marker, cleanup
    marker, cleanup-ready marker, and `.dux-reset-origin-v1`; there is no SQLite
    header, initialization sentinel, sidecar, snapshot, cache, `ai`, or `logs`.
    The fixed origin binds transaction, old identity, and fresh identity. All
    five private single-link files and the exact directory inventory are
    synchronized and revalidated before no-replace random→typed publication,
    then the typed stage is synchronized/read back before no-replace
    typed→canonical publication and another durability/read-back proof. Each
    rename receives a bounded 250 ms post-effect certainty budget.
  - The published typestate is bound to the exact current durable journal,
    transaction, old identity, typed fresh stage, coordinator-parent identity,
    and the journal's durable canonical root name before its fresh identity can
    enter the journal. Cross-transaction admission, restart under a different
    sibling root, and commit misuse fail before namespace effect or durable
    advance. Recovery reports the latest durable phase after partial progress,
    rather than the phase first observed at open.
    Reopen at `FreshNamespaceReady` is validation-only; canonical/stage
    coexistence, foreign collisions, wrong origin, replacement, alias, extra
    entries, and marker drift fail closed without replacement. This checkpoint
    never opens/migrates SQLite, repairs ordinary storage, creates
    snapshots/cache, removes detached or random provisioning stages, reports
    reclaimed bytes, advances to `Draining`/`Complete`, or admits an ordinary
    engine. Public reset transport/UI, preferences, relaunch, user-file effects,
    and Windows support remain absent.
  - Verified with all 109 app-data-reset-focused core cases. The serialized
    full-core lane passed 1,431 cases with three intentional ignores; five
    host-load-sensitive review/Cargo deadline probes also passed on exact
    immediate replay. All 13 projection cases, 120 active FFI cases plus the
    ignored cleanup lane when invoked directly, 54 CLI cases, 40 repository
    policy cases, the clean 311-source destructive-call audit, locked Rust
    1.88 workspace check, warnings-as-errors Clippy, and formatting pass. All
    680 linked native tests pass. XcodeGen is deterministic at SHA-256
    `b26fba38dbce56d8fc9225e173ee27f4050eba56d811d1612ceca28cebce1fdc`,
    and Debug/Release UniFFI generation leaves committed Swift unchanged at
    SHA-256
    `b6b71a89c20c480ec1e72c574640dc4c92c5d248c095d8aeb8034f8eefe7edd5`.
    Clean Debug and Release payload inventories match apart from Debug's two
    expected Swift support binaries. DUX, the byte-identical bundled CLI,
    Sparkle 2.9.2 framework, and all four reviewed Sparkle helpers are exact
    arm64/x86_64 universals. DUX and its CLI target macOS 14.0,
    `LSUIElement=true`, and Release omits the internal permanent-cleanup
    condition. The exact Release app and nested Sparkle code were signed
    inside-out with ad-hoc Hardened Runtime signatures and passed strict deep
    all-architecture verification at
    `/private/tmp/dux-fresh-namespace.ska6Y1/Qualified/DUX.app`; its main
    executable SHA-256 was
    `3530f5d4fba44da9649bca1e1b21115757bf679318535e44d5db9a1736166a96`
    and its signed embedded CLI SHA-256 was
    `989b523ec90c0549be9f4d8a42452184e462be8eecc2cca6ebdb1aab871c1e38`.
  - [x] 2026-08-01 durable `Draining` and first bounded cache-debt batch:
    private pre-open recovery now continues an exact
    `FreshNamespaceReady` journal only after revalidating the current V2
    canonical-root binding, distinct fresh identity, old detached root, and
    detached-or-proven-absent managed cache. Generic journal advancement can
    no longer reach `FreshNamespaceReady`, `Draining`, or `Complete`. The
    coordinator alone can compare-and-publish `Draining`, read back that exact
    journal, repeat every data/cache witness check, and mint the private opaque
    capability required by the cache unlink primitive. Any uncertain journal
    publication returns no deletion authority; an already-`Draining` restart
    adopts the durable phase without rewriting it.
  - One recovery pass consumes one exact cache candidate and removes at most
    the lexicographically first recognized managed-cache entry or temporary.
    The complete bounded inventory excludes both retained controls; the
    descriptor-relative unlink is followed by detached-directory durability,
    exact stage identity/location validation, and a fixed 250 ms inventory
    read-back. Progress contains only `removed_objects` and
    `cache_payload_has_more`,
    never a name, path, byte count, or reclaimed-capacity claim. A journaled
    absent cache returns zero without provisioning its outer container. Unknown
    objects, links, replacement, wrong stage/identity, budget exhaustion, and
    final journal/fresh/cache drift all fail before unlink and leave the
    disputed object untouched.
  - Every unlink attempt consumes its candidate. The before-unlink gap is
    retryable only from a new exact recovery admission; all post-unlink,
    directory-sync, and read-back ambiguity remains recovery-required and is
    reconciled from durable `Draining` without repeating an already absent
    object. Focused tests also prove a journal change, fresh-database drift,
    cache-inventory drift, and a cache witness from another transaction cannot
    cross the final authority seam. Cache controls and the detached cache shell,
    all old data/snapshot objects, the fresh namespace, random provisioning
    debt, unknown siblings, and user files remain untouched. The runner still
    cannot remove those remaining classes, advance to `Complete`, validate a
    completed physical state, admit an ordinary engine, expose public
    reset/UniFFI/Swift/UI/CLI transport, mutate preferences, relaunch, or claim
    reclaimed bytes.
  - Focused verification passes all 114 app-data-reset-filtered core cases and
    all 39 managed-cache-local cases, including present/absent shapes,
    deterministic one-object progress, exact binding refusals, disputed
    objects, all journal/unlink durability gaps, and restart convergence.
  - Broad qualification passes the serialized full-core lane with 1,445 cases
    and three intentional ignores, all 13 projection cases, 120 active FFI
    cases with two intentional direct-Cargo cleanup ignores, all 54 CLI cases,
    all 40 repository policy cases, the clean 311-source destructive-call
    audit, locked Rust 1.88 workspace check, warnings-as-errors Clippy, and
    formatting. All 680 linked native tests pass. XcodeGen remains
    deterministic at SHA-256
    `b26fba38dbce56d8fc9225e173ee27f4050eba56d811d1612ceca28cebce1fdc`,
    and Debug/Release UniFFI generation leaves committed Swift unchanged at
    SHA-256
    `b6b71a89c20c480ec1e72c574640dc4c92c5d248c095d8aeb8034f8eefe7edd5`.
    Clean Debug and Release payload inventories match apart from Debug's two
    expected Swift support binaries. DUX, the byte-identical bundled CLI,
    Sparkle 2.9.2 framework, and all four reviewed Sparkle helpers are exact
    arm64/x86_64 universals. DUX and its CLI target macOS 14.0,
    `LSUIElement=true`, and Release omits the internal permanent-cleanup
    condition. The exact Release app and nested Sparkle code were signed
    inside-out with ad-hoc Hardened Runtime signatures and passed strict deep
    all-architecture verification at
    `/private/tmp/dux-draining.wC3C18/Qualified/DUX.app`; its signed main
    executable SHA-256 is
    `6bcd6ca5f5cfd9aae2ace867d47a32c51b4539aa6b36793b652bc4d45883568f`
    and its signed embedded CLI SHA-256 is
    `161de8641f047ce62d07484b70550c0d8ead4b1882f8d9a047ace7d37152cd6f`.
  - [x] 2026-08-01 detached managed-cache structural tail retirement: a
    recovery pass already observing exact durable `Draining` now uses one
    typed cache admission that returns either the existing payload candidate
    or a distinct consume-once structural candidate. A full marker-owned store
    with any recognized payload can only remove its lexicographically first
    payload. Once the complete bounded inventory is empty, later passes advance
    exactly `FullControlsEmpty` → `WriterOnly` → `EmptyStage` → `Absent`,
    removing at most one ownership marker, retained-and-locked writer control,
    or exact empty transaction-derived stage shell. The pass that first enters
    `Draining` cannot retire a structure, and no pass loops while `has_more`.
  - The retirement candidate remains bound to the exact V2 `Draining`
    journal, transaction, cache stage and original identity, fresh canonical
    identity/root, old detached identity/layout, coordinator parent, and
    canonical-cache absence. Persistence owns a separate private opaque
    retirement capability and repeats the journal/data/cache join immediately
    before effect. Control unlink synchronizes and reads back the stage;
    `rmdir` synchronizes the retained `Caches/Dux` container and proves both
    exact cache names absent. Pre-effect failure is retryable only through a
    new admission; every post-effect, sync, or read-back ambiguity remains
    recovery-required and resumes from the exact monotonic tail.
  - `MarkerOnly`, partial controls with any payload or unknown child, links,
    unsafe permissions, case-folded aliases, canonical/stage coexistence,
    replacement, wrong transaction/identity, writer contention, deadline
    exhaustion, and interposed journal/fresh/cache drift fail closed without
    an error-based fallback to payload draining. Cache progress remains private,
    path-free, byte-free, and makes no reclaimed-capacity claim. Repeated exact
    absence is a no-effect success and never provisions the outer cache
    container. Outer siblings, the fresh namespace, all old data/snapshot
    objects, random provisioning debt, and user files remain untouched. The
    journal remains `Draining`; old-data draining, `Complete`, completed-state
    admission, public reset transport/UI, preferences, and relaunch remain
    future checkpoints.
  - Focused qualification passes all 119 app-data-reset-filtered core cases and
    all 46 managed-cache-local cases. Coverage includes the complete four-state
    structural tail, recognized-payload routing, one-effect-per-open restart
    convergence, all before/after-effect/sync/read-back fault seams, retained
    writer exclusion, unsafe partial and case-alias refusals, no error-based
    fallback, exact absence without provisioning, final journal/fresh/cache
    authority drift, and cross-transaction rejection. Locked workspace check,
    warnings-as-errors Clippy, the Rust 1.88 compatibility check, formatting,
    all 40 repository policy cases, and the clean 311-source destructive-call
    audit pass. Broad qualification passes the serialized full-core lane with
    1,457 cases and three intentional ignores, all 13 projection cases, 120
    active FFI cases with two intentional direct-Cargo cleanup ignores, and all
    54 CLI cases. All 680 linked native tests pass against the regenerated
    Debug archive. XcodeGen remains deterministic at SHA-256
    `b26fba38dbce56d8fc9225e173ee27f4050eba56d811d1612ceca28cebce1fdc`,
    and Debug/Release UniFFI generation leaves committed Swift unchanged at
    SHA-256
    `b6b71a89c20c480ec1e72c574640dc4c92c5d248c095d8aeb8034f8eefe7edd5`.
    Clean Debug and Release payload inventories match apart from Debug's two
    expected Swift support binaries. DUX, the byte-identical bundled CLI,
    Sparkle 2.9.2 framework, and all four reviewed Sparkle helpers are exact
    arm64/x86_64 universals. DUX and its CLI target macOS 14.0,
    `LSUIElement=true`, and Release omits the internal permanent-cleanup
    condition. The exact Release app and nested Sparkle code were signed
    inside-out with ad-hoc Hardened Runtime signatures and passed strict deep
    all-architecture verification at
    `/private/tmp/dux-cache-retirement.ZZSZhr/Qualified/DUX.app`; its signed
    main executable SHA-256 is
    `ad30048645f8b799094d4e1f01afc50c9ab48a3ba17cc0197d2c1e65352bc0af`
    and its signed embedded CLI SHA-256 is
    `d5605ace624e9d8142b1c90f2ebe3b045eff9ad7a86c46b9cc81add417506078`.
  - [x] 2026-08-01 detached old snapshot-payload draining: once the managed
    cache is proven exactly absent, a later recovery pass already observing
    durable `Draining` may consume one distinct old snapshot candidate. The
    complete retained writer inventory selects only the lexicographically
    first final or quiescent recognized temporary; any active temporary,
    unknown object, unsafe link/permissions, identity or usage change,
    replacement, budget failure, inventory drift, or filesystem boundary from
    the old root through the snapshot directory, controls, or any payload
    refuses the whole batch. The pass that removes the cache shell cannot also
    remove a snapshot, and an empty snapshot payload inventory remains a
    validation-only no-effect pass.
  - The candidate owns the exact old/fresh namespace witness and is joined to
    the durable V2 journal plus a typed cache-absence witness. Persistence alone
    constructs the separate opaque consume-once capability after repeating the
    journal, transaction, coordinator-parent, canonical/fresh root, detached
    old root, snapshot inventory, and canonical/detached cache absence checks.
    The original recovery deadline remains the sole pre-effect deadline through
    the final complete inventory gate immediately before descriptor-relative
    unlink. Only a successful unlink starts one shared reset-specific 250 ms
    post-effect deadline for directory synchronization and every snapshot,
    old/fresh-root, cache-absence, and journal read-back. Progress contains only
    `removed_objects` and `snapshot_payload_has_more`; it never exposes a name,
    path, byte count, or reclaimed-capacity claim. Before-effect refusal
    requires a new admission; unlink, directory-sync, deadline, and read-back
    ambiguity remain recovery-required and resume without repeating an already
    absent object.
  - Focused coverage proves lexical final/temporary progress, one effect per
    open, cache-before-snapshot ordering, active-writer and unsafe-inventory
    refusal, exact store/object/transaction binding, final journal/fresh/cache/
    snapshot authority rechecks, all four uncertainty seams, and restart
    convergence. Snapshot controls and directory, the old database and
    remaining data-root tail, random provisioning debt, `Complete`, completed-
    state admission, public reset transport/UI, preferences, relaunch, and
    Windows support remain future checkpoints.
  - Verified 2026-08-01 with all 137 app-data-reset-filtered core cases, all 56
    snapshot-storage cases, and all 46 managed-cache cases. Formatting, locked
    workspace/all-target checks on the current toolchain and Rust 1.88,
    warnings-as-errors workspace Clippy, all 40 repository policy cases, and
    the clean 311-source destructive-call audit pass. The serialized full-core
    lane ran 1,478 cases: 1,469 passed, three intentional host/performance
    helpers were ignored, and six established host-load/order-sensitive
    registry/walker fixtures failed in the aggregate before every exact case
    passed its immediate isolated replay. All 13 projection cases, 120 active
    FFI cases plus both intentional direct-Cargo cleanup cases through their
    dedicated runner, and all 54 CLI cases pass. All 680 linked native tests
    pass against the regenerated Debug archive. XcodeGen remains deterministic
    at SHA-256
    `b26fba38dbce56d8fc9225e173ee27f4050eba56d811d1612ceca28cebce1fdc`,
    and Debug/Release UniFFI generation leaves committed Swift unchanged at
    SHA-256
    `b6b71a89c20c480ec1e72c574640dc4c92c5d248c095d8aeb8034f8eefe7edd5`.
    Clean Debug and Release app inventories match exactly, including
    byte-identical bundled CLI and metadata. DUX, that CLI, Sparkle 2.9.2, and
    all four reviewed Sparkle helpers are exact arm64/x86_64 universals. DUX
    and its CLI target macOS 14.0, `LSUIElement=true`, Release has empty
    entitlements and omits the internal permanent-cleanup condition, and the
    CLI metadata matches its exact signed bytes. The Release app was signed
    inside-out with ad-hoc Hardened Runtime signatures and passed strict deep
    all-architecture verification at
    `/private/tmp/dux-snapshot-drain-qualified.gsrvdO/DUX.app`; its signed main
    executable SHA-256 is
    `4cb4666bdb7f48aa61327d4649d6c5beb0802db99613442a88cc76e771fcc8f6`
    and its signed embedded CLI SHA-256 is
    `be43846e2d882b9effb9b7ea96eead604edd23f4040dbe10137b28aee1e2bd46`.
  - [x] 2026-08-01 detached old snapshot-store structural-tail retirement:
    after exact managed-cache absence and an empty old snapshot payload
    inventory, later recovery passes already observing durable `Draining` now
    recognize only `FullControlsEmpty` → `WriterOnly` → `EmptyDirectory` →
    `Absent`. Each pass removes exactly one descriptor-retained ownership
    marker, exclusively locked writer control, or exact empty snapshot
    directory. The state produced after every effect is admitted only by a new
    recovery open; no pass loops while `snapshot_store_has_more`, and the pass
    that removes the last payload cannot also retire a structure. Exact absence
    is a distinct validation-only witness and cannot fabricate removal
    authority or provision a new snapshot directory.
  - Structural admission is enabled only after the journal already says
    `Draining`; earlier fresh-bootstrap phases still require a complete normal
    snapshot store. During draining, marker-only, payload-without-controls, a
    writer-only or empty-directory tail with any other child, unsafe ownership/
    modes/ACL/link/alias/identity, replacement, mount boundary, writer
    contention, and inventory or deadline drift are hard failures. They never
    fall back to payload removal. The retained detached data root, snapshot
    directory, present control, old database guard, and fresh canonical root
    remain descriptor-bound and on the exact original filesystem through the
    final effect gate.
  - The structural candidate carries no effect primitive. Persistence may mint
    its separate consume-once authority only after joining the exact V2
    `Draining` journal, sealed transaction, coordinator parent/canonical-root
    binding, old/fresh identities, typed snapshot state, and exact cache-
    absence witness, then repeating those facts immediately before unlink or
    `rmdir` under the original five-second recovery deadline. A successful
    effect alone starts one new reset-specific 250 ms deadline shared by parent
    synchronization, snapshot-state read-back, old/fresh namespace read-back,
    cache-absence read-back, and exact journal read-back. Any uncertainty after
    effect remains recovery-required; restart observes the next exact state and
    cannot repeat the already absent object. Progress remains private, path-
    free, byte-free, and makes no reclaimed-capacity claim.
  - Focused coverage proves the complete four-state chain, one effect per open,
    exact-absence idempotence, all before-effect/final-gate and after-effect/
    sync/read-back/deadline uncertainty seams at every state, bounded writer
    contention and retry, unsafe marker-only/unknown-child/case-alias refusal,
    exact data-candidate/cache-witness cross-transaction refusal, final-payload
    handoff into the structural tail without false post-effect uncertainty, a
    fresh post-effect deadline that cannot be clipped by the expired admission
    deadline, and final coordinator authority rechecks after journal,
    fresh-root, cache-absence, and snapshot-inventory drift. The old database
    and detached data-root shell, random provisioning debt, completed-state
    validation, `Complete`, ordinary engine admission, public reset transport/
    UI, native preference allowlist/relaunch, reclaimed-capacity claims, and
    Windows support remain future checkpoints.
  - Verified 2026-08-01 with all 148 app-data-reset-filtered core cases and all
    62 snapshot-storage cases. The complete workspace is green: 1,487 of 1,490
    core cases passed with only three intentional host/performance helpers
    ignored, plus all 54 CLI cases, all 13 projection cases, all 120 active FFI
    cases, and both intentional direct-Cargo FFI cleanup cases through their
    dedicated runner. Formatting, locked workspace/all-target checks on the
    current toolchain and Rust 1.88, warnings-as-errors workspace Clippy, all
    40 repository policy cases, and the clean 311-source destructive-call audit
    pass. All 680 linked native tests pass. XcodeGen remains deterministic at
    SHA-256
    `b26fba38dbce56d8fc9225e173ee27f4050eba56d811d1612ceca28cebce1fdc`,
    and Debug/Release UniFFI generation leaves committed Swift unchanged at
    SHA-256
    `b6b71a89c20c480ec1e72c574640dc4c92c5d248c095d8aeb8034f8eefe7edd5`.
    Clean Debug and Release bundles have matching 127-entry inventories,
    byte-identical bundled CLI/metadata, `LSUIElement=true`, and exact macOS
    14.0 deployment. DUX, that CLI, Sparkle 2.9.2, and all four reviewed Sparkle
    helpers are exact arm64/x86_64 universals. Release retains the reviewed
    empty entitlement set and omits the internal permanent-cleanup condition.
    The inside-out ad-hoc Hardened Runtime copy passes strict deep all-
    architecture verification at
    `/private/tmp/dux-snapshot-structural-qualified.MVdvNN/DUX.app`; its signed
    main executable SHA-256 is
    `5a78a00053911bf02ab405015278ebeb56c75bfd3f73bb7361e20672ebb7b1dd`
    and its manifest-bound embedded CLI SHA-256 is
    `97a2369b819600a1397fff3fec7456cc8d1f43b8c30329e4e61648115d249520`.
  - [x] 2026-08-01 detached old SQLite payload draining: after exact managed-
    cache absence and retirement of the old snapshot directory, a reset-only
    opener now recognizes either the complete database-present tail or the
    exact `DatabaseAbsentControlsFull` state. It never provisions, repairs,
    opens, or migrates SQLite. Present sidecars are selected in raw-byte
    lexical order (`-journal`, `-shm`, `-wal`); only after every sidecar is
    absent may a later pass select the retained main database. One pre-open
    pass removes at most that one private, single-link, same-filesystem object.
    The initialization sentinel, cleanup/ready controls, exclusively locked
    writer control, detached data-root shell, fresh bootstrap, and durable
    `Draining` journal remain intact. Exact controls-only absence is a
    validation-only handoff for the later structural checkpoint and cannot
    fabricate another payload effect.
  - The old-root observation is bounded to the fixed database, three known
    sidecars, four controls, and an optional exact private snapshot directory.
    Snapshot presence and a typed non-absent cache admission are the only two
    no-effect reasons to resume the earlier pipeline; every unsafe inventory,
    alias, replacement, permission/link/identity error, mount boundary,
    deadline failure, or admission error remains recovery-required without
    fallback. This prevents an `ai`, `logs`, unknown, or disputed child from
    weakening the strict tail into a snapshot/cache effect. Both the initial
    effect-path journal read and every later read are exact and non-reconciling,
    so an unreconciled journal publication stage can never be consumed in the
    same pass as a SQLite unlink.
  - The candidate carries no unlink primitive. Only the coordinator may mint
    its consume-once capability after joining the exact V2 `Draining` journal,
    sealed transaction, canonical parent/root name, old/fresh identities,
    fresh five-entry bootstrap, exact snapshot absence encoded by the old-root
    inventory, cache-absence witness, retained cleanup/writer exclusions, and
    one shared original deadline. Every fact is repeated at admission, run,
    and the final descriptor-relative effect gate. A successful unlink alone
    mints one fresh 250 ms deadline covering old-directory synchronization,
    exact old/fresh-root inventory, cache-absence, and journal read-back. The
    fresh-root inventory now consumes that outer deadline directly rather than
    starting a nested budget. Any post-effect uncertainty consumes the
    candidate; restart observes the next exact state and cannot repeat an
    already absent object. Progress remains private, path-free, byte-free, and
    makes no reclaimed-capacity claim.
  - Focused coverage proves sidecar-first/main-last one-effect-per-open
    convergence, exact controls-only idempotence, certain final-main success,
    all six before/after-effect/sync/read-back/deadline fault seams, cache and
    journal postcheck deadline exhaustion, both cross-transaction joins,
    unreconciled journal-stage debt, and final authority refusal after journal,
    fresh-root, cache, old-inventory, or snapshot-absence drift. A reserved
    old-root child specifically proves strict rejection cannot fall back to a
    snapshot structural unlink. The remaining initialization/lock controls,
    detached root shell, random provisioning debt, completed-state validation,
    `Complete`, ordinary engine admission, public reset transport/UI, native
    preference allowlist/relaunch, reclaimed-capacity claims, and Windows
    support remain future checkpoints.
  - Verified 2026-08-02 with all 156 app-data-reset-filtered core cases and all
    62 snapshot-storage cases. The serialized 1,501-case full-core lane passed
    1,493 cases and ignored three intentional host/performance helpers; five
    Rust-target deadline/provenance cases failed only in the loaded aggregate
    and each passed its exact isolated replay. After Clippy prompted the
    repeated transaction/name arguments to be replaced by typed open/authority
    bindings, the affected 156-case reset lane passed again. All 13 projection
    cases, all 54 CLI cases, all 120 active FFI cases, and both intentional
    direct-Cargo FFI cleanup cases through their dedicated runner pass.
    Formatting, locked workspace/all-target checks on Rust 1.96 and minimum
    Rust 1.88, warnings-as-errors workspace Clippy, fuzz-adapter and isolated
    fuzz-harness Clippy, the Rust 1.88 fuzz-adapter check, all 40 repository
    policy cases, and the clean 311-source destructive-call audit pass. All
    680 linked native tests pass against the regenerated Debug archive.
    XcodeGen remains deterministic at SHA-256
    `b26fba38dbce56d8fc9225e173ee27f4050eba56d811d1612ceca28cebce1fdc`,
    and Debug/Release UniFFI generation leaves committed Swift unchanged at
    SHA-256
    `b6b71a89c20c480ec1e72c574640dc4c92c5d248c095d8aeb8034f8eefe7edd5`.
    Universal Debug and Release builds have matching 127-entry inventories,
    byte-identical manifest-bound CLI payloads, `LSUIElement=true`, and exact
    macOS 14.0 deployment. DUX, that CLI, Sparkle 2.9.2, and all four reviewed
    Sparkle helpers are exact arm64/x86_64 universals. Debug alone carries the
    internal permanent-cleanup condition; Release retains the reviewed empty
    entitlement set. The inside-out ad-hoc Hardened Runtime copy passes strict
    deep all-architecture verification at
    `/private/tmp/dux-old-database-qualified.tvzFuy/DUX.app`; its signed main
    executable SHA-256 is
    `959479c86a98bf95f9b96b721f289463ac19aa9bcb4276af28814c8f11613ab6`
    and its manifest-bound embedded CLI SHA-256 is
    `d0a5ceca62e5829b22ee6373e45e00751163ae8916bcd67a02db789df84dce98`.
  - [x] 2026-08-02 detached old-store structural-tail retirement: once the
    managed cache and snapshot store are exactly absent and every old SQLite
    sidecar plus the main database has drained, the reset-only opener now
    recognizes exactly `FullControlsEmpty` → `LockControlsFull` →
    `CleanupAndWriterControls` → `WriterOnly` → `EmptyDirectory` →
    `RootAbsent`. A new recovery open is required between states. Each of the
    first five states mints at most one consume-once effect: initialization
    sentinel unlink, cleanup-ready unlink, cleanup-lock unlink, writer-control
    unlink, then detached-root `rmdir`. Exact absence is a distinct no-effect
    witness; it can neither repeat the final effect nor admit ordinary storage.
  - Every state is derived from a complete bounded old-root inventory. A
    payload or snapshot requires all four controls; sidecars without the main
    database, an unknown child, and every non-monotonic partial-control shape
    fail closed. The data-parent publication fence remains held throughout.
    Cleanup and writer exclusions are retained while both controls exist,
    writer exclusion remains after cleanup retirement, and only the exact
    empty root may proceed without either old-root lock. All names, retained
    descriptors, old/fresh identities, transaction, filesystem boundary,
    canonical-root binding, and fresh five-entry bootstrap are revalidated at
    the final effect gate.
  - Persistence alone composes the structural candidate with the exact V2
    `Draining` journal and typed managed-cache absence under one original
    pre-effect deadline. Only a successful descriptor-relative unlink or
    `rmdir` creates the shared reset-specific 250 ms post-effect deadline.
    Synchronization targets the old root for control removal and the retained
    publication parent for root removal; exact old/fresh/cache/journal
    read-backs share that same deadline. Progress is private, path-free,
    byte-free, and never claims reclaimed capacity. Any post-effect ambiguity
    remains recovery-required, and restart can observe only the exact next
    typestate rather than recreating or repeating an absent object.
  - Focused storage and engine coverage proves the complete five-effect chain,
    one effect per open, exact-absence idempotence, cleanup-lock contention,
    non-monotonic-shape refusal, final inventory drift refusal, certain first-
    effect success, all six local pre/post-effect fault seams, both shared
    coordinator postcheck deadline seams, restart convergence, unchanged fresh
    bootstrap, and durable `Draining` state after root absence. Completed-state
    physical validation, journal publication to `Complete`, validation of that
    durable completed state before ordinary engine admission, explicit random
    `.dux-stage-*` provisioning-debt handling, public reset transport/UI,
    native preference allowlist/relaunch, capacity claims, and Windows evidence
    remain later checkpoints.
  - Verified 2026-08-02 with all 161 app-data-reset-filtered core cases and the
    serialized full-core lane: 1,507 passed, three intentional host/performance
    helpers were ignored, and zero failed. All 13 projection cases, 54 CLI
    cases, 120 active FFI cases, and both intentionally isolated FFI cleanup
    cases pass. Formatting, locked workspace/all-target checks on Rust 1.96 and
    minimum Rust 1.88, warnings-as-errors workspace Clippy, fuzz-adapter and
    isolated fuzz-harness Clippy, the Rust 1.88 fuzz-adapter check, all 40
    repository policy cases, and the clean 311-source destructive-call audit
    pass. All 680 linked native tests pass against the regenerated Debug Rust
    archive. XcodeGen 2.44.1 remains deterministic at SHA-256
    `b26fba38dbce56d8fc9225e173ee27f4050eba56d811d1612ceca28cebce1fdc`,
    and Debug/Release UniFFI generation leaves committed Swift unchanged at
    SHA-256
    `b6b71a89c20c480ec1e72c574640dc4c92c5d248c095d8aeb8034f8eefe7edd5`.
    Clean universal Debug and Release bundles contain 129 and 127 entries
    respectively; Debug's only extra entries are `__preview.dylib` and
    `DUX.debug.dylib`, and both embed byte-identical CLI/metadata. DUX, that
    CLI, Sparkle 2.9.2, and all four reviewed Sparkle helpers are exact
    arm64/x86_64 universals targeting macOS 14.0. `LSUIElement=true`; Release
    has empty entitlements and omits the internal permanent-cleanup condition.
    The inside-out ad-hoc Hardened Runtime copy passes strict deep all-
    architecture verification at
    `/private/tmp/dux-old-store-structural-qualified.PEp6gG/DUX.app`; its signed
    main executable SHA-256 is
    `4b2fad4be436d5b9ced4b678e81da98b3545a6d365d0bdc1bcf8ee19443037f2`
    and its manifest-bound embedded CLI SHA-256 is
    `ba22915c8947800196d2aaf898caba5a19cb9d507b68cb15cb59ae4e40f694f4`.
  - [x] 2026-08-02 completed reset tombstone and physical-state-gated ordinary
    admission checkpoint:
    - Once detached old-root and managed-cache absence are both exact under the
      durable V2 `Draining` journal, one completion pass can retire only the
      descriptor-retained `.dux-reset-origin-v1`. That consume-once unlink alone
      creates a fresh 250 ms synchronization/read-back deadline. A later pass
      that proves origin absence publishes the durable V2 `Complete` tombstone
      inside the original recovery deadline. Both passes remain typed recovery-
      required, so the incomplete recovery runner never opens SQLite or admits
      workers.
    - Ordinary open turns V2 `Complete` into a move-only validation intent that
      retains the original shared coordinator lease, storage descriptors, and
      exact journal. It can open only the existing journal-bound fresh root and
      never provisions or replaces a missing canonical root. Admission proves
      the exact fresh device/inode and raw canonical parent/name, data/fresh/
      cache transaction-stage absence, and absence of every sibling
      `.dux-stage-<32 lowercase hex>` provisioning debt. Prefix shape never
      grants removal authority, a case-folded root/cache/stage alias never
      satisfies exact spelling or absence, and a configured canonical root that
      itself has a stage-shaped name is excluded from the sibling-debt test.
    - Before the first initialization sentinel, the only accepted root inventory
      is the database plus writer/cleanup/ready controls and recognized SQLite
      crash sidecars; snapshots, snapshot provisioning stages, `ai`, `logs`, and
      unknown children refuse without mutation. Canonical cache must be absent.
      After initialization, normal DUX reserved directories, recognized snapshot
      stages, same-object macOS case aliases inside the evolved root, and a
      later valid cache distinct from the retired cache are ordinary state.
      Cache publication-fence configuration, container, deadline, contention,
      alias, and drift failures still block because exact transaction-stage
      absence is mandatory. Only `UnsafeStore`, `UnsafeObject`,
      `UnrecognizedStore`, `CorruptData`, and `Unavailable` for the exact
      canonical cache become optional ordinary managed-cache state after that
      proof.
    - The original absolute engine-open deadline now spans coordinator/root/
      cache/journal inspection, both publication registries, the in-process
      store registry, connection and writer-lock acquisition, and every
      pre-effect gate, including reuse of a live same-process
      `StoreCoordinator`. The state-appropriate narrow or evolved root
      inventory plus cache and journal validation repeat under that writer lock
      immediately before sidecar repair or SQLite open for both new and
      reused coordinators; root/cache/journal envelope validation repeats
      after store open. The final cache publication fence remains owned across
      the first store effect; release is nonblocking
      and fails closed by retaining its registry identity under contention. The
      preliminary initialized state is bound into the later fenced store
      preparation, and that preparation may not repair permissions or provision
      controls before the final gate.
      SQLite recovery or migration is not advertised as interruptible once
      started. Regression hooks prove last-moment drift cannot initialize,
      refresh, repair, or mutate the database.
    - Valid same-version and future-schema crash prefixes resume through the
      existing store path; future schema remains `ReadOnlyNewer` and gains the
      durable initialization sentinel. Ordinary corruption, migration, and
      availability failures keep their ordinary database classification.
      Exact V1 `Complete` remains direct legacy admission. A safe private
      coordinator journal stage beside the final tombstone is tolerated and
      untouched; malformed or unsafe stage shapes still fail closed.
    - The destructive-call policy admits exactly one new production primitive,
      `unlink_app_data_reset_fresh_origin_with_before_unlink`, descriptor-
      relative, internally fixed to `.dux-reset-origin-v1`, and bound to its
      single persistence caller plus a final whole-envelope callback. Policy
      self-tests reject a caller-controlled name, copied IDs, the wrong file/
      function/operation, and any unreviewed raw unlink. Focused regressions
      cover origin/complete restart convergence, missing/replaced/case-aliased
      roots, stage resurrection, cache aliases and random debt,
      publication/connection/writer deadline exhaustion, shared-lease
      retention, narrow/evolved inventories, stale initialized-state and
      permission non-mutation, crash prefixes, future schema, cache evolution,
      V1 completion, safe journal debt, ordinary-error classification, both
      new/reused root/cache/journal pre-effect drift seams, cache fence-error
      propagation, exact coordinator
      reuse, and a legitimate cache publisher that is `Busy` at the final store-
      effect seam but succeeds immediately after the retained fence drops.
    - Qualification on 2026-08-02 passes all 167 app-data-reset-filtered and all
      36 completed-reset-filtered core cases. Two serialized full-core passes
      each ran 1,551 cases: 1,546 passed, three intentional host/performance
      helpers were ignored, and two unrelated macOS Cargo-manifest replay
      guards refused transiently before authority. The four distinct affected
      Rust-target cases all passed in immediate exact-name isolation; no reset
      admission or persistence case failed. All 13 projection cases, 54 CLI
      cases, and 120 active FFI cases pass; two FFI cleanup cases are
      intentionally ignored in the loaded lane and both pass in exact
      isolation. Formatting, locked workspace/all-target checks on Rust 1.96
      and minimum Rust 1.88, warnings-as-errors workspace Clippy, fuzz-adapter
      and isolated fuzz-harness Clippy, the Rust 1.88 fuzz-adapter check, all 41
      repository policy cases, and the clean 311-source destructive-call audit
      pass.
    - All 680 linked native tests pass against the regenerated Debug Rust
      archive. XcodeGen 2.44.1 under Xcode 26.5/Swift 6.3.2 remains
      deterministic at SHA-256
      `b26fba38dbce56d8fc9225e173ee27f4050eba56d811d1612ceca28cebce1fdc`,
      and Debug/Release UniFFI generation leaves committed Swift unchanged at
      SHA-256
      `b6b71a89c20c480ec1e72c574640dc4c92c5d248c095d8aeb8034f8eefe7edd5`.
      Clean universal Debug and Release bundles have identical 128-node
      inventories and embed a byte-identical CLI payload at SHA-256
      `e4c02625a5e5789709953b4542a443da66f150b57bce3ee1b2182e552e39ead3`
      plus byte-identical metadata at SHA-256
      `af241f76127a5bff076a6c4447d01a8417a43e5b2027f4c0ef037afd8bee6eb5`.
      DUX, that CLI, Sparkle 2.9.2, and all four reviewed Sparkle helpers are
      exact arm64/x86_64 universals; DUX and the CLI target macOS 14.0.
      `LSUIElement=true`; Release has empty entitlements and omits the internal
      permanent-cleanup condition.
    - The exact Release copy was signed inside-out with ad-hoc Hardened Runtime
      while preserving the manifest-bound CLI and passes strict deep
      all-architecture verification at
      `/private/tmp/dux-completed-reset-qualified.tlCBnt/DUX.app`. Its signed
      main executable SHA-256 is
      `1b20c00fed3933cc107dda93f285bfe786cb1a233354d69d0e9ffdccfdd83984`,
      and its embedded CLI remains
      `e4c02625a5e5789709953b4542a443da66f150b57bce3ee1b2182e552e39ead3`.
- [x] Add the bounded schema-v14 cleanup-owner provenance checkpoint. New
  cleanup claims can bind separate domain-separated stable-host and boot-scope
  digests plus the only current recovery policy, `resumable`; every migrated
  row remains explicitly unproven. Partial, malformed, foreign-host,
  prior-boot, and otherwise unproven provenance has a typed non-executable
  classification and remains a journal no-op. This checkpoint adds no
  reconciliation handle, generation claim, validation, resume, or effect
  authority. A populated v13 active journal graph now migrates through the
  production loader as `Unproven`, and a recovery attempt is proven to leave
  its complete mutable journal state byte-identical. Verified 2026-07-30 with
  all 72 cleanup-journal cases (71 in the final host-loaded lane plus the sole
  250 ms query-budget trip passing alone), the schema-v14 fingerprint and
  populated-v13 migration cases, the real unsandboxed macOS provenance probe,
  warnings-as-errors workspace Clippy, 92 FFI tests with the two
  quiescence-only effect tests ignored, 560/561 linked native tests in the
  loaded lane plus the sole cadence-timing trip passing alone, the 31
  script-policy tests, and the 269-file destructive-call scan. Debug/Release
  bindings are byte-identical; the unsigned Release app and Rust archive are
  universal arm64/x86_64, the app targets macOS 14.0 with `LSUIElement=true`,
  and resolved Release settings omit the internal permanent-cleanup condition.
- [x] Resolve persistent crash debt before production release: define and test
  cross-reboot/foreign-scope claimed-running-row behavior, a non-fabricating
  legacy-v8 policy, bounded-exhaustion recovery, and diagnostics for
  unattributable legacy external stages. Never infer death from age or PID.
  - [x] 2026-07-30 running-scan debt census foundation: schema v15 adds one
    partial running-row index and a synchronous, read-only census over at most
    64 unclaimed `running` scan records plus one lookahead. It reports only
    inspected, exact-pristine, and unexplained counts with an explicit
    `has_more`; scan IDs, roots, timestamps, ages, PIDs, process owners,
    recovery scopes, bytes, paths, and row selectors never cross the engine or
    UniFFI v48 boundary. A record without a claim remains observation-only: it
    is never called abandoned or recoverable, and no process probe, claim,
    transaction, recovery, filesystem enumeration, or mutation is reachable.
    The Settings **Storage & Privacy** disclosure loads this census lazily,
    strictly revalidates its accounting, keeps an earlier valid result when a
    refresh fails, and presents `0`, an exact `1…64`, or `64+` with redundant
    text/icon and VoiceOver wording. It explicitly says the result is
    bookkeeping rather than disk usage or reclaimable space and that DUX does
    not search temporary folders or attribute legacy external snapshot stages.
    This checkpoint does not resolve claimed prior-boot/foreign-host behavior,
    bounded recovery convergence, or unattributable external-stage policy, so
    the parent remains open. Verified 2026-07-30 with all 1,186 runnable core
    tests (1,182 in the final host-loaded lane plus four FSEvents/revalidation
    timing cases passing individually), three intentionally ignored host/helper
    cases, all 94 runnable FFI tests with the two quiescence-only cleanup tests
    ignored, all CLI tests, and all 566 linked native tests. Workspace Clippy
    passed with warnings denied; the 31 script-policy tests and 273-file
    destructive-call scan passed. Debug/Release Swift bindings are
    byte-identical; the unsigned Debug and Release apps and Release Rust
    archive are universal arm64/x86_64, both apps target macOS 14.0 with
    `LSUIElement=true`, and resolved Release settings omit the internal
    permanent-cleanup condition.
  - [x] 2026-07-30 claimed prior-boot recovery policy: checksummed schema v16
    adds immutable nullable stable-host and boot-scope digests plus the sole
    `interrupt_only` policy to each new running-scan process claim. The tuple
    is all `NULL` or complete; v9–v15 claims migrate without fabricated
    provenance. A global 64-row keyset page plus one lookahead classifies
    complete same-host/current-boot, same-host/prior-boot, foreign-host, and
    unproven claims. Current-boot claims retain the exact-owner probe and only
    `DefinitelyGone` is recoverable; a complete same-host prior-boot claim may
    interrupt one exact pristine parent as history-only reconciliation without
    probing its old PID. Foreign-host and unproven claims remain typed no-ops,
    while a legacy all-`NULL` claim whose combined recovery-scope value exactly
    matches the current owner retains its existing probe; an absent scope can
    confirm only `Alive` or `Unknown`. The final compare-and-set binds the
    complete immutable claim and parent; it preserves normal-completion and
    competing-recovery races, rejects pre-start clocks and schema drift, and
    adopts an uncertain commit only from the exact terminal post-state.
    Recovery still retains every snapshot-temp lease and cannot inspect or
    mutate a file, snapshot, stage, candidate, plan, journal, cleanup lock, AI
    record, or user path. Public engine/UniFFI/Swift shapes are unchanged.
    Bounded multi-page tests prove that foreign, unproven, live, and unknown
    rows do not starve later recoverable work or cause an unbounded immediate
    retry. Verification: all 23 focused claim tests, the v16 fingerprint and
    both provenance migration/constraint tests, the exact snapshot-temp lease
    regression, and all three engine-maintenance projections pass; workspace
    clippy is clean with warnings denied. The serial core lane completed 1,195
    tests with three
    intentional ignores; two unrelated load-sensitive registry tests failed in
    that nine-minute aggregate run and then both passed alone. UniFFI passes
    94 tests with two intentional Rust-target ignores, the CLI passes 37 tests,
    repository scripts pass 31 tests, and the destructive-call audit covers
    273 source files. Native XCTest passes 566 tests. Generated Debug/Release
    Swift is byte-identical; the Rust archive and both unsigned apps are
    universal arm64/x86_64, both apps target macOS 14.0 with
    `LSUIElement=true`, and resolved Release settings omit the internal
    permanent-cleanup condition. The parent remains open for a non-fabricating
    legacy-v8 policy and unattributable external-stage diagnostics.
  - [x] 2026-07-30 claimed-scan provenance census: UniFFI v49 adds a separate
    synchronous, read-only census over one global page of at most 64 claimed
    `running` scans plus one lookahead. It exposes only the inspected total,
    same-host/current-boot, same-host/prior-boot, foreign-host,
    stored-unproven, and current-context-unavailable counts plus `has_more`.
    Stored all-`NULL` provenance remains distinct from a complete stored tuple
    that cannot be compared because current host/boot context is unavailable;
    partial, malformed, owner-inconsistent, or unknown-policy tuples in the
    inspected page or lookahead fail that census instead of being hidden in
    either category. Scan IDs, roots, timestamps, ages, PIDs, owners, scopes,
    digests, policies, row selectors, paths, and byte estimates do not cross
    the engine or v49 boundary. The query performs no liveness probe, recovery
    admission, filesystem traversal, or mutation and cannot start the
    separately sealed scan-recovery task. Settings **Storage & Privacy**
    presents this result independently from the v48 unclaimed-row census and
    strictly revalidates all category arithmetic, current-context exclusivity,
    and truncation. It retains an earlier valid observation after refresh
    failure and identifies the source as DUX bookkeeping plus bounded current
    operating-system provenance rather than disk usage, reclaimable space, or
    an inspection of user or temporary files. No category is labeled alive,
    dead, abandoned, recoverable, or actionable.
    Verified 2026-07-30 with the focused persistence and engine census
    regressions, all 25 scan-claim regressions, formatting, and workspace Clippy
    with warnings denied. The serialized core lane passed 1,198 tests, ignored
    three intentional host/helper cases, and exposed five load-sensitive
    FSEvents/revalidation cases; each of those five passed under its exact
    isolated invocation. The ordinary FFI lane passed 97 tests with two
    intentional Rust-target cleanup tests ignored, and the separate
    quiescence-only lane ran and passed each of those exact tests once. The CLI
    passes 37 tests, repository scripts pass 31 tests, the destructive-call
    audit covers 273 source files, and native XCTest passes all 571 tests.
    Generated Debug/Release Swift is byte-identical; the Rust archive and both
    unsigned apps are universal arm64/x86_64, both apps target macOS 14.0 with
    `LSUIElement=true`, and resolved Release settings omit the internal
    permanent-cleanup condition. The parent remains open for a non-fabricating
    legacy-v8 policy and diagnostics for unattributable external stages.
  - [x] 2026-08-08 unattributable external-stage diagnostic: UniFFI v55 extends
    the existing DUX-owned footprint observation with a separate, non-additive
    census of possible pre-correction snapshot-stage names in the retained
    data-root parent. The Unix/macOS implementation walks raw direct-child
    names through a cloned retained directory descriptor, counts only the
    exact `.dux-snapshot-stage-<32 lowercase hex>` grammar, and never opens a
    child. A 4,096-entry, 1-MiB aggregate-name, and 250-ms budget returns an
    explicitly incomplete lower bound rather than silently claiming a complete
    inventory. Parent and store identities are revalidated around the read.
    Windows and unsupported platforms report an empty incomplete observation
    until equivalent retained-handle evidence exists.
  - The record exposes only inspected and matching counts plus completeness:
    no name, path, identity, marker, type, byte estimate, ownership claim,
    selector, candidate, or cleanup authority crosses core, UniFFI, or Swift.
    Two stores sharing the parent deliberately report the same unattributed
    count. The existing database/snapshot/cache chart and physical totals do
    not change. **Storage & Privacy** instead shows a dashed question-mark
    callout that states ownership and size are unknown, totals exclude the
    entries, and no cleanup action is available. Swift independently validates
    version, bounds, and count algebra and provides equivalent VoiceOver copy.
  - Focused qualification covers exact/malformed/non-UTF-8 names, files,
    directories, symlinks, root-local exclusion, unchanged identities and
    contents, shared-parent non-attribution, bounded truncation, additive
    accounting exclusion, FFI projection/rejection, and native copy/adapter
    behavior. The parent remains open only for the non-fabricating legacy-v8
    policy and bounded-exhaustion recovery semantics; this observation cannot
    be reused as recovery or removal authority.
  - Qualification evidence: all 1,556 core unit cases were exercised serially;
    1,540 passed in the aggregate, 3 stayed intentionally ignored, and the 13
    fail-closed Cargo/FSEvents load cases all passed when replayed individually
    in fresh processes. All 13 projection tests, 120 ordinary UniFFI tests, both
    isolated UniFFI Rust-target cleanup cases, 48 CLI unit tests, 6 CLI process
    tests, and 681 linked native tests pass. Repository policy tests (41), the
    destructive-call audit, formatting, current-toolchain workspace check and
    warnings-as-errors Clippy, fuzz-adapter and isolated-harness Clippy, and
    Rust 1.88 workspace/fuzz checks pass. Debug and Release generation produce
    the same v55 Swift API and exact bundled CLI SHA-256
    `6c09b3cf413eda4ddd4abe0b8be00d1f1093be33b0f28dbbe4056236a9d27b94`.
    Both unsigned app builds are universal `arm64`/`x86_64`, target macOS 14,
    embed the same signed universal CLI, and retain `LSUIElement=true`; Release
    settings omit the internal permanent-cleanup condition.
  - [x] 2026-08-08 legacy-v8 dismissal core boundary: ADR 0008 accepts only an
    explicit user-confirmed, history-only **Dismiss old unfinished
    bookkeeping** policy. Missing process ownership is never interpreted as
    death, and no claim, host/boot provenance, PID, age, or AI conclusion is
    fabricated. The private durable preparation selects exact pristine
    unclaimed rows directly in `(started_at_unix_ms, scan_id)` order, so an
    arbitrary prefix of retained unexplained rows cannot starve later eligible
    work. Each consume-once engine preview binds at most 64 exact witnesses
    plus one lookahead to one engine for two minutes; its public shape contains
    only eligible count, truncation, and preparation/expiry times.
  - Commit exact-CASes the complete scalar row and absence of claims, issues,
    aggregates, candidates, evaluations, tombstones, and review pins in one
    all-or-nothing transaction, then changes only status and completion time.
    Snapshot-temp leases are permitted and remain byte-identical. Commit
    ambiguity accepts only the complete exact interrupted post-state. Eight
    focused persistence/engine tests cover selective history-only mutation,
    child-table races and rollback, 64+1 convergence past 70 retained rows,
    lease preservation, applied/not-applied/ambiguous reconciliation,
    path-free projection, wrong-engine use, inclusive expiry, and closed-state
    refusal; both affected census suites and warnings-as-errors core Clippy
    pass. This core-only checkpoint left the parent open for UniFFI/native
    confirmation, full qualification, and bounded-exhaustion production
    evidence.
  - [x] 2026-08-08 legacy-v8 dismissal transport and native confirmation:
    UniFFI v56 carries only the bounded eligible count, `has_more`, and exact
    two-minute preparation/expiry envelope behind an engine-bound,
    consume-once opaque preview. Preview registries are capacity-limited,
    refuse use across engines, release on ordinary close, poisoned close,
    background close, and app-data-reset drain, and preserve outcome-unknown
    rather than treating a malformed post-commit success as retryable.
    Swift independently validates the version, `1...64` count, timestamp
    domain, exact lifetime, and result correlation off the main thread.
    **Storage & Privacy** offers the action only when the earlier read-only
    census reports pristine rows, requires a fresh explicit destructive-style
    confirmation, automatically expires or releases an unaccepted preview,
    and joins accepted work during app shutdown. The copy states that the
    action changes DUX history only: it does not reclaim space, delete a file,
    remove a snapshot or stage, stop a process, infer death, or become callable
    by AI, low-disk handling, schedules, or the CLI. A changed exact witness
    rolls the whole page back and a `64+` result requires another independent
    preview and confirmation, establishing bounded convergence without an
    automatic loop. Focused Rust and native tests cover projection rejection,
    wrong-engine and consume-once use, close/reset draining, exact SQLite
    mutation, cancel/release, stale confirmation, result correlation,
    outcome-unknown presentation, and shutdown fencing. This closes the
    non-fabricating legacy-v8 and bounded-exhaustion portions of the crash-debt
    gate; the external-stage observation remains deliberately non-actionable.
  - Qualification evidence: the serialized full-core lane exercised all 1,564
    cases; 1,558 passed in the aggregate, three intentional host/performance
    helpers stayed ignored, and the three accumulated-load Rust-target review
    deadline cases each passed its exact isolated fresh-process replay. All 122
    active UniFFI cases pass and its two real-effect Rust-target helpers remain
    intentionally ignored; all 689 linked native cases and all 41 repository
    policy cases pass. The destructive-call audit covers 339 source files with
    no unregistered calls. Formatting, locked workspace/all-target checks, and
    warnings-as-errors workspace Clippy pass. Debug and Release binding
    generation are byte-identical at SHA-256
    `23f2811c70c8c3662ea11b62adfe9fdeba048becfbc6a328df46e8f8a6f1fdf3`;
    both embed the same CLI at SHA-256
    `24bdbed1e704aaa8237d504fb3761bfa7421aae27a7163f401115103412902f5`.
    The isolated Release app is ad-hoc signed with Hardened Runtime, passes
    strict deep verification, retains `LSUIElement=true`, targets macOS 14,
    contains universal `arm64`/`x86_64` app and CLI executables, and omits the
    internal permanent-cleanup compilation condition. Its main executable is
    SHA-256
    `4624ac295eaf426dac890e6e9df0daaa123fda2384ecd159d7154e99883c1c50`.
- [ ] Require native Windows CI evidence for temp/final/stage mutation,
  DACL/reparse handling, and sparse/compressed allocation before claiming
  Windows persistence-maintenance support.
- [x] Preserve standalone Homebrew/crates.io release.
  - Verified 2026-08-08 as an independent CLI-only tag lane with no Developer
    ID, notarization, Sparkle, Xcode, or app-bundle prerequisite. The ordered
    chain remains cross-platform tests and dependency policy, four locked
    target builds, native `dux --version` smoke tests, producer-side SHA-256
    checksums, `dux-core` then `dux-cli` publication, immutable GitHub release
    assets plus `SHA256SUMS`, and finally a fine-grained-token Homebrew tap
    update for Intel macOS, Apple Silicon macOS, and x86_64 Linux. Existing
    crates.io bytes must match a freshly packaged crate exactly before a rerun
    proceeds, and the app-only `dux-ffi` crate remains unpublished.
  - Six repository policy regressions now freeze the public-crate/version
    relationship, full-SHA action pins, dependency order, absence of app-only
    credentials and tools, exact target/archive coverage, checksum use,
    crates.io ordering, Homebrew formula inputs/install smoke test, and both
    documented standalone install routes. Local qualification packages and
    verifies the complete workspace from the lockfile, including both public
    crates from their packaged sources, and the optimized standalone binary
    reports `dux 0.5.0`. App and Sparkle release work must remain a separate
    authority lane and must not make these CLI channels depend on Apple
    credentials or silently adopt an external installation.
- [ ] Freeze the production bundle identifier, Apple Developer team, signing
  identity, and designated requirement before TCC and launch-at-login testing;
  then validate enable, approval-required recovery, disable, relocation policy,
  and a real sign-out/sign-in cycle from a signed stable installation.
  - [x] Production identity frozen 2026-08-08 as bundle identifier
    `se.mjukis.dux`, Team ID `SMQ3E8Y57T`, Developer ID identity
    `Developer ID Application: MJUKIS AB (SMQ3E8Y57T)`, and the exact
    certificate/designated requirement recorded in
    `dux-macos/Config/ProductionIdentity.json`. Release builds use this bundle
    identifier while Debug keeps `se.mjukis.dux.spike`; the release script
    rejects environment, record, signed-Team, authority, or designated-
    requirement drift. A disposable production-identifier executable signed
    with the actual timestamped certificate passed strict signature and
    designated-requirement verification outside the managed build sandbox.
  - [ ] Perform the TCC and launch-at-login enable, approval-required recovery,
    disable, relocation, and real sign-out/sign-in matrix from a signed stable
    app installation. Do not use an unsigned/ad-hoc app or the Debug spike for
    this identity-sensitive qualification.
- [ ] Add the primary notarized/stapled DMG with an Applications link; optionally
  publish a notarized ZIP as a secondary artifact.
  - [x] The fail-closed local script and protected manual CI scaffold construct
    the universal app, sign code inside-out, notarize/staple the app and primary
    Applications-link DMG, mount and reverify it, then atomically materialize
    the checksum, manifest, sanitized submissions, and complete notarization
    logs. The three-phase boundary completes all build/test/dependency execution
    on an unprivileged runner before it repeats the clean HEAD/tag gate and
    seals the unsigned app. That runner transfers only the three-file SHA-256
    envelope as a one-day repository-readable Actions artifact and carries a
    deterministic whole-envelope digest through the separate job-output
    channel. A fresh protected runner requires the Actions service digest,
    recomputes the independent envelope digest, and verifies exact app/code
    layout, metadata, and Sparkle policy without executing project code before
    credentials exist. The signing phase revalidates the
    envelope and invokes no Cargo, Python, XcodeGen, Xcode build/test,
    dependency, or bundled DUX executable. CI admits exactly those seven signed
    files, verifies them after Keychain deletion, never uploads them, and
    performs no public publication.
  - [ ] Execute the lane with the real protected credentials, install the exact
    preserved DMG on clean Apple Silicon and Intel systems, and retain the
    signed/notarized/Gatekeeper qualification evidence before checking this
    parent complete.
- [ ] Add Developer ID signing, notarization, and stapling CI.
  - [x] Added `.github/workflows/release-macos-app.yml` as a manual-only lane
    separate from standalone CLI release. An unprivileged job binds stable
    `vX.Y.Z`, input version/build, exact tag commit, public production identity,
    policy tests, and the destructive-call audit. A credential-free `macos-15`
    job pins Rust 1.96.0, Xcode 16.4 build 16F6, checksum-verified XcodeGen
    2.44.1, and full-SHA Node 24 action generations; it builds/tests the exact
    commit, rechecks clean tag identity immediately before sealing, and uploads
    only its one-day three-file prepared envelope. The protected
    `macos-release-signing` job starts on a fresh runner, checks out only the
    validated commit, downloads with hard-fail service-digest verification,
    independently compares the separately carried whole-envelope digest, then
    imports the Developer ID and notarization API key into a random-password
    ephemeral file Keychain. It calls only the fixed signing phase while the
    Keychain exists, removes raw imports, handles import-failure cleanup,
    attempts Keychain deletion before exact manifest/output verification,
    never uploads signed output, and publishes nothing. The unprivileged gate
    also proves workspace version and default-branch ancestry. Policy tests
    confine all five secret references to the import step, freeze the exact
    per-job steps and action SHA allowlist, forbid any action after credential
    import, and preserve the independent crates/Homebrew/GitHub CLI chain.
  - [x] 2026-08-09 independent-review hardening: the complete release script is
    byte-sealed by a reviewed SHA-256 policy checkpoint. Any change to its
    functions, global code, definitions, quoting, indirection, substitutions,
    or child-process syntax now fails closed until the complete script is
    reviewed and its pinned digest is explicitly updated. `.gitattributes`
    fixes that script to LF so the raw-byte checkpoint is identical on macOS,
    Linux, and Windows checkouts, and its policy test rejects removal of that
    portability guard. The existing semantic policy checks remain as readable
    invariants; the source seal is the syntax-independent backstop. All 31
    macOS release script/workflow policy tests pass.
  - [ ] Configure and audit GitHub environment `macos-release-signing` with
    required reviewer, self-review prevention, protected stable-tag rules, and
    the five documented Apple secrets; then complete one real accepted app/DMG
    notarization run. Choose a separately reviewed encrypted restricted store
    or supervised local custody before retaining signed release bytes. The
    public-repository Actions transfer is explicitly limited to the unsigned
    prepared envelope and is not release custody. Repository tests cannot prove
    this external state.
- [ ] Complete the frozen Sparkle 2 updater rollout only after the production
  identity and signing lane are stable, following ADR 0002.
  - [x] Integration scaffold completed 2026-07-31 and security-upgraded
    2026-08-08: XcodeGen and the committed package resolution pin Sparkle 2.9.5
    at revision `79bc9e872948e47877e76f194cb0c8e0412b0b90`, incorporating the
    upstream symlink hardening and appcast-item race fix that supersede 2.9.2.
    Settings uses `SPUStandardUpdaterController` and exposes **Check for
    Updates…** only when the host bundle is exactly `se.mjukis.dux`, has an
    HTTPS `SUFeedURL`, embeds the exact frozen DUX `SUPublicEDKey`, and retains
    `SURequireSignedFeed=true`, `SUVerifyUpdateBeforeExtraction=true`, and
    `SUSignedFeedFailureExpirationInterval=0`. Debug and the current Release
    build create no updater and perform no update-network request because no
    feed URL is configured. Seven focused tests prove placeholder/other
    identity, insecure/missing feed, missing/unexpected key including another
    valid 32-byte key, every weakened policy field, and complete configuration
    boundaries.
    The direct-release workflow recognizes only Sparkle's exact 2.9.5 nested
    updater/XPC layout, verifies its framework plus four helpers are universal,
    and signs the installer service, downloader service, autoupdater, updater
    app, and framework inside-out before signing DUX. The downloader alone
    preserves its reviewed upstream entitlements; arbitrary nested bundles
    still fail closed.
  - Historical 2.9.2 checkpoint evidence remains in the completed M8 slices
    above; it is not evidence for this upgrade. Current 2.9.5 verification is
    recorded at the end of this milestone after the complete release-policy,
    native, universal Debug/Release, generated-project, and artifact-policy
    gates run.
  - [x] Freeze the production identity and generate a dedicated DUX Sparkle
    Ed25519 key. The private key remains in the login Keychain under account
    `se.mjukis.dux`; only public key
    `UmMI6TWBdBm2fKEmmk5xi2T+lu7K5KJl1abwIBRLSQo=` is committed, recorded in
    the production-identity record, and embedded as `SUPublicEDKey`. The key is
    independent of Claudex and other projects.
    Qualification read the public half back from the named Keychain account and
    matched it byte-for-byte. An isolated universal arm64/x86_64 Release build
    uses `se.mjukis.dux`, retains `LSUIElement=true`, embeds the same key, and
    omits `SUFeedURL`. All five focused native updater tests use the real DUX
    identity/key and pass. The current complete verification counts supersede
    the earlier five-test/12/49/340 checkpoint and are recorded below.
  - [ ] Establish recoverable private-key custody before the first update:
    create an encrypted offline backup, document recovery/rotation owners and
    drills, and import the key into protected release CI without exposing it to
    source, artifacts, logs, pull requests, or the appcast host.
    - [x] `docs/MACOS_RELEASE_OPERATIONS.md` freezes Release Owner, Reviewer,
      dual-Custodian, and Incident Lead roles; a two-person direct-to-encrypted-
      removable-media export ceremony; two geographically separated copies;
      mode-0700/mode-0600, single-link, exact-size, canonical-path, recorded
      encrypted external APFS Volume UUID, and read-only drill-mount checks;
      exact Sparkle 2.9.5 tool ZIP provenance at its reviewed SHA-256; silent canonical 32-byte seed
      prevalidation; a disposable-user import/public-key/canary-sign recovery
      drill with an independent public-key-only CryptoKit verifier; six-month
      cadence; future protected secret name and standard-input appcast use; and
      compromise, loss, rotation, withdrawal, certificate, and higher-version
      corrective-release procedures. The document explicitly records that no
      export, backup, drill, or Sparkle CI import has happened yet.
    - [ ] Perform the witnessed two-copy encrypted offline backup, pass a restore
      drill from each custody copy, record only the public attestation, and only
      then import the private bytes into the future protected appcast lane.
  - [ ] Resolve the updater trust model before adding a feed. Sparkle 2.9.5's
    stock application validator intentionally accepts the old archive EdDSA
    signature **or** matching Apple code-signing identity to permit key
    rotation. A signed, non-expiring feed protects metadata, but does not prove
    DUX's stronger current requirement that every installed app has both the
    old exact DUX EdDSA signature and old exact DUX Developer ID identity.
    Explicitly accept and threat-model the stock rotation policy or implement a
    maintainable reviewed enforcement layer and adversarially prove it; keep
    `SUFeedURL` absent until the chosen boundary is amended into ADR 0002.
  - [ ] Select and review the HTTPS stable feed URL, then preserve Sparkle's
    explicit consent for automatic checks and its user-controlled automatic-
    download setting. Until the feed exists, keep `SUFeedURL` absent so the
    updater remains dormant.
  - Publish one HTTPS stable-channel appcast first. Embed only the EdDSA public
    key; keep the private key out of the repository, app, artifact host, and
    public pull-request jobs.
  - Generate and verify the signed appcast in protected release CI only after
    the immutable enclosure passes Developer ID, Hardened Runtime,
    notarization, staple, architecture, deployment-target, and checksum gates.
    Reject invalid feed/enclosure signatures, lengths, URLs, non-monotonic
    versions, incompatible minimum system versions, and identity drift.
  - Treat rollback as a higher-version corrective release; never silently
    downgrade an installed app. Document key rotation, key compromise,
    appcast withdrawal, interrupted update, and failed relaunch recovery.
  - Keep Sparkle scoped to the application bundle. It must not overwrite an
    optional separately installed CLI; Settings detects a version mismatch and
    offers the explicit atomic CLI upgrader.
  - Test a real signed previous-to-current update on Intel and Apple Silicon,
    including settings/history preservation, stable TCC and login-item
    identity, menu-bar relaunch, offline/interrupted download, tampered feed and
    archive, read-only volume, App Translocation, already-current behavior,
    downgrade refusal, and a withdrawn release.
  - Add beta channels and phased rollout only after the stable channel is
    proven; channel changes must be explicit and reversible.
  - 2026-08-08 current M9 checkpoint verification: all 691 linked native tests,
    including the seven focused updater boundary tests, pass against the exact
    Sparkle 2.9.5 resolution. All 68 repository policy tests pass and the
    destructive-call audit is clean across 343 source files. Cargo format,
    workspace check, and warning-denying Clippy gates pass. XcodeGen regenerates
    the committed project without drift; shell syntax and workflow YAML parse;
    the descriptor-bound public-key-only CryptoKit verifier accepts the RFC
    8032 empty-message Ed25519 vector and rejects modified signatures/payloads,
    a wrong key, wrong decoded lengths, noncanonical base64, symlinks, hard
    links, and oversized input. Fresh
    universal Debug and Release builds succeed at
    `/private/tmp/dux-m9-universal-debug-20260808/Build/Products/Debug/DUX.app`
    and
    `/private/tmp/dux-m9-universal-release-20260808/Build/Products/Release/DUX.app`.
    Their layouts and bundled CLI bytes/metadata match. DUX, the CLI, Sparkle
    framework, Autoupdate, Updater, Downloader, and Installer are all exact
    arm64/x86_64 universals; both apps embed Sparkle 2.9.5, the frozen DUX key,
    signed-feed and pre-extraction enforcement, zero failure expiry, and no
    `SUFeedURL`. Release uses `se.mjukis.dux`; Debug retains the spike identity.
- [ ] Publish privacy, security, and cleanup-rule documentation.
- [ ] Add crash-report opt-in only if desired; never include paths by default.

Exit criteria:

- Fresh install works on Intel and Apple Silicon.
- Gatekeeper accepts the notarized app.
- A signed previously released build updates through Sparkle 2 to the candidate
  on Intel and Apple Silicon while preserving settings, history, TCC identity,
  login-item identity, and the separately installed CLI; tampered, withdrawn,
  interrupted, incompatible, and downgrade updates fail closed.
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

Mitigation: ADR 0009 permanently blocks direct local command adapters after the
adversarial macOS spike demonstrated retained same-user ambient reads. A
deprecated custom Seatbelt profile is not a shipping boundary. ADR 0013 instead
accepts a closed metadata-only direct-vendor HTTPS architecture that executes
no provider code under DUX's local authority. The fixed Anthropic wire adapter
is compiled but unreachable; no core proof or production caller can invoke it,
so disabled remains the only runtime state. A separately App-Sandboxed or
virtualized architecture still needs its own ADR and supported-release proof.
Fixed arguments, no shell, an empty working directory, disabled tools, a
sanitized environment, structured metadata, timeouts, and no planner/executor
connection remain defense in depth, not local-process confinement.

### UniFFI/Swift concurrency friction

Mitigation: Phase 0 spike, coarse API, generated-binding smoke tests, and the
replacement-only C ABI fallback and reconsideration triggers in
[ADR 0005](docs/adr/0005-uniffi-swift-rust-transport.md).

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
6. Retire the legacy CLI deletion path; reintroduce CLI cleanup only through
   the same reviewed-plan executor used by the native product.
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
