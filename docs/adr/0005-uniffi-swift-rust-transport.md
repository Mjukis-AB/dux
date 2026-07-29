# ADR 0005: UniFFI for the Swift/Rust transport

- Status: Accepted
- Date: 2026-07-15
- Scope: private in-process ABI and generated Swift bindings

## Context

[ADR 0004](0004-shared-rust-engine.md) assigns portable storage policy to a
shared Rust engine and gives the macOS application a narrow `dux-ffi` adapter.
It deliberately left the transport choice open between UniFFI and a handwritten
C ABI until a native app spike could exercise the build and concurrency model.

This is an internal application boundary, not a public SDK or plug-in ABI. The
Rust static library, low-level C header, generated Swift source, and app are
built and released together from one source revision. Independent third-party
clients do not need binary compatibility with an already-installed library.

The boundary will eventually carry immutable summaries, pages, stable errors,
opaque engine/task/plan handles, and bounded event streams. It must not expose
the scanner arena, Rust references, platform-dependent integer widths, or one
foreign object per filesystem node. Destructive authority remains in the
engine's planner, validator, and executor; the choice of binding generator must
not weaken that architecture.

## Spike evidence

The Phase 0 spike established the following with UniFFI 0.31.2:

- proc-macro exports generate a typed immutable Swift record without a UDL file;
- one generator dependency produces the Swift source, C header, and module map;
- the generated Swift source imports and links from an XCFramework in Xcode;
- UniFFI's library contract and per-function checksum checks remain enabled;
- an explicit DUX library/FFI-contract handshake crosses the boundary;
- synchronous Rust work runs on `EngineService`'s dedicated queue and only a
  Sendable application DTO crosses back to the main actor;
- linked XCTest coverage verifies real Rust values and the main-actor handoff;
- a typed `EngineError` becomes a catchable Swift error and is mapped into an
  application-owned `EngineServiceError` before reaching render state;
- an opaque application-scoped `DuxEngine` proves construction, method calls,
  idempotent explicit close, typed use-after-close rejection, Swift ARC release,
  and the corresponding Rust object drop;
- the same source-only input independently produces Debug and Release universal
  `arm64` + `x86_64` applications;
- generated Swift is deterministic for the pinned dependency and is reviewable
  as committed source.

No packaging, linking, value-conversion, or current Swift concurrency blocker
was found. The spike intentionally does not treat the formatted-size display
string as a stable API or localization contract.

The first handle deliberately contains only lifecycle state and the Phase 0
smoke methods. It proves transport behavior, not engine extraction, task
cancellation, callback retention, paging performance, or cleanup authority.
Those capabilities retain their own gates below and in ADR 0004.

## Current realization

FFI contract v38 now carries the real shared engine session first introduced in
v4. The app supplies input-only private data/cache roots; storage paths never
return across the boundary. Eight maintenance kinds use opaque task objects
with nonblocking versioned path-free poll/cancel records, and exact Explorer
reviews use opaque scan-bound lease objects. No maintenance task accepts a path,
cap, inventory, victim, candidate, or cleanup instruction. Review sessions
expose scan ID, expiry, renewal, idempotent release, and bounded read-only
snapshot navigation.

Contract v34 changes the existing v22 global permanent-cleanup setting to an
explicit opt-in without adding a path or authority-bearing field. Rust projects
rowless and reset state as disabled, rejects impossible enabled-Default shapes,
and version-decodes legacy settings before the final journal gate. Swift accepts
only those shapes, requires a loaded disabled observation plus the exact typed
sentence before its sole product enable call, and treats disable/reset as
immediate protection strengthening. A repository regression pins that trusted
Settings → AppModel → EngineService → UniFFI call graph. The internal
permanent-safe action remains Debug-only; this contract change does not satisfy
or bypass the Release gates below.

Contract v35 adds a distinct effect-free Rust-target dry-check task that
consumes one exact opaque review but cannot create an effect witness or accept
a platform driver. Its path-free terminal result has zero-effect accounting
and cannot be upgraded or chained into permanent cleanup.

Contract v36 adds one synchronous, bounded pressure-episode history query. Its
request contains only record version, stable startup-volume identity, exact
capacity anchor, and a 1–64 limit. The newest-first response contains only
Warning/Critical level, entry time, optional recovery/escalation time, policy
revision, and a truncation bit. Rust validates the referenced volume lifetime,
proves the anchor as an exact raw sample or the current exact `last_seen`
observation when hourly cadence suppressed its raw row, and rejects arbitrary
between-observation timestamps. It also validates strict order, non-overlap,
single-newest-open shape, and one lookahead row;
Swift independently repeats the version, identity, anchor, interval, ordering,
open-state, and truncation checks off the main actor. The query cannot express
a path, candidate, recommendation, plan, approval, AI input, or mutation
command. Explorer and the menu-bar popover therefore consume it only as
anchored presentation telemetry.

