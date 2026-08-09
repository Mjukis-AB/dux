# M8 Automation Scheduler and Wake Prerequisite Review

Date: 2026-08-09

Status: accepted as an effect-dormant clock, cohort, and architecture boundary.
This review does not approve activation state, persisted recurrence, an
activation-capable scheduler source, runtime revalidation, notifications, or
cleanup execution.

## Decision

ADR 0014 freezes the minimum timing boundary for an effect-dormant
implementation. Core owns validated wall-clock and future calendar policy and
will pre-materialize any path-free deadline supplied to native code. Native
code may sample current wall time only to validate and derive a bounded delay
from that observation; waiting and bounded backoff use `Task.sleep(for:)`.
Those delays never persist and never become schedule authority.

Launch, wake, and significant-time-change events—and, once their authoritative
fact adapters exist, energy-change and low-disk events—form bounded assessment
cohorts. Each cohort may yield globally at most
one path-free request for fresh revalidation. Events received during an
assessment may cause at most one later coalesced assessment. No layer creates
one job per elapsed recurrence.

The result is a revalidation request, not a cleanup run or authority-bearing
object. A future execution review must still prove all eight current
eligibility gates, build a fresh immutable plan, perform immediate pre-effect
revalidation, and enter the existing durable journal protocol.

## Current production state

This checkpoint changes no durable schema or cleanup authority:

- schema v20 stores only revisioned `disabled_draft` preferences;
- no enabled, paused, next-run, last-run, due-occurrence, claim, or failure
  state exists;
- `global_enabled` and `execution_available` remain false;
- all shipped catalog rules remain `schedule_eligible=false`;
- UniFFI v64 has only overview, suggestion, and disabled-draft mutation calls;
- no app runtime or maintenance scheduler consumes a draft, eligibility
  assessment, or suggestion; and
- the production source of enabled or due schedules is empty.

A pure Rust decision kernel accepts only injected path-free schedule, runtime,
and already-materialized clock observations and returns at most one
revalidation observation. A separate native decision actor owns event
coalescing, validated wall-deadline comparison, `Task.sleep(for:)` waiting,
bounded retry, generation fencing, and terminal joining. `AppRuntime` constructs
it only with `NoEnabledSchedulesDuxAutomationDecisionSource`, whose result has
no due observation or future deadline. Neither implementation calls UniFFI,
the engine, a planner, or an executor.

The native source protocol's current `at:` parameter supports deterministic
tests and the inert source only. A future UniFFI endpoint must not trust caller
wall time: it accepts a bounded reason/cohort signal, samples `observed_at` in
core, and returns that core-owned observation with any pre-materialized
deadline.

## Clock boundary

The future production core assessment is the only place that may interpret a
durable wall deadline as due. Its response pairs its validated wall observation
time with the pre-materialized wall deadline. A native scheduler may compare
that pair with a fresh wall sample to derive a bounded delay and arm
`Task.sleep(for:)`; it cannot calculate weekly/monthly recurrence, persist the
delay, or use a wake timestamp as authority.

Wake and significant-time-change invalidate the wait optimization and request
a fresh assessment. Clock rollback therefore cannot make native code start
early, while clock-forward cannot expand into one request per skipped interval.
Malformed, future, unavailable, or inconsistent durable time must fail closed
in the later core implementation.

No time-zone, daylight-saving, calendar anchor, month-end, pause/resume, or
recurrence-advance semantics are selected here. They belong to the activation
schema because schema v20 cannot express or validate them.

## Wake and missed-run boundary

An assessment cohort has one admission generation. Multiple wake or time
signals before its result settles merge into that generation; additional
signals can retain only one pending follow-up. The core response is bounded to
zero or one revalidation request across all schedules for that cohort.

This global ceiling prevents a wake storm, long sleep, or large clock jump from
becoming a backlog. It does not decide which future activated schedule wins,
whether another overdue occurrence is skipped, or how its recurrence advances.
Those are durable activation-schema decisions. No implementation may retain an
in-memory queue as a substitute for that missing policy.

A wake request never bypasses startup delay, global disable, per-schedule
pause, failure state, energy gates, active manual work, current evidence, or
fresh revalidation. Low disk may request an earlier assessment but cannot
weaken any of those checks.

