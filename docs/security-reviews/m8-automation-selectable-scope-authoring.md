# M8 Automation Selectable-Scope Authoring Security Review

Date: 2026-08-10

Status: accepted only for effect-dormant, catalog-bound creation and exact
disabled-state re-review of category schedules. This review does not approve a
production due source, runtime eligibility adapters, occurrence claiming,
notifications, planning, journaling, cleanup, or execution.

## Decision

DUX may expose a separate core-owned automation authoring catalog and use one
exact fresh category option to prepare a disabled category schedule. The user
must first choose and review the category plus its complete exact exclusions,
then separately choose **Save disabled schedule**. Loading or displaying the
catalog and opening or changing the form perform no durable write.

DUX may also use that catalog to rebind one existing exact revision of the same
disabled category after a complete explicit review. Rebind is a separately
named mutation and never follows from load, refresh, ordinary editing, or an
activation action.

Authoring catalog policy revision 1 admits only exact current shipped rules
that are schedule-eligible, `SafeRegenerable`, permanent-safe,
`UserCacheDirectory` scoped, and free of protected-descendant selectors. Core
is the sole source of the option set. Native code cannot derive it from a
closed category enum, `allCases`, localized strings, a free-form rule ID,
history, AI, or existing persisted schedules.

Every shipped rule remains unschedulable, so the production catalog is empty.
Positive behavior is exercised only with injected test policy. This checkpoint
does not change the bundled candidate catalog or make any real rule selectable.

## Catalog contract

UniFFI v66 adds one separate, bounded, path-free query. Automation overview
stays record version 3 so a catalog error cannot hide the last valid saved
activation-management graph.

The catalog record contains only record and authoring-policy versions, the
maximum selected-exclusion count, the total statically selectable rule count,
and ordered categories. A category contains its closed discriminator, an exact
SHA-256 membership binding, and ordered rule records. A rule record contains
only exact rule ID/revision and a code-owned title key.

Core, FFI, and Swift independently reject:

- wrong record or authoring-policy versions;
- invalid or noncanonical digest encoding;
- excessive category, rule, exclusion, identifier, or title-key bounds;
- duplicate or noncanonical categories and rules;
- a rule repeated across categories or assigned to the wrong category;
- impossible top-level or per-category counts; and
- empty categories or a nonempty result inconsistent with the exact total.

Syntax validation alone is insufficient for the membership binding. UniFFI
recomputes the policy-v1 domain-separated digest from each category and its
exact projected canonical rule sequence before returning it. Swift independently
repeats the byte-exact computation with CryptoKit before constructing the
catalog model. Either boundary rejects a digest/rule mismatch, including a
syntactically valid current digest paired with an incomplete displayed list.

The records expose no path, root, candidate or scan ID, schedule ID,
eligibility result, runtime fact, plan, approval, journal lease, task, callback,
trigger, notification command, driver, or effect capability. The title key is
display-only and cannot return as a mutation identifier.

## Exact category consent

Category membership is authority-relevant. Core binds authoring policy
revision 1, the category, and its exact canonical current selectable `RuleRef`
sequence with a domain-separated SHA-256 digest. Schema v22 persists the
reviewed policy revision and digest on category schedules.

Migration from schema v21 preserves every existing schedule, state, cursor,
preference, and exclusion byte, but does not manufacture a membership binding.
Eligibility policy revision 2 treats missing, malformed, unsupported, or stale
category membership as blocked static policy. A later application version that
adds, removes, or revises an eligible member cannot silently widen or reinterpret
the old schedule. Explicit disabled-state re-review is required before current
membership can be saved.

The digest is only a consistency witness. It is not a candidate, approval,
execution claim, or substitute for current eligibility. Exact-rule schedules
seeded by the separate manual-history suggestion flow retain their exact
`RuleRef` semantics and do not borrow category consent.

## Category creation boundary

The new category draft input carries the exact catalog policy revision,
membership digest, closed category, reviewed exact exclusions, and the existing
bounded schedule preferences. Rust re-derives the current catalog and refuses
a category, digest, or exclusion that was not present in the exact reviewed
option. Exclusions must be unique, canonical, within the selected category,
exactly current, and within the fixed limit. Excluding every selectable member
is refused for new authoring rather than saved as a misleading empty scope.

