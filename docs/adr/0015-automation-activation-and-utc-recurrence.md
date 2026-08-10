# ADR 0015: Automation activation and UTC recurrence

- **Status:** Accepted
- **Decision date:** 2026-08-10

## Context

ADR 0014 separated durable wall-clock policy in Rust from process-local
monotonic waiting in the native application. It intentionally deferred the
calendar and lifecycle choices needed to persist an activated schedule. The
existing automation foundation stores only disabled drafts, while the
production scheduler source is statically empty and every shipped cleanup rule
is unschedulable.

Milestone 8 now needs a durable activation and management model before a later
checkpoint can implement a production due-schedule source. That model must not
turn a preference, stale cursor, clock jump, resume, or master-switch change
into cleanup authority. It must also preserve the repository's established
multi-process compare-and-swap and commit-reconciliation guarantees.

This decision admits activation persistence and management only. It does not
admit occurrence claiming, planning, journaling, cleanup, failure handling,
notifications, or a production scheduler source.

## Decision

### Authority and state

Automation has a dedicated logical global control. Before its first explicit
mutation, an absent row reads as `enabled=false`, `Default`, revision `0`
without writing. The activation migration deliberately seeds no global row.
The first explicit enable, disable, or reset persists revision `1`; every later
mutation increments that revision. Disable and reset update an existing row
rather than deleting it, so returning to the default-disabled value cannot
create revision ABA. A duplicated, malformed, or unreadable persisted row
fails closed.

Each schedule has exactly one closed state:

- `Disabled` has configuration but no active recurrence authority;
- `Enabled` has an active, revisioned recurrence cursor; or
- `Paused(User)` or `Paused(Failure)` has no due authority while retaining the
  recurrence anchor and cursor needed for an explicit resume.

`Paused(Failure)` is reserved for a later trusted core failure transition. This
checkpoint stores the closed discriminator but does not implement failure
classification, retries, counters, automatic pausing, or notifications. A
client cannot nominate `Failure` as a user pause reason.

The global control and per-schedule state are independent defenses. Global
disable suppresses every assessment without rewriting enabled schedules, and
an enabled schedule does not override the global control, static rule
eligibility, execution availability, or current-evidence gates.

### Recurrence policy revision 1

Recurrence policy revision 1 is UTC-only. Rust core samples the activation
instant, validates it as a Unix-millisecond wall time, and records the original
UTC anchor. Native code never creates or changes that anchor.

- A weekly due instant is the original anchor plus `n * 7 * 24` hours for a
  positive occurrence ordinal `n`, using checked arithmetic.
- A monthly due instant has the original anchor's UTC time of day in the
  `n`th later UTC calendar month. Its day is the lesser of the original anchor
  day and the number of days in that target month.
- Monthly calculations always start from the original anchor and ordinal, not
  the previously clamped due instant. Thus a January 31 anchor becomes
  February 28 or 29 and then March 31; it never drifts permanently to the 28th
  or 29th.
- An overflow, invalid ordinal, malformed stored value, or unrepresentable UTC
  calendar instant fails closed. It is never wrapped, saturated into a due
  instant, or recomputed by native code.

Time zone, daylight-saving transitions, and travel affect presentation only.
Changing the system time zone cannot move the durable UTC anchor or due
instant. The displayed local hour may therefore change across a daylight-saving
transition or travel. Stable-local-time recurrence would be a different policy
revision and requires a new decision and migration; it must not reinterpret an
existing revision-1 schedule.

Low-disk-only cadence is episode-based rather than calendar-based. It has no
weekly or monthly deadline, and a Boolean pressure flag or caller timestamp is
not sufficient episode identity.

### Lifecycle transitions

All transition timestamps and calendar calculations come from core-owned wall
time. Enable and resume also run the trusted Rust static eligibility preflight
against the exact saved scope and pinned rule revisions before changing state.
Missing, changed, ineligible, or otherwise invalid static evidence refuses the
transition. A native view of eligibility is not authority. The admitted
transitions are:

| Operation | Required prior state | Result |
| --- | --- | --- |
| Create | None | Creates `Disabled`; no active cursor or due occurrence. |
| Enable | `Disabled` | Creates a new activation anchor and cursor. Weekly or monthly first becomes due after one complete interval, never immediately. |
| Pause | `Enabled` | Creates `Paused(User)` for user management, or later `Paused(Failure)` for a trusted failure path; preserves anchor and cursor values while revision changes invalidate stale observations. |
| Resume | Either paused state | Returns to `Enabled` and advances the cursor to the first due instant strictly after the core observation time. No paused interval is emitted as catch-up work. Resuming a failure pause requires a later reviewed failure-management path. |
| Disable | `Enabled` or either paused state | Ends the activation and removes active cursor authority while retaining editable configuration. |
| Edit configuration | `Disabled` only | Replaces configuration under exact revision and remains `Disabled`. Enabled or paused configuration is never edited in place. |
| Delete | Any state | Deletes only under the exact reviewed revision tuple. It does not create an occurrence or authorize cleanup. |

Re-enabling a disabled schedule creates a new anchor; it does not revive an
older activation. Pause is the only operation that preserves an activation's
anchor. If a due instant equals the resume observation, it is elapsed and the
cursor advances beyond it because “strictly after” means `due > observed_at`.

No transition constructs one record per elapsed interval. Resume, re-enable,
clock-forward handling, and future due assessment advance arithmetically to a
future cursor. ADR 0014's global ceiling of at most one revalidation request
per assessment cohort remains in force.

### Cursor and occurrence identity

Schedule configuration and state have a schedule revision. Active or preserved
recurrence position has a separate cursor revision. Every cursor mutation,
including rebase or advancement, changes that revision even when an anchor is
preserved.

Any future due observation must identify an occurrence by the exact tuple of
schedule ID, schedule revision, cursor revision, recurrence policy revision,
and the cursor's weekly/monthly ordinal or authoritative low-disk episode
identity. The schedule-and-cursor revisions are mandatory occurrence identity,
not advisory metadata. A pause, disable, edit, delete, resume, global rebase, or
competing cursor advance makes an older tuple stale and inadmissible.

This checkpoint does not create a claim table, an execution lease, a cleanup
session link, or a terminal occurrence result. A future production source must
atomically compare and advance the exact tuple before publishing at most one
path-free revalidation observation. It must skip all other elapsed intervals
rather than queue a backlog. An occurrence tuple is not a plan, approval,
journal claim, cleanup task, effect request, or retry capability.

### Global re-enable and low-disk baselines

While the global control is disabled, elapsed calendar intervals and pressure
episodes are suppressed rather than accumulated. Before a future production
source may assess schedules after the control changes from disabled to
enabled, core must first, under the reviewed CAS boundary:

- rebase every enabled weekly or monthly cursor to its first due instant
  strictly after the core observation time; and
- baseline every enabled low-disk-only schedule against the authoritative
  current startup-volume pressure episode.

Only after that rebase commits may a production source assess schedules. The
management operation exposed by this checkpoint persists global intent but
cannot assess or return a due occurrence. A later production source must not
consume that enabled value without completing the rebase, so time spent
globally disabled cannot become immediate catch-up work.

Enabling or resuming a low-disk-only schedule likewise requires the
authoritative current episode identity and records it as already observed. It
must not trigger for pressure that predates the activation or resume. The
authoritative episode adapter is not part of this checkpoint, so those
transitions—and global re-enable when it must baseline an enabled low-disk
schedule—remain unavailable rather than guessing from a Boolean, timestamp, UI
state, or notification.

### Transactions and uncertain outcomes

Every activation or management write uses the persistence writer guard,
requires the current schema, opens an immediate SQLite transaction, validates
the complete original graph, and compares every affected global, schedule, and
cursor revision. Multi-process writers therefore contend on the same durable
CAS boundary rather than on process-local state.

After a commit error, reconciliation reads the complete affected graph:

- the exact expected graph proves success;
- the exact original graph proves that the mutation did not commit; and
- any third state, missing or extra row, unreadable state, invalid guard, or
  reconciliation failure returns `OutcomeUnknown`.

An outcome-unknown mutation is fenced and is never retried automatically.
Callers must refresh the complete graph before proposing another mutation.
Delete, global control changes, schedule transitions, configuration edits, and
cursor rebases all use this rule; none receives a last-writer-wins exception.

## Effect-dormant implementation boundary

The activation checkpoint stops at persistence and management:

- UniFFI and native Settings may expose global enable, disable, and reset plus
  schedule enable, user-pause, resume, disable, and delete management calls;
- every enable or resume still passes the core static preflight, which rejects
  all currently shipped unschedulable rules, and low-disk enable/resume remains
  unavailable without its authoritative episode adapter;
- the production automation decision source remains statically empty;
- all shipped rules remain `schedule_eligible=false` and execution remains
  unavailable;
- no production clock, wake, energy, manual-work, history, scope, or current
  fact adapter consumes an enabled row;