## Energy, manual work, sleep, and quit

Before a production source becomes nonempty, a separate implementation review
must prove:

- missing power facts and unknown thermal states fail closed;
- Low Power Mode, serious/critical thermal state, and battery below a
  documented conservative threshold defer automation;
- late energy results recheck the scheduler generation;
- the core, rather than UI render state, establishes whether a manual scan or
  competing manual cleanup is active;
- sleep cancels or defers pre-effect work and preserves journal truth for any
  already-admitted effect;
- quit installs a terminal admission fence before suspension, invalidates
  waits and result publication, requests cancellation, and joins retained
  work before review release and engine close; and
- cancellation requested remains distinct from execution quiesced.

Energy denial, active manual work, sleep, and ordinary quit are deferrals, not
evidence that a target became safe or that an occurrence succeeded.

## Authority separation

Future automation timing and scheduler sources must not import, construct, or
call:

- disabled drafts or advisory history suggestions;
- arbitrary paths, candidate or scan IDs, cleanup plans, approvals, journal
  claims, effect requests, platform drivers, or cleanup task handles;
- planner, executor, Trash, iCloud eviction, AI, CLI, notification, or remote
  rule operations;
- the maintenance scheduler kind/rotation as an automation transport; or
- the ADR 0010 cleanup-recovery diagnostic or ADR 0011 retained crash debt.

The timing boundary may use only path-free, revisioned observations. A later
adapter must be reviewed before an activated schedule, exact-scope history,
current fact, plan, or effect capability can enter its graph.

## Threat review

| Threat | Required refusal |
| --- | --- |
| Wake or notification storm | Coalesce to one assessment plus at most one pending follow-up. |
| Long sleep or clock-forward | At most one global revalidation request; never one job per recurrence. |
| Clock rollback | Native monotonic wait cannot reinterpret durable due state; request a fresh core assessment. |
| Persisted uptime | Schema and architecture tests reject next/last-run state in v20; monotonic values remain native and process-local. |
| Draft or suggestion confused with consent | Scheduler sources may not reference either record family. |
| Low disk used as override | Priority only; all safety and current-evidence gates remain unchanged. |
| Maintenance task used as cleanup transport | Automation is absent from the maintenance rotation and task API. |
| Late result after sleep/quit | Generation and terminal fences reject publication or new admission. |
| Unresolved cleanup debt reused | ADR 0010/0011 records remain diagnostic-only and non-executable. |
| Calendar policy invented in Swift | Core supplies a pre-materialized deadline; recurrence remains deferred. |

## Repository guardrails

`scripts/tests/test_automation_schedule_boundary.py` continues to pin the v64
path-free disabled-only contract, the exact five automation registry methods,
schema v20, and the all-unschedulable shipped catalog. It additionally:

- discovers core and native source files whose names describe automation
  timing, clocks, wake handling, or scheduling;
- rejects target/effect authority, draft/suggestion consumption, maintenance
  kinds, and planner/executor calls in those files;
- rejects an automation maintenance kind or rotation entry; and
- preserves the absence of FFI enable/run/execute/trigger methods.

These are negative architecture tests. Passing them proves absence of the
forbidden graph, not correctness of a scheduler.

## Deferred gates

Before enabled schedules or durable due state, DUX still needs:

1. an accepted activation-schema ADR covering calendar/time-zone/recurrence
   and crash-consistent occurrence state;
2. persisted global kill, enable, pause, delete, and revision-CAS controls;
3. sealed exact-scope history and current-fact adapters;
4. the production core-owned deadline source and complete injected-clock
   qualification;
5. authoritative manual-work and complete energy gates;
6. fresh plan/revalidation plus serialized journaled execution;
7. bounded failure pausing and notification semantics; and
8. the remaining public-Release permanent-cleanup gates.

The Milestone 8 scheduler task remains open.

## Verification

The focused repository test must pass together with native-runtime layering,
cleanup-recovery diagnostic, and permanent-cleanup consent boundaries. Final
command counts belong in the implementing checkpoint report; this review does
not claim UniFFI, activation-state, cleanup, or full application build coverage.
