# M8 Automation Cleanup-Admission Protocol Security Review

Date: 2026-08-11

Status: accepted only for the schema-v23 cleanup-admission protocol
prerequisite. This review does not approve `CleanupWork: Passed`, a production
scheduler source, current candidate evidence, occurrence claiming, planning,
notifications, or automated cleanup execution.

## Decision

Every production cleanup entry point represented by the M8 runtime blocker
must acquire the existing store-wide cleanup exclusion synchronously before it
publishes process-local work. The exclusion is move-only and must remain the
same retained object through queueing, cancellation, journal admission,
filesystem-effect settlement, or process-lifetime quarantine.

The covered entry points are exhaustive for the current engine:

1. Explorer Trash selection;
2. Rust-target dry run; and
3. Rust-target permanent-safe cleanup.

The protocol reuses the already hardened cleanup advisory lock. It adds no new
path, sidecar, owner generation, selector, cleanup mode, approval, or effect
authority. `CleanupAdmissionLease` can prove exclusion, verify exact
`StoreCoordinator` identity, revalidate the live protocol, publish only a
supplied non-executable `planned` record under that exclusion, or be consumed
into the existing `CleanupJournalLease`. The raw planned-row store method is
test-only, so production cannot publish first and acquire the lease later.

## Publication invariant

No covered entry point may set `active_cleanup_operation`, insert a task
record, enqueue a worker closure, reserve Trash, write a `planned` session, or
invoke a platform callback before retaining its cleanup-admission lease.

The cleanup OS-lock acquisition and final registry acquisition are each one
zero-wait attempt. Live protocol inspection may use the store's existing
bounded connection/writer deadline, and process-quarantine inspection uses its
short blocking mutex. Before publication the engine:

1. performs cheap ownership and quarantine checks;
2. attempts the cleanup exclusion without holding the registry, quarantine,
   persistence connection, or writer guard;
3. rechecks process quarantine;
4. validates the exact retained cleanup control and live schema-v23
   read/write status; and
5. takes the registry only for the final lifecycle, capacity, active-operation,
   review-consumption, and publication checks.

Refusal drops the lease by RAII and returns the exact unconsumed opaque review
where the public API promises retry. No refusal creates cleanup history.

## Retained handoff by path

| Path | Pre-publication point | Retained handoff | First durable state/effect boundary |
| --- | --- | --- | --- |
| Explorer Trash | Before `TrashCleanupReservation` sets the local active token | Reservation moves the exact lease into the synchronous executor | Live protocol revalidation, then `planned`, claim, effect receipt, callback, and terminal settlement under the same exclusion |
| Rust-target dry run | Before task record insertion and queue publication | The queued `Work` closure owns the exact lease | Worker revalidates the live protocol and records the terminal effect-free observation under the converted journal lease |
| Permanent-safe cleanup | Before task record insertion and queue publication | The queued `Work` closure owns the exact lease | Worker revalidates, writes `planned`, claims it, validates, executes, and settles under the converted lease |

Queued cancellation deliberately creates no journal. Removing the queued job
drops its captured lease after local exclusivity is cleared, which is a safe
conservative overlap. A worker may release the cleanup exclusion before its
terminal task bookkeeping clears local activity; the overlap can only produce
extra blocking, never a false absence observation.

## Lock order

The protocol has partial orders rather than one misleading total order:

- a cheap quarantine precheck is taken and released before cleanup admission;
- after cleanup admission, the quarantine recheck is taken and released;
- live protocol validation retains cleanup while taking connection then writer,
  and releases writer then connection; and
- the final publication retains cleanup while taking the registry with one
  nonblocking attempt.

Code must never wait for cleanup exclusion while holding quarantine, registry,
connection, or writer state. Later durable journal operations retain cleanup
while acquiring connection then writer and release writer then connection
before cleanup. Outer FFI review/engine state locks remain lifecycle guards;
the cleanup OS-lock and registry attempts are zero-wait, and the M8 observer
does not acquire those FFI locks.

## Schema-v23 protocol epoch

Schema v23 changes no table, index, trigger, or data shape. Its checksummed
marker establishes the minimum writer protocol for trusting pre-publication
cleanup admission.

This epoch closes the mixed-version race:

- an older cleanup already executing holds the same cleanup exclusion;
- an older task queued before acquiring exclusion has not begun an effect;
- after v23 is installed, that older worker's mandatory live-schema check
  rejects its next journal write before a claim or filesystem effect; and
- a v23 worker repeats live protocol validation after queue delay before it
  writes `planned` or records a dry-run result.

