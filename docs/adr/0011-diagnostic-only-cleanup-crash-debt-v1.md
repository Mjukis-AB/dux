# ADR 0011: Diagnostic-only cleanup crash debt in v1

- **Status:** Accepted
- **Decision date:** 2026-08-09

## Context

DUX records permanent-cleanup intent before it can call a filesystem effect.
After an abrupt process exit, a durable `running` or `recovering` cleanup
session can therefore contain one of several materially different facts:

- work that was still planned;
- validation that had started but had not reached an effect;
- an effect intent whose operating-system outcome is unknown; or
- settled earlier paths beside later unfinished paths.

Schema v14 binds new claims to separate stable-host and boot-scope digests when
the platform can supply them. The existing recovery state machine may recover
only an exact same-boot owner that the platform proves is definitely gone.
Prior-boot, foreign-host, migrated, and otherwise unproven claims are typed
no-ops. Partial, malformed, and unknown-policy evidence fails closed without a
write. Age, heartbeat staleness, PID absence, cleanup-lock availability, and a
successful later launch do not replace that proof.

ADR 0010 adds a bounded, aggregate-only native diagnostic for this retained
bookkeeping. It intentionally grants no selector or recovery authority and
left one release decision open: whether v1 needs a new non-resumable metadata
reconciler, or whether durable non-executability plus the diagnostic is the v1
policy.

Retained crash debt does not block a new independent cleanup session with a
distinct current candidate. New work uses a new random session identity, a
fresh reviewed plan, the current process's cleanup lease and execution claim,
and complete execution-time revalidation. Even an overlapping current path
must derive all authority again; the older row supplies none. A process-local
outcome ambiguity still quarantines that store for the rest of the process,
but an older durable row is not silently reopened or installed as
current-process authority after restart.

Retained crash debt does block the separate whole-app-data-reset store
admission. That fail-closed blocker is intentional: resetting the store must
not erase an unresolved effect journal. The effectful reset composition is not
currently exposed to users. A reconciler would therefore widen mutation
authority to improve a future reset path, not to restore ordinary scanning,
history reads, or independently reviewed cleanup.

## Decision

DUX v1 accepts durable non-executability plus the ADR 0010 read-only diagnostic
as the complete product policy for cleanup crash debt that cannot meet the
existing exact same-boot recovery proof.

The following stored relationships remain unchanged and non-executable:

| Stored/current relationship | v1 behavior |
| --- | --- |
| Same host and current boot | Existing exact process probe applies; only `DefinitelyGone` may enter the existing recovery state machine. The diagnostic count alone grants nothing. |
| Same host and prior boot | Retain the complete journal graph unchanged. Do not probe the old PID, claim, resume, reconcile, or terminalize it. |
| Foreign host | Retain the complete journal graph unchanged. Do not claim that the copied or moved store describes this host. |
| Stored provenance absent | Retain the complete journal graph unchanged. Migration or missing platform evidence must not fabricate ownership. |
| Current host/boot context unavailable | Retain the complete journal graph unchanged. Stored evidence cannot be compared safely. |
| Partial, malformed, or unknown policy | Fail the observation or recovery read as corrupt/incompatible; do not normalize or repair it. |

This is a durable policy, not a temporary automatic retry. An affected row may
remain in the DUX store indefinitely. Automatic retention, cleanup-history
clearing, pressure handling, notifications, AI, schedules, and the CLI must not
delete, annotate, hide, select, or mutate it. Settings may show only the
bounded aggregate diagnostic defined by ADR 0010. `0`, `64+`, or any individual
category is an observation and never a cleanup predicate.

V1 exposes no action to recover, resume, reconcile, retry, dismiss, clear, or
remove one of these cleanup rows. Support copy must not label a row abandoned,
dead, stale, safe, reclaimable, or removable. Existing cleanup history may
continue to describe the unresolved session as active or recovering; DUX must
not invent a terminal outcome to make the UI look complete.

## Authority boundaries

This decision adds no production command or mutation. In particular it must
not add:

- a session ID, row cursor, path, owner, PID, timestamp, digest, or opaque
  recovery handle to the public diagnostic;
- a process-liveness probe reachable from Settings or UniFFI;
- a cleanup lock, recovery lease, execution generation, validation witness,
  plan, candidate transition, effect receipt, or capacity claim;
- a persistence update, history annotation, terminal transition, row delete,
  filesystem enumeration, or filesystem call; or
- a CLI, AI, low-disk, notification, maintenance, or scheduler consumer.

The existing exact same-boot `DefinitelyGone` recovery path is unchanged. It
remains private, owner/generation-fenced, and cannot be reached through the
ADR 0010 census.

## Release boundary