Contract v37 adds one synchronous, bounded configured-project-root settings
surface. It accepts and returns at most 16 lossless path-byte observations with
explicit source, revision, timestamp, and changed state. Rust and Swift both
reject unsupported record versions, invalid encodings, relative/root/control
paths, duplicates, nested or overlapping roots, noncanonical order, oversized
input, and contradictory Default/Stored state. The methods only load, replace,
or reset durable discovery scope. They cannot start a scan, open a path,
produce a candidate or plan, approve cleanup, invoke AI, or reach an executor.

Contract v38 adds a separate path-free configured-root pressure-scan surface.
One admission request contains only the canonical startup-volume identity,
exact accepted capacity anchor, stored root ordinal, and optional expected
registry revision. Rust alone rereads and selects the lossless stored root,
proves the latest open Warning/Critical episode, enforces the per-root and
aggregate node limits, checks startup-volume membership including the fixed
macOS APFS System/Data pair, and returns either a typed root-local result,
qualifying durable result, exact targeted task, or newly owned targeted task.
User or ancestor scan handles are never projected as targeted ownership.
Targeted durable IDs are excluded from generic Home latest/history selection.
A second path-free call repeats only the exact pressure proof, registry
revision, and count for end-of-pass validation. Swift independently checks the
entire tagged shape, lossless root, exact echoed values, node arithmetic,
timestamp order, targeted scan-ID/result invariants, and the final checkpoint.
Neither call accepts a caller path, candidate, recommendation group, plan,
approval, AI request, cleanup mode, callback, platform driver, or effect
command.

Contract v9 adds one path-free newest-available review acquisition. The core
selects the deterministic newest succeeded, non-tombstoned snapshot and then
uses the same exact scan-bound lease acquisition, which repeats catalog,
tombstone, retained-file identity, and full-format checks while pinning. The
Swift adapter receives the authoritative selected scan ID from the lease and
owns renewal/release; recent-history metadata remains only a presentation hint.
Contract v10 adds one root record and direct-child pages capped at 200 under
that exact lease. Records preserve historical host bytes and an explicitly
lossy display string, expose only snapshot observations, and omit Unix identity,
live path handles, candidates, plans, and effects. Every page revalidates the
durable pin and retained immutable file. A session decodes once, release drops
the cache, invalid/expired sessions surrender admission immediately, and a
separate per-engine budget permits at most two retained trees within a
conservative 1 GiB decoded-memory estimate. A compact decode-time child index
and one sorted-child cache avoid full-subtree work on repeated pages. Generated
Release fixtures exercise balanced and wide million-node snapshots through the
real repository/review path. The measured wide page and treemap justify an
exact 999,999-direct-child ceiling with post-work lease revalidation; greater
fan-out remains budget-gated. Swift
converts into app-owned models, preserves typed expiry/navigation failures,
generation-fences controller results, and immediately removes an expired lease.
Contract v11 adds one logical-size treemap call under the same exact lease. It
returns at most 64 positive-size direct-child records with deterministic logical
ranks and exact path-free Other count/byte accounting, including zero-size
children. It reuses the retained document, logical ordering cache,
decoded-memory admission, and 100,000-child sort ceiling; it grants no path,
category, candidate, reclaimability, plan, AI, or cleanup authority.

Contract v12 adds one path-free Large Files query under the same review lease.
It requires a positive threshold, optionally filters strictly before an
observed modification timestamp, and returns no more than 200 deterministic
file records with exact match count/logical-byte totals and at most eight
historical parent-name components. Rust retains only O(k) top-result state and
revalidates the lease after the whole-snapshot pass. The DTO cannot express a
live path, reclaimability decision, candidate, AI input, plan, or cleanup
instruction.

Contract v13 adds an exact paged scan-coverage history query independent of
snapshot leases. A request names only a stable scan ID, version, offset, and a
limit capped at 64. Rust fully validates the durable record and returns exact
coverage totals plus canonical issue ordinals and all 13 semantic kinds.
Historical locations are scoped as global, scan root, or descendant and expose
at most the nearest eight root-relative display components; the absolute root,
URLs, and current filesystem handles never cross FFI. This metadata endpoint
therefore remains useful after snapshot pruning and for failed, cancelled, or
interrupted scans without granting review or cleanup authority.