Successful creation still uses a core-generated opaque ID and creates only
`Disabled` state with no cursor. The persistence writer guard, current-schema
check, immediate transaction, exact graph validation, and commit-error
reconciliation remain mandatory. A pre-write catalog conflict creates no row.
An unprovable post-write outcome is `OutcomeUnknown`; native retains and fences
the proposal, never retries an uncertain create, and requires refresh plus a
new explicit review.

The existing 64-schedule and 32-exclusion limits remain fixed. The catalog
reports the exact exclusion limit rather than permitting native code to invent
one.

## Disabled category re-review boundary

Migration deliberately leaves old category rows unbound, and catalog drift
deliberately makes an existing binding stale. Recovery is one separately named
operation, `rebind_automation_category_schedule_draft`. It accepts an existing
schedule ID, exact expected revision, and the same fresh catalog-bound category
draft used for reviewed creation. Core re-derives the current catalog and
requires all of the following before mutation:

- the stored schedule exists at the exact expected revision and is `Disabled`;
- both stored and proposed scopes are the same closed category;
- the new policy revision and digest match the complete current membership;
- every exclusion is canonical, exact, current, inside that membership, and
  within the fixed limit; and
- at least one exact current member remains included.

The operation cannot switch a rule schedule to a category, switch categories,
enable or resume a schedule, or preserve an unreviewed stale binding. It uses
the existing exact writer and post-commit reconciliation. Any unprovable write
is `OutcomeUnknown`; native never retries it and requires a complete overview
and catalog refresh followed by a new review.

Settings offers re-review only for an existing disabled category schedule that
can be matched to one exact fresh catalog category. It displays the complete
current rules and exact exclusions before the separate **Save reviewed
membership** action. Opening the form, refreshing either graph, or acknowledging
a warning writes nothing. Normal disabled editing cannot rebind consent, and
catalog refresh cannot silently invoke or pre-author the transition.

## Staleness and presentation

Settings loads the catalog through its own cancellable, generation-fenced
operation. Saved schedules remain visible if catalog loading fails, and an
earlier valid catalog may remain visible only as a stale observation that
cannot seed a new mutation after its generation is invalidated.

An editor retains the exact category, membership digest, authoring-policy
revision, and exclusions it opened with. Refresh cannot merge new choices into
that session or silently rebase the proposal. Missing current membership,
catalog conflict, revision conflict, malformed response, or uncertain write
requires explicit re-review or close.

The production empty state states that current shipped policy offers no
category scope. It must not fall back to a hardcoded category picker, accept a
free-form identifier, or imply that catalog absence is a storage/history
failure. Category membership and exact exclusions are shown before Save with
stable accessibility identifiers and keyboard behavior.

## Authority and AI separation

The catalog and membership digest are configuration observations only. They
cannot satisfy manual-history, newest-two-run, current candidate, process,
evidence, runtime-identity, energy, low-disk episode, manual-work, planning, or
pre-effect gates.

AI explanation input/output, provider state, cached output, and model text have
no type or call edge to the catalog loader, category editor, category create or
rebind operation, or persisted digest. A history suggestion cannot nominate a
category or populate exclusions. The CLI, scheduler, maintenance rotation,
planner, cleanup journal, executor, notification delivery, recovery diagnostics,
and platform drivers do not consume the catalog or its binding.

## Threat review

