# ADR 0014: Automation clock, wake, and missed-run semantics

- **Status:** Accepted
- **Decision date:** 2026-08-09

## Context

Milestone 8 has admitted bounded disabled automation drafts, a path-free
eight-gate eligibility observation, and advisory suggestions derived from
manual cleanup history. None of those records is enabled or executable. Schema
v20 cannot store an enabled state, a next or previous run, a due occurrence, a
failure counter, or a scheduler claim. All shipped rules remain
`schedule_eligible=false`, and the global and execution gates remain false.

The roadmap nevertheless requires an in-process scheduler, durable next-run
state, wake handling with no missed-job backlog, current revalidation, energy
and manual-work gates, and clean app termination. A scheduler that derives
calendar policy in Swift, persists a monotonic instant, or treats a wake event
as execution authority would split policy from the Rust engine and could turn
clock manipulation or an event storm into repeated destructive work.

The product has not yet selected activation-schema semantics for weekly and
monthly recurrence. In particular, no accepted decision defines a calendar
anchor, time zone, daylight-saving transition behavior, month-end behavior,
pause/resume behavior, or the transaction relating a durable occurrence to a
cleanup journal. Those decisions are unnecessary for an effect-dormant timing
and lifecycle prerequisite and must not be guessed here.

## Decision

DUX will use two deliberately separate clock domains for automation:

- Rust core owns wall-clock validation, future calendar policy, durable due
  state, and selection of any due schedule. Any deadline supplied to native
  code is a pre-materialized, path-free core observation; Swift does not derive
  weekly or monthly recurrence.
- The native in-process scheduler may compare a core-provided wall observation
  and deadline to a fresh current wall sample, but it owns only monotonic
  waiting, bounded backoff, event coalescing, platform energy observation, and
  application lifecycle integration. The resulting `Task.sleep(for:)` delay is
  process-local, is never persisted, and never becomes schedule authority.

A core response pairs its validated wall observation time with the
pre-materialized wall deadline. Native code may derive only a bounded waiting
delay from that pair and arm it against its monotonic clock. Wake or significant
system-time change invalidates the wait as a timing optimization and requests a
fresh core assessment. It does not declare a schedule due, create a plan, or
authorize an effect. Wall-clock rollback cannot make native code independently
accelerate a run; a forward jump cannot manufacture one occurrence per elapsed
interval.

Launch, wake, and significant-time-change signals—and, once their authoritative
fact adapters exist, energy-change and low-disk signals—are coalesced into
bounded **assessment cohorts**. Each accepted cohort
may produce globally at most one path-free request for fresh revalidation.
Signals received while one assessment is admitted may produce at most one
later coalesced assessment. Neither the native actor nor the core assessment
may build a queue containing one entry per missed recurrence. This is the
Milestone 8 meaning of “at most one missed job” at this prerequisite boundary.

The single result is only a revalidation request. It is not a cleanup run,
candidate, scan, plan, approval, journal claim, task handle, callback, path, or
effect witness. Any later execution checkpoint must create a fresh immutable
plan from current facts and independently revalidate it before an effect.

Low-disk pressure may make a cohort eligible for earlier assessment, but it
does not alter rule policy, exact scope, current evidence, protected-path
handling, energy policy, manual-work exclusion, bytes-per-run limits,
confirmation policy, or journal requirements.

## Effect-dormant implementation boundary

The prerequisite checkpoint remains deliberately inert. A pure Rust decision
kernel may assess injected, pre-materialized deadlines, and a separate native
actor may exercise clocks, bounded coalescing, validation, and terminal
quiescence. Its production source must return no enabled or due schedule:

- schema v20 remains disabled-draft-only and stores no runtime timing state;
- all shipped rules remain unschedulable;
- `global_enabled` and `execution_available` remain false;
- no automation enable, run, execute, claim, or trigger method crosses UniFFI;
- no draft, eligibility assessment, or history suggestion enters a scheduler;
- no automation source is added to the maintenance scheduler rotation;
- no CLI, AI, planner, cleanup, journal, notification, or filesystem consumer
  is added; and
- the production source of enabled/due schedules is empty.

Native timing code is a separate in-process owner in the persistent menu-bar
application. Its current source protocol may accept an injected `observed_at`
only for deterministic tests and the statically empty source; that parameter is
not trusted production authority. A future UniFFI assessment endpoint must
accept only a bounded reason/cohort signal, sample and validate `observed_at`
inside core, and return its own observation time with any pre-materialized
deadline. Production cannot materialize a revalidation request until that core
source and an activation schema are separately accepted. No launch agent,
daemon, helper, privileged process, or background CLI scheduler is authorized.

Wake, sleep, and quit remain lifecycle events rather than authority. Runtime
integration must generation-fence late observations, reject admission after
its terminal fence, request cancellation, and join its retained driver before
engine close. Cancellation requested is not proof that admitted work is
quiescent. If a later checkpoint admits an effect, the existing cleanup
journal and outcome-unknown rules remain authoritative.

## Explicitly deferred decisions

The activation-schema ADR and security review must decide all of the following
before a schedule can be enabled or a due instant can be persisted:

