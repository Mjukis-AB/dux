# M8 Automation Schedule Authoring Controls Security Review

Date: 2026-08-10

Status: accepted only for effect-dormant schedule creation and disabled-state
editing. This review does not approve a production due source, occurrence
claiming, notifications, planning, journaling, cleanup, or execution.

## Decision

DUX may expose a native editor for the already-versioned schema-v21 schedule
configuration. Creation begins only from an explicit user action on one
path-free manual-history suggestion and therefore carries that suggestion's
exact current `RuleRef`. There is no generic category picker, free-form rule
identifier, arbitrary path, AI-created scope, or automatic suggestion-to-draft
conversion.

The user must first choose **Review disabled schedule…**, review the complete disabled
configuration, and then choose **Save disabled schedule**. Reading or displaying
a suggestion still creates nothing. Saving calls the existing Rust-owned
create operation, which generates the opaque schedule ID and returns a complete
post-mutation overview. All currently shipped rules remain unschedulable, so
the production suggestion feed is empty and no new schedule can presently be
seeded through this path.

An existing schedule may be edited only while its exact reviewed revision is
`Disabled`. Editing preserves its scope, exclusions, notification preference,
confirmation mode, and every unedited field exactly. A category-scoped legacy
or previously stored schedule therefore remains editable without allowing the
native client to invent a category authoring catalog. Enabled or paused
configuration must first be explicitly disabled through the separately
reviewed state transition. The four preserved policy fields are immutable in
the editor draft, displayed as read-only reviewed facts, and compared with the
original editor session again at Save; replacing the draft with another scope,
exclusion set, notification preference, or confirmation mode is rejected before
FFI.

## Editable values and exact arithmetic

The form exposes cadence, minimum candidate age, minimum reclaimable size, and
maximum bytes per run. The native draft is not trusted merely because a Save
button was enabled: submission reparses and validates every field before the
FFI call, and Rust validates the complete configuration again.

- age is an exact whole count of seconds, hours, or days and is converted with
  checked integer multiplication into `0...3_153_600_000` seconds;
- minimum reclaimable size uses an exact locale-aware GiB decimal parser and
  must map into `0...i64::MAX` bytes;
- maximum bytes per run uses the same exact parser and must map into
  `1...i64::MAX` bytes;
- no `Double`, binary floating-point rounding, grouping separator, negative
  value, exponent, unit guess, overflow, or silent clamp is admitted; and
- minimum reclaimable size may exceed the per-run cap. The former gates whether
  a scope is worth considering while the latter limits work; inventing an
  ordering constraint would change the core policy.

When editing, the age field selects the largest whole unit that reproduces the
stored seconds exactly. Exact GiB formatting round-trips every admitted byte
value. Opening and saving a form without a user change therefore cannot alter
the stored configuration through presentation rounding.

## Revision and outcome boundary

Replacement binds the schedule ID and exact revision visible when the editor
opened. A refresh or competing writer cannot silently rebase that draft onto a
new revision. A mismatch returns a revision conflict, retains the user's input,
fences further mutation, and requires a complete refresh plus explicit
re-review. Creation is bounded by the existing 64-schedule core limit and a
limit refusal creates no row.

Every write still uses the persistence writer guard, current-schema check,
immediate transaction, exact compare-and-swap, and complete-graph commit-error
reconciliation from ADR 0015. UniFFI additionally correlates a changed core
mutation with the complete refreshed overview:

- create, replace, and state transitions must contain the exact returned
  schedule;
- global-control mutations must contain the exact returned control; and
- delete must prove the exact schedule is absent.

If a changed write cannot be followed by a readable, valid, and correlating
complete overview, UniFFI returns `OutcomeUnknown` regardless of the narrower
post-write read or projection error. Native Settings preserves its last
confirmed overview, retains the editor, fences every later mutation, and
requires refresh. It never retries an unknown create, because doing so could
create a second core-generated schedule. A mutation that returns through UniFFI
but fails native projection is equally treated as `OutcomeUnknown`, because the
write may already have committed. Missing, newly active, conflicted, and
uncertain edit targets cannot be resubmitted: refresh leads to an explicit
re-review of the current disabled revision or to closing the stale form.

## Suggestion boundary

