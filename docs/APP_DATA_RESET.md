# DUX app-data reset

Status: accepted implementation contract for the Milestone 9 reset boundary.

This document defines the only operation that may reset DUX's own application
state. It does not authorize cleanup of scanned storage. The feature must stay
unavailable in the app until every prerequisite in
[User-visible admission](#user-visible-admission) is proven.

## Product promise

**Reset DUX** starts the application over without touching the user's files.
It resets only exact, marker-validated storage owned by DUX and an explicit
allowlist of native preferences. It preserves the separately installed CLI,
operating-system permissions and integrations, Sparkle state, and every
unknown or unowned object.

Reset is a terminal application-lifecycle operation. It is not another
storage-clear button and is not implemented as recursive removal, SQL table
deletion, or a sequence of the existing clear APIs.

Logical reset and physical reclamation are different facts:

- Logical reset means the old proven DUX namespaces have been atomically
  detached and fresh canonical namespaces can be provisioned.
- Physical reclamation means every object in the detached namespaces has been
  removed by the bounded reset-debt drainer.

The UI must not claim reclaimed bytes until physical reclamation is observed.

## Implemented checkpoints

The following foundations exist as of 2026-08-02, without making Reset DUX
available through UniFFI, the CLI, or native code:

- The independently marker-owned Unix/macOS reset coordinator stores the
  bounded crash-safe journal described below. It owns no reset target and
  cannot detach or remove storage. One callback-scoped session now retains its
  writer lock across typed journal inspection/read-back, provisioning-debt
  inspection, begin, and exact forward transitions. A same-instance fence
  makes nested and concurrent attempts immediately busy without weakening the
  outer cross-process lock.
- Ordinary engine open now opens or provisions that coordinator and acquires a
  shared cross-process lease before database, snapshot, cache, or worker
  publication. The lease remains held through worker quiescence and lifecycle
  `Closed`. A decoded incomplete phase yields a move-only recovery intent that
  retains the original coordinator descriptors, shared lease, and exact
  journal observation. Pre-open recovery drops that shared lease, acquires the
  exclusive lock through the same retained storage, and requires the exact
  journal to survive the handoff before any namespace effect. Corrupt or unsafe
  coordinator state remains separately unavailable. Reset can acquire its
  exclusive coordinator session only after every ordinary engine has released
  this lifetime lease.
- Checksummed SQLite schema v18 adds two partial indexes for bounded unresolved
  cleanup item/path probes without rewriting history. Under the retained
  coordinator session, a higher-ranked callback acquires cleanup exclusion
  before the database writer/connection and cannot return that inner guard.
  Admission reports only path-free scalar blockers for cleanup-lock
  contention, running/recovering cleanup, durable uncertain effects, running
  scan/process-claim evidence, and scan-scope leases. An admitted move-only
  guard retains both store exclusions and can revalidate them plus the same
  bounded evidence before future namespace handoff.
- Core engine terminal arbitration records either ordinary close or app-data
  reset under the task-registry mutex. Exactly one reset contender can receive
  a move-only shutdown capability. Winning reset admission cancels queued and
  running work, rejects every later task and reset admission, and cannot be
  converted back to ordinary close. The capability yields a separate
  move-only quiescence proof only after lifecycle `Closed` and every worker
  handle has been joined. A bounded wait failure consumes the capability and
  leaves the old engine terminal without reset-effect authority.
- A private core composition now derives the coordinator root only from the
  store's revalidated canonical database path and retains that coordinator
  session first. It refuses incomplete journal state and unproven coordinator
  provisioning debt before terminal arbitration; these outcomes leave the
  prior engine lifecycle unchanged by the attempt. After a winning terminal
  claim, it proves the complete registry worker/maintenance inventory
  quiescent, joins every worker, checks active and process-quarantined cleanup,
  and then acquires the data-parent publication fence before
  cleanup/database exclusion. Active snapshot-review pins are path-free
  durable blockers, while expired pins are inactive but still strictly
  validated. It then acquires the snapshot writer only while the matching
  database admission is live, retains a complete canonical-name-bound
  snapshot inventory, and refuses an active staged snapshot as busy. Last, it
  fences the conventional cache parent and, when present, `Caches/Dux`, then
  read-only re-probes the fixed child. A present child retains its writer plus
  complete canonical inventory; an absent outer container or child is proven
  absent without provisioning either one.
- One freshly generated, 128-bit lower-hex transaction derives distinct typed
  `.dux-reset-data-<id>`, `.dux-reset-cache-<id>`, and
  `.dux-reset-fresh-<id>` destination components.
  Callers cannot swap roles or supply an arbitrary destination. The data
  witness binds the exact canonical root name, retained root and parent
  identities, private root policy, same-filesystem detach boundary, and absent
  data destination. Until separately owned witnesses exist, `ai` and `logs`
  must be absent. Revalidation also requires the exact database guard issued
  by the same store. The cache witness binds canonical parent/container/child
  identities, requires a present child to share the container filesystem,
  proves the typed cache destination absent, and never inventories or adopts
  unknown outer siblings. Every normal DUX data-root and managed-cache
  publisher participates in the same publication fences.
- One monotonic deadline bounds the complete coordinator, core-worker, cleanup,
  data/cache publication fences, database, snapshot, and cache-writer
  acquisition sequence; later layers receive only the remaining duration.
  Terminal-registry and runtime-blocker mutexes use the same deadline, and
  queued closures are discarded by tracked workers rather than synchronously
  during terminal arbitration. Expiry is checked again after the final
  complete revalidation and immediately before callback handoff, so slow
  validation cannot mint late admission. Every owned lock remains in its
  lexical acquisition scope. The higher-ranked callback receives only
  borrowed, validation-only wrappers, so return, panic, or forgetting a
  wrapper releases cache writer/publication, snapshot, database, cleanup,
  data publication, and coordinator exclusion in reverse order. The borrowed
  cache proof becomes mutable only inside the consume-once committed-intent
  transition described next.
- The callback-scoped core admission can now be consumed exactly once to
  publish the durable singleton `Prepared` journal. Data and optional managed
  cache device/inode facts are projected only from their retained
  descriptor-backed witnesses; callers cannot supply them. Before publication,
  the operation repeats every namespace/runtime/store/snapshot check, then
  rechecks the empty coordinator journal, absent provisioning debt, and original
  deadline after identity projection. Failures proven before the journal rename
  leave no intent. Once rename is attempted, every write, directory-sync, and
  read-back failure is outcome-unknown and returns only recovery-required—never
  a retryable admission. A committed higher-ranked continuation retains the
  exact journal and every admission proof and cannot escape the callback.
- That committed continuation can now be consumed exactly once to detach the
  managed-cache namespace and advance `Prepared` to `CacheDetached`. For a
  present cache, the operation revalidates the sealed journal binding,
  canonical child, controls, complete inventory, typed destination, and
  original deadline immediately before a descriptor-relative atomic
  no-replace rename. It synchronizes the retained `Caches/Dux` directory, then
  proves canonical absence, the exact device/inode at the transaction-derived
  stage, unchanged controls, and unchanged inventory before exact journal
  compare-and-advance. A prepared-absent cache provisions and renames nothing
  and advances only after re-proving absence. The post-effect proof uses the
  detached name rather than weakening canonical validation.
- Every cache-detach API layer consumes its witness, so a rename-attempt or
  later uncertainty cannot be retried. After durable `Prepared`, validation
  drift, deadline expiry, a destination collision, any rename/sync/read-back
  failure, and any journal-advance failure all collapse to payload-free
  recovery-required. A panic between the namespace effect and journal advance
  leaves the exact recoverable mixed state and releases every retained lock.
  Unknown outer siblings and the canonical data root remain untouched.
- The resulting `CacheDetached` continuation can now be consumed exactly once
  to detach the complete data root and advance to `DataDetached`. It first
  revalidates the sealed journal binding, canonical root and retained identity,
  exact database/control/sidecar/snapshot layout, absent transaction-derived
  destination, detached-or-absent cache state, reserved `ai`/`logs` absence,
  refusal of unproven snapshot-provisioning stages, and the original deadline.
  It then performs a descriptor-relative atomic no-replace rename, synchronizes
  the retained data parent, proves canonical absence plus the exact staged
  identity and retained store, and only then exact-compare-and-advances the
  journal. Unknown siblings of the data root remain untouched.
- The data-detach witness is consumed before any rename attempt. Destination
  collision, canonical or detached-cache replacement, cache appearance after
  an absence-bound detach, reserved-child or snapshot-stage drift, deadline
  expiry, every rename/sync/read-back failure, panic gap, and every journal
  uncertainty therefore return only payload-free recovery-required. No caller
  can repeat an uncertain effect from the old witness.
- A private pre-open roll-forward runner now reconciles the detach phases and
  publishes the exact pre-SQLite fresh namespace before ordinary storage
  publication. It reconstructs the transaction from the canonical
  32-character lower-hex ID
  and rejects malformed, mismatched, or role-swapped stage names. Under one
  exclusive coordinator session it opens the data and cache namespaces only
  through descriptor-backed recovery admissions. `Prepared` requires canonical data and a canonical,
  detached, or proven-absent cache, reconciles cache detachment, durably
  advances to `CacheDetached`, then reconciles data detachment and advances to
  `DataDetached`. `CacheDetached` refuses a canonical cache and may reconcile
  only canonical-or-detached data. From `DataDetached`, recovery retains the
  old data/cache exclusions while it constructs, validates, synchronizes, and
  atomically publishes only the transaction-bound fresh bootstrap described
  below. It then seals the fresh device/inode into journal V2 and advances to
  `FreshNamespaceReady`. A later pre-open pass may revalidate that exact fresh
  root and detached-or-proven-absent cache, durably advance to `Draining`, and
  remove at most one exact managed-cache payload object. Passes already
  observing `Draining` finish recognized payloads first, then retire at most
  one exact cache ownership marker, locked writer control, or empty stage shell
  through the monotonic structural tail described below. Only after exact cache
  absence may a later pass remove one bounded old snapshot final or quiescent
  recognized temporary. Once that payload inventory is empty, still-later
  passes retire exactly one snapshot marker, locked writer control, or empty
  snapshot directory through a second monotonic structural tail. After exact
  snapshot-directory absence, still-later passes remove at most one retained
  SQLite sidecar in raw-byte lexical order or, only after every sidecar is
  absent, the old main database. Exact database absence leaves the four
  structural controls and detached root as a typed no-effect tail. Later
  passes retire one protocol-ordered control or the empty root. Once old-root
  and cache absence are joined to the exact durable `Draining` journal, one
  pass may remove only `.dux-reset-origin-v1`; a later origin-absent pass may
  publish V2 `Complete`. Both passes still return typed recovery-required. A
  subsequent ordinary engine open must independently validate the completed
  physical envelope before SQLite or any worker is admitted.
- One five-second admission deadline covers the runner's coordinator handoff,
  descriptor-only namespace admission, and every pre-effect validation. Cache
  detachment and its read-back remain inside that deadline. Once the atomic
  data-root or fresh-root rename begins, its fixed 250 ms post-effect
  durability/read-back budget is allowed to finish so deadline expiry cannot
  turn an already moved namespace into an uninspected success. Each fresh
  rename has its own post-effect certainty budget; neither budget authorizes a
  later effect or ordinary engine admission. Snapshot draining carries
  the original recovery deadline through every journal/fresh/old/snapshot/cache
  pre-effect revalidation and checks it again at the final inventory gate
  immediately before unlink or `rmdir`. Only a successful snapshot payload or
  structural effect starts one shared, reset-specific 250 ms post-effect
  deadline; directory synchronization and all snapshot, old/fresh-root,
  cache-absence, and journal read-backs must finish within it or the pass
  returns outcome unknown. Old SQLite payload draining uses the same original
  pre-effect deadline and fresh shared post-effect deadline across old/fresh
  inventories, cache absence, and exact journal read-back; fresh-root inventory
  consumes that caller deadline directly and cannot mint a nested budget. The
  origin unlink likewise starts one fresh shared 250 ms certainty deadline;
  the namespace-effect-free `Complete` journal publication remains inside the
  original recovery deadline. Completed-state admission receives the original
  absolute engine-open deadline through coordinator, root/cache/journal
  validation, registry acquisition, writer exclusion, and every pre-SQLite
  effect gate. SQLite
  recovery or migration is not interruptible after it has started, so the
  deadline is checked immediately before and after rather than described as a
  hard syscall/runtime cancellation bound.
- The recovery admissions retain the normal publication fences and exact
  writer/inventory locks, admit exactly one of canonical or transaction-derived
  detached storage, and consume their detach/publication operation once. The
  published witness is sealed to the current durable journal, transaction,
  old identity, typed fresh stage, coordinator parent, and canonical root name;
  it cannot advance another transaction. Missing, changed,
  or replaced coordinator state during shared-to-exclusive handoff can never
  downgrade into ordinary engine admission. Busy, changed-since-read, invalid
  transition, unavailable, or outcome-unknown results after an incomplete
  observation remain recovery-required; corrupt coordinator envelopes remain
  coordinator-unavailable. The incomplete-state runner never opens SQLite,
  migrates or repairs ordinary storage, creates snapshots/cache/`ai`/`logs`,
  reports reclaimed bytes, or admits an ordinary engine. The separate
  completed-state intent may open or migrate only the existing identity-bound
  canonical store. Ordinary SQLite corruption, migration, or availability
  failures retain the ordinary typed database error taxonomy rather than being
  relabeled as reset recovery.
- The first physical-debt primitive is deliberately narrower than a general
  drainer. A consume-once cache candidate exists only for the exact journaled
  detached cache identity and transaction-derived stage, or for proven cache
  absence. The coordinator can mint its opaque unlink capability only after
  the exact `Draining` journal is known durable and the fresh root, old root,
  cache location, controls, and complete bounded inventory still match. One
  invocation removes only the lexicographically first recognized cache entry
  or temporary, synchronizes the detached directory, and performs a bounded
  identity/inventory read-back. It returns only
  `{removed_objects, cache_payload_has_more}`; neither a path nor byte count
  crosses the boundary. Unknown objects block
  admission before effect. Journal uncertainty yields no unlink capability;
  uncertainty after unlink consumes the candidate and is reconciled by a new
  `Draining` recovery pass.
- A separate structural capability is available only on a later pass that
  already observed durable `Draining`. One typed admission routes a full store
  with recognized payloads back to the payload candidate; it can mint the
  structural candidate only for `FullControlsEmpty`, the protocol-produced
  `WriterOnly` crash tail, `EmptyStage`, or exact `Absent`. One pass removes at
  most the ownership marker, the retained-and-locked writer control, or the
  empty journal-bound stage shell, with directory synchronization and exact
  read-back after each effect. Marker-only, a partial tail with any other
  child, unsafe aliases/identity/permissions, and canonical/stage coexistence
  are hard errors and never fall back to payload removal.
- After typed cache absence, a separate snapshot-payload capability joins the
  exact durable `Draining` journal, old/fresh data witness, complete retained
  snapshot inventory, and cache-absence witness. The retained old data root,
  snapshot directory, marker and writer controls, and every final or recognized
  temporary must all share one filesystem, including at the final pre-effect
  gate. Any mount boundary refuses the whole inventory. One pass may remove
  only the lexicographically first final or quiescent recognized temporary;
  active temporaries, controls, the snapshot directory, and every other old
  data-root object are unreachable. The effect and progress carry no name,
  path, byte count, or reclaimed-capacity claim.
- Once snapshot payload selection returns empty, a separate structural
  admission recognizes only `FullControlsEmpty`, the protocol-produced
  `WriterOnly` crash tail, `EmptyDirectory`, or exact `Absent`. A distinct
  coordinator-only, consume-once capability may remove exactly one ownership
  marker, retained-and-exclusively-locked writer control, or exact empty
  snapshot directory per later pass. Marker-only, a partial tail with any
  payload or unknown child, unsafe aliases/identity/permissions, writer
  contention, and mount or deadline drift are hard errors and never fall back
  to payload removal. Exact absence is a typed validation-only witness; it
  cannot mint an effect or provision the directory. The full journal,
  transaction, coordinator parent, old/fresh roots, snapshot state, and cache
  absence are repeated under the original recovery deadline immediately before
  descriptor-relative unlink or `rmdir`. A successful effect synchronizes its
  retained parent and shares one newly minted 250 ms deadline across all local
  and coordinator read-back. Any later uncertainty is recovery debt and a new
  pass resumes from the next exact state without repeating the effect.
- After exact snapshot-directory absence, a reset-only old-SQLite admission
  accepts only the bounded database-present shape or the first exact
  controls-only structural state. The fixed sidecars sort by raw bytes as
  `-journal`, `-shm`, `-wal`; the main database is unreachable until they are
  all absent. One pass removes at most the selected private, single-link,
  same-filesystem file descriptor-relatively and leaves the initialization
  sentinel, cleanup/ready controls, exclusively locked writer control, and
  detached root. Snapshot presence or typed cache debt may resume the earlier
  pipeline only after the strict old-root observation succeeds. Every opener,
  inventory, admission, journal-stage, binding, or deadline error fails closed
  without fallback. The coordinator joins the exact durable journal,
  transaction, parent and canonical name, old/fresh roots, snapshot absence,
  cache-absence witness, retained locks, and original deadline before minting
  one consume-once unlink authority. A successful effect alone starts one
  fresh 250 ms deadline for old-directory synchronization and all old/fresh,
  cache, and journal read-back. Exact payload absence can only hand off to the
  structural tail; it cannot repeat a payload effect or claim bytes.
- The separate old-store structural capability recognizes exactly
  `FullControlsEmpty`, `LockControlsFull`, `CleanupAndWriterControls`,
  `WriterOnly`, `EmptyDirectory`, or `RootAbsent`. One later pass removes at
  most the initialization sentinel, cleanup-ready control, cleanup lock,
  writer control, or empty detached root in that order. The data-parent fence
  spans every state; cleanup and writer exclusions remain retained while
  present, and writer-only exclusion remains until its control is retired.
  Every other partial-control shape is unsafe. Each successful unlink or
  `rmdir` starts one fresh 250 ms synchronization and cross-layer read-back
  budget. Exact root absence hands off to a distinct completion batch. If the
  fresh transaction-origin record is present, that batch can unlink only that
  descriptor-retained exact record and uses a fresh 250 ms synchronization and
  read-back deadline. Only a later origin-absent pass can durably publish V2
  `Complete`; neither pass admits ordinary storage.
- A V2 `Complete` engine open retains the original shared coordinator lease in
  a move-only validation intent. It never provisions or replaces a missing
  canonical root. It requires the exact journal-bound fresh device/inode,
  canonical parent/name, absent old data/fresh/cache transaction stages, and
  no sibling `.dux-stage-<32 lowercase hex>` provisioning debt. A canonical
  root whose configured name itself matches that grammar is excluded from the
  sibling-debt test. Before first initialization, the root inventory is only
  the database plus writer/cleanup/ready controls and recognized SQLite crash
  sidecars; snapshots, `ai`, `logs`, snapshot stages, and unknown children
  refuse admission. The conventional canonical cache must still be absent.
  After the initialization sentinel is durable, ordinary DUX-owned reserved
  directories, recognized snapshot stages, and same-object macOS case aliases
  are accepted as evolved store state; a later valid cache may exist only with
  an identity distinct from the retired reset cache. The retained cache
  publication fence must first prove exact transaction-stage absence, so
  configuration, container, contention, deadline, and namespace-drift errors
  still block. Only `UnsafeStore`, `UnsafeObject`, `UnrecognizedStore`,
  `CorruptData`, and `Unavailable` for the exact canonical cache remain
  managed-cache state after that proof.
- Completed admission repeats the state-appropriate narrow or evolved
  namespace inventory after acquiring the database writer lock and immediately
  before sidecar repair or SQLite open, including when reusing a live same-
  process `StoreCoordinator`; its connection and writer waits share the
  original deadline. Cache and journal validation repeat under the same final
  writer gate. The returned cache publication fence remains retained through
  the first store effect; its release never waits on the in-process registry
  and leaves the identity claimed if that registry is contended. Root/cache/
  journal envelope checks repeat after store open. A test-only hook proves that
  an interposed reserved child blocks an uninitialized root
  without creating the initialization sentinel or changing database bytes.
  Valid same-version and newer-schema crash prefixes may resume; a newer schema
  remains `ReadOnlyNewer` and receives durable initialization evidence. Exact
  V1 `Complete` remains the legacy direct-admission case and does not enter the
  V2 physical-validation path.
- The FFI crate now has a private, non-UniFFI terminal-validation handoff. One
  session gate owns `Open`, typed ordinary-close/reset `Closing`, and terminal
  `Closed` state plus the exact count of admitted child operations. Engine
  calls are fenced by the retained engine-state lock; every child method and
  task poll/cancel carries the same gate. A winning reset claim removes and
  checks every live child in the plan-review, snapshot-diff, snapshot-review,
  direct-Cargo, cleanup-history-clear, managed-cache-clear, and
  snapshot-storage-clear registries, joins admitted callbacks and plan
  operations, and then invokes core validation exactly once with the caller's
  original absolute deadline. Losing ordinary-close/reset contenders wait for
  the same terminal result. Any release failure, poisoned child/tracker, or
  exhausted deadline prevents core validation and remains unquiesced for every
  later observer. The core adapter accepts no callback or payload and returns
  only bounded path-free lifecycle/recovery classification.

These checkpoints now include the internal durable `Prepared` intent, exact
managed-cache detachment through `CacheDetached`, exact data-root detachment
through `DataDetached`, transaction-bound fresh canonical publication through
`FreshNamespaceReady`, durable entry into `Draining`, one exact managed-cache
payload object per pre-open pass, the monotonic managed-cache structural tail,
and, after exact cache absence, one old snapshot final or quiescent recognized
temporary per later pass followed by the monotonic snapshot marker/writer/
directory structural tail, one old SQLite sidecar or main database per later
pass, and the monotonic old-control/detached-root structural tail. They also
include the ordinary-engine lifetime gate, pre-open roll-forward convergence,
origin retirement, durable V2 `Complete`, and physical-state-gated ordinary
admission through the existing store. They still have no UniFFI, CLI, Swift,
or UI caller and authorize no user-data cleanup. Ownership-proven retirement
of random provisioning debt, public path-free transport, native confirmation/
preference handling/relaunch, final public-reset release qualification, and
Windows storage evidence remain prerequisites.

The cache-detachment checkpoint is verified by 57 focused reset cases covering
present and absent caches, namespace and inventory drift, every pre/post-rename
fault boundary, directory durability, journal publication uncertainty,
deadline expiry, stage collision, stale writers, panic unwinding, second-open
crash shapes, and retained publication fences. The serialized full-core lane
passed 1,386 cases with three intentional ignores; its sole historical
host-load-sensitive `ChangedDuringReview` passed immediately in isolation. The
120 active FFI, 54 CLI, 40 repository-policy, 309-source destructive-call, and
680 linked native cases all pass. Universal Debug/Release qualification is
recorded with the checkpoint in `ROADMAP.md`.

The data-detachment and ordinary-engine-gate checkpoint is verified by 92
focused reset cases. They cover exact data and cache identity/content binding,
both present and absent cache drift branches, canonical-root replacement,
reserved children, unproven snapshot stages, preexisting and last-moment
no-replace collisions, every data/journal fault boundary, panic lock release,
mixed crash-state startup refusal, shared multi-engine exclusion, completed and
corrupt journal controls, and unsafe journal-stage shapes without mutation. A
host-loaded serialized full-core lane passed 1,382 cases with three intentional
ignores and 18 conservative query/FSEvents/Cargo-probe budget trips; every exact
failure passed in a fresh isolated replay, using the already-built test binary
and quiet windows for the home-bound FSEvents cases. The 120 active FFI cases,
both intentionally ignored FFI cleanup cases when run directly, 54 CLI cases,
40 repository policy cases, 309-source destructive-call audit, locked workspace
check, warnings-as-errors Clippy, formatting, and all 680 linked native tests
pass. Debug and Release bindings preserve the committed generated Swift hash;
the universal macOS bundles retain the byte-identical CLI and Sparkle 2.9.2
payloads. The exact inside-out ad-hoc Hardened Runtime-signed Release app passes
strict deep all-architecture verification at
`/private/tmp/dux-data-detach.VOvau4/Qualified/DUX.app`.

The fresh-canonical checkpoint is verified by 109 focused reset cases covering
the exact five-entry bootstrap, lossless canonical-root binding and legacy
upgrade rules, a different-sibling restart, truthful phase reporting after
durable progress, all six publication effect gaps, foreign canonical
collisions, wrong-origin and identity drift, stale durable handoffs, and
cross-transaction witness misuse. The serialized
full-core lane passed 1,431 cases with three intentional ignores; its five
host-load-sensitive review/Cargo deadline probes passed on exact immediate
replay. The 13 projection, 120 active FFI, 54 CLI, 40 repository-policy,
311-source destructive-call, locked Rust 1.88, warnings-as-errors Clippy,
formatting, and 680 linked native cases all pass. Debug and Release bindings
preserve the committed generated Swift hash; the universal bundles retain the
byte-identical CLI and Sparkle 2.9.2 payloads. The exact inside-out ad-hoc
Hardened Runtime-signed Release app passed strict deep all-architecture
verification at
`/private/tmp/dux-fresh-namespace.ska6Y1/Qualified/DUX.app`.

The first bounded-draining checkpoint is covered by 114 reset-focused core
cases plus 39 cache-local cases. They include present and absent caches, exact
lexical one-object progress, control and stage preservation, disputed unknown
objects, wrong identity/stage bindings, journal uncertainty before and after
publication, all four unlink/durability/read-back fault gaps, final authority
rechecks after journal/fresh/cache drift, cross-transaction witness rejection,
and restart convergence from durable `Draining`. Broader workspace and macOS
qualification evidence is recorded with the checkpoint in `ROADMAP.md`.

The cache structural-tail checkpoint adds cache-local state, restart, fault,
unsafe-shape, binding, writer-contention, and case-alias coverage plus engine
integration for one-effect-per-open convergence, exact-absence idempotence,
no unsafe fallback, final authority rechecks, cross-transaction refusal, and
post-effect recovery. Exact broad qualification counts are recorded with the
checkpoint in `ROADMAP.md`.

The old snapshot-payload checkpoint adds snapshot-store-local lexical
selection, final/temporary removal, active-writer refusal, binding, object
drift, and all four uncertainty seams plus engine integration for exact cache
absence ordering, one-effect-per-open progress, journal/fresh/cache/snapshot
interposition, cross-transaction refusal, unsafe inventory preservation, and
restart convergence. Exact qualification counts are recorded with the
checkpoint in `ROADMAP.md`.

The old snapshot structural-tail checkpoint adds the complete four-state
snapshot-store chain, exact absence as a no-effect witness, one-effect-per-open
restart convergence, final-gate and post-effect deadline coverage, retained
writer contention, unsafe marker-only/unknown-child/case-alias refusal, exact
data-candidate/cache-witness cross-transaction refusal, certain final-payload
handoff into the structural tail, a fresh post-effect deadline independent of
the expired admission deadline, and coordinator journal/fresh/cache/snapshot
drift coverage. Exact qualification counts are recorded with the checkpoint in
`ROADMAP.md`.

The old SQLite-payload checkpoint adds exact reset-only database-present and
controls-only states, raw-byte lexical sidecar selection with main-last
ordering, one-effect-per-open restart convergence, all six local uncertainty
seams, shared cache/journal post-effect deadline exhaustion, strict no-fallback
refusal, certain final-main completion, both cross-transaction joins, and final
authority rechecks for journal rollback/stage debt plus fresh/cache/old-root/
snapshot drift. Exact qualification counts are recorded with the checkpoint in
`ROADMAP.md`.

The old-store structural-tail checkpoint adds the complete five-effect
control/root chain, exact root absence as a no-effect witness, one-effect-per-
open restart convergence, all six local uncertainty seams, shared cache/journal
deadline exhaustion, retained cleanup-lock contention, non-monotonic partial-
shape and interposed-inventory refusal, certain first-effect completion, and an
unchanged fresh bootstrap plus durable `Draining` journal after root removal.
Exact broad and macOS qualification evidence is recorded with the checkpoint in
`ROADMAP.md`.

The completed-reset checkpoint adds exact origin retirement, a separate
origin-absent V2 `Complete` publication pass, a move-only shared-lease
validation intent, existing-root-only store open, strict pre-initialization and
evolved-store inventories, random sibling provisioning-debt refusal, first-
initialization cache absence, later optional cache evolution, state-appropriate
writer-locked root/cache/journal pre-effect checks plus post-open envelope
checks for both new and reused coordinators, deadline-bounded registry/
connection/writer reuse, exact raw-name alias refusal, initialized-state
binding, permission non-mutation before the final gate, an exact optional-
cache-object error allowlist, preserved ordinary database error classification,
safe coordinator journal-stage tolerance, and direct V1 `Complete` admission.
Exact broad and macOS qualification evidence is recorded with the closed
checkpoint in `ROADMAP.md`.

## Exact scope

### Included core storage

The reset coordinator may operate only on:

1. The exact marker-validated data root that contains the configured
   `dux.sqlite3` database and its fixed DUX layout:
   - database, sidecars, and control files;
   - scan, candidate, cleanup, capacity, and rule history;
   - core settings and policy records;
   - AI insight data;
   - snapshots and recognized snapshot maintenance objects;
   - fixed future DUX-owned reserved children admitted by the compiled layout.
2. The fixed `scan-cache-v1` child beneath the configured conventional
   `Caches/Dux` container, but only when its own marker and controls prove
   ownership.

The outer `Caches/Dux` container is conventional and unowned. Reset must never
rename, adopt, traverse, or remove it.

### Included native preferences

Only after the core result requires a restart may native code clear this exact
allowlist:

- `menuBar.labelMode.v1`
- `menuBar.visibility.v1`
- `storageAccess.introduction.v1`
- the versioned DUX disk-pressure notification cooldown keys

The implementation must centralize this list, test its exact expansion, and
remove no other defaults domain or key. Core reset recovery does not depend on
native preference clearing.

### Explicitly excluded

Reset preserves:

- all scanned files, projects, build products, downloads, and other user data;
- every unmarked, unknown, replaced, linked, aliased, or unsafe object;
- legacy and unknown siblings in `Caches/Dux`;
- the optional installed `~/.local/bin/dux` CLI and its manifest;
- Launch at Login registration;
- Notification Center authorization and delivered alerts;
- Full Disk Access and all other TCC state;
- the DUX application bundle;
- Sparkle consent, update preferences, feed state, and downloaded update state.

An unknown child inside a namespace whose compiled layout is exact blocks
reset. Unknown content is never treated as evidence of DUX ownership.

## Why reset detaches namespaces

Closing the public engine prevents new work and quiesces its workers, but
opaque FFI children can retain cloned Rust handles. Those handles may retain
SQLite connections, snapshot files, cache state, or code-owned identities
after ordinary close.

The reset boundary therefore atomically renames proven canonical namespaces
to exact transaction-specific detached stages. Stale handles continue to
refer to the detached objects and fail canonical path/identity revalidation.
They cannot alias the fresh canonical store. Physical removal happens later,
relative to retained descriptors for the detached stages.

No implementation may infer safety from Swift ARC release, Rust `Arc` counts,
the disappearance of a window, or `DuxEngine.close() == true`.

## Independent reset coordinator

The reset journal must survive detachment of both reset targets. It lives in a
fixed, independently marker-owned coordinator directory beside the configured
data root, under the same trusted platform container. It is not stored inside
the data root or the managed-cache child.

The coordinator has:

- an immutable format marker;
- a permanent writer lock;
- a checksummed, versioned singleton journal;
- exact private owner, mode, link, ACL, no-follow, and retained-identity
  validation;
- a bounded, deny-unknown inventory;
- atomic create-new journal publication/replacement and parent-directory
  durability.

The journal contains only the bounded facts needed for recovery:

- record and transaction format versions (new writes use journal V2);
- a random transaction identifier;
- exact canonical data/cache presence and filesystem identities captured from
  retained objects;
- the canonical data-root component as lossless lowercase hex of its raw Unix
  bytes, minted only from the retained descriptor-backed root witness;
- exact generated single-component detached and fresh-stage names;
- the fresh canonical device/inode from `FreshNamespaceReady` onward;
- the current monotonic phase;
- a checksum over the complete record.

No caller-provided path, row identifier, selector, cleanup capability, or
arbitrary filename may enter the journal or public API.

The V2 decoder remains byte-canonical. Exact V1 `Prepared` and `CacheDetached`
records can upgrade only after reopening the old canonical root by its sealed
identity; the lossless canonical component is written durably in V2 before any
namespace effect follows. V1 `DataDetached` is incompatible because detachment
destroyed the only safe name-to-identity association, and V1
`FreshNamespaceReady`/`Draining` are incompatible because neither the missing
root binding nor fresh identity can be reconstructed safely. An exact V1
`Complete` tombstone remains admissible for ordinary engine open under an
explicit legacy-complete invariant.

An invalid marker, newer record version, malformed checksum, impossible phase,
unknown coordinator entry, replaced identity, or unsafe permission is never
ignored or automatically overwritten. Corrupt or structurally unsafe
coordinator state maps to coordinator-unavailable; a valid incomplete reset,
valid completed reset with busy/drift/uncertain physical state, or disputed
provisioning debt remains recovery-required. Ordinary database corruption,
migration, and availability errors after a valid completed envelope remain
ordinary database errors.

## Journal state machine

The durable phases are:

1. `Prepared`
   - Exact final witnesses and locks are held.
   - The journal durably records intent.
   - No namespace effect has occurred.
2. `CacheDetached`
   - The marker-owned cache child is absent from its canonical name and present
     at its exact recorded detached name, or cache absence was the prepared
     fact.
3. `DataDetached`
   - The proven data root is absent from its canonical name and present at its
     exact recorded detached name.
4. `FreshNamespaceReady`
   - A fresh canonical data namespace has been securely provisioned and cannot
     alias the detached identity. Its exact identity is sealed in journal V2.
5. `Draining`
   - Logical reset is complete. Bounded physical removal of detached objects is
     in progress or still owed.
6. `Complete`
   - Both detached stages and the fresh transaction-origin record are absent
     after descriptor-relative, bounded removal and durable parent
     synchronization.
   - V2 records retain the exact fresh-root identity and canonical-root binding;
     the tombstone alone is not ordinary-open authority.

Before `Prepared` commits, any failure is a proven no-effect refusal. After it
commits, reset is non-cancellable and recovery always rolls forward. A caller
must never roll back a detached namespace or repeat an effect from an old
preview.

Each phase transition proves the prior phase, current canonical name, exact
detached name and identity again. A crash between an effect and its journal
transition is reconciled from both names and retained identity; absence alone
is insufficient proof.

## Locking and lifecycle

The reset lock order extends the existing storage order:

1. reset-coordinator writer lock;
2. data-root parent publication fence;
3. database cleanup lock;
4. database writer lock;
5. snapshot writer lock;
6. conventional cache-parent publication fence;
7. `Caches/Dux` publication fence, when that container exists;
8. managed-cache writer lock, when the fixed child exists.

Locks remain held from final preview revalidation through both namespace
detachments. Reset is refused before durable intent while:

- another reset owns the coordinator;
- any worker, scan, review, clear preview, cleanup execution, maintenance
  effect, or confirmed CLI mutation has not quiesced;
- cleanup journal evidence is active, recovering, quarantined, or
  outcome-unknown and could prevent a repeated user-file effect;
- any required store or control witness is unsafe or changed.

The engine lifecycle gains one terminal reset transition. It atomically wins
against ordinary close and every new admission, cancels queued/running work,
waits for full quiescence, performs the reset handoff, and never becomes usable
again. Ordinary close cannot turn into reset, and reset cannot start after
ordinary closing has won.

Ordinary engines acquire a shared coordinator lease before opening the
database, snapshots, cache, or publishing workers and retain it until worker
quiescence publishes `Closed`. Reset requires the exclusive form after its own
terminal quiescence, so it cannot overlap another live engine. Engine open
inspects the journal under that shared lease. An incomplete intent is retained
in a move-only handoff and the exact journal is re-read under an exclusive lock
on the same coordinator storage. Recovery derives canonical database and all
transaction-stage names internally, retains the normal data/cache publication
and writer fences, and admits only exact canonical, detached, staged-fresh, or
proven-absent shapes. It reconciles `Prepared` → `CacheDetached` →
`DataDetached`, then publishes or adopts the exact fresh bootstrap and commits
`FreshNamespaceReady`. A subsequent pass may advance to durable `Draining` and
remove one validated detached-cache payload object; a pass already observing
`Draining` resumes without any earlier phase capability, retires the cache tail
one structure per pass, and only after exact cache absence may remove one
validated old snapshot payload per later pass. Once payloads are empty, later
passes retire the snapshot marker, locked writer, and empty directory one
structure at a time, then drain one old SQLite sidecar or the main database per
still-later pass. Once the detached root is absent, one pass retires only the
fresh transaction-origin record and a later pass publishes `Complete`. Missing
or changed handoff state remains recovery-required; corrupt or unsafe
coordinator state remains coordinator-unavailable.

The current runner prevents mixed old/new publication and converges crash gaps
across both detach effects, both fresh-root renames, the `Draining` journal
transition, each detached-cache payload unlink, both cache-control unlinks, the
empty cache-stage removal, each old snapshot-final or quiescent-temporary
unlink, both snapshot-control unlinks, empty snapshot-directory removal, every
old-SQLite sidecar/main unlink, four protocol-ordered old-control unlinks, the
empty detached-root removal, the fresh-origin unlink, and the namespace-effect-
free `Complete` journal publication. It returns recovery-required after every
bounded recovery pass. A later engine open retains the shared coordinator
lease in a move-only completed intent, proves the exact physical state, opens
only the existing identity-bound store, and repeats the envelope before
admission.

## Fresh canonical bootstrap

Recovery creates a random private work directory beside the canonical root and
exclusive-creates exactly five entries:

- the configured database file at length zero;
- the current V2 writer-control marker;
- the cleanup-lock marker;
- the cleanup-ready marker; and
- `.dux-reset-origin-v1`.

The fixed-size origin record binds the exact transaction ID, old detached
device/inode, and distinct fresh device/inode. Every file is private,
single-link, descriptor-retained, synchronized, and revalidated; the 0700
directory must contain exactly those five entries. It intentionally has no
SQLite header, initialization sentinel, WAL/SHM/journal sidecar, snapshots,
cache, `ai`, or `logs`.

Only after complete validation may recovery atomically no-replace rename the
random work directory to `.dux-reset-fresh-<id>`, synchronize and read it back,
then atomically no-replace rename that typed stage to the canonical root and
synchronize/read back again. A typed stage or canonical root is adopted only
when all controls, inventory, origin, old detached identity, and fresh identity
match the current durable transaction. Canonical/stage coexistence, a foreign
collision, wrong transaction, replacement, alias, extra entry, or marker drift
fails closed and remains untouched. Errors before random-to-typed publication
may leave a private `.dux-stage-*` as explicit provisioning debt. Completed
admission refuses every sibling matching exactly `.dux-stage-<32 lowercase
hex>` without removing it; a stage-shaped configured canonical-root name is
excluded from that sibling check. Prefix shape alone never proves ownership.

Terminal engine arbitration is implemented privately at the core boundary.
The private FFI validation handoff still carries no filesystem authority. No
public UniFFI or native reset action is admitted until the remaining public,
native lifecycle, preference, relaunch, accessibility, and release work is
complete.

## Detached-stage draining

Physical removal is a separate, bounded recovery operation:

- only exact stage names and identities from the validated journal are
  reachable;
- traversal is descriptor-relative, no-follow, same-filesystem, and bounded by
  entry count, name bytes, decoded bytes, and elapsed time;
- one bounded batch removes only validated DUX layout objects;
- each removed object and parent transition is durably synchronized;
- `has_more` is explicit and no batch promises a capacity delta;
- an unknown child, mount boundary, link, replacement, permission change, or
  budget excess blocks the stage without touching the disputed object;
- reset debt remains visible until `Complete`.

Implemented subset: while the journal is durably `Draining`, pre-open recovery
may consume exactly one journal-bound managed-cache candidate and remove at
most one recognized non-control file. A later pass already observing
`Draining` may instead consume one distinct structural candidate and remove
the ownership marker, the retained-and-locked writer control, or the exact
empty detached stage shell in that order. A proven-absent or fully retired
cache reports a typed no-effect absence without provisioning the outer
container. Only on a later pass after that exact absence, recovery may consume
one separately joined snapshot candidate and remove the lexicographically
first old snapshot final or quiescent recognized temporary. The complete
retained snapshot inventory must contain no active temporary or unsafe child;
the retained old root, snapshot directory, both controls, and every payload
must share one filesystem. The journal, fresh and old roots, snapshot store,
and cache absence are repeated under the original recovery deadline at the
coordinator-only effect seam. A successful unlink starts one shared
reset-specific 250 ms deadline for directory synchronization and all remaining
post-effect revalidation. Once the payload inventory is empty, a later pass may
instead consume the distinct snapshot structural candidate and advance exactly
`FullControlsEmpty` → `WriterOnly` → `EmptyDirectory` → `Absent`, removing one
marker, retained-and-locked writer control, or exact empty directory. Each
effect repeats the full state at the final gate and uses the same post-effect
deadline rule. Exact absence is a typed no-effect witness. Marker-only and any
other partial shape are unsafe. After exact snapshot absence, another
coordinator-bound candidate selects at most one raw-byte-lexical SQLite sidecar
or, only after sidecars are absent, the main database. The strict reset-only
opener accepts no `ai`, `logs`, snapshot, unknown, aliased, unsafe, or changed
child at the payload effect boundary. Each unlink repeats the exact journal,
old/fresh roots, cache absence, retained locks, inventory, and deadline, then
uses one fresh 250 ms budget for directory and cross-layer read-back. Once all
payloads are absent, a distinct old-store structural candidate recognizes only
`FullControlsEmpty` → `LockControlsFull` → `CleanupAndWriterControls` →
`WriterOnly` → `EmptyDirectory` → `RootAbsent`. Later passes remove exactly one
initialization sentinel, cleanup-ready control, cleanup lock, writer control,
or empty detached root in that order. The data-parent fence remains retained;
cleanup/writer locks are held while present, then writer-only exclusion remains
until its control is retired. Each effect repeats the complete typestate and
transaction/root binding, synchronizes the old root or retained parent as
appropriate, and shares one fresh 250 ms budget across old/fresh/cache/journal
read-back. Unknown children and every non-monotonic partial-control shape are
unsafe. Exact old-root absence hands off to the origin-retirement/`Complete`
tail described above. Completed-state validation refuses random provisioning
debt but does not remove it, then opens only the existing exact fresh root.
Ownership-proven random-stage retirement remains outside this subset.

The drainer may run after relaunch. It is never a general recursive deletion
primitive and cannot accept a path from Swift, CLI, AI, settings, or a journal
field that was not generated and sealed by core.

## FFI contract after core admission

Implementation checkpoint: the private Rust-to-Rust handoff already performs
typed terminal arbitration, checked seven-registry child release, admitted
operation/callback drain, plan-operation drain, and callback-free core
validation under one absolute deadline. It is deliberately outside every
`uniffi::export` block, changes no generated binding, and cannot write reset
intent or perform a namespace effect. The public preview/result transport below
remains unimplemented.

The boundary will expose an input-free, path-free, engine-bound,
consume-once preview:

```text
prepare_app_data_reset() -> AppDataResetPreviewSession
reset_app_data(preview) -> AppDataResetResult
```

The preview discloses only bounded scalar accounting, included/excluded
categories, blockers, preparation time, and monotonic expiry. It contains no
path or authority-bearing identifier.

Reset consumption:

- verifies exact engine affinity before consuming the rightful preview;
- globally conflicts with cache, snapshot, and history clear previews;
- consumes before any suspension or fallible terminal work;
- transitions the owning FFI engine into reset-closing exactly once;
- releases registered children and verifies core worker quiescence;
- invokes the core reset primitive only after quiescence;
- never retries or remeasures through the old engine.

Results distinguish:

- `RefusedBeforeEffect`: the reset did not commit durable intent;
- `CompletedRestartRequired`: logical reset is proven; physical debt may be
  separately reported;
- `RecoveryRequired`: durable intent or an effect may have happened, so the
  app must restart into journal recovery and must not retry;
- `ShutdownIncomplete`: quiescence was not proven and no reset effect ran.

Every result leaves the old FFI engine terminal. Starting over uses a new
process and new engine instance.

## Native flow after FFI admission

Reset is owned by `AppRuntime`, not an individual settings model:

1. Prepare one short-lived preview.
2. Present a custom sheet with exact accounting, scope, exclusions, restart
   requirement, and the typed phrase `RESET DUX`.
3. Atomically consume the UI lease and enter the runtime's mutually exclusive
   terminal-reset state before the first suspension.
4. Quiesce settings operations, previews, confirmed CLI mutation, scans,
   Explorer work, reviews, capacity sampling, and maintenance in documented
   shutdown order.
5. Call the reset-and-close FFI operation. Swift performs no filesystem
   mutation and never calls ordinary close first.
6. On proven logical completion, clear only the native preference allowlist,
   launch exactly one new DUX instance, then explicitly terminate the old one.
7. On recovery-required, relaunch without retrying so pre-open core recovery
   owns reconciliation.
8. On a proven pre-effect refusal, show the reason. Whether the runtime can
   remain usable depends on whether terminal reset admission had already
   begun; the transport must state this explicitly.

Terminal admission is a top-level lifecycle/UI operation. No model, engine
adapter, Settings owner, Explorer owner, review controller, scan driver,
scheduler, clock, or other operation joined by the native drain may retain,
receive, call, or await `AppRuntime` or a terminal-request closure. Otherwise
an accepted child could await the terminal task while that task awaits the
child. Startup and the retained drain are the only allowed pre-terminal
ancestors; their recursion returns an explicit no-proof reentrant result. The
repository source-layering regression allowlists only the app/runtime ingress
files, and any future Reset UI must consume its UI lease and request terminal
from an external view/lifecycle task rather than an AppModel-owned operation.

Implementation checkpoint: `AppRuntime` now installs one synchronous native
terminal fence and retains one immutable winning task for ordinary quit or
effect-dormant app-data reset. AppModel, owned-storage Settings, CLI
installation, Explorer, reviews, scans, capacity sampling, and maintenance
retain and join accepted work even after presentation slots are cleared. CLI
installation returns a `ConfirmedCLIMutationQuiescence` marker only after
discard and service close; the runtime composes it into the broader
`NativeRuntimeResetQuiescence` marker only after every native owner drains.
Ordinary quit closes the engine exactly once after that aggregate proof; the
dormant reset path never ordinary-closes it. Neither marker proves FFI/core
reset quiescence, writes durable intent, or authorizes an effect.

Quit racing with reset waits for the same terminal task. Popover or Settings
dismissal cannot cancel a consumed reset.

The sheet's destructive action is not the default Return action. Cancel is
initially focused. Accessibility identifiers and VoiceOver status cover the
section, preview, scope, exclusions, typed phrase, confirmation, progress,
blocker, recovery, and relaunch state.

## User-visible admission

The Reset DUX action remains absent until all of the following are verified:

- terminal engine reset wins every close/admission race deterministically;
- the external journal and every phase survive exhaustive crash injection;
- pre-open recovery converges without mixed namespaces;
- stale closed FFI children cannot write through canonical names;
- active or ambiguous cleanup evidence blocks reset before intent;
- both data and cache namespace detachment preserve unknown external siblings;
- private detached-stage draining is bounded, resumable, reaches `Complete`,
  and completed physical admission is independently validated;
- FFI transport is path-free, engine-bound, consume-once, and no-retry;
- native runtime arbitration, exact preference allowlist, relaunch, dialog,
  accessibility, and failure-path tests pass;
- macOS native rename, ACL, link, mount, allocation, signing, Debug, and
  Release evidence pass on both supported architectures.

Windows remains unsupported until equivalent handle, DACL, reparse,
no-replace rename, and allocation evidence exists. Unsupported platforms fail
closed; they do not receive a weaker reset.
