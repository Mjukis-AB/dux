# M8 Automation History Suggestions Review

Date: 2026-08-09

Status: implemented as a read-only advisory boundary. This review does not
approve schedule creation, enablement, wake handling, notifications, or
cleanup execution.

## Decision

DUX may show a bounded list of exact shipped rule revisions whose recent
manual cleanup history is repetitive enough to justify reviewing future
automation. A suggestion is presentation evidence only. It is not an
`AutomationManualHistoryEvidence` value, an eligibility result, a disabled
draft, a saved schedule, a plan, or an execution capability.

The implementation adds a separate `automation_schedule_suggestions` query.
It does not widen automation overview v2, so a busy, corrupt, incompatible, or
unavailable history query cannot hide otherwise valid disabled-draft state.
The query performs no durable write and schema remains v20.

## Source window and exact scope

The persistence query reads at most the newest 32 schema-v2 Manual,
PermanentSafe cleanup sessions, plus one ID used only to report that older
source sessions exist. Sessions are ordered by descending start time and
ascending stable session ID. Active, failed, rejected, interrupted, or
otherwise unresolved matching sessions remain attempts; they are not removed
from the newest-two decision.

Only exact current `RuleRef` values from the checksummed bundled catalog may
be considered. Current policy must still mark the rule schedule-eligible,
`SafeRegenerable`, and `RemoveKnownRegenerableContents`. The rule must use the
code-owned `UserCacheDirectory` scope and declare no protected-descendant
selectors. Selected-scan-root and configured-project-root observations are not
promoted into a broader rule schedule suggestion.

The private grouping key additionally contains the lossless source root and
its durable filesystem-identity digest. Different roots or identities never
combine. Neither value crosses the engine boundary. If multiple private scopes
for one exact `RuleRef` qualify, only the highest canonically ranked scope is
projected.

Each matching item must bind exactly to the complete source scan, successful
candidate evaluation, frozen candidate identity, category, path set, byte and
mtime observations, evidence, safety, action, and stored schedule-policy bit.
The source scan must be complete, snapshot-backed, identity-bound, and created
by the known user-cache scan scope before the cleanup plan. Missing or
unclassifiable facts fail closed.

## Suggestion predicate

One exact private rule/scope group is nominated only when:

1. at least two matching Manual attempts exist in the bounded window;
2. the newest two matching attempts are both complete successful permanent
   removals with no item or path error;
3. every matching item in each counted successful session retains exact source
   candidate binding; and
4. at least one successful Manual session has a compatible, explicit
   zero-to-nonzero `Regrown` observation before any superseding cleanup.

A rule counts at most once per session, even when the session contains several
matching items. CLI and future scheduled triggers never contribute to the
Manual counts. Prior rule revisions cannot satisfy a current revision.

The result is ordered by newest confirmed regrowth, successful Manual session
count, confirmed Manual regrowth-session count, and exact `RuleRef`. At most 12
unique rules cross the boundary. The feed separately reports the full bounded
qualifying-rule count, source-session count, and older-source sentinel so
truncation is visible.

## Authority separation

The public core and UniFFI records contain only record/derivation versions,
rank, exact rule ID and revision, bounded counts, observation timestamps, and
window/truncation counts. They contain no path, root identity, candidate ID,
scan ID, schedule or draft ID, cadence, limits, confirmation mode, plan,
approval, journal lease, filesystem witness, callback, task, trigger, effect,
or eligibility/runnable Boolean.

UniFFI v64 independently validates record versions, derivation revision,
source and result caps, full-versus-truncated counts, unique exact rule
references, contiguous ranks, minimum success/regrowth evidence, timestamps,
and canonical order. Swift repeats those checks before creating app-owned
immutable values.

No cleanup, planner, journal, scheduler, CLI, AI, app-runtime, or maintenance
component consumes a suggestion. No conversion from a suggestion into the
eight-gate automation eligibility kernel exists. Before any future production
eligibility assessment consumes durable history, a separate reviewed adapter
must still represent the newest attempts for the exact sealed schedule scope,
including protected-descendant observations, without treating this advisory
feed or the older storage-thief aggregate as authority.

## Native presentation

Settings loads suggestions in an independent cancellable, generation-fenced
state. Overview and disabled drafts remain visible when suggestion loading
fails, and a failed refresh retains the previous validated feed. Shutdown
cancels and drains both loads independently.

The UI labels the section **Ideas from manual cleanup history** and states that
the rows are ideas to review, not permission to schedule or run cleanup.
Refresh reads stored history only and starts no scan, draft creation, schedule,
or cleanup. Rows expose the exact rule revision, successful Manual run count,
confirmed regrowth count, and observation times. Empty, loading, error,
coverage, truncation, list, refresh, and ranked-row accessibility states are
distinct.

All currently shipped rules remain `schedule_eligible=false`, so the real
production feed is intentionally empty. Positive derivation is exercised with
injected test-only policy. No user action in this slice creates a disabled
draft.

## Verification

Focused core tests cover the positive two-success/one-regrowth predicate,
newest unresolved-attempt suppression, stale rule revision rejection,
current-catalog empty behavior, and byte-identical database evidence for the
read. Core and FFI projections test bounds, counts, order, time conversion,
error mapping, closed-engine behavior, and path-free response shape. Native
tests cover strict malformed-response rejection, independent load/failure
state, retained prior results, shutdown fencing, copy, truncation disclosure,
and accessibility identifiers. Repository architecture tests reject authority
fields and any suggestion consumer in the planner, executor, CLI, AI,
scheduler, runtime, or draft-mutation paths.

Final command counts, generated-binding hashes, and application artifact
evidence are recorded in the matching Milestone 8 roadmap checkpoint.

## Still excluded

- converting history into runtime eligibility evidence;
- creating or editing a disabled draft from a suggestion;
- choosing cadence, age, size, run cap, or notification policy;
- global or per-schedule enable, pause, delete, or kill-switch controls;
- an in-process scheduler, wake or missed-run handling, and test clock;
- scheduled planning, revalidation, execution, retry, or failure pausing; and
- any AI-derived history fact, safety decision, suggestion, or action.
