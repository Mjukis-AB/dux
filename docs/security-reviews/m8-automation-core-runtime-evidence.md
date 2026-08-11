# M8 Automation Core Runtime Evidence Security Review

Date: 2026-08-10

Status: accepted only for a sealed, core-owned, observation-only runtime-
blocker assessment. This review does not approve current candidate evidence,
exact-scope history, platform energy evidence, a production scheduler source,
occurrence claiming, planning, journaling, notification, cleanup, or execution.

Update 2026-08-11: schema v23 closes the cleanup queue-before-exclusion gap for
all current production entry points. The runtime assessment still keeps
cleanup admission unresolved until its own retained witness is completed. See
[`m8-automation-cleanup-admission-protocol.md`](m8-automation-cleanup-admission-protocol.md).

## Decision

DUX may add policy revision 1 of a crate-private core runtime assessment. One
call to `EngineHandle::observe_automation_core_runtime` samples its own wall
time and returns exactly four ordered gate assessments:

1. engine lifecycle;
2. ordinary macOS current-user, non-privileged runtime identity;
3. active or unresolved scan work; and
4. active or unavailable cleanup work.

Each gate is `Passed`, `Blocked`, or `Unproven` with a closed reason. Missing,
unreadable, over-budget, unsupported, privileged, unresolved, or active facts
never become a pass. The observation is path-free and bounded. It contains no
schedule, rule, scope, candidate, scan-row identity, cleanup-session identity,
plan, approval, journal claim, task, callback, notification, path, or effect
capability.

This is a runtime-blocker observation only. The retained scan-admission
follow-up can now prove a time-bounded clear scan gate because scan scope leases
are durably committed before worker publication and the observer retains both
the local admission mutex and cross-process writer guard. Cleanup admission is
not yet synchronously visible before another process queues work, so a clear-
looking cleanup gate remains `Unproven`. Even a future four-pass result after a
separate cleanup witness exists would not mean that a
schedule is enabled, due, eligible, runnable, or safe. The assessment is
not `AutomationSchedulerRuntimeEvidence`, cannot be converted into it, and is
not accepted by the scheduler decision kernel or any cleanup API.

## Core-owned observation boundary

The public eligibility kernel still accepts constructible diagnostic facts.
This checkpoint does not treat those inputs as sealed authority. Instead, the
engine owns the observation entry point and:

- samples `assessed_at` inside core rather than accepting caller time;
- opportunistically retains the local scan-admission mutex while taking one
  single-lock snapshot of lifecycle and in-process scan/cleanup work before the
  bounded persistence observation and another afterward; admission or registry
  contention returns an unproven local observation instead of waiting, and
  lifecycle/local-work gates can pass only when both snapshots are open and
  clear;
- establishes runtime identity from the current process rather than a caller or
  native presentation value;
- observes retained in-process scan and cleanup work under the core registry
  boundary, then reads only bounded aggregate cross-process work state through
  a dedicated read-only persistence adapter; and
- maps every incomplete or unprovable result to a closed blocked or unproven
  reason.

The persistence adapter returns only aggregate classifications needed for the
four gates. Under the cleanup observation guard and connection mutex it retains
one nonblocking cross-process writer guard through the query, revalidation, and
second local snapshot. It uses inspect-only validation that never repairs
SQLite sidecars or control objects. It exposes no selector and cannot claim,
recover, cancel, pause, terminalize, or otherwise mutate a scan or cleanup
record. Observation writes nothing and does not admit a task. The accepted
scan-only proof and exact lock order are in
[`m8-automation-scan-admission-witness.md`](m8-automation-scan-admission-witness.md).

## Effect-dormant integration boundary

The assessment and its observation method remain crate-private. They are not
re-exported by `dux-core`, do not cross UniFFI, and have no native model, service,
Settings, CLI, or AI surface. `AppRuntime` continues to construct the native
automation actor only with `NoEnabledSchedulesDuxAutomationDecisionSource`.
Schema v22, UniFFI v66, automation overview v3, the default-off global control,
unavailable execution, and the all-unschedulable shipped catalog remain
unchanged.

No scheduler or timing source consumes this assessment. In particular, the
checkpoint adds no conversion to `AutomationSchedulerRuntimeEvidence`. That
existing type represents a complete scheduler runtime observation including
authoritative thermal and battery facts, which this checkpoint deliberately
does not provide. Defaulting unavailable platform facts to an all-clear value
is forbidden.

## Current-evidence and history separation

This checkpoint is the narrow core-runtime portion of the roadmap's deferred
runtime/current-evidence boundary. It does not provide:

- exact-scope protected-descendant-complete durable history;
- current candidate age or size;
- complete/current scan, evaluation, coverage, or validation evidence for an
  exact rule and schedule scope;
- rule-derived process inactivity evidence;
- platform energy, battery, thermal, or Low Power Mode evidence; or
- authoritative startup-volume low-disk episode identity.

The existing advisory history suggestion feed and recurring-storage aggregate
remain presentation observations and cannot fill those gaps. Category
membership bindings remain authoring-consent consistency witnesses, not runtime
facts or candidate authority.

## Threat review