Contract v14 adds a purpose-bound live-target resolver to the existing exact
snapshot review lease. Its versioned request contains only a snapshot node ID
and Reveal, Copy Path, or Quick Look purpose. Rust reconstructs the lossless
host path from the validated immutable snapshot graph; callers cannot submit or
concatenate a path. It then uses descriptor-relative no-follow validation and
requires the current root, every ancestor, and the target to match the
snapshot's Unix device/inode identities and entry kinds. The review lease is
revalidated after that work. Only files and directories are eligible, Quick
Look is file-only, and missing identities, changed objects, symlinks, special
entries, cross-volume paths, and access failures remain typed and path-free.

The response is deliberately ephemeral read-only evidence: it contains the
echoed request, current kind, lossless absolute host bytes, lossy display text,
and exact Unicode text only when available. It exposes no identities, snapshot
digest, candidate, plan, or cleanup witness. Finder and Quick Look are
path-based macOS APIs, so validation and presentation cannot be atomic; a
same-user filesystem mutation can race after return. That residual risk is
accepted only for these non-destructive conveniences and must never be reused
to authorize reading for AI, reclaimability, planning, or cleanup.
The current Unix identity witness is device plus inode. Inode reuse after an
object is removed is therefore an accepted residual ambiguity for these
read-only actions; it is not cleanup authority and should be strengthened with
birth-time or filesystem-generation evidence if the platform-neutral snapshot
schema later carries it.

Contract v15 adds a display-only storage category to each bounded snapshot node
record. The category is joined from the exact scan's immutable, validated
candidate evaluation; Rust uses the nearest classified historical root and
returns Unclassified when evaluation evidence is absent, ambiguous, or exceeds
the independent 4,096-root/1 MiB path-payload budget. An exact-root index makes
lookup proportional to node depth and is discarded on release or expiry.
Candidate persistence currently rejects non-Unicode host paths, so those
evaluations fail closed to Unclassified; display names are never used as a
substitute. This is
not a candidate transport: candidate identity, paths, evidence, status, safety,
action, reclaimability, AI data, plan, and execution authority remain sealed.
Swift maps the closed enum directly and never infers a category from lossy
display names. Color is redundant with a stable symbol, visible legend/table
text, inspector disclosure, and VoiceOver copy.

Contract v16 adds a path-free selected-folder scan bound to an exact retained
snapshot review. Its request carries only record version and snapshot node ID;
the engine verifies review ownership, resolves a directory from the immutable
graph, and requires its current filesystem identity to match the recorded
device/inode witness. That identity is fenced before durable admission, before
traversal, against the completed artifact root, and immediately before atomic
snapshot publication. The response reuses the ordinary opaque scan task and
its explicit polling/cancellation contract. Success creates a new standalone
immutable snapshot rooted at the selected folder; it never merges into or
rewrites the source snapshot. Swift keeps the source review pinned until the
exact result review, root, first page, and treemap validate, then performs a
generation-fenced lease handoff. No path, merge authority, candidate detail,
plan, AI input, or cleanup capability is added.

The seventh original task is scan recovery. It exposes only bounded page counts and
typed outcomes; process-instance identities and recovery-scope keys remain
private to Rust. As with every maintenance task, the transport cannot select a
scan, owner, or victim and cannot acquire cleanup authority.

Contract v32 adds the eighth task: pending candidate-evaluation recovery. It
accepts no path, scan/snapshot/candidate identity, timestamp, evaluator or
catalog input, AI output, plan, approval, or command. Rust samples the clock and
processes at most one oldest pending row by replaying only its exact retained
immutable snapshot. The path-free result carries only `None`, `Recovered`, or
`Incompatible`, recovered candidate count, observation time, and `has_more`.
The native scheduler requests it immediately after scan recovery, and both FFI
and Swift reject mismatched kinds, phases, outcome families, counts, or nonzero
unused fields before using the result.

Contract v5 added one synchronous, versioned, path-free
startup-volume observation/status pair. Swift supplies optional canonical
volume UUID evidence, optional Foundation metadata, a timestamp, total
capacity, and independent ordinary/important availability; the adapter fixes
the observed mount to `/` inside Rust rather than accepting an arbitrary path.
Rust returns the selected headline source, deterministic pressure, effective
boundaries, prior durable pressure, and an explicit history disposition. Missing
identity, incomplete metadata, and important-only observations remain ephemeral
and cannot mutate volume or sample history. A bounded per-engine session slot
retains only the newest display-pressure baseline so those observations still
receive hysteresis without becoming durable identity evidence. The generated
call remains confined to `EngineService`'s utility queue, and neither input nor
output can express a scan, candidate, plan, path, or cleanup instruction.