- weekly and monthly recurrence anchors and time-of-day semantics;
- time-zone capture, travel or time-zone edits, daylight-saving gaps and
  repetitions, and month-end behavior;
- how create, edit, enable, pause, resume, failure, confirmation, and deletion
  change recurrence state;
- durable next-run representation, recurrence identity, revision binding,
  compare-and-swap behavior, and multi-process exclusion;
- how a selected or skipped overdue occurrence advances without accumulating a
  backlog;
- the crash-consistent relationship among a due occurrence, revalidation,
  fresh plan, cleanup session, trigger source, and terminal result; and
- conservative battery and repeated-failure thresholds.

The “one revalidation request per cohort” rule does not answer those durable
calendar questions and must not be cited as authority to invent them.

## Authority and threat boundaries

Automation timing must fail closed against:

- wall-clock rollback, large forward jumps, malformed or future stored times,
  wake storms, and repeated platform notifications;
- stale or superseded draft revisions, rule revisions, global state, or
  current facts;
- energy facts that are missing or change while an asynchronous assessment is
  in progress;
- active manual scans or competing manual cleanup;
- pause, delete, global-disable, sleep, or quit racing an admitted assessment;
  and
- late callback or task completion after a newer generation or terminal fence.

The scheduler may never consume the ADR 0010 active-cleanup diagnostic or use
ADR 0011 cleanup crash debt as a selector, retry source, or recovery authority.
It may not use AI, arbitrary paths, Trash-emptying, advanced overrides, remote
unsigned rules, or iCloud eviction.

## Consequences

The clock split keeps portable due policy and durable state in the shared Rust
engine while allowing native code to respond efficiently to macOS lifecycle
and power events. Cohort coalescing provides a deterministic, testable ceiling
before destructive authority exists and prevents wake or clock events from
creating a backlog.

This decision intentionally does not make an automation useful to users. It
adds no durable recurrence, enabled schedule, revalidation adapter, planner,
executor, notification, or effect. A later checkpoint must extend or
supersede this ADR where it selects the deferred calendar and transactional
semantics.

## Alternatives considered

- **Persist native monotonic deadlines.** Rejected because process-uptime
  instants are not meaningful after restart and are not calendar authority.
- **Let Swift compute weekly/monthly deadlines.** Rejected because cadence and
  missed-run policy belong to the shared Rust engine and would diverge across
  clients.
- **Queue every elapsed recurrence after wake.** Rejected because it violates
  the no-backlog requirement and turns sleep or clock changes into a work
  multiplier.
- **Reuse the maintenance scheduler rotation.** Rejected because maintenance
  tasks carry no saved consent, fresh-plan, failure-pausing, or cleanup-journal
  semantics.
- **Select timezone and DST behavior now.** Rejected because schema v20 has no
  activation state and this prerequisite has no need to invent product-visible
  recurrence semantics.
- **Add a launch agent for reliable cadence.** Rejected by ADR 0001 and the
  roadmap's initial in-process boundary.

## Validation

This decision is preserved while repository architecture tests prove:

- schema v20 remains disabled-only and cannot persist next/last-run state;
- all shipped rules remain unschedulable and both runtime gates remain false;
- UniFFI exposes no automation enable/run/execute/trigger operation;
- automation timing or scheduler sources are discovered and remain free of
  drafts, suggestions, paths, target/plan/journal/effect authority, maintenance
  kinds, and planner/executor calls;
- the ordinary maintenance rotation contains no automation kind;
- CLI, AI, cleanup, planner, runtime maintenance, and recovery diagnostics do
  not consume automation drafts or suggestions; and
- documentation does not claim that activation-capable scheduling, persisted
  recurrence, production wake revalidation, or cleanup execution is
  implemented.

Future implementation requires deterministic injected-clock tests for clock
rollback/forward, wake and signal coalescing, one revalidation request per
cohort, bounded follow-up, energy/manual-work deferral, stale generations,
sleep, and terminal quiescence before its production source can become nonempty.

## Reconsideration triggers

Write a new ADR or explicit extension before adding activation or persistence
semantics, choosing calendar/time-zone behavior, returning more than one
revalidation request per cohort, adding another process, or permitting a
native scheduler to derive recurrence. Any scheduler-to-planner, journal, or
executor edge requires a new security review and architecture tests rather
than an incidental extension of this prerequisite.

## Related decisions and plans

- [ADR 0001: Native SwiftUI macOS application](0001-native-swiftui-macos-application.md)
- [ADR 0003: Primary build without App Sandbox](0003-primary-build-without-app-sandbox.md)
- [ADR 0004: Shared Rust engine](0004-shared-rust-engine.md)
- [ADR 0005: UniFFI for the Swift/Rust transport](0005-uniffi-swift-rust-transport.md)
- [ADR 0010: Read-only active-cleanup provenance diagnostic](0010-read-only-active-cleanup-provenance-diagnostic.md)
- [ADR 0011: Diagnostic-only cleanup crash debt in v1](0011-diagnostic-only-cleanup-crash-debt-v1.md)
- [Milestone 8 roadmap](../../ROADMAP.md#milestone-8-automations)
- [Security design](../../SECURITY_DESIGN.md#10-automation)
- [M8 scheduler/wake security review](../security-reviews/m8-automation-scheduler-wake.md)
