# M8 Automation Scan-Admission Witness Security Review

Date: 2026-08-10

Status: accepted only for a retained, inspect-only scan-admission observation
witness. This review does not approve cleanup admission, current candidate or
history evidence, scheduler eligibility, occurrence claiming, planning,
journaling, notification, cleanup, or execution.

## Correction to the initial runtime review

The initial core-runtime review described both scan and cleanup work as lacking
durable publication before worker start. That was too broad. Scan admission
already commits an exact durable scope lease before the admitting engine
publishes its worker to the local registry. The missing proof was a retained
atomic observation interval: the observer released each local or store guard
before completing the next observation, so another scan could be admitted
between a clear store read and the second local snapshot.

Cleanup is different. A process can queue cleanup work before acquiring the
cross-process cleanup exclusion or publishing a durable cleanup session. That
queued work is not visible to another process. This checkpoint therefore
proves only scan admission. The cleanup gate remains `Unproven` after an empty
store observation.

## Decision

DUX may retain one scan-only admission witness while the sealed policy-
revision-1 runtime observer takes its two local snapshots and bounded store
observation. A `ScanWork` gate may be `Passed` only when all of these facts are
simultaneously established:

1. the engine's local `scan_admission` mutex was acquired with `try_lock` and
   remains held;
2. the first local registry snapshot is open and has no active scan;
3. the store cleanup lock, connection mutex, and cross-process writer guard
   were all acquired without waiting and remain held;
4. the live schema is still exactly current after writer acquisition rather
   than merely matching the coordinator's cached open status;
5. the bounded query finds no running scan, scan process claim, or scan scope
   lease;
6. the retained store controls revalidate without repair; and
7. the second local registry snapshot is still open and has no active scan
   before any admission guard is released.

The local admission mutex prevents the observed engine from entering its scan
lease/publication sequence during that interval. The writer guard prevents a
standalone or other-engine process from committing a new scan scope lease.
Any scan admitted before writer acquisition has already committed its lease
and is visible to the query. The result is a time-bounded absence observation,
not a promise that no scan will start after the guards are released.

A missing, poisoned, contended, invalid, incompatible, or over-budget witness
is never softened into a pass. Existing durable scan rows remain unresolved;
they are not treated as proof of current-process liveness.

## Lock and release order

The observer uses this exact acquisition order:

1. engine `scan_admission` mutex;
2. store cleanup observation lock;
3. store connection mutex;
4. store cross-process writer observation lock.

It retains all four through the bounded query, retained-control revalidation,
and second local snapshot. Release is the reverse safety order:

1. cross-process writer guard;
2. connection mutex guard;
3. cleanup observation guard;
4. engine `scan_admission` mutex.

The retained store wrapper declares its guard fields in the required writer,
connection, cleanup order so Rust's field drop order is explicit. No callback,
row identity, selector, or guard escapes the crate-private observation call.

## Inspect-only and zero-wait boundary

Every contention point is opportunistic: registry, local scan admission,
persistence status, connection, cleanup lock, and writer lock use `try_lock` or
one nonblocking advisory-lock attempt. Observation never sleeps, retries,
waits for a lease, repairs permissions or sidecars, provisions a control file,
opens a transaction, migrates schema, performs recovery, or changes durable
state. Retained cleanup and writer controls are validated using observation-
specific inspect-only helpers before their facts are trusted.

Cleanup-lock contention still reports active cross-process cleanup and leaves
scan admission unresolved because lock order prevents acquiring the writer
behind an unavailable cleanup guard. Writer or local scan-admission contention
maps to an unproven scan observation. Store/query failure and malformed state
retain their existing closed mappings.

## Cleanup remains deliberately unproven

The retained cleanup lock proves only that no process currently holds the
cleanup exclusion. It does not reveal cleanup work already queued in another
process and waiting to acquire that lock. The split store facts therefore keep
scan-admission uncertainty separate from cleanup-admission uncertainty:

- a successful retained writer observation can clear only scan-admission
  uncertainty;
- cleanup-admission uncertainty remains set even when durable cleanup tables
  are empty; and
- cleanup-lock contention can classify cleanup as active but cannot make the
  scan observation complete.

No production path may return `CleanupWork: Passed` until a separately
reviewed synchronous cross-process cleanup-admission protocol closes the queue-
before-publication gap.