Contract v6 adds three synchronous, versioned, path-free pressure-policy calls:
typed get, set, and reset. Exact integer thresholds, Default/Stored provenance,
revision, update time, and changed disposition cross the boundary; generated
errors distinguish every semantic validation failure from storage failures.
Startup observation/status remain record v1, and policy cannot be supplied with
an observation. Swift converts app-owned exact decimal strings to integer
bytes/basis points before calling Rust, while `DiskPressureConfig` remains the
sole semantic validator and evaluator. A changed save/reset signals the native
capacity scheduler exactly once; an unchanged, failed, cancelled, or invalid
operation cannot signal. No policy record can name a volume, path, candidate,
plan, schedule, notification, or cleanup action.

Contract v7 adds one coarse read-only discovery task. Its versioned request
accepts a nonempty absolute root bounded to 32 KiB of control-free UTF-8. The
root is observation scope only: core repeats filesystem-kind, symlink,
accessibility, stable-identity, overlap, schema, and admission checks, and no
root or descendant path returns across this M3 boundary. The native adapter
fixes production requests to the current user's Home directory; injected roots
exist only for isolated linked tests. An opaque `ScanTask` retains its event
cursor privately and exposes nonblocking poll/cancel calls with authoritative
task phase, coarse stage, cancellation intent, revision, optional cumulative
aggregate progress, sticky event-truncation evidence, typed path-free failure,
and a path-free terminal summary. Absence of progress is unknown, never a
measured zero. A late cancellation cannot overwrite a successful terminal
result. The task has no candidate detail, review selection, plan, approval,
execution fence, or cleanup capability, and its input cannot be reused as
planner or executor authority.

Contract v8 adds one bounded newest-first recent-scan history call for Explorer
selection. Dedicated immutable records expose stable scan identity,
timestamps/status, succeeded counts, coverage aggregates, and whether history
recorded a snapshot reference. No root, node, file name, candidate, evidence,
snapshot location, digest, handle, plan, or effect capability crosses. The
recorded-reference bit is only a UI hint: every selection must acquire the
existing scan-bound expiring review lease, whose repository checks are the
availability and safety authority. Swift performs the call on the utility queue
and rejects malformed versions, ordering, identity, timestamps, lifecycle/count
shape, coverage, or snapshot hints before returning app-owned values.

Contract v27 adds explicit direct-Cargo discovery enrollment without adding a
cleanup edge. Its only input is one versioned, bounded, control-free UTF-8 Unix
path selected by the user; the boundary accepts no command, `PATH`,
environment, expected identity, candidate, plan, or effect data. Static
inspection returns one opaque engine-bound preview and runs no selected bytes.
At most one preview may be live per engine. Its bounded path, digest, and
static-code records are display observations only; commit accepts only the
opaque object, checks engine affinity before consuming it, and consumes it
permanently before the core's fallible Cargo-version validation. Release is
explicit and idempotent, close drains retained previews, status is
observational, and revoke accepts no identity.

Inspect and commit remain synchronous bounded core calls on
`EngineService`'s utility queue. This settings lifecycle deliberately has no
pretend Swift cancellation after the user confirms execution of the fixed
version command: Settings shows a finishing state, ordered shutdown waits
behind the serialized call, and task cancellation only prevents stale
presentation. Failure or outcome uncertainty requires authoritative status
reload and a newly inspected preview; the app never automatically retries the
consumed capability. The confirmation carries the exact displayed generation
and evidence. A malformed or non-correlating returned mutation record is
outcome uncertainty, not a retryable presentation error; one read-only status
reload runs, and Settings visibly blocks mutations until an authoritative read
succeeds. Once core reports mutation success, a transport-projection failure is
likewise outcome-unknown; pre-mutation core errors retain their typed mapping.
Dismissal releases a preview even when static inspection completes after the
view disappears. Runtime shutdown memoizes one shared task before its first
suspension, preventing reentrant callers from duplicating the stop/close
pipeline while a confirmed mutation is finishing. This is a narrow exception
to task-style cancellation for a revisioned settings mutation, not precedent
for scan, planner, or cleanup work. No candidate, plan, approval, journal,
scheduler, AI, or executor handle crosses v27.

Contract v28 adds the first reviewed-plan observation without adding an
approval or cleanup call. The request contains one candidate ID and the
already-retained opaque Explorer review; the caller cannot provide a scan ID,
path, rule, mode, time, warning, estimate, plan ID, schedule, or command.
Rust derives those values by running the exact production Rust-target
acquisition chain and retains the resulting non-cloneable reviewed plan behind
a second opaque object. That child exports only immutable `info` and
idempotent `release`.

