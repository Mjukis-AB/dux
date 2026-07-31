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
available to FFI, CLI, or native code:

- The independently marker-owned Unix/macOS reset coordinator stores the
  bounded crash-safe journal described below. It owns no reset target and
  cannot detach or remove storage.
- Core engine terminal arbitration records either ordinary close or app-data
  reset under the task-registry mutex. Exactly one reset contender can receive
  a move-only shutdown capability. Winning reset admission cancels queued and
  running work, rejects every later task and reset admission, and cannot be
  converted back to ordinary close. The capability yields a separate
  move-only quiescence proof only after lifecycle `Closed` and every worker
  handle has been joined. A bounded wait failure consumes the capability and
  leaves the old engine terminal without reset-effect authority.

This is lifecycle proof, not reset authority. It creates no durable reset
intent, opens no coordinator, performs no storage effect, and has no FFI or UI
caller. Retained coordinator sessions, cleanup/scan blockers, exact namespace
witnesses, detachment, pre-open roll-forward recovery, and bounded draining
remain prerequisites.

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
2. database cleanup lock;
3. database writer lock;
4. snapshot writer lock;
5. managed-cache writer lock.

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

Engine open runs reset-journal recovery before opening the database,
snapshots, cache, or publishing workers. This prevents a crash after one
detach from creating a mixed old/new session. A safe open either advances the
exact journal toward the fresh namespace or returns a typed recovery-required
failure.

Terminal engine arbitration is implemented privately at the core boundary.
The quiescence proof still carries no filesystem authority. No FFI or native
action is admitted until the retained storage handoff, detached-stage
draining, and pre-open recovery are complete.

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