| Threat | Required refusal or containment |
| --- | --- |
| Swift invents a selectable category | The picker is populated only from the independently validated core catalog; no `allCases`, hardcoded list, localized-ID round trip, or free-form input exists. |
| An app update silently adds a rule to saved consent | Schema v22 persists exact membership; eligibility policy v2 blocks any membership drift pending explicit disabled-state re-review. |
| An unrelated rule is used as an exclusion | Core requires every exclusion to be an exact selectable member of the reviewed category. |
| Every rule is excluded | New category creation fails before writing instead of persisting an empty or misleading scope. |
| A stale form saves against a new catalog | The request carries the exact policy and digest; core recomputes both and refuses mismatch. |
| Refresh silently rebases an open form | The editor retains its reviewed catalog generation and becomes stale; the user must review again. |
| A malformed catalog widens choices | Core, FFI, and Swift independently validate versions, bounds, membership, counts, uniqueness, and order; FFI and Swift recompute the digest over the exact displayed membership. |
| A migrated or stale row is silently rebound | Rebind is a separately named exact-revision mutation admitted only for the same disabled category after a fresh complete review; refresh and ordinary editing cannot call it. |
| A title or localized label becomes authority | Mutation uses only the closed category plus exact core binding and rule references; title keys never round-trip. |
| A suggestion becomes category consent | Suggestion-seeded exact-rule creation remains a separate two-action flow and cannot populate a category request. |
| AI proposes or submits a category | No AI source imports the catalog/editor/create types or calls their operations. |
| An uncertain create or rebind is retried | Outcome-unknown fences mutation and preserves the form; only authoritative refresh and new review can proceed. |
| Catalog records reach cleanup | Production source remains empty and no occurrence/planner/journal/executor/effect edge exists. |

## Execution remains unreachable

The checkpoint preserves every independent execution barrier:

1. every shipped rule remains unschedulable and the production authoring
   catalog is empty;
2. the global control remains default-off and execution remains unavailable;
3. `AppRuntime` still uses only `NoEnabledSchedulesDuxAutomationDecisionSource`;
4. no exact-scope history or current-evidence adapter can consume the saved
   category; and
5. no occurrence claim, fresh plan, cleanup journal, executor, notification,
   CLI, AI, or platform effect is connected.

Passing this review proves only that Settings can safely present core-owned
choices and store exact disabled category consent. It is not evidence that a
schedule is eligible, enabled, runnable, or safe to execute.

## Required verification

Coverage must prove:

- the complete authoring-policy predicate and an empty production catalog;
- positive injected rule/category catalogs without modifying shipped policy;
- stable domain-separated per-category digests and membership-drift refusal;
- independent FFI and CryptoKit/Swift digest recomputation, including
  syntactically valid digest/rule mismatch refusal;
- populated-v21 migration with no fabricated binding;
- eligibility-policy-v2 blocking for missing, stale, changed, and malformed
  category bindings;
- category/exclusion bounds, uniqueness, canonical ordering, cross-category
  refusal, and all-excluded refusal;
- category create is core-bound, disabled-only, core-ID-generated, exact, and
  commit-reconciled;
- category rebind is exact-revision, disabled-only, same-category,
  catalog-bound, explicitly reviewed, and commit-reconciled;
- independent FFI and Swift validation of every field and relation;
- explicit review then Save, stale-generation fencing, malformed response,
  accessibility, shutdown joining, and no unknown-create/rebind retry; and
- dynamic repository guards for core-only catalog origin, no hardcoded/free-
  form picker, no AI/CLI/timing/planner/executor consumer, no due/run/trigger/
  claim API, the empty production source, and unavailable execution.

## Deferred gates

Exact-scope durable history, sealed current facts, energy/battery/thermal and
manual-work gates, low-disk episode identity, global re-enable rebasing,
production deadline selection, occurrence claiming and cleanup-session
linkage, notifications, fresh planning and immediate revalidation, per-run-cap
enforcement, failure pausing, terminal settlement, execution-era crash tests,
and Release cleanup qualification all remain separate reviews.

Automatic future category widening, arbitrary paths, selected-scan-root
category invention, AI-derived schedules, CLI automation, and maintenance-task
reuse are not approved.

## Related decisions and plans

- [ADR 0016: Automation category-scope membership consent](../adr/0016-automation-category-scope-membership-consent.md)
- [ADR 0015: Automation activation and UTC recurrence](../adr/0015-automation-activation-and-utc-recurrence.md)
- [Automation schedule authoring controls review](m8-automation-schedule-authoring-controls.md)
- [Automation eligibility review](m8-automation-eligibility.md)
- [Milestone 8 roadmap](../../ROADMAP.md#milestone-8-automations)
- [Security design](../../SECURITY_DESIGN.md#10-automation)
