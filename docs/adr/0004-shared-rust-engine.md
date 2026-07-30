# ADR 0004: Shared Rust engine

- Status: Accepted
- Date: 2026-07-15
- Scope: portable domain ownership, clients, and FFI boundary

## Context

The repository already has a cross-platform Rust scanner/tree/cache crate and a
Ratatui CLI. The new macOS app needs the same scan results, classifications,
candidate rules, safety checks, cleanup plans, execution history, and durable
schemas. Reimplementing policy in Swift or treating the CLI's text output as an
application API would create divergent safety decisions and make destructive
behavior difficult to review.

The current boundary is not yet the complete target boundary. `dux-cli` still
owns some computed views and scan/cache orchestration, but its former
CLI-specific destructive filesystem path was retired on 2026-07-29. The CLI
continues as a read-only companion while reusable engine services are
extracted. If cleanup returns to the CLI, this decision requires it to consume
the shared reviewed-plan executor rather than restore client-owned effects.

The FFI technology was intentionally not decided here. The roadmap defaulted to
UniFFI, with a narrow C ABI fallback, while the Phase 0 spike tested Swift
concurrency, binding generation, and XCFramework packaging. [ADR 0005](0005-uniffi-swift-rust-transport.md)
records the resulting transport decision and the remaining error/lifetime gates.

## Decision

Use one shared Rust engine as the authority for portable storage behavior. Both
the macOS application and CLI are clients of that engine.

The boundary consists of:

- `dux-core`: portable domain model, policies, orchestration, and persistence;
- `dux-ffi`: a small, coarse adapter exposing versioned DTOs and task APIs;
- `dux-cli`: terminal presentation and command parsing over engine services;
- `dux-macos`: Swift presentation and platform adapters over `dux-ffi`.

Do not run the CLI as the app backend. Do not parse human-readable CLI output in
Swift. Do not duplicate cleanup or classification policy in the FFI or UI.

## Ownership boundaries

### `dux-core` owns

- scan requests, progress semantics, cancellation, coverage, and snapshots;
- portable tree/node models and size calculations;
- artifact and cleanup-rule evaluation;
- candidate identity, evidence, safety tiers, and overlap resolution;
- cleanup plan construction, freshness, path/volume validation, and execution
  authorization;
- serialized cleanup orchestration and operation-result semantics;
- disk-pressure evaluation from capacity samples supplied by a platform;
- database/snapshot schemas, migrations, compatibility checks, and history;
- AI input shaping/redaction policy and read-only insight records;
- stable errors and typed events that clients can localize and present.

The engine must remain usable without Swift, AI, or a graphical application.
Platform-specific modules may be selected with Rust target configuration, but
portable policy must not depend on Swift callbacks to decide whether a target
is valid or safe.

### `dux-ffi` owns

- conversion between core types and immutable/versioned boundary DTOs;
- opaque engine/task/plan handles and lifecycle checks;
- panic containment and stable error mapping;
- callback/event adaptation;
- pagination and size budgets for large results;
- generated-binding configuration and smoke-test fixtures.

`dux-ffi` must not contain a second planner, cache, rule registry, or execution
path. Boundary-specific copies are DTOs, not independent domain models.

### `dux-cli` owns

- argument parsing, TTY behavior, Ratatui state, keyboard navigation, and text;
- rendering engine snapshots, candidates, plans, progress, and history;
- CLI-specific JSON serialization contracts layered over shared engine DTOs;
- optional platform commands such as reveal-in-Finder through a narrow adapter.

Existing CLI-owned policy moves into core in reviewable slices. The retired
legacy deletion adapter must not be restored or exposed to Swift as an engine
contract.

### `dux-macos` owns

- SwiftUI scenes, navigation, charts, accessibility, and localization;
- render state and conversion of engine events into structured Swift
  concurrency;
- Foundation's important-usage volume-capacity sample;
- notifications, Finder integration, System Settings links, login-item control,
  and other macOS-only services;
- settings UI, AI provider configuration, and optional CLI installation UI.

Platform effect adapters may perform a macOS operation such as moving an item to
Trash when the shared executor requests a validated effect. An adapter receives
an already authorized operation and returns a typed result; it cannot add paths,
change safety, approve plans, or bypass revalidation.

## FFI contract

The boundary is coarse-grained and asynchronous where work may block.

Required properties:

- The first spike exports only engine/library version and one size-format DTO or
  value. It proves packaging, not the final API.
- Long operations return a task ID and produce typed events; they do not block
  the Swift main actor.