Preparation does not hold a Swift/FFI parent mutex or engine-state mutex across
Cargo, filesystem, database, or reviewed-plan work. The core splits admission,
unlocked preparation, exact-parent validation, unlocked materialization, and
post-validation; an unforgeable per-session identity and liveness token prevent
another review of the same scan from substituting for the admitted parent.
The FFI registry reserves one preparation/live slot atomically, rechecks close
at publication, and treats parent release/drop/expiry, child expiry, evidence
drift, and explicit release as terminal. Child deadlines are frozen to the
minimum reviewed-authority and parent horizon; renewing the parent never
extends a child.

Plan information is a display observation only. The exact current path crosses
as bounded platform bytes plus a display generated from those bytes. The v28
display codec byte-escapes invalid UTF-8, backslashes, controls, and a pinned
Unicode-16 union of format and default-ignorable scalars. Swift reconstructs
the codec and compares UTF-8 bytes, not canonically equivalent `String`
values, then independently checks the terminal `target` component and complete
one-item permanent-safe rule/candidate/warning/lifetime shape. A dedicated
utility queue keeps the bounded live checks off both the main actor and the
serialized engine queue. The app controller owns each child, refreshes the
unchanged observation every 15 seconds, releases children before parents, and
generation-fences every candidate, mode, snapshot, cancellation, expiry, and
shutdown transition. No reconstructed preview DTO can be passed to or mint
authority for an approval, journal, scheduler, AI, Trash, or executor
endpoint. The existing confirmation-gated Explorer Trash route is separate;
v28 adds no permanent-safe cleanup endpoint.

Contract v29 exposes the core's already bounded exact-session cleanup-history
observation without adding an authority edge. The request contains only a
versioned stable session ID copied from the recent path-free summary feed.
Rust reloads and completely validates the stored session graph, then projects
one versioned path-free session summary, ordered item summaries, and ordered
warnings. The projection preserves the verified capacity delta, every typed
session/item status, bounded stable error categories, and complete-versus-
legacy policy shape while independently checking versions, identifiers,
ordinals, totals, status counts, timestamps, warnings, and lifecycle
consistency.

No path, evidence payload, candidate ID, execution owner/generation, claim,
journal receipt, or effect input crosses v29. The endpoint has no clear,
cleanup-retry, recovery, approval, callback, scheduler, AI, or executor
operation; the supplied session ID remains a presentation selector rather than
a capability. Swift independently repeats the complete version, ID, lifecycle,
legacy/complete shape, ordinal, aggregate, status-count, warning-order, and
bounded-category validation, including checked estimate sums and unique
summary-page session IDs, before publishing app-owned immutable models.
Generation-fenced selection, summary refresh, read retry, close, and shutdown
feed an in-window, path-free drill-down; its retry action reads the record
again and cannot repeat cleanup.

Contract v30 adds a separate Settings-only privacy operation for clearing
terminal cleanup-history metadata. Preparation accepts no row, session,
candidate, path, plan, approval, AI result, or effect input. It returns one
opaque engine/store-bound, consume-once preview containing only an exact
terminal-session count and date range. Rust validates and SHA-256 fingerprints
the complete five-table graph in keyset pages with fixed per-page and
per-session budgets; the two-minute authority lifetime is enforced with a
monotonic deadline, so wall-clock rollback cannot extend or invalidate it.
Active, recovering, and outcome-unknown evidence stays outside the terminal
selection, while a terminal parent with unfinished items or live claims fails
closed as inconsistent.

Commit consumes the preview before mutation, reacquires the cleanup exclusion,
opens an immediate transaction, and recomputes the exact terminal witness.
An SQLite authorizer admits only reads plus deletes from the five cleanup
history tables, and internal child-first statements delete terminal sessions
without exposing selectors across FFI. Post-commit observation failures are
reconciled against the exact witness: proven applied returns the correlated
count, proven not-applied returns the original typed failure, and every
unproven state becomes outcome-unknown. Neither Swift nor Rust retries the
mutation.

Swift independently validates the preview/result envelope, owns the raw child
on the engine utility queue, and binds confirmation to the exact immutable
count and range. AppModel generation-fences preparation, confirmation,
cancellation, concurrent history reads, Settings disappearance, and shutdown.
Every terminal response performs exactly one observation-only history reload.
The Settings copy states that this deletes only DUX activity metadata and does
not remove files, snapshots, scans, candidates, settings, exclusions, capacity
samples, or AI insights; it does not compact storage, resample capacity, or
promise free space.

