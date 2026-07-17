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

FFI contract v7 now carries the real shared engine session first introduced in
v4. The app supplies input-only private data/cache roots; storage paths never
return across the boundary. Seven maintenance kinds use opaque task objects
with nonblocking versioned path-free poll/cancel records, and exact Explorer
reviews use opaque scan-bound lease objects. No maintenance task accepts a path,
cap, inventory, victim, candidate, or cleanup instruction. Review sessions
expose only scan ID, expiry, renewal, and idempotent release.

The seventh task is scan recovery. It exposes only bounded page counts and
typed outcomes; process-instance identities and recovery-scope keys remain
private to Rust. As with every maintenance task, the transport cannot select a
scan, owner, or victim and cannot acquire cleanup authority.

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

The Swift adapter lazily constructs and synchronizes the engine on its utility
queue. FFI close invalidates renewal, attempts exact release for every
still-live registered review, then performs bounded core shutdown; a failed
durable release expires naturally, and concurrent close callers observe the same
result. The app first requests cancellation and quiesces its scan publication
driver, then stops maintenance and releases reviews before close. This is
evidence that the accepted opaque-object design scales to coarse asynchronous
ownership; it does not satisfy the still-separate snapshot paging, planner,
executor, or cleanup gates.

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