- Cancellation is explicit and idempotent.
- Published snapshots and DTOs are immutable from a client perspective.
- Boundary types are owned values. Never expose Rust references, memory layout,
  enum discriminants, `usize`, `PathBuf`, or `SystemTime` across FFI.
- Node IDs are fixed-width opaque values scoped to one snapshot generation;
  clients cannot retain them as durable filesystem identity.
- Large trees cross by page, child query, aggregate, or treemap budget. Never
  create one foreign object or callback per filesystem node.
- Product versions, FFI protocol versions, CLI JSON versions, snapshot schemas,
  and SQLite schemas are distinct. Startup performs a compatibility handshake;
  one version number must never stand in for all five contracts.
- Every persisted or externally serialized DTO carries its relevant schema
  version.
- Rust errors become stable error codes/kinds plus diagnostic context. UI copy
  is localized by the client; raw Rust panic strings are never product copy.
- Rust panics are caught at the FFI boundary and never unwind into Swift.
- Handles reject use-after-close, unknown task IDs, stale plan IDs, and schema
  mismatches without undefined behavior.
- Invoke client callbacks only after releasing engine locks. Callback ordering,
  completion, cancellation, reentrancy, and object lifetime are tested.
- Engine configuration accepts explicit database, snapshot, and cache paths so
  tests and clients do not depend on hidden process-global directories.

[ADR 0005](0005-uniffi-swift-rust-transport.md) selects UniFFI for the private
in-process Swift/Rust transport after the shell spike. Its documented C ABI is
a replacement path if a reconsideration trigger is met, not a parallel API.

## Concurrency and lifecycle

Rust engine:

- owns a bounded task registry and bounded worker pools;
- permits one active full scan per overlapping volume/root scope;
- uses cancellation tokens and publishes immutable snapshots;
- serializes cleanup execution and coordinates cache/database writes;
- never depends on the Swift main actor making progress to preserve filesystem
  safety or database integrity;
- reports that cancellation was requested separately from confirmation that a
  blocking operating-system call has returned.

Swift client:

- owns one `EngineService` for the application lifecycle;
- calls blocking/synchronous generated functions from a worker executor;
- bridges callbacks to `AsyncStream` or an equivalent checked abstraction;
- applies render-state changes on `@MainActor` only;
- throttles progress and ignores events from superseded task generations;
- cancels subscriptions and engine work explicitly during teardown.

CLI client:

- may use synchronous waits appropriate for terminal commands, but it uses the
  same task, cancellation, plan, and result semantics;
- must not keep a separate database writer or cleanup executor once migration is
  complete.

Current migration debt is explicit: `DiskTree` contains presentation state such
as `is_expanded` and is not an FFI DTO; the CLI still owns projections,
orchestration, and deletion; and existing `DuxError` display strings are not a
stable boundary contract. The Phase 0 formatted-size call proves linking only,
not localization or the final API shape.

## Persistence and multi-process rules

The app, bundled CLI, standalone CLI, and later scheduler may be separate
processes sharing data.

- SQLite owns queryable aggregates, settings requiring queries, and history.
- Large scan trees stay in versioned binary snapshots.
- Database migrations are transactional and guarded by a process-wide file lock
  or SQLite coordination appropriate to the operation.
- Schema v17 owns a bounded durable scan-scope lease registry. App scans and
  uncached progressive CLI scans acquire one random move-only lease before
  queue publication or terminal takeover. Exact, ancestor, and descendant
  canonical roots conflict; component-disjoint siblings may proceed.
- Lease acquisition, stale-row recovery, overlap comparison, and insertion are
  one immediate transaction under the current-schema writer lock. Every
  legacy `running` scan root is also a transitional blocker, even if its
  process claim is absent.
- A lease grants observation exclusion only. It cannot create a scan record,
  snapshot, candidate, plan, approval, AI request, or filesystem effect.
- Cancellation and shutdown drop queued work outside the task-registry lock;
  running work retains its lease through scanner quiescence. Release deletes
  only the exact random token and reconciles uncertain commit outcomes.
- Stale leases are recoverable only from same-host prior-boot provenance or a
  reliable same-scope exact-process observation proving the owner gone. Age,
  PID alone, foreign-host evidence, unavailable platform scope, malformed
  rows, and unclaimed legacy scans never authorize lease removal.
- Snapshot publication retains its unique temporary files, durable temp
  leases, exact scan lifecycle, and atomic no-replace semantics inside the
  acquired scan scope.
- Older clients detect unsupported schema versions and fail read-only rather
  than attempting downgrade writes.
- A pre-v17 CLI binary that never opened shared storage cannot be retroactively
  fenced while it is already running. Updating every app/CLI installation is
  therefore the one-time compatibility boundary for this protocol.