- no occurrence claim or cleanup-session linkage exists;
- no planner, cleanup journal, executor, platform effect driver, AI, CLI,
  notification, maintenance rotation, or recovery diagnostic consumes the new
  state; and
- UniFFI and native code expose no due, assess, run, or trigger operation;
  native code does not derive recurrence or receive an executable occurrence.

An enabled row is durable consent state for later review, not proof that a
target is currently safe and not authority to execute. The eight automation
gates, fresh immutable planning, immediate pre-effect revalidation, bounded
run limits, serialized cleanup journal, and terminal result handling remain
unimplemented and unreachable.

## Consequences

UTC revision 1 is deterministic across restart, time-zone change, travel, and
platform implementations. Original-anchor monthly calculation avoids calendar
drift, and explicit cursor revisions make stale due observations rejectable.
Users may see the local display hour shift, which the UI must disclose before
activation controls ship.

Pause and global disable cannot create a restart backlog, and authoritative
low-disk baselining prevents an already-active pressure episode from becoming
new work. The cost is that low-disk activation and resumption cannot ship until
the episode adapter exists, and stable-local-time recurrence is deferred.

Persisting enabled state before a production source is safe only while the
negative architecture barriers remain pinned. This checkpoint is therefore
not user-visible automation completion.

## Alternatives considered

- **Stable local wall time with a named time zone.** Deferred because gap,
  repetition, travel, and time-zone database upgrade rules would add a second
  policy before a production source exists.
- **Compute monthly dates from the previous due date.** Rejected because
  month-end clamping would permanently drift an original 29th, 30th, or 31st
  anchor.
- **Treat resume or global enable as immediately due.** Rejected because it
  converts a safety control into a catch-up trigger.
- **Store a low-disk Boolean or notification timestamp.** Rejected because it
  cannot distinguish a new authoritative pressure episode from a stale or
  duplicated signal.
- **Use one revision for schedule and cursor state.** Rejected because cursor
  advancement and configuration review have different contention and stale
  observation boundaries; an occurrence must bind both exactly.
- **Automatically retry an uncertain commit.** Rejected because an ambiguous
  activation or cursor advance could be applied twice or resurrect stale
  authority.

## Validation

The implementation and repository architecture tests must prove:

- migration seeds no global row, reads its absence as false/`Default`/revision
  `0` without writing, and does not activate any existing draft;
- first global mutation writes revision `1`, later disable/reset never deletes
  the row, and every subsequent mutation advances revision without ABA;
- the closed schedule and pause-reason states reject unknown values and invalid
  field combinations;
- UTC weekly arithmetic, leap years, month-end clamp/no-drift, checked
  overflow, and time-zone independence are deterministic;
- pause preserves its anchor/cursor, while resume and global re-enable advance
  to a strictly future position without materializing elapsed intervals;
- low-disk activation/resume/re-enable refuses missing or stale authoritative
  episode evidence;
- schedule enable/resume requires exact trusted static preflight, while the
  management FFI/native surface exposes no due/assess/run/trigger operation;
- every mutation requires exact schedule, cursor, and global revisions as
  applicable, and commit ambiguity follows exact expected/original/full-graph
  reconciliation;
- concurrent process writers can admit only one exact CAS result; and
- production still has an empty scheduler source, unschedulable shipped rules,
  unavailable execution, and no planner/journal/executor/AI/CLI/notification
  consumer.

## Reconsideration triggers

Write a new ADR or explicit superseding extension before adding stable-local
recurrence, changing revision-1 arithmetic, permitting catch-up work, deriving
calendar policy outside Rust, accepting caller time as authority, or changing
the occurrence identity/CAS rules.

A separate security review and architecture guards are required before making
the production source nonempty, admitting current-evidence or energy/manual
work adapters, claiming an occurrence, linking it to a cleanup session,
handling execution failures, notifying a user, or reaching a planner, journal,
executor, CLI, AI, or platform effect.

## Related decisions and plans

- [ADR 0014: Automation clock, wake, and missed-run semantics](0014-automation-clock-wake-and-missed-run-semantics.md)
- [ADR 0011: Diagnostic-only cleanup crash debt in v1](0011-diagnostic-only-cleanup-crash-debt-v1.md)
- [Milestone 8 roadmap](../../ROADMAP.md#milestone-8-automations)
- [Security design](../../SECURITY_DESIGN.md#10-automation)
- [M8 activation-controls security review](../security-reviews/m8-automation-activation-controls.md)