Contract v31 adds the narrow consuming transport for the already engine-owned
Rust-target permanent-safe task. Its only start argument is the opaque
`RustTargetPlanReviewSession` produced by v28. No path, candidate or plan ID,
timestamp, approval Boolean, callback, AI result, command, or retry token
crosses the edge. FFI checks exact engine affinity before mutation. Every
owning-engine attempt then consumes the review irreversibly before core
revalidation; a concurrent `info` operation is converted to release-pending,
and a core refusal releases rather than restores the returned review. This
prevents a response, retry, or reconstructed DTO from becoming reusable
approval.

The returned `RustTargetCleanupTask` is an opaque observer over the existing
core registry. It can request cancellation or poll only versioned phase,
cancellation state, revision, bounded failure, path-free aggregates, optional
verified capacity change, and an exact history-correlating session ID. FFI
rejects the wrong task kind, unknown failure family, malformed session ID, and
inconsistent phase/failure/result combinations. Dropping the observer has no
cancellation or retry semantics. Start uses the existing operation tracker so
engine close either wins admission or waits for the admitted call without
holding the FFI engine-state mutex across live revalidation.

The native adapter now consumes this transport only through the exact
controller-owned child. The controller stores and compares the complete
immutable plan information shown to the user before removing that child, so a
same-UUID handle with altered plan, target, estimate, warning, rule, or expiry
facts cannot nominate the real opaque review. Explorer binds a separate
generation token and handle UUID into explicit confirmation, then observes only
the path-free task state. It does not retry, ordinary window dismissal does not
cancel, and app shutdown explicitly requests cancellation and waits.

The destructive action and confirmation are compiled only under
`DUX_INTERNAL_PERMANENT_SAFE_CLEANUP`, which XcodeGen assigns to Debug. Public
Release retains the observation-only preview and a locked unavailable label;
the notarized-release script independently rejects resolved Release settings
that contain the condition. The remaining permanent-cleanup release gates are
therefore still mandatory before this action can ship. AI, CLI, schedules,
history rows, and display DTOs still have no route to the consuming call.

The Swift adapter lazily constructs and synchronizes the engine on its utility
queue. FFI close invalidates renewal, attempts exact release for every
still-live registered review, then performs bounded core shutdown; a failed
durable release expires naturally, and concurrent close callers observe the same
result. The app first requests cancellation and quiesces its scan publication
driver, then stops maintenance and releases reviews before close. This is
evidence that the accepted opaque-object design scales to coarse asynchronous
ownership, bounded snapshot paging, and coarse treemap projection; candidate
detail, planner,
executor, and cleanup gates remain separate.

## Decision

Use UniFFI's built-in Swift bindings for DUX's private in-process Swift/Rust
transport.

Use UniFFI through the `dux-ffi` crate, currently with proc-macro definitions and
library-mode binding generation. Keep the runtime/scaffolding dependency and
the Swift generator on exactly the same locked UniFFI release. Upgrade them as
one reviewed change that regenerates bindings and runs the complete macOS gate.

Do not build and maintain a handwritten C ABI beside UniFFI. The C ABI is an
escape hatch: if a reconsideration trigger is met, replace the transport behind
the same coarse `dux-ffi` semantics and Swift `EngineService` boundary. Do not
fork engine policy or expose both transports as product APIs.

UniFFI-generated low-level symbols, headers, and Swift types are implementation
details. DUX makes no source- or binary-compatibility promise for them outside a
single matched app build.

## Contract boundaries

### Export shape

- Keep exports coarse and task-oriented. Prefer one page, summary, plan, or
  event batch per call over repeated property access.
- Use owned UniFFI values at the boundary. Copy core values into dedicated,
  immutable boundary DTOs.
- Use fixed-width integers. Never export `usize`, borrowed data, `PathBuf`,
  `SystemTime`, Rust enum layout, arena indices without generation scope, or raw
  filesystem handles.
- Bound every collection that can grow with disk contents. Large trees cross as
  pages, aggregates, or treemap budgets.
- Keep user-facing localized text in Swift. Rust supplies stable kinds, numeric
  facts, evidence, and diagnostic context.
- Never expose an arbitrary-path delete function. Cleanup must use immutable
  plan identity and engine-side revalidation.

### Swift isolation

- Handwritten Swift imports generated functions only inside `EngineService` or
  a similarly narrow adapter. Views and `AppModel` do not call UniFFI directly.
