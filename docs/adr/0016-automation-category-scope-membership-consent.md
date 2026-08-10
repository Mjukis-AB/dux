# ADR 0016: Automation category-scope membership consent

- **Status:** Accepted
- **Decision date:** 2026-08-10

## Context

ADR 0015 admitted effect-dormant automation activation and UTC recurrence.
The later schedule-authoring checkpoint deliberately allowed new schedules to
start only from one exact, path-free manual-history suggestion. Native Settings
therefore never had to invent a category or rule catalog.

Milestone 8 also calls for generic category authoring with exact rule
exclusions. A category name alone is not durable consent: the set of shipped
rules in that category can change when the application or bundled catalog is
updated. If an existing category schedule automatically included a newly
schedule-eligible rule, saved consent could silently widen to permanent cleanup
the user never reviewed. Exact exclusions do not prevent that widening because
the new rule did not exist when the exclusions were chosen.

The authoring boundary must also avoid turning a Swift enum, localized label,
history suggestion, AI result, or caller-authored rule identifier into the
source of selectable cleanup policy. All currently shipped rules remain
unschedulable, so the production catalog is empty. The design nevertheless
needs a positive, testable contract before future separately reviewed rule
policy can populate it.

## Decision

### Core-owned authoring catalog

Rust core owns authoring-catalog policy revision 1. A rule is statically
selectable for category authoring only when its exact current shipped revision:

- is marked schedule-eligible;
- is `SafeRegenerable` with
  `RemoveKnownRegenerableContents`;
- uses the code-owned `UserCacheDirectory` scope; and
- declares no protected-descendant selector.

These are authoring constraints, not runtime eligibility. History, current
candidate state, process activity, energy state, and filesystem evidence cannot
add an option or weaken this predicate. A category is present only when at
least one exact rule in it satisfies the complete predicate.

Core exposes a separate bounded, path-free catalog query through UniFFI v66.
It does not widen automation overview v3. The top-level record contains:

- `record_version`;
- `authoring_policy_revision`;
- `maximum_selected_exclusions`;
- `statically_selectable_rule_count`; and
- canonically ordered `categories`.

Each category contains its record version, closed category discriminator,
`scope_membership_digest_sha256`, and canonically ordered rules. Each rule
contains only its record version, exact rule ID/revision, and code-owned title
key. Counts, collections, identifier lengths, digest encoding, uniqueness,
category membership, and canonical order are bounded and independently
validated by core, UniFFI, and Swift.

The digest is not treated as an unrelated opaque string at either transport
boundary. Before projecting a category, UniFFI independently recomputes the
domain-separated policy-v1 digest from the projected closed category and exact
canonical `RuleRef` sequence and rejects a mismatch. Swift repeats the same
byte-exact computation with CryptoKit before admitting the catalog model. A
response whose displayed rules do not match its digest therefore cannot seed a
picker or mutation even if the digest is syntactically valid.

The catalog contains no path, scan or candidate identity, schedule ID,
eligibility result, plan, approval, journal claim, task, callback, runtime fact,
notification command, or effect capability. Its title keys are presentation
hints and never round-trip as identifiers. Swift may localize a validated key
or closed category, but it cannot derive the choice set from localized text,
`allCases`, a free-form value, history, or AI.

### Exact membership consent

For each selectable category, core computes a domain-separated SHA-256 binding
over authoring policy revision 1, the closed category discriminator, and the
complete canonical sequence of exact selectable `RuleRef` values. Presentation
metadata such as a title key does not change authority membership and is not a
substitute for that sequence.

Schema v22 persists the reviewed authoring-policy revision and exact category
membership binding with a category schedule. Migration preserves every v21
schedule, configuration, exclusion, state, and cursor, but fabricates no
binding for an existing row. A missing, malformed, unsupported, or stale
binding is unproven consent.

Automation eligibility policy revision 2 extends the existing shipped-policy
gate so a category schedule is blocked unless its persisted binding matches the
current exact selectable membership for that category. Newly eligible rules
therefore block the old category schedule pending explicit review; they never
enter it automatically. Removed, revised, or newly ineligible membership has
the same fail-closed result. This does not change the eight-gate structure or
turn a favorable result into execution authority.

The binding is a consistency and consent witness, not a cleanup capability. It
does not approve any candidate and cannot be accepted by a planner, journal, or
executor.

Migration and membership drift need a recovery path without implicit consent.
The only such transition is
`rebind_automation_category_schedule_draft`: it requires one existing exact
schedule ID and revision, `Disabled` state, the same category scope, and a fresh
catalog-bound category draft containing the complete reviewed current
membership and exclusions. Core validates the current catalog immediately
before the write and persists the new binding under the existing exact writer
and commit-reconciliation boundary. It refuses rule/category scope switching,
enabled or paused schedules, stale revisions, stale catalog inputs, and every
invalid exclusion. Loading or refreshing a catalog never invokes rebind.

### Category authoring transition

Native Settings can create only a `Disabled` category schedule through the new
catalog-bound operation. The user first chooses one exact fresh catalog
category and reviews the complete exact exclusion set, then separately chooses
**Save disabled schedule**. Opening, displaying, refreshing, or changing a
picker writes nothing.

The category draft request carries the exact authoring-policy revision,
membership digest, category, reviewed exact exclusions, and the existing
bounded cadence, age, size, run-cap, notification, and confirmation
preferences. Core repeats the complete catalog derivation and refuses:

- a stale or unknown policy revision or membership digest;
- a category absent from the current catalog;
- an exclusion outside that category or selectable membership;
- a missing, stale, duplicate, noncanonical, or over-limit exclusion; or
- exclusion of every selectable rule in the category.

