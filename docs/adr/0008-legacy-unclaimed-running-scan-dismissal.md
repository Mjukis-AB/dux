# ADR 0008: User-confirmed legacy unclaimed running-scan dismissal

- Status: Accepted
- Date: 2026-08-08
- Scope: legacy schema-v8 running-scan history debt

## Context

Schema v8 could persist a `running` scan without durable process ownership.
Schema v9 made a process claim mandatory for every newly admitted scan, but its
migration correctly left existing rows unclaimed: the current process, host,
boot, PID, and wall clock cannot prove who created an older row or whether that
owner is still executing.

The schema-v15 census therefore reports unclaimed rows without changing them.
This is safe but incomplete. Retained rows can remain visible as unfinished
bookkeeping and can conservatively block a later scan of the same root. Rows
with partial results or related discovery records are even less attributable
and must not be normalized into the pristine legacy shape.

Scan history interruption is not cleanup. It cannot resume traversal, publish
a snapshot, validate a candidate, grant a cleanup plan, or perform a filesystem
effect. A snapshot-temp lease, if present, is independent evidence owned by the
terminal-temp reconciler.

## Decision

DUX may offer an explicit, user-confirmed action named **Dismiss old unfinished
bookkeeping** for exact pristine, unclaimed `running` rows. The action records
that the user no longer wants DUX to treat those history entries as active. It
does not state that an old process is dead or that the entry is abandoned.

Automatic maintenance, low-disk handling, scheduled cleanup, CLI recovery, AI,
PID probing, elapsed time, and launch-time migration cannot invoke this action.
The confirmation is the sole authority. DUX must explain that the action frees
no disk space, removes no files, does not resume or complete a scan, and may
cause a still-running legacy scanner's later result to be rejected.

A row is eligible only when all of the following are true at preview and commit:

- its complete stored scalar shape is valid and its status is `running`;
- completion time, snapshot reference, allocated bytes, and coverage value are
  absent;
- every persisted count is zero and coverage is `unknown`;
- no process claim, issue, aggregate, candidate, or candidate-evaluation row
  refers to the scan; and
- its start time is no later than the canonical confirmation time.

The preview selects eligible rows directly rather than paging through
unrecognized rows first. It is ordered by `(started_at_unix_ms, scan_id)`,
contains at most 64 exact private witnesses plus one lookahead, and is subject
to the ordinary fixed SQLite progress and elapsed-time budget. This means an
arbitrary prefix of retained unrecognized debt cannot starve later eligible
rows. Repeating the operation converges by making each confirmed eligible page
terminal. `has_more` is only an eligible-row lookahead; it never asks the app to
self-loop or dismiss another page without a fresh confirmation.

The public preview is short-lived, consume-once, and bound to one open engine.
It exposes counts and expiry only—not scan IDs, roots, timestamps, volume IDs,
row selectors, paths, or byte estimates. Commit uses one immediate transaction
and exact compare-and-set witnesses. It updates only `scans.status` to
`interrupted` and `scans.completed_at_unix_ms` to the canonical confirmation
time. Every row in the preview must still match or the transaction changes
nothing. Commit uncertainty is accepted only when every exact interrupted
post-state is present.

Snapshot-temp leases are intentionally neither eligibility blockers nor
mutation targets. All files, snapshots, leases, candidates, claims, AI records,
cleanup journals, settings, and scope leases remain byte-for-byte unchanged.
Unrecognized or malformed unclaimed rows remain visible as retained debt.

## Safety boundary

The operation is a history annotation with no cleanup authority. Its module may
open the DUX SQLite store and use its writer guard, but it must have no call path
to filesystem enumeration, process liveness, snapshot publication or removal,
candidate execution, cleanup planning, AI providers, or scheduled maintenance.

The operation does not prove quiescence. A later scan remains a separate user or
engine decision and must pass the current scan-scope admission rules. Nothing
about this policy weakens claimed-row recovery in ADR 0007 or cleanup-journal
recovery.

## Consequences

- Users can intentionally clear safe-shaped legacy bookkeeping without DUX
  inventing provenance or silently mutating history.
- Unrecognized legacy rows remain explicit and may continue to block an exact
  root until app-data reset or a future separately reviewed policy exists.
- A live schema-v8 scanner may lose its ability to publish that scan result
  after the user confirms dismissal; this is disclosed and preferable to
  falsely accepting two owners for one history row.
- The action reports no reclaimed bytes because it performs no physical work.

## Validation criteria

Tests must prove:

- exact eligibility for a pristine unclaimed row and rejection of claimed,
  terminal, future-started, partially populated, malformed, and related-child
  shapes;
- deterministic 64-plus-one selection and convergence past unrecognized rows;
- short-lived engine-bound consume-once authority with no public identities;
- all-or-nothing compare-and-set behavior under row and child-table races;
- exact completion-time write and unchanged scalar fields;
- preservation of snapshot-temp leases and absence of all filesystem effects;
- exact reconciliation for applied, not-applied, and ambiguous commit outcomes;
  and
- fail-closed newer-schema, busy, corrupt, budget-exhaustion, store-replacement,
  clock, and closed-engine behavior.

Source-policy review must confirm that only this explicit confirmation path can
reach the mutation and that it cannot reach liveness or filesystem operations.

## Reconsideration triggers

Revisit this decision if DUX gains durable resumable traversal state, can prove
schema-v8 ownership without fabrication, intentionally shares a writable store
between hosts, or wants to expose row identities for individual selection. Any
such capability requires a new versioned policy rather than widening this one.

## Alternatives rejected

### Interrupt every unclaimed row automatically

Rejected. Missing ownership is not evidence that the creator is dead.

### Infer death from PID, age, reboot, or the current host

Rejected. None of those observations is durably tied to a schema-v8 row.

### Backfill a process claim or provenance

Rejected. That would manufacture historical evidence.

### Delete the history row or its snapshot-temp lease

Rejected. Deletion obscures the decision, and the lease belongs to an
independent physical-reconciliation protocol.

### Let AI decide whether to dismiss

Rejected. AI is advisory in DUX and cannot receive mutation authority.

## Related decisions and contracts

- [ADR 0004: Shared Rust engine](0004-shared-rust-engine.md)
- [ADR 0007: Prior-boot running-scan history interruption](0007-prior-boot-running-scan-interruption.md)
- [Roadmap Milestone 9](../../ROADMAP.md#milestone-9-cli-companion-and-production-distribution)
- [Security design](../../SECURITY_DESIGN.md)
- [Retention and maintenance](../RETENTION.md)
- [Binary snapshot format](../SNAPSHOT_FORMAT.md)
