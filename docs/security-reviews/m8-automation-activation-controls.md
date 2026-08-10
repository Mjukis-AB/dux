# M8 Automation Activation Controls Security Review

Date: 2026-08-10

Status: accepted only for activation persistence and management. This review
does not approve a production due-schedule source, occurrence claiming,
notifications, planning, journaling, cleanup, or execution.

## Decision

ADR 0015 admits a dedicated default-off global control and a closed per-schedule
activation state: `Disabled`, `Enabled`, `Paused(User)`, or
`Paused(Failure)`. It also freezes UTC recurrence policy revision 1 and the
revision/CAS boundary needed for a later production source.

An enabled row remains effect-dormant. The production scheduler source is
statically empty, execution is unavailable, and every shipped rule remains
unschedulable. The checkpoint adds no edge from activation persistence to a
planner, cleanup journal, executor, platform effect, AI, CLI, notification, or
maintenance task.

## Persisted authority boundary

The global control is not inferred from the presence of schedules. The
migration seeds no row: absence reads as `enabled=false`, `Default`, revision
`0` without a write. Its first explicit enable, disable, or reset persists
revision `1`. Later mutations advance the revision, and disable or reset never
deletes an existing row, preventing revision ABA. A duplicated, unreadable, or
invalid persisted row fails closed.

Schedule state and recurrence cursor state are separately revisioned. A future
occurrence is bound to the exact schedule ID, schedule revision, cursor
revision, recurrence policy revision, and cadence identity. A stale schedule
or cursor revision is a refusal, not a reason to reinterpret the occurrence.
The persisted state contains no path, candidate, scan, plan, approval, cleanup
claim, task handle, callback, effect request, or retry capability.

Existing drafts remain disabled through migration. No read creates a global
row, activates a schedule, advances a cursor, or toggles the global control.

This checkpoint exposes bounded management operations through UniFFI and the
native client: global enable, disable, and reset, plus schedule enable,
user-pause, resume, disable, and delete. It exposes no due, assess, run, or
trigger operation. Native code presents results but does not derive recurrence
or choose eligibility.

## State-transition review

- Creation produces only `Disabled` state.
- Enabling a disabled calendar schedule uses core-owned time, creates a fresh
  UTC anchor and cursor, and makes the first due instant one complete interval
  later. Trusted core static preflight over the exact scope and pinned rule
  revisions must succeed first.
- A user pause changes the schedule revision and preserves anchor/cursor
  values. The `Failure` discriminator is reserved for a later trusted core
  failure path and cannot be selected by a client.
- Resume changes the schedule and cursor revisions and advances directly to
  the first due instant strictly after core-owned current time. It emits no
  paused-period occurrence and repeats the trusted static preflight.
- Disable ends active recurrence authority. Re-enable creates a new anchor
  rather than reviving an earlier activation.
- Configuration may be changed only while disabled and remains disabled.
  There is no edit-in-place of enabled or paused authority.
- Delete requires the exact reviewed revisions and cannot materialize work.

These transitions do not bypass the independent global control. While it is
disabled, every schedule is dormant and elapsed time is suppressed rather than
queued. Before a future production source may assess after global re-enable,
core must rebase all enabled calendar cursors to a position strictly after its
observation under the exact CAS boundary. The management operation in this
checkpoint persists intent only and cannot return an occurrence.

Static preflight is not current execution evidence: it refuses a missing,
changed, statically ineligible, or otherwise invalid scope/rule binding, but it
does not replace the deferred runtime, energy, manual-work, planning, or
pre-effect gates. Because every shipped rule remains
`schedule_eligible=false`, current schedule enable and resume attempts fail
closed.

## UTC recurrence review

Revision 1 treats weekly cadence as exactly seven UTC days from the original
anchor. Monthly cadence reuses the original anchor's UTC day and time in the
ordinal target month, clamping only that target month's day. Each ordinal is
calculated from the original anchor, so January 31 returns to March 31 after a
February clamp.

System time zone, daylight-saving transitions, and travel are presentation
facts only. They do not rewrite persisted authority or change a due instant.
Native code may display local time but may not derive or persist recurrence.
Invalid calendar values, arithmetic overflow, and unknown recurrence revisions
fail closed.

There is no catch-up backlog. Resume and global re-enable skip all elapsed
calendar positions. A future production assessment may publish at most the one
global revalidation request allowed by ADR 0014 and must arithmetically advance
cursors past omitted intervals rather than create one row or task per interval.

## Low-disk episode review

Low-disk activation is bound to the authoritative current pressure episode for
the startup volume, not a Boolean, UI value, notification, or caller timestamp.
Enable and resume must baseline that current episode as already observed so
pre-existing pressure cannot look newly triggered. Global re-enable must do the
same for enabled low-disk schedules before assessment.

This checkpoint has no admitted authoritative adapter from the durable disk
pressure episode state into automation activation. Low-disk enable and resume,
and a global re-enable that needs a low-disk baseline, therefore fail closed as
unavailable. Persistence cannot substitute a guessed episode or defer the
baseline until after assessment.

## Concurrency and commit ambiguity

All management writes use the existing writer guard, current-schema check,
immediate SQLite transaction, and exact compare-and-swap over each affected
global, schedule, and cursor revision. The full original graph is validated
before mutation. This is the multi-process serialization boundary; a native
actor or in-memory lock is not sufficient.

Commit-error reconciliation accepts only two proofs: the complete graph is
exactly expected, proving success, or exactly original, proving no commit. Any
third graph, missing or additional row, invalid field, read error, or lost
guard yields `OutcomeUnknown`. Unknown outcomes are fenced and never retried
automatically. The caller must reload before another proposal.