Only after that validation may the existing core-generated schedule-ID,
disabled-state, writer guard, immediate transaction, exact revision, and
commit-reconciliation boundary persist the row. A catalog conflict is a
pre-write refusal. Any unprovable post-write outcome remains `OutcomeUnknown`,
is never retried automatically, and requires a complete refresh and new
review.

The existing suggestion-seeded exact-rule creation path remains separate. A
history suggestion is still advisory and cannot nominate a category or provide
category membership consent. Existing disabled-state editing continues to
preserve scope and exclusions. Scope is immutable across ordinary edit and
rebind. A migrated or stale category schedule can change its membership binding
or exclusions only through the separate exact-revision, same-category rebind
flow: the user opens the disabled schedule against one fresh catalog category,
reviews the complete current membership and exact exclusions, then separately
chooses **Save reviewed membership**. An unprovable rebind outcome is
`OutcomeUnknown`, is never retried, and requires full refresh and a new review.

### Effect-dormant boundary

This decision adds configuration discovery and durable consent binding only:

- all shipped rules remain `schedule_eligible=false`, so the production
  authoring catalog is empty;
- execution remains unavailable and the global default remains disabled;
- `AppRuntime` continues to use only the statically empty production
  automation decision source;
- no due, assess, trigger, run, claim, or occurrence operation is added; and
- no planner, cleanup journal, executor, CLI, AI, notification, maintenance,
  recovery, or platform-effect component consumes the catalog or binding.

## Consequences

Category consent cannot silently widen after a catalog change. A user may need
to revisit a disabled schedule after an application update, including when the
changed rule is unrelated to the exclusions they previously chose. That
explicit friction is preferable to unattended permanent cleanup under newly
expanded consent.

The production UI truthfully has no category options until a separate rule
review changes shipped policy. Positive catalog and Settings behavior is tested
with injected policy rather than by making a production rule schedulable.

Keeping the catalog query separate preserves overview v3 and isolates catalog
loading failure from saved activation-management state. An editor retains the
exact catalog generation it reviewed; refresh never silently substitutes a new
membership or rebases an open proposal.

Schema v22 and eligibility policy revision 2 add compatibility work even
though no production category can currently be authored. That cost establishes
the durable consent boundary before it can carry authority.

## Alternatives considered

- **Automatically include future eligible rules in a saved category.**
  Rejected because a signed catalog update would broaden permanent-cleanup
  consent without a new user review.
- **Trust the Swift category enum or hardcode a picker.** Rejected because the
  client would become the source of cleanup-policy choices and could drift from
  the exact bundled catalog.
- **Validate only when the form opens.** Rejected because stale UI state or a
  different process version could persist a category membership the current
  core did not present.
- **Store only exclusions.** Rejected because a deny list cannot prove the
  complete category membership the user reviewed.
- **Reuse history suggestions as the category catalog.** Rejected because
  suggestions are bounded historical observations for exact rules, not shipped
  authoring policy or category consent.
- **Let AI propose or preselect categories.** Rejected because AI remains
  outside every candidate, schedule, approval, and cleanup authority graph.
- **Put the catalog in automation overview v3.** Rejected for this revision so
  catalog availability and refresh remain independent from the complete saved
  schedule graph.

## Validation

Implementation and repository architecture tests must prove:

- authoring catalog policy revision 1 admits only the complete reviewed rule
  predicate and the real bundled catalog produces zero categories/rules;
- the catalog graph is bounded, unique, canonical, internally reconciled, and
  path/capability-free across core, UniFFI, and Swift;
- per-category digests change when and only when authority membership changes;
- UniFFI and Swift independently recompute the byte-exact policy-v1 digest and
  reject a syntactically valid digest paired with different displayed rules;
- schema v22 preserves populated v21 graphs without fabricating a binding;
- eligibility policy revision 2 blocks missing, malformed, stale, revised, or
  changed category membership;
- category creation repeats policy/digest/category/exclusion validation before
  its write and persists only `Disabled` state under the existing exact writer
  and reconciliation boundary;
- Settings choices come only from the validated core catalog, require review
  then Save, and stale refresh/conflict/outcome-unknown state requires a new
  explicit review;
- the only category-consent update is an exact-revision, disabled-only,
  same-category rebind that repeats current catalog validation and never runs
  automatically;
- suggestion exact-rule creation stays independent;
- no shipped rule is made schedulable and the production catalog/source remain
  statically empty; and
- no due/run/trigger/claim, planner, journal, executor, AI, CLI, notification,
  maintenance, recovery, or effect edge is introduced.

## Reconsideration triggers

A new ADR is required before automatically widening category membership,
changing the digest membership semantics, allowing a client or remote catalog
to nominate choices, admitting arbitrary or selected-scan-root paths, or
allowing AI/history to create category consent.

A separate security review remains required before making a production rule
schedule-eligible, populating the production catalog, changing the production
deadline source, claiming an occurrence, consuming runtime evidence, notifying
a user, or reaching planning or execution.

## Related decisions and plans

- [ADR 0015: Automation activation and UTC recurrence](0015-automation-activation-and-utc-recurrence.md)
- [ADR 0014: Automation clock, wake, and missed-run semantics](0014-automation-clock-wake-and-missed-run-semantics.md)
- [Milestone 8 roadmap](../../ROADMAP.md#milestone-8-automations)
- [Security design](../../SECURITY_DESIGN.md#10-automation)
- [M8 selectable-scope authoring security review](../security-reviews/m8-automation-selectable-scope-authoring.md)