Accepting diagnostic-only v1 crash debt closes only the reconciliation-policy
decision. It does not authorize permanent cleanup in a public Release build.
`DUX_INTERNAL_PERMANENT_SAFE_CLEANUP` remains Debug-only until the independent
security-reporting, signed-app destructive qualification, exposed-platform,
and remaining release gates are completed and reviewed.

Removing that compilation boundary must be a separate checkpoint. Its review
must prove that the shipped UI still requires a current exact reviewed plan,
explicit confirmation, durable opt-in, execution-time revalidation, and the
central Rust authority chain. No historical or diagnostic record may satisfy
one of those requirements.

## Consequences

- V1 gains an explicit, testable answer for every unsupported cleanup-owner
  relationship without adding another writer or recovery capability.
- A crash-debt row remains honest evidence. An unknown effect is never retried
  or rewritten as success, failure, rejection, or cancellation.
- Later independent cleanup remains available through a new reviewed session;
  old diagnostic debt cannot supply or inherit its authority.
- The Settings count can remain nonzero indefinitely, and there is no per-row
  remediation action in v1. This is deliberately less convenient than an
  under-specified reconciler.
- Whole-app-data reset remains fail-closed while any cleanup session is active
  or recovering. V1 must not erase unresolved effect history merely to make
  reset available.
- Cleanup-history clearing and automatic retention continue to preserve active
  and recovering sessions. They cannot be used as an indirect debt escape.

## Alternatives considered

- **Automatically terminalize prior-boot rows.** Rejected. A row may retain
  effect intent or an outcome whose filesystem result cannot be reconstructed.
- **Add a user-confirmed non-resumable reconciler now.** Deferred. It would
  need a separately versioned witness, a complete path-state transition matrix,
  exact candidate-claim settlement, cleanup exclusion, compare-and-swap
  semantics, commit-ambiguity handling, and proof that no effect can be called.
  Ordinary v1 cleanup has no availability dependency that justifies that
  authority; future app-data-reset usability is an explicit reconsideration
  trigger.
- **Treat age, heartbeat, PID absence, or lock availability as owner death.**
  Rejected. None proves stable host, boot, process instance, or effect outcome.
- **Hide unresolved rows.** Rejected. Bounded honest visibility is necessary
  for user support and release qualification.
- **Block every later cleanup while debt exists.** Rejected. Independent new
  work already has fresh reviewed authority and full revalidation; retained
  history must not become an unrelated global denial condition.

## Validation

The v1 decision remains valid only while tests and source boundaries prove:

- prior-boot, foreign-host, migrated/unproven, missing-provenance, and
  Windows-unproven recovery attempts leave the complete mutable journal and
  candidate-claim graph byte-for-byte unchanged;
- same-boot recovery still requires `DefinitelyGone`, while `Alive` and
  `Unknown` are no-ops;
- the ADR 0010 census stays bounded, aggregate-only, path-free, read-only, and
  disconnected from the recovery state machine;
- the FFI, Swift, CLI, AI, pressure, notification, maintenance, and scheduler
  surfaces expose no cleanup-debt reconciliation operation;
- cleanup-history clearing and automatic retention preserve active and
  recovering sessions;
- whole-app-data reset remains blocked by any active or recovering cleanup
  session and cannot erase diagnostic debt as an indirect workaround;
- a new independent reviewed cleanup cannot adopt an older session, plan,
  claim, path, diagnostic result, or execution generation; and
- public Release continues to exclude the permanent-cleanup action until its
  separate gates close.

## Reconsideration triggers

Write a new ADR before adding any cleanup-debt row selector or mutation. A
future non-resumable capability must be separately versioned and may be
considered only if durable debt causes material support or product harm. It
must never validate, resume, or start an effect; must preserve every unknown
effect as unknown; must settle candidate claims conservatively; must consume
an exact bounded witness once; and must fail commit ambiguity closed without
retrying a filesystem operation.

Reconsider this decision if an exposed platform cannot start a new independent
reviewed cleanup while unsupported debt exists, or if retained debt prevents a
required data-portability or recovery operation. That evidence would justify a
new design; it would not authorize silently weakening this ADR.

## Related decisions and plans

- [ADR 0007: Prior-boot running-scan history interruption](0007-prior-boot-running-scan-interruption.md)
- [ADR 0008: Legacy unclaimed running-scan dismissal](0008-legacy-unclaimed-running-scan-dismissal.md)
- [ADR 0010: Read-only active-cleanup provenance diagnostic](0010-read-only-active-cleanup-provenance-diagnostic.md)
- [Milestone 5 roadmap](../../ROADMAP.md#milestone-5-deterministic-recommendations-and-reviewed-cleanup)
- [Security design](../../SECURITY_DESIGN.md)
- [Retention contract](../RETENTION.md)
