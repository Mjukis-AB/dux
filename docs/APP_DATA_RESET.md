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

The following foundations exist as of 2026-07-31, without making Reset DUX
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
  `.dux-reset-data-<id>` and `.dux-reset-cache-<id>` destination components.
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
- A private pre-open roll-forward runner now reconciles only the
  already-implemented detach phases before ordinary storage publication. It
  reconstructs the transaction from the canonical 32-character lower-hex ID
  and rejects malformed, mismatched, or role-swapped stage names. Under one
  exclusive coordinator session it opens the data and cache namespaces only
  through descriptor-backed recovery admissions: no fresh directory, control,
  database, snapshot, or cache object may be provisioned or repaired, and
  SQLite is never opened. `Prepared` requires canonical data and a canonical,
  detached, or proven-absent cache, reconciles cache detachment, durably
  advances to `CacheDetached`, then reconciles data detachment and advances to
  `DataDetached`. `CacheDetached` refuses a canonical cache and may reconcile
  only canonical-or-detached data. `DataDetached` validates the exact final
  detached shapes without another effect. Later phases are not advanced by
  this runner. Every successful or refused reconciliation still returns typed
  recovery-required because no fresh canonical namespace exists yet.
- One five-second admission deadline covers the runner's coordinator handoff,
  descriptor-only namespace admission, and every pre-effect validation. Cache
  detachment and its read-back remain inside that deadline. Once the atomic
  data-root rename begins, its fixed 250 ms post-effect durability/read-back
  budget is allowed to finish so deadline expiry cannot turn an already moved
  namespace into an uninspected success. That bounded proof does not authorize
  another effect or ordinary engine admission.
- The recovery admissions retain the normal publication fences and exact
  writer/inventory locks, admit exactly one of canonical or transaction-derived
  detached storage, and consume their detach operation once. Missing, changed,
  or replaced coordinator state during shared-to-exclusive handoff can never
  downgrade into ordinary engine admission. Busy, changed-since-read, invalid
  transition, unavailable, or outcome-unknown results after an incomplete
  observation remain recovery-required; corrupt and structurally unsafe state
  remains coordinator-unavailable. The runner never deletes a stage, reports
  reclaimed bytes, or admits an ordinary engine.
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
through `DataDetached`, the ordinary-engine lifetime gate, and pre-open
roll-forward convergence for those three implemented phases. They still have
no UniFFI, CLI, Swift, or UI caller and authorize no user-data cleanup. Fresh
canonical namespace provisioning, `FreshNamespaceReady`, bounded draining
through `Complete`, validation of completed physical state, public path-free
transport, native confirmation/preference handling/relaunch, release
qualification, and Windows storage evidence remain prerequisites.

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

- record and transaction format versions;
- a random transaction identifier;
- exact canonical data/cache presence and filesystem identities captured from
  retained objects;
- exact generated single-component detached-stage names;
- the current monotonic phase;
- a checksum over the complete record.

No caller-provided path, row identifier, selector, cleanup capability, or
arbitrary filename may enter the journal or public API.

An invalid marker, newer record version, malformed checksum, impossible phase,
unknown coordinator entry, replaced identity, or unsafe permission is
`ResetRecoveryRequired`. It is never ignored or automatically overwritten.

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
     alias the detached identity.
5. `Draining`
   - Logical reset is complete. Bounded physical removal of detached objects is
     in progress or still owed.
6. `Complete`
   - Both detached stages are absent after descriptor-relative, bounded
     removal and durable parent synchronization.

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
in a move-only handoff, the exact journal is re-read under an exclusive lock on
the same coordinator storage, and only the `Prepared` → `CacheDetached` →
`DataDetached` prefix may be reconciled. Recovery derives canonical database
and transaction-stage names internally, retains the normal data/cache
publication and writer fences, and admits only exact canonical, detached, or
proven-absent shapes. It never provisions or repairs storage, opens SQLite,
removes a detached stage, or admits an ordinary engine. Missing or changed
handoff state remains recovery-required; corrupt or unsafe coordinator state
remains coordinator-unavailable.

The current runner prevents mixed old/new publication and converges crash gaps
across the two implemented detach effects. It deliberately stops at validated
`DataDetached` and returns recovery-required. A later checkpoint must securely
publish a provably fresh canonical namespace before any interrupted reset can
become usable again.

Terminal engine arbitration is implemented privately at the core boundary.
The private FFI validation handoff still carries no filesystem authority. No
public UniFFI or native reset action is admitted until fresh canonical
namespace publication, detached-stage draining, full-phase pre-open recovery,
and the remaining lifecycle work are complete.

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
- detached-stage draining is bounded, resumable, and reaches `Complete`;
- FFI transport is path-free, engine-bound, consume-once, and no-retry;
- native runtime arbitration, exact preference allowlist, relaunch, dialog,
  accessibility, and failure-path tests pass;
- macOS native rename, ACL, link, mount, allocation, signing, Debug, and
  Release evidence pass on both supported architectures.

Windows remains unsupported until equivalent handle, DACL, reparse,
no-replace rename, and allocation evidence exists. Unsupported platforms fail
closed; they do not receive a weaker reset.