- App and CLI compatibility is part of release validation.

## Safety and AI constraints

- Only deterministic engine rules create candidates and cleanup plans.
- AI can label, summarize, group, and explain already-computed metadata; it
  cannot populate candidate fields or acquire plan/executor handles.
- Provider adapters are outside the cleanup authority graph.
- Local command providers are also outside the trusted confinement model.
  Sanitized environment variables, empty working directories, redaction, and
  tool-disable flags do not remove the ambient filesystem/TCC authority of a
  subprocess spawned by an unsandboxed app. ADR 0003's adversarial security
  spike is a shipping gate for every such adapter.
- No FFI function accepts an arbitrary AI-produced path as an actionable target.
- Destructive execution always revalidates immutable plan identity and current
  filesystem evidence.
- The app and CLI must operate with AI absent or disabled.

## Consequences

Benefits:

- One reviewed safety and classification implementation serves every client.
- The existing CLI remains useful and becomes an integration test for the
  engine instead of a competing backend.
- Cross-platform scanner and policy work is not trapped in a macOS target.
- Swift remains focused on native presentation and platform services.
- AI is structurally separated from cleanup authority.

Costs and risks:

- Core extraction is substantial because the current CLI owns orchestration,
  views, and deletion.
- FFI adds DTO copies, lifecycle rules, code generation, packaging, and two-
  language debugging.
- Coarse queries require deliberate pagination and aggregate APIs instead of
  convenient direct tree-object access.
- Rust/Swift concurrency and `Sendable` behavior may expose generator friction.
- Platform effects need carefully designed adapters without leaking policy into
  Swift.
- Shared storage requires multi-process schema and writer coordination.

## Alternatives considered

### Rewrite the engine in Swift

Rejected because it discards the tested cross-platform foundation, duplicates
CLI behavior, and creates two safety implementations during migration.

### Keep CLI as the engine and invoke it as a subprocess

Rejected because process text/JSON is too weak for cancellation, progress,
typed callbacks, plan handles, and high-volume Explorer navigation. A CLI JSON
contract remains valuable for automation, not as the in-process app backend.

### Put all application logic in Swift and keep Rust only for scanning

Rejected because classification, planning, validation, and execution safety
would diverge between app and CLI—the exact behavior most important to share.

### Build the macOS UI in Rust

Rejected by ADR 0001. Portable engine ownership does not require portable UI.

### Introduce a daemon or XPC service now

Rejected for the initial architecture. It adds signing, lifecycle, protocol,
update, and authority boundaries before the engine contract is proven. A later
service must have a concrete isolation or scheduling requirement and its own
threat model; it must not become a privilege broker.

### Expose the full arena as foreign objects

Rejected because millions of fine-grained calls and cross-language objects
would create unacceptable latency, memory, lifetime, and cancellation behavior.

## Validation criteria

Phase 0 must prove:

- `dux-ffi` returns its version and one formatted-size result;
- arm64 and x86_64 Rust builds form a universal XCFramework;
- generated Swift bindings import into a minimal app from a clean checkout;
- Swift calls Rust off the main actor and renders a typed value;
- Debug and Release builds pass with reproducible generation steps;
- panic/error mapping and one binding-lifetime smoke test pass;
- UniFFI-versus-C-ABI evidence is recorded in a separate ADR.

Before cleanup reaches the app:

- classification and computed views used by both clients live in core;
- candidate, rule, plan, validator, and executor APIs have no Swift-only policy;
- any future CLI deletion uses the centralized executor;
- no raw destructive UI/FFI call can bypass plan revalidation;
- shared schema compatibility tests cover app and CLI.

## Reconsider when

Revisit only if the Phase 0 integration shows an unresolved packaging,
performance, concurrency, or safety blocker. A problem with UniFFI alone should
trigger the documented C ABI fallback, not a Swift policy rewrite. A new client
must consume the same engine rather than fork its rules.

## References

- [ADR 0001: Native SwiftUI macOS application](0001-native-swiftui-macos-application.md)
- [ADR 0005: UniFFI for the Swift/Rust transport](0005-uniffi-swift-rust-transport.md)
- [Roadmap §3.2](../../ROADMAP.md#32-technology-boundaries)
- [Roadmap §17 FFI boundary](../../ROADMAP.md#17-ffi-boundary)
- [Roadmap §18 concurrency and state management](../../ROADMAP.md#18-concurrency-and-state-management)
- [UniFFI Swift bindings](https://mozilla.github.io/uniffi-rs/latest/swift/overview.html)
- [UniFFI async overview](https://mozilla.github.io/uniffi-rs/latest/internals/async-overview.html)
