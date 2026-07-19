# DUX macOS Product and Implementation Roadmap

Status: Draft implementation specification

Last updated: 2026-07-17

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
- Absolute paths MUST be shortened to home-relative paths before being sent to AI unless the user explicitly enables full paths.
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
- `ai_insights`: input digest, provider, adapter version, model label if known, output, created time, expiration.
- `schedules`: rule/category scope, enabled, cadence, age, size cap, last/next run.

Do not store full millions-node trees in SQLite initially. Continue using versioned, checksummed snapshot files. Add atomic write and migration/invalidation behavior.

### 11.2 Retention

- Hourly disk samples: 30 days.
- Daily rolled-up samples: one year.
- Cleanup history: retained until user clears it.
- AI insights: default 30 days, user-clearable, and regenerated on input digest change.
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

Initial adapter candidates:

- Claude CLI adapter;
- Codex CLI adapter;
- disabled/no-provider adapter.

The disabled adapter is the only adapter allowed to ship before the security
gate in §14.2 passes.

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

These process controls are defense in depth, not confinement. A command spawned
by an unsandboxed app may retain ambient filesystem and TCC authority,
especially when the app has Full Disk Access. Do not ship a local Claude,
Codex, or custom-command adapter until an adversarial security/TCC spike proves
the authority boundary on every supported macOS release. If it cannot, use a
metadata-only remote API or another architecture with real confinement. Tool-
disable flags alone do not satisfy this gate.

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
record_ai_insight(input_digest, insight)
```

Current native realization (FFI contract v17):
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
  Signed install and sign-in-cycle validation remains explicitly gated by
  Milestone 9's production identity.
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
  at the reported private staging path for diagnosis. The production identity
  is deliberately not invented here; freezing it and performing the first real
  Developer ID/notary run remain Milestone 9 release prerequisites.

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
  - [ ] Promote the bounded Cargo observation into trusted planning authority,
    then add authoritative volume and protected-root grants,
    change/process/descendant guards, and executor-time revalidation before
    removing `ProtectedPath` or enabling scheduling.
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
      and ambiguous-write cases fail closed. This adds discovery provenance
      only: no FFI/Swift surface, blocker removal, plan, schedule, or effect was
      added.
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
- [ ] Implement permanent-safe executor for approved rules.
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
- [ ] Implement execution-time revalidation.
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
- [ ] Implement cleanup session/item history.
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
- [ ] Implement pre/post capacity verification.
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
- [ ] Implement exclusions and global permanent-cleanup disable setting.
  - [x] 2026-07-19 slice: add the revisioned global permanent-cleanup kill
    switch to the typed settings store. Missing state defaults to enabled;
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
    `ENABLE PERMANENT CLEANUP` before re-enable or reset can restore the enabled
    default. Path-bearing exclusion presentation remains separate and open.
  - [x] 2026-07-19 slice: expose the path-free global permanent-cleanup kill
    switch through UniFFI contract v22. Versioned get/set/reset records carry
    only enabled state, Default/Stored provenance, monotonic revision, and
    optional update time; Rust remains the semantic validator and the switch
    can only deny effects. Closed engines, malformed state, storage failures,
    and write uncertainty map to typed errors. The generated Swift bindings
    were regenerated from the universal Debug XCFramework. Swift EngineService
    maps every typed error and rejects malformed status shapes; AppModel
    generation-fences load/mutation/reset work and invalidates it during ordered
    shutdown. Settings exposes the deny-only gate with VoiceOver identifiers and
    an exact typed confirmation for re-enable/reset. Exclusions remain a
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
- [ ] Add partial failure, retry, cancellation, and changed-since-plan UI.
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
- [ ] Add bounded pending-evaluation discovery and snapshot-backed restart
  recovery; malformed or incompatible state fails closed.
- [ ] Wire deterministic evaluator → reviewed plan → journal → executor through
  the core engine and FFI/Swift while preserving generation, cancellation,
  recovery, and outcome-unknown fencing.
- [ ] Expose review intent, plan lifecycle, and path-free cleanup history through
  FFI/Swift UI without turning history into planner authority.
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
- [ ] Expand production cleanup session/item history with exact-session detail,
  verified capacity outcomes, and separately confirmed history clearing.
- [ ] Define and test cross-reboot and Windows-unproven cleanup-journal recovery;
  unknown ownership must remain non-executable.

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
  - [x] 2026-07-19 slice: add schema v11 durable pressure episodes. Warning
    and Critical entries, Healthy recovery, Warning→Critical escalation, and
    policy-revision boundaries are updated in the same immediate transaction
    as the raw capacity sample. Unknown pressure never opens or claims
    recovery; exact retries are idempotent; bounded readers reject malformed
    or overlapping history. `StoredPressureEpisode` remains path-free,
    telemetry-only, and is not exposed through FFI yet.
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
- [ ] Add 24-hour/7-day changes and 30-day chart.
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
- [ ] Add transition-based notifications and cooldown.
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
- [ ] Deep-link notifications to urgent Recommendations.
  - [x] 2026-07-19 slice: validate notification responses with the exact bounded
    versioned payload before dispatching. Valid responses now route the
    long-lived runtime to Explorer, select a Recommendations destination, and
    open/focus the window through the SwiftUI scene action; malformed,
    unknown-route, oversized, or non-string user-info values are ignored.
    Recommendations is intentionally review-only: it explains that future
    groups require a completed read-only scan and carries no deletion or AI
    execution authority. The affected-volume identity is retained in the
    validated payload but is never used as a filesystem path.
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
- [ ] Run the adversarial macOS security/TCC spike and record whether local AI
  subprocesses can be confined when DUX has broad access.
- [ ] Implement the Claude CLI probe/invocation adapter only if that spike
  approves its authority boundary.
- [ ] Implement the Codex CLI probe/invocation adapter only if that spike
  approves its authority boundary.
- [ ] Implement timeout, output limit, cancellation, and process-tree cleanup.
- [ ] Validate tools-disabled behavior for each approved adapter as defense in
  depth; reject adapters that cannot guarantee it, without treating it as
  subprocess confinement.
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
- [ ] Add final JSON scan-detail, candidate, review-state, and cleanup-history
  commands with golden schema tests.
- [ ] Add cross-process scan-scope leasing and version-skew tests so app and CLI
  cannot run conflicting overlapping scans.
- [ ] Add Storage & Privacy settings: expose snapshot-cap get/set/reset through
  FFI/Swift, report DUX-owned SQLite/snapshot/AI footprint, and provide
  separately confirmed cache, history, snapshot, and reset-app-data operations
  through narrow marker-validated core boundaries that never touch user data.
- [ ] Resolve persistent crash debt before production release: define and test
  cross-reboot/foreign-scope claimed-running-row behavior, a non-fabricating
  legacy-v8 policy, bounded-exhaustion recovery, and diagnostics for
  unattributable legacy external stages. Never infer death from age or PID.
- [ ] Require native Windows CI evidence for temp/final/stage mutation,
  DACL/reparse handling, and sparse/compressed allocation before claiming
  Windows persistence-maintenance support.
- [ ] Preserve standalone Homebrew/crates.io release.
- [ ] Freeze the production bundle identifier, Apple Developer team, signing
  identity, and designated requirement before TCC and launch-at-login testing;
  then validate enable, approval-required recovery, disable, relocation policy,
  and a real sign-out/sign-in cycle from a signed stable installation.
- [ ] Add the primary notarized/stapled DMG with an Applications link; optionally
  publish a notarized ZIP as a secondary artifact.
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

Mitigation: block local command adapters until an adversarial macOS/TCC spike
proves an actual authority boundary on every supported release. If it cannot,
use a metadata-only remote API or another confined architecture. Fixed adapters,
no shell, empty working directory, disabled tools, sanitized environment,
structured metadata, timeouts, and no planner/executor connection remain
defense in depth, not confinement.

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