The history suggestion remains a bounded, path-free observation. The new
button is a user-mediated presentation edge into an inert draft editor, not an
authority edge into scheduling or cleanup. The editor receives only the exact
rule ID and revision already present in the suggestion; it receives no private
root identity, path, candidate, scan, plan, approval, runtime fact, or effect
capability.

Core re-applies current shipped static policy after create and on every future
enable/resume attempt. A stale, removed, revised, unsafe, or unschedulable rule
therefore produces a blocked saved configuration rather than executable work.
AI cannot populate the editor, invoke create/replace, or become a suggestion
source.

## Notification counter on edit

Every changed full-configuration replacement starts the saved pre-run notice
preference at its configured beginning: three notices when enabled and zero
when disabled. An exact no-op replacement changes neither revision nor the
counter. This reset is conservative because an edited cadence or limit is a
new configuration the user has not yet observed running. This checkpoint has
no notification delivery or execution consumer, so the counter remains inert.

## Threat review

| Threat | Required refusal or containment |
| --- | --- |
| A suggestion silently creates a schedule | Display and refresh are read-only; creation needs Review then explicit Save. |
| Native invents a cleanup scope | New creation accepts only the exact `RuleRef` from a validated history suggestion; there is no generic/free-form scope picker. |
| A caller substitutes another draft after review | Scope, exclusions, notification preference, and confirmation mode are structurally retained and rechecked against the exact editor session at Save. |
| AI converts an explanation into a schedule | No AI model, cache, adapter, or result reaches the editor or create/replace API. |
| Unit conversion changes a limit | Integer age arithmetic and exact decimal-to-byte parsing round-trip or reject; no floating-point conversion or clamp exists. |
| Edit resets hidden policy | The complete stored scope, exclusions, notification flag, and confirmation mode are preserved unless a future reviewed control explicitly changes them. |
| A paused/enabled schedule is edited in place | Native and Rust both require `Disabled`; replacement uses the exact reviewed revision. |
| A refresh silently rebases an open editor | The editor retains its original revision; conflict requires refresh and a new explicit review. |
| Unknown create is retried | Outcome-unknown fences all mutations and retains the form; no automatic retry occurs. |
| A successful write is followed by a misleading old overview | UniFFI correlates the exact returned mutation object with the complete refreshed graph or reports outcome-unknown. |
| Native cannot project a returned post-write response | The result is outcome-unknown, the editor is fenced, and create is never retried. |
| Minimum size exceeds run cap | The pair remains valid by design; later execution must enforce the cap before and during work. |
| Saved configuration reaches cleanup | Production due source stays empty, execution stays unavailable, all shipped rules stay unschedulable, and no occurrence/planner/journal/executor edge exists. |

## Execution remains unreachable

This checkpoint changes preferences and presentation only. It adds no clock or
wake consumer, production schedule source, runtime/history eligibility adapter,
low-disk episode identity, occurrence claim, cleanup-session link, candidate,
scan, fresh plan, approval, journal lease, executor, notification delivery,
CLI operation, AI operation, or platform effect. The global control remains
default-off, `executionAvailable` remains false, and `AppRuntime` continues to
construct only `NoEnabledSchedulesDuxAutomationDecisionSource`.

## Required verification

Coverage must prove exact numeric boundaries and overflow refusal, locale-aware
lossless byte conversion, no-op round trips, preservation of hidden fields,
suggestion-only creation, disabled-only exact-revision editing, the 64-row
limit, serialized mutations, shutdown joining, revision/outcome-unknown fences,
no blind retry, complete-overview projection, FFI post-write correlation, and
unique accessible identifiers and disclosures. Repository guards must continue
to reject every due/run/trigger/claim/execute method and every AI/CLI/effect
consumer.

Passing those gates proves only that a user can safely store and revise an
effect-dormant configuration. It is not evidence that a schedule can run.

## Related decisions and plans

- [ADR 0015: Automation activation and UTC recurrence](../adr/0015-automation-activation-and-utc-recurrence.md)
- [Automation activation controls review](m8-automation-activation-controls.md)
- [Automation history suggestions review](m8-automation-history-suggestions.md)
- [Selectable-scope authoring review](m8-automation-selectable-scope-authoring.md)
- [Milestone 8 roadmap](../../ROADMAP.md#milestone-8-automations)
- [Security design](../../SECURITY_DESIGN.md#10-automation)