## Authority and version boundary

This witness changes no authority graph. It remains a crate-private input to
the sealed runtime-blocker assessment. It does not establish that a schedule
is enabled, due, eligible, runnable, or safe and cannot be converted to
`AutomationSchedulerRuntimeEvidence`. No scheduler, planner, cleanup API,
notification source, CLI, AI adapter, FFI method, or native model consumes it.

Schema remains v22, UniFFI remains v66, automation overview remains v3,
runtime policy remains revision 1, every shipped rule remains unschedulable,
execution remains unavailable, and `AppRuntime` continues to construct only
`NoEnabledSchedulesDuxAutomationDecisionSource`.

## Threat review

| Threat | Required refusal or containment |
| --- | --- |
| Same-engine scan starts between clear observations | Retain `scan_admission` from before the first local snapshot through the second. Contention or poison is unproven. |
| Another engine or process commits a scan after the query | Retain the nonblocking cross-process writer guard through the second local snapshot. |
| Scan was admitted just before writer acquisition | Scan admission commits its scope lease before queue publication, so the bounded query observes the lease and keeps `ScanWork` unproven. |
| A newer process upgrades the live schema after this coordinator opens | Reinspect the live schema under the retained writer guard; cached-current status alone cannot clear scan uncertainty. |
| Observer blocks a user operation | Every acquisition is zero-wait; contention closes the gate instead of sleeping or retrying. |
| Observation repairs hostile controls | Observation-specific validation is inspect-only and rejects unsafe or incomplete controls without provisioning or repair. |
| Cleanup lock is mistaken for complete cleanup admission | Split uncertainty keeps cleanup unproven even after a retained empty store read. |
| Four runtime gates are treated as scheduler authority | Cleanup cannot yet pass in production, and the assessment has no conversion or consumer even if a future checkpoint closes it. |
| A retained guard leaks effect authority | Guards remain crate-private inside the observation wrapper and are dropped before the assessment returns. |

## Repository guardrails

`scripts/tests/test_automation_schedule_boundary.py` must prove that:

- stored scan- and cleanup-admission uncertainty are distinct facts;
- the engine observation attempts and retains its local `scan_admission`
  witness around both local snapshots;
- the store observation retains cleanup, connection, and writer guards, with
  writer released before connection and cleanup;
- the writer observation helper is zero-wait and inspect-only;
- observer code contains no repair, provisioning, transaction, migration,
  recovery, scheduling, task-admission, or effect edge;
- schema v22, UniFFI v66, automation overview v3, runtime policy revision 1,
  the all-unschedulable catalog, unavailable execution, and the static-empty
  production source remain unchanged; and
- this checkpoint is documented as scan-only while the M8 scheduler parent
  remains open.

Focused concurrency tests must prove observer-first exclusion, admission-first
durable visibility, local scan-admission contention, cross-process writer
contention, cleanup contention, standalone lease visibility, second-snapshot
retention, live-newer schema refusal, and no durable mutation. Full repository
and universal app qualification remain integration requirements rather than
evidence supplied by this document.

## Deferred gates

Before any production scheduler source can become nonempty, DUX still needs:

- a synchronous cross-process cleanup-admission witness;
- protected-descendant-complete exact-scope manual history;
- sealed exact-rule/scope current candidate, process, coverage, evaluation,
  and live-validation evidence;
- authoritative platform energy and low-disk episode evidence;
- production deadline selection and occurrence CAS/claiming;
- fresh planning, immediate revalidation, run-cap enforcement, serialized
  journal execution, terminal settlement, failure pausing, and notifications;
  and
- execution-era crash tests and every remaining Release cleanup gate.

Passing this review proves only a retained scan-admission absence observation.
The Milestone 8 scheduler task remains open.

## Related decisions and plans

- [Core runtime evidence review](m8-automation-core-runtime-evidence.md)
- [ADR 0014: Automation clock, wake, and missed-run semantics](../adr/0014-automation-clock-wake-and-missed-run-semantics.md)
- [ADR 0015: Automation activation and UTC recurrence](../adr/0015-automation-activation-and-utc-recurrence.md)
- [Milestone 8 roadmap](../../ROADMAP.md#milestone-8-automations)
- [Security design](../../SECURITY_DESIGN.md#10-automation)