A cached schema value is insufficient. Publication and worker handoff inspect
the live store and require exactly `DATABASE_SCHEMA_VERSION` with
`ReadWriteCurrent` access while the exact cleanup control remains retained.

## Runtime-observer boundary

This checkpoint fixes cleanup admission, not the M8 observation proof. The
stored observation still leaves `cleanup_admission_unresolved` set and exposes
no `with_retained_cleanup_admission` conversion. Therefore production
`CleanupWork` remains `Unproven` after an empty query.

The following slice must separately retain and revalidate the observer's exact
cleanup witness across both local snapshots, distinguish known local cleanup
from unrelated cleanup-lock contention, and clear uncertainty only when the
local snapshots, durable query, live schema, and retained control all agree.
Raw cleanup-lock contention cannot by itself prove active cleanup because
policy/exclusion changes, history clearing, and app-data reset also use the
same exclusion.

## Threat review

| Threat | Required refusal or containment |
| --- | --- |
| Another process observes between queue publication and cleanup locking | Impossible for v23 paths: the exact cleanup exclusion precedes local publication. |
| Worker drops and reacquires exclusion after queue delay | Forbidden. The move-only lease captured by the reservation or closure is consumed directly into the journal lease. |
| A second `EngineHandle` has a distinct local registry | The store-wide advisory exclusion, not the registry, is the cross-engine/process linearization point. |
| Queue-full, closed, poisoned, or locally busy refusal leaks the exclusion | Every pre-publication return owns the lease only on the stack; RAII releases it without a row or owner. |
| Queued cancellation fabricates durable cleanup | Cancellation drops the closure and lease without writing a journal row. |
| Schema changes while work waits in the queue | Every worker revalidates the live v23 protocol before its first durable cleanup write. |
| An older binary queued work before v23 | Its live-schema guard observes a newer schema and refuses before claim/effect. |
| A lease from another store is substituted | Exact `Arc` store identity is checked before publication and again at the workflow handoff. |
| Admission itself becomes effect authority | The type carries no path, plan, claim, effect receipt, or callback. Its sole write accepts a frozen record and can only publish non-executable `planned` state under the retained exclusion. |
| Cleanup-lock contention is reported as known cleanup | Deferred observer work must classify unknown holders as unavailable/unproven unless local/durable evidence independently proves cleanup. |

## Repository guardrails and tests

`scripts/tests/test_automation_schedule_boundary.py` must enforce that:

- schema v23 contains only the checksummed protocol marker;
- `CleanupAdmissionLease` is move-only, store-bound, and live-schema
  revalidated, and its OS-lock acquisition is zero-wait;
- the raw late-acquiring journal constructor and legacy plan handoff are
  test-only;
- raw planned-row publication is test-only and the production acquisition,
  conversion, and typed planned-write call sites are exact allowlists;
- Trash, dry-run, and permanent-safe paths acquire before registry
  publication and consume the exact retained lease without reacquisition;
- the runtime store still cannot clear cleanup-admission uncertainty; and
- no FFI, native, scheduler, CLI, AI, notification, or new effect edge is
  introduced.

Focused Rust tests must prove independent-coordinator OS-lock exclusion,
live-newer protocol refusal, queued permanent and dry-run retention, no-journal
queued cancellation, lease release, and Trash callback retention. Existing
review-reuse, quarantine, panic, ambiguous-settlement, close, and no-owner-
retention tests remain mandatory integration coverage.

## Deferred gates

Before any production scheduler source can become nonempty, DUX still needs:

- the retained cleanup observer witness described above;
- protected-descendant-complete exact-scope history;
- sealed current candidate, process, coverage, evaluation, and validation
  evidence;
- authoritative energy and low-disk episode evidence;
- production deadline selection and occurrence CAS;
- fresh planning, immediate revalidation, run-cap enforcement, serialized
  execution, terminal settlement, failure pausing, and notifications; and
- execution-era crash qualification plus every remaining Release cleanup gate.

Passing this review proves only that v23 cleanup cannot become locally visible
before it owns the store-wide cleanup exclusion.

The Milestone 8 parent remains open.

## Related decisions and plans

- [Scan-admission witness review](m8-automation-scan-admission-witness.md)
- [Core runtime evidence review](m8-automation-core-runtime-evidence.md)
- [ADR 0014: Automation clock, wake, and missed-run semantics](../adr/0014-automation-clock-wake-and-missed-run-semantics.md)
- [ADR 0015: Automation activation and UTC recurrence](../adr/0015-automation-activation-and-utc-recurrence.md)
- [Milestone 8 roadmap](../../ROADMAP.md#milestone-8-automations)
- [Security design](../../SECURITY_DESIGN.md#10-automation)
