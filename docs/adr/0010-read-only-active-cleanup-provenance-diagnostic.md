# ADR 0010: Read-only active-cleanup provenance diagnostic

- **Status:** Accepted
- **Decision date:** 2026-08-09

## Context

DUX durably journals permanent-cleanup intent before an effect can begin. An
unexpected process exit can therefore leave `running` or `recovering` journal
records whose outcome must not be guessed. Schema v14 records a stable-host
digest, boot-scope digest, and the sole `resumable` recovery policy for new
claims. Older and unsupported-platform records can remain unproven.

The cleanup recovery state machine already refuses prior-boot, foreign-host,
migrated, partial, malformed, and otherwise unproven ownership without issuing
a recovery claim or changing the journal graph. That refusal is intentionally
private. Native users have had no bounded way to see that unfinished DUX
bookkeeping exists, and Release permanent cleanup must not be enabled while
the state is invisible.

ADR 0007 applies only to scan-history interruption. Its terminal annotation
policy cannot be reused for cleanup journals: a cleanup journal may retain
effect intent or an outcome-unknown operation. No accepted decision currently
authorizes prior-boot cleanup-history mutation, resumption, or reconciliation.

## Decision

DUX exposes a separate, synchronous, read-only active-cleanup diagnostic. One
call inspects at most 64 `running` or `recovering` cleanup sessions plus one
validated lookahead. It returns only:

- a record version and inspected active count;
- aggregate `running` and `recovering` phase counts;
- aggregate same-host/current-boot, same-host/prior-boot, foreign-host,
  stored-unproven, and current-context-unavailable provenance counts; and
- whether the validated bounded page has more records.

The two phase counts and five provenance counts must independently partition
the inspected total. `has_more` requires a full 64-record page. When current
host/boot evidence is unavailable, none of the three comparable provenance
categories may be nonzero. All-null schema-v2 provenance and legitimate
ownerless legacy-v1 active journals count as stored-unproven.

The query observes current host/boot provenance once and never observes stored
process liveness. It validates every selected row and the lookahead using a
path-free scalar journal shape. Any malformed lifecycle, item/path state,
provenance tuple, owner relationship, storage type, or lookahead fails the
whole response. A dedicated conjunctive budget stops after 200 million SQLite
VM instructions or 10 seconds. The legal maximum 65 fully populated scalar
graphs must succeed within that budget; exhaustion is a typed whole-response
failure and removes the progress handler. It exposes no session, plan,
candidate, rule, owner, PID, generation, heartbeat, timestamp, age, digest,
policy, path, byte estimate, selector, cursor, or opaque handle.

The operation is not an authorization predicate. Neither a zero result nor a
same-host/current-boot count may be consumed as cleanup, recovery, scheduling,
maintenance, low-disk, notification, AI, or CLI authority.

## Native presentation

Settings → Storage & Privacy presents the result as **Unfinished cleanup
bookkeeping**. It loads lazily, supports one explicit read-only refresh, keeps
an earlier valid result visible when a later refresh fails, and generation-
fences superseded, dismissed, shutdown, and late replies. A terminal cleanup
event refreshes it only if Settings already requested it; this does not create
polling.

The view provides separate phase and provenance charts with redundant text,
counts, icons/colors, and VoiceOver labels. It states that the result is DUX
journal bookkeeping—not user-file inspection, disk usage, reclaimable space,
process liveness, or cleanup permission. It has no recovery, resume,
reconciliation, retry-cleanup, dismissal, clearing, or history-mutation action.

## Authority boundaries

The diagnostic must not:

- call a process probe or `try_recover`;
- acquire a cleanup lock or recovery lease;
- mint a cleanup claim, execution owner, generation, approval, plan, target,
  capacity claim, or effect witness;
- invoke target validation, filesystem enumeration, a planner, task, platform
  callback, or executor;
- update, annotate, terminalize, clear, or reconcile durable state; or
- appear in the CLI or its JSON contract.

Permanent-safe cleanup remains compiled out of public Release builds. Existing
prior-boot, foreign-host, migrated, malformed, and Windows-unproven recovery
refusals remain byte-for-byte no-ops.

## Consequences

Users and support can distinguish bounded unfinished cleanup bookkeeping from
scan debt without exposing private paths or creating recovery authority. The
diagnostic improves visibility and reviewability while preserving the most
conservative durable behavior.

The diagnostic does not reduce or resolve cleanup debt. A count can remain
indefinitely after a crash. The bounded result is incomplete when `has_more`
is true, contains no reclaim estimate, and cannot explain an individual row.

ADR 0011 subsequently selected durable non-executability plus this diagnostic
as the v1 policy. This read-only ADR itself still does not authorize that
decision, any remediation action, or Release permanent cleanup.

## Alternatives considered

- **Reuse scan-history interruption.** Rejected because cleanup journals can
  contain effect intent and outcome ambiguity that scan history does not.
- **Expose individual rows or paths.** Rejected because identity and selectors
  add privacy risk and could become ambient authority.
- **Probe processes to label records stale or abandoned.** Rejected because a
  diagnostic liveness result would be easy to misread as recovery permission.
- **Automatically reconcile prior-boot rows.** Rejected for v1 by ADR 0011. A
  future change still requires a separate ADR defining exact admissible states,
  compare-and-swap semantics, commit ambiguity, and non-resumable authority
  boundaries.
- **Hide the state until reconciliation exists.** Rejected because invisible
  durable debt makes qualification and user support materially harder.

## Validation

Acceptance requires:

- five-way provenance and two-way phase classification, including legacy-v1
  and current-context-unavailable cases;
- exact 64-row and validated-lookahead behavior;
- legal maximum-shape/page success plus typed budget exhaustion and handler
  removal;
- malformed selected-row and lookahead rejection;
- query-budget, busy, unsafe-store, schema, and corruption error mapping;
- byte-for-byte database/journal/claim immutability across the read;
- source-boundary proof that no probe, recovery, planner, filesystem, or
  destructive edge is reachable;
- independent core, UniFFI, and Swift record/arithmetic validation;
- native lazy-load, retained-result, refresh-race, shutdown, and accessibility
  tests; and
- deterministic generated bindings plus universal Debug and Release builds
  with Release permanent cleanup still absent.

## Reconsideration triggers

Write a new ADR before adding any cleanup-journal mutation, row selector,
recovery handle, liveness label, automatic refresh/pagination, CLI command, AI
consumer, low-disk consumer, or individual-record detail. In particular,
prior-boot reconciliation requires a new ADR superseding ADR 0011 and may not
be added as an extension of this read-only record.

## Related decisions and plans

- [ADR 0005: UniFFI for the Swift/Rust transport](0005-uniffi-swift-rust-transport.md)
- [ADR 0007: Prior-boot running-scan history interruption](0007-prior-boot-running-scan-interruption.md)
- [ADR 0008: Legacy unclaimed running-scan dismissal](0008-legacy-unclaimed-running-scan-dismissal.md)
- [ADR 0011: Diagnostic-only cleanup crash debt in v1](0011-diagnostic-only-cleanup-crash-debt-v1.md)
- [Milestone 5 roadmap](../../ROADMAP.md#milestone-5-deterministic-recommendations-and-reviewed-cleanup)
- [Security design](../../SECURITY_DESIGN.md)
