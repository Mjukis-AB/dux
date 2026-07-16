# DUX macOS Product and Implementation Roadmap

Status: Draft implementation specification

Last updated: 2026-07-16

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
  admits only SQLite objects and exact reserved `snapshots`, `ai`, and `logs`
  siblings so later Milestone 2 stores do not invalidate the database. Failed
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
- [ ] Add scan/session/candidate/cleanup persistence.
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
    and unscoped behavior; native macOS/Linux subprocess coverage proves live,
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
    and retention remain later work. The broad persistence checkbox stays open
    for candidate status plus evaluator/planner lifecycle integration.
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
  marker-complete private directory, validates a bounded exact inventory, and
  uses exclusive PID-plus-random temps, durable atomic no-replace publication,
  collision winner validation, and read-only reopened final handles. Unix
  creation repairs exact 0700/0600 modes even beneath a restrictive umask;
  macOS accepts deny-only publication-parent ACLs but rejects final-object
  ACLs; Windows uses protected owner-only DACLs, retained IDs, handle-relative
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
  selection, latest-two/2 GiB retention,
  active-review pins, and abandoned-stage/temp scavenging remain later tasks
  and are not claimed by this checkpoint. Typed coverage/issues and the
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
- [ ] Add final JSON commands and golden schema tests.
- [ ] Preserve standalone Homebrew/crates.io release.
- [ ] Freeze the production bundle identifier, Apple Developer team, signing
  identity, and designated requirement before TCC and launch-at-login testing.
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