| Threat | Required refusal or containment |
| --- | --- |
| Caller supplies a favorable clock | Core samples `assessed_at`; the method accepts no timestamp. |
| Closed or closing engine appears available | The lifecycle gate blocks and no task or source publication occurs. |
| Close or same-process work admission races the store read | Single-lock local snapshots bracket the bounded store observation; `scan_admission` is retained across both, and a lifecycle or local-work pass requires both snapshots to be open and clear. Mutex contention is unproven. |
| Another process admits a scan between a clear store read and the second local snapshot | Scan scope admission commits its lease before worker publication; the observer retains the cross-process writer guard through the second snapshot, so earlier work is visible and later admission waits. |
| Another process queues cleanup before publishing a durable row or acquiring cleanup exclusion | Empty durable cleanup tables never prove absence. Cleanup admission uncertainty remains separate and production `CleanupWork` stays unproven. |
| `sudo`, set-ID, or a non-macOS runtime appears ordinary | Runtime identity is core-observed; privileged or unsupported state blocks and unavailable identity is unproven. |
| Active scan is hidden by UI state | Core combines retained in-process work with bounded durable cross-process scan state; active work blocks and unresolved/over-budget state does not pass. |
| Cleanup debt becomes runnable or recoverable | The adapter returns only aggregate active/unavailable classification and no identity, claim, selector, liveness proof, or recovery handle. |
| A future four-pass result is presented as eligibility | No FFI/native projection or eligibility/scheduler conversion exists; types and documentation call the result a runtime-blocker observation only. |
| Missing energy becomes battery/thermal all-clear | No scheduler-runtime conversion exists and platform energy remains a separate deferred adapter. |
| Observation starts work | No scan, plan, task, cleanup, notification, or effect consumer imports the assessment. |
| Observation changes durable state or repairs permissions | The adapter is query-only and inspect-only; unsafe sidecars are rejected without repair, schema remains v22, and unchanged state is verified around reads. |
| Timing source reaches core authority through the new method | Automation timing files and `AppRuntime` retain the exact static-empty source boundary and cannot import the assessment. |

## Forbidden authority edges

The core runtime modules must not import, construct, expose, or call:

- automation schedules, category bindings, due deadlines, cursor advancement,
  occurrence identity, claims, or retries;
- arbitrary paths, roots, candidate or scan IDs, cleanup-session IDs, plans,
  approvals, journal claims, effect requests, platform drivers, or task handles;
- scheduler decisions or `AutomationSchedulerRuntimeEvidence` conversions;
- planner, executor, Trash, iCloud eviction, notification, AI, CLI, remote-rule,
  maintenance-rotation, or cleanup-recovery operations; or
- persistence writes, schema migration, task admission, cancellation, recovery,
  or terminal settlement.

No due, assess, run, execute, start, trigger, claim, admit, poll, wake, or
runtime-observation method crosses UniFFI. The production source remains exactly
the statically empty implementation.

## Repository guardrails

`scripts/tests/test_automation_schedule_boundary.py` must dynamically discover
the core runtime evidence modules and prove:

- policy revision 1 and the fixed four-gate shape exist;
- the assessment and engine method are crate-private and absent from
  `dux-core/src/lib.rs` public re-exports;
- the runtime modules contain no path/target/plan/journal/effect authority,
  scheduler-runtime conversion, persistence write, or task-admission edge;
- scan- and cleanup-admission uncertainty remain separate, with a retained
  zero-wait local scan-admission plus cross-process writer witness clearing only
  the scan uncertainty;
- UniFFI remains v66, automation overview remains v3, and no new FFI method or
  record mentions the core runtime assessment;
- schema remains v22 and no automation runtime migration is added;
- every shipped rule remains unschedulable and execution remains unavailable;
- `AppRuntime` still constructs only
  `NoEnabledSchedulesDuxAutomationDecisionSource`; and
- documentation records this checkpoint as effect-dormant while leaving the M8
  scheduler parent open.

Focused Rust tests must cover the synthetic four-gate pass shape while proving
the production entry point can pass only the scan gate under its retained
admission witness and leaves cleanup unproven, plus closed-engine and contended-
registry behavior,
unavailable/privileged/unsupported identity, in-process and cross-process active
or unresolved scan work, in-process and cross-process active or unavailable
cleanup work, bounded overflow, query/storage failure, core-owned observation
time, observer-first and admission-first races, local admission and writer
contention, before/after-snapshot close and work-admission races, and unchanged
durable state.

## Deferred gates

Before the production source can become nonempty, DUX still needs separate
review and implementation of:

- a synchronous cross-process cleanup-admission witness;
- protected-descendant-complete exact-scope history;
- sealed exact-rule/scope current candidate, process, coverage, evaluation, and
  live-validation evidence;
- authoritative energy, battery, thermal, Low Power Mode, and sleep facts with
  documented conservative policy;
- authoritative low-disk episode identity and global re-enable rebasing;
- production core-owned deadline selection and occurrence CAS/claiming;
- fresh immutable planning, immediate revalidation, run-cap enforcement,
  serialized journal execution, and terminal settlement;
- bounded failure pausing and notifications; and
- execution-era crash tests and every remaining Release cleanup gate.

Passing this review proves only that core can conservatively observe four
runtime blockers. The Milestone 8 scheduler task remains open.

## Related decisions and plans

- [ADR 0014: Automation clock, wake, and missed-run semantics](../adr/0014-automation-clock-wake-and-missed-run-semantics.md)
- [ADR 0015: Automation activation and UTC recurrence](../adr/0015-automation-activation-and-utc-recurrence.md)
- [ADR 0016: Automation category-scope membership consent](../adr/0016-automation-category-scope-membership-consent.md)
- [M8 scheduler/wake review](m8-automation-scheduler-wake.md)
- [M8 activation-controls review](m8-automation-activation-controls.md)
- [M8 eligibility review](m8-automation-eligibility.md)
- [M8 scan-admission witness review](m8-automation-scan-admission-witness.md)
- [Milestone 8 roadmap](../../ROADMAP.md#milestone-8-automations)
- [Security design](../../SECURITY_DESIGN.md#10-automation)