- Synchronous UniFFI calls that can block run on the service's dedicated worker
  executor, never the main actor.
- Convert generated values to application-owned Sendable DTOs before resuming
  Swift structured concurrency.
- Keep `@MainActor AppModel` limited to renderable state.
- Do not adopt UniFFI-generated async functions merely to remove the handwritten
  queue. UniFFI documents partial Swift 6 support and known async Sendable rough
  edges. First prove cancellation, executor behavior, and generated signatures
  with the pinned version. The Rust task/event model may remain synchronous at
  the ABI while `EngineService` exposes an idiomatic async Swift API.

### Errors and panics

- Every operation with an expected failure mode returns `Result<T, StableError>`
  where `StableError` is a boundary enum with stable semantic kinds.
- Swift handles the generated throwing API and maps it into application state.
  Rust diagnostic strings may be logged or shown as technical details, but are
  not localized product copy.
- Infallible exports are limited to truly invariant operations. Unexpected
  UniFFI call failures in generated infallible Swift wrappers can trap because
  those wrappers use forced error handling.
- Panics are defects, not domain errors. UniFFI contains them at its scaffolding
  boundary, but DUX must test the behavior and must not intentionally use panic
  as control flow.
- No panic payload becomes user-facing text. Release builds retain an unwind
  strategy compatible with the tested containment path unless a separate
  process boundary replaces it.

### Handles and object lifetime

- Introduce a UniFFI object only for state that requires identity, such as one
  engine session. Results and pages remain records.
- Exposed Rust objects are `Send + Sync`, internally synchronized, and owned by
  `Arc` through UniFFI's object model.
- Swift must not infer cancellation or task completion from ARC destruction.
  Long work has explicit, idempotent cancellation and completion semantics.
- Handles expose an explicit idempotent `close`/shutdown operation when timely
  resource release matters. Deinitialization remains a fallback, not the safety
  protocol.
- The first object binding must have a linked Swift test proving construction,
  method use, release of the foreign reference, explicit close behavior, and a
  typed error for use after close. Callback work additionally tests retention,
  completion, cancellation, reentrancy, and cycle avoidance.

### Versioning

DUX maintains separate version domains for:

- application/library release version;
- DUX FFI contract version;
- UniFFI scaffolding contract and generated function checksums;
- persisted snapshot/database schemas;
- CLI JSON schemas.

Do not collapse these into one number. The app and library are always packaged
together, but startup still performs the explicit DUX handshake so accidental
artifact mixing fails clearly. Keep UniFFI checksum checks enabled. Breaking DUX
contract changes increment the FFI contract version and update both sides in
one review.

### Generation and packaging

- Declare UniFFI once in workspace dependencies and consume that declaration in
  `dux-ffi`.
- Build the generator from the same locked package graph as the Rust library;
  do not use an unrelated globally installed `uniffi-bindgen`.
- Commit the generated high-level Swift source for review. Generate the low-
  level header, module map, static library, and XCFramework from source.
- CI regenerates committed Swift and rejects a diff, then builds/tests Debug and
  independently regenerates/builds Release from a clean checkout.
- `CONFIGURATION` for Rust generation must match the Xcode configuration that
  consumes the XCFramework.
- Public releases use the universal static library packaged in the signed app.
  The `cdylib` remains useful for tooling and smoke hosts but is not a second
  shipped ABI contract.

## C ABI fallback design

If UniFFI is rejected later, retain these semantics behind a minimal C surface:

- one opaque engine pointer and fixed-width opaque task/plan IDs;
- explicit create/retain/release or create/close/destroy functions;
- status codes plus an owned, length-delimited error record;
- pointer-plus-length buffers with a single matching Rust free function;
- caller-owned callback context and documented callback-thread rules;
- no Rust-owned pointer access from Swift and no exceptions or panics crossing C;
- explicit protocol/version query before any stateful operation;
- generated or mechanically checked C declarations and Swift wrappers;
- AddressSanitizer, leak, malformed-input, double-close, and callback-lifetime
  tests.

The fallback replaces UniFFI serialization and generated wrappers only. It does
not change the shared engine, DTO semantics, plan safety model, pagination,
concurrency ownership, or application-facing `EngineService` API.

## Consequences

Benefits:

- Typed Swift records, enums, errors, protocols, and object wrappers are
  generated from the Rust boundary instead of duplicated by hand.
- UniFFI supplies consistent value lowering/lifting, buffer ownership, panic
  status transport, contract checks, and reference-counted object plumbing.