A future occurrence source must CAS the exact schedule-and-cursor tuple before
publishing an observation. A competing pause, resume, disable, edit, delete,
global rebase, or cursor advance invalidates the old tuple. This review does
not admit claims, leases, owner recovery, cleanup-session linkage, or outcome
settlement.

## Threat review

| Threat | Required refusal or containment |
| --- | --- |
| Migration accidentally activates a draft | Every migrated schedule remains `Disabled`; no global row is seeded and absence reads false/`Default`/revision `0`. |
| Default reset creates revision ABA | First explicit mutation writes revision `1`; later disable/reset updates rather than deletes and increments the revision. |
| Invalid global storage interpreted as enabled | Duplicated, malformed, or unreadable persisted state fails closed. |
| DST or travel changes cleanup time | UTC revision-1 due instants are invariant; only presentation changes. |
| Month-end cadence drifts | Every monthly ordinal derives from the original anchor with target-month clamp. |
| Resume creates a burst | Cursor advances directly to the first strictly future due instant; no elapsed occurrence is created. |
| Global enable creates catch-up work | All enabled cursors rebase before assessment; the toggle itself cannot publish due work. |
| Existing disk pressure looks new | Authoritative current episode must be baselined; transition is unavailable without the adapter. |
| Stale occurrence survives management | Exact schedule and cursor revisions make the tuple inadmissible after any competing transition. |
| Two processes mutate the same authority | Immediate transaction plus exact revision CAS admits only one state transition. |
| Commit result is ambiguous | Full-graph reconciliation returns `OutcomeUnknown` for any state other than exact expected or exact original; no blind retry. |
| Enabled state reaches cleanup | Production source stays statically empty and no claim/planner/journal/executor edge exists. |
| Client manufactures a failure pause | `Paused(Failure)` is reserved to a later trusted core path; client management can request only user pause. |
| Management API becomes a scheduler API | UniFFI/native exposes enable/pause/resume/disable/delete controls but no due/assess/run/trigger operation. |

## Execution remains unreachable

The checkpoint preserves independent barriers, any one of which prevents an
effect:

1. the dedicated global control reads default-off until explicitly changed;
2. the production automation source returns no enabled or due observation;
3. all shipped rules remain `schedule_eligible=false` and execution remains
   unavailable; and
4. no occurrence can be claimed or linked to a fresh plan, cleanup journal,
   executor, or platform effect.

Activation state is not current evidence and cannot serve as saved cleanup
approval. No scope/history adapter, wake event, pressure notification, draft,
suggestion, manual-maintenance task, ADR 0010 diagnostic, or ADR 0011 cleanup
crash debt can bridge these barriers. AI and CLI surfaces cannot consume or
mutate activation authority, and no notification action can trigger it.

## Deferred reviews

Before the production source can become nonempty, separate implementation and
security review must admit:

- sealed exact-scope history and authoritative current-evidence adapters;
- the low-disk episode adapter and production clock/wake assessment source;
- authoritative energy, battery, thermal, and active-manual-work gates;
- occurrence selection, CAS claim/lease, owner recovery, and cleanup-session
  linkage;
- fresh immutable planning, immediate pre-effect revalidation, run limits,
  serialized journal execution, and terminal settlement;
- operational failure classification, bounded retry/backoff, automatic
  `Paused(Failure)`, user recovery, and notifications; and
- production recurrence presentation and notification disclosure beyond the
  effect-dormant management controls in this checkpoint.

Stable-local-time recurrence, catch-up queues, planner or journal access, AI or
CLI automation, maintenance-scheduler reuse, and background helper/daemon
execution are not approved.

## Repository guardrails

Architecture and persistence tests must continue to prove:

- the migration is checksummed, preserves existing records as disabled, and
  seeds no global row;
- absent global state reads false/`Default`/revision `0` without writing, first
  explicit mutation writes revision `1`, and later disable/reset never deletes
  or reuses a revision;
- invalid state combinations, unknown recurrence revisions, timestamps,
  ordinals, and arithmetic overflow fail closed;
- weekly and month-end recurrence, leap years, no-drift behavior, strict-future
  resume/rebase, and time-zone independence are deterministic;
- low-disk transitions cannot use absent, Boolean-only, stale, or caller-owned
  episode evidence;
- enable/resume refuses failed exact static preflight and all currently shipped
  unschedulable rules;
- exact revisions and full-graph post-commit reconciliation cover every
  activation-management write and multi-process contention;
- the production source is the statically empty implementation;
- UniFFI/native management exposes global enable/disable/reset and schedule
  enable/pause/resume/disable/delete, but no due/assess/run/trigger operation;
- no automation timing or activation module imports path, candidate, plan,
  approval, cleanup, journal, executor, driver, AI, CLI, notification, or
  maintenance-task authority; and
- every shipped rule is unschedulable and execution remains unavailable.

Passing these guards proves only a dormant persistence and management boundary.
It does not qualify due assessment or cleanup execution.

## Related decisions and plans

- [ADR 0015: Automation activation and UTC recurrence](../adr/0015-automation-activation-and-utc-recurrence.md)
- [ADR 0014: Automation clock, wake, and missed-run semantics](../adr/0014-automation-clock-wake-and-missed-run-semantics.md)
- [ADR 0011: Diagnostic-only cleanup crash debt in v1](../adr/0011-diagnostic-only-cleanup-crash-debt-v1.md)
- [Milestone 8 roadmap](../../ROADMAP.md#milestone-8-automations)
- [Security design](../../SECURITY_DESIGN.md#10-automation)