- The proven XCFramework pipeline stays small and reproducible.
- Generated Swift is inspectable, testable, and hidden behind a handwritten
  service that protects the rest of the application from generator churn.
- A future Kotlin or Python diagnostic client is possible without making
  cross-platform UI a product requirement.

Costs and risks:

- UniFFI is pre-1.0 and generator upgrades can change generated source or build
  behavior.
- Generated Swift is substantial and can produce compiler/concurrency warnings
  outside DUX's direct control.
- Values cross through UniFFI's serialization/lifting layer and must be budgeted;
  it is unsuitable for millions of chatty node objects.
- Debugging spans Rust scaffolding, a low-level C module, generated Swift, and
  handwritten Swift.
- Object and callback cycles can leak even when each side is individually
  reference counted.
- UniFFI contract checks detect mismatched builds but are not a public stable
  ABI guarantee.

## Alternatives considered

### Handwritten C ABI now

Rejected because the spike found no UniFFI blocker and a manual ABI would make
DUX own buffer allocation, destruction, error transport, type conversion,
header synchronization, Swift wrappers, object lifetime, and callback plumbing.
It offers tighter low-level control, but that control does not currently offset
the extra unsafe surface and maintenance cost.

### Maintain UniFFI and C APIs in parallel

Rejected because two transports double compatibility and safety testing without
adding product value. The C design is documentation for replacement, not a
permanent compatibility layer.

### Use the CLI JSON protocol in-process

Rejected by ADR 0004. Process text is useful for automation but is too weak and
expensive for typed callbacks, cancellation, plan handles, and Explorer paging.

### Direct Swift/Rust interoperability

Rejected for the production boundary at this time. DUX needs a conservative,
testable interface over owned values and opaque handles; it must not depend on
unstable direct language-layout interoperability or expose Rust implementation
types to Swift.

### Rewrite the engine boundary in Swift

Rejected because transport inconvenience is not a reason to duplicate scanner,
classification, planning, or cleanup-safety policy.

## Validation criteria

The decision remains valid while all of the following hold:

- locked binding generation is deterministic and clean-checkout builds pass;
- the generated code compiles under the project's Swift language/concurrency
  mode without unchecked UI-level workarounds;
- one typed domain error round-trips into a catchable Swift error before a real
  fallible engine API is exposed;
- the first opaque handle passes the lifecycle suite described above;
- representative page and event batches stay within the roadmap's latency,
  memory, and transfer budgets;
- cancellation and callbacks never depend on main-actor progress for engine
  safety;
- fuzzing and integration tests find no ownership or malformed-value defect at
  the boundary;
- generated APIs remain containable inside `EngineService`.

## Reconsider when

Create a superseding ADR and evaluate the documented C fallback if any of these
conditions occurs:

- a reproducible memory-safety, panic-containment, or object-lifetime defect
  cannot be fixed or safely isolated on the pinned UniFFI release;
- required Swift 6 concurrency behavior needs pervasive `@unchecked Sendable`,
  main-actor blocking, or generated-source patching;
- generation or universal static-library packaging is unreliable in supported
  Debug, Release, signing, or notarization builds;
- representative page/event performance misses its budget after API batching;
- a required callback or cancellation semantic cannot be expressed and tested;
- UniFFI becomes unmaintained or cannot support the project's Rust, Swift, or
  macOS deployment baseline;
- DUX intentionally creates a separately versioned public SDK or plug-in ABI
  whose independently deployable binary-compatibility needs differ from the
  private app boundary.

A single awkward generated name or wrapper is not sufficient reason to replace
the transport; contain ordinary generator friction in `EngineService`.

## References

- [ADR 0001: Native SwiftUI macOS application](0001-native-swiftui-macos-application.md)
- [ADR 0004: Shared Rust engine](0004-shared-rust-engine.md)
- [Roadmap §17: FFI boundary](../../ROADMAP.md#17-ffi-boundary)
- [UniFFI user guide](https://mozilla.github.io/uniffi-rs/latest/)
- [UniFFI Swift bindings](https://mozilla.github.io/uniffi-rs/latest/swift/overview.html)
- [UniFFI Swift configuration](https://mozilla.github.io/uniffi-rs/latest/swift/configuration.html)
- [UniFFI procedural macros](https://mozilla.github.io/uniffi-rs/proc_macro/index.html)
- [UniFFI records](https://mozilla.github.io/uniffi-rs/latest/types/records.html)
- [UniFFI object-reference model](https://mozilla.github.io/uniffi-rs/latest/internals/object_references.html)
- [Rust Nomicon: Foreign Function Interface](https://doc.rust-lang.org/nomicon/ffi.html)
