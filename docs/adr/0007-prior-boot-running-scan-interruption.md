# ADR 0007: Prior-boot running-scan history interruption

- Status: Accepted
- Date: 2026-07-30
- Scope: durable running-scan ownership and crash recovery
- Clarified: 2026-07-30, bounded aggregate diagnostics through UniFFI v49

## Context

Schema v9 atomically couples every new `running` scan to an immutable process
claim. Its version-1 owner identity includes a combined boot or namespace
scope, which is sufficient to probe an exact owner only while DUX remains in
that reliable scope. It cannot distinguish a prior boot on the same host from a
database copied from another host. As a result, a crash followed by a reboot
can retain claimed `running` history indefinitely.

This debt is different from an interrupted cleanup journal. Running-scan
recovery changes only DUX history. It cannot resume traversal, publish a
snapshot, validate a cleanup plan, or perform a filesystem effect. A retained
snapshot-temp lease remains a separate physical-reconciliation input.

Schema v15 deliberately observes only *unclaimed* running rows. It provides no
authority for claimed-row recovery and does not solve this ambiguity.

## Decision

Schema v16 adds three nullable immutable columns to `scan_process_claims`:

- `execution_host_identity_v1_sha256`;
- `execution_boot_scope_v1_sha256`; and
- `execution_recovery_policy`.

The two digests reuse the private, domain-separated operating-system
observations already defined for cleanup-owner provenance. The only accepted
scan policy is `interrupt_only`. The tuple is either entirely `NULL` or
complete with two exact 32-byte blobs and that policy. Existing v9–v15 claims
remain entirely `NULL`; migration never fabricates provenance.

New claims persist a complete tuple only when the process owner and stable-host
and boot-scope observations agree. Unsupported, unavailable, or conflicting
platform evidence produces the all-`NULL`, explicitly unproven form and does
not prevent a scan from starting. Hostnames, usernames, network addresses,
database paths or randomness, PIDs, timestamps, and claim age are never
substitutes.

The recovery classifier applies these rules:

| Stored relationship | Recovery behavior |
|---|---|
| Complete same-host, same-boot provenance | Probe the exact process owner; only `DefinitelyGone` is recoverable. |
| Complete same-host, prior-boot provenance with `interrupt_only` | Treat the pristine scan history as interruptible without probing the old PID. |
| Complete foreign-host provenance | Non-executable and unchanged. |
| All-`NULL` legacy provenance whose combined recovery-scope value exactly matches the current owner | Preserve the existing exact-owner probe; an absent scope can return only `Alive` or `Unknown`. |
| All-`NULL` provenance with a different combined recovery-scope value | Non-executable and unchanged. |
| Partial, malformed, owner-inconsistent, or unknown-policy provenance | Fail closed without a recovery write. |

A prior-boot decision is not process liveness and is not generalized recovery
authority. It proves only that an `interrupt_only` scan from a different boot
on this same stable host cannot still be executing in the current boot. The
writer transaction compare-and-sets the complete immutable claim and pristine
parent tuple, deletes that exact claim, and changes at most one parent to
`interrupted`. A completion race wins without overwrite. A pre-start recovery
clock cannot write. Commit uncertainty is adopted only when the exact
interrupted post-state is present.

Recovery reads one global keyset page of at most 64 claims plus one sentinel
through `scan_process_claims_by_recovery_time`, drops every SQLite guard before
any same-boot process probe, and advances only a process-local cursor. One batch
interrupts at most one row and never loops or self-enqueues. Repeated batches
can pass foreign, unproven, live, and liveness-unknown rows without starvation.
`has_more` means a sentinel page or more than one recoverable row in the
inspected page; retained foreign or unproven debt alone does not create an
unbounded immediate retry.

The schema-v16 checkpoint left the existing Rust engine maintenance outcome
path-free and unchanged.
Prior-boot and definitely-gone rows contribute to its existing
`recoverable_count`; foreign, unproven, and liveness-unknown rows contribute to
`unknown_count`; exact live rows contribute to `alive_count`. That checkpoint
did not change UniFFI or Swift.

### Diagnostic clarification: UniFFI v49

Contract v49 implements the separately versioned diagnostic anticipated by
this decision without changing `interrupt_only` or recovery behavior. One
synchronous, read-only query inspects a global page of at most 64 claimed
`running` scans plus one lookahead. It exposes only the inspected total,
same-host/current-boot, same-host/prior-boot, foreign-host, stored-unproven,
and current-context-unavailable counts plus `has_more`.

Stored-unproven means the immutable provenance tuple is entirely `NULL`.
Current-context-unavailable means stored provenance is complete but the current
bounded operating-system observation cannot support a host/boot comparison.
Neither category is silently combined with the other. Partial, malformed,
owner-inconsistent, and unknown-policy tuples in the inspected page or
lookahead fail the census instead of becoming an aggregate category. Every
successful response satisfies exact category arithmetic, contains at most 64
inspected rows, and may report truncation only with a full page. A nonzero
current-context-unavailable count may coexist with stored-unproven rows but not
with any category that requires a current host/boot comparison.

The diagnostic exports no claim identity, scan ID, root, timestamp, age, PID,
owner, recovery scope, provenance digest, policy, row selector, path, or byte
estimate. It performs no process probe, recovery admission, filesystem
traversal, or mutation and has no edge to the sealed recovery-maintenance
task. Its categories are provenance observations, not statements that an owner
is alive or dead or that a row is abandoned, recoverable, or actionable.

## Safety boundary

Prior-boot interruption:

- changes DUX scan history only;
- does not inspect or mutate a snapshot, temp file, provisioning stage,
  candidate, plan, journal, cleanup lock, AI record, or user path;
- leaves an exact snapshot-temp lease attached for the independent
  terminal-temp reconciler;
- cannot validate, resume, or publish a scan;
- cannot authorize cleanup or any filesystem effect; and
- does not classify unclaimed legacy-v8 rows.

The v49 diagnostic adds no exception. Its bounded current host/boot
observation and aggregate read cannot authorize the history transition above,
select a row, or inspect a snapshot-temp lease or physical file.

Cleanup-journal recovery remains stricter. Prior-boot and foreign-host cleanup
claims stay non-resumable typed no-ops because they may retain effect authority
or an outcome-unknown operation. This ADR creates no exception to that rule.

## Consequences

- Supported macOS and Linux stores can converge claimed scan history after a
  reboot without treating PID reuse, age, or a copied database as owner death.
- Foreign-host and out-of-scope unproven claims remain explicit durable debt
  until a later diagnostic or user-directed policy is accepted.
- Existing same-boot v9–v15 claims retain their safe recovery path without
  receiving fabricated host provenance.
- Settings can distinguish the bounded aggregate shapes of retained claimed
  debt without receiving or manufacturing recovery authority.
- The crash-debt production gate remains open for the non-fabricating
  legacy-v8 policy and diagnostics for unattributable external stages.

## Validation criteria

The decision is complete only when tests prove:

- exact schema-v16 checksum, fingerprint, migration-chain, rollback, and
  populated-v15 preservation behavior;
- complete/all-`NULL` tuple admission and rejection of every partial, malformed,
  unknown-policy, and owner-inconsistent shape;
- unchanged same-boot `Alive`/`Unknown` behavior and exact
  `DefinitelyGone` recovery, including legacy all-`NULL` current-scope claims;
- prior-boot interruption without a process probe and byte-for-byte no-op
  foreign-host and out-of-scope unproven outcomes;
- exact completion/recovery races, schema replacement, pre-start clock refusal,
  and commit-ambiguity reconciliation;
- preservation of snapshot-temp leases and absence of filesystem calls;
- indexed 64-plus-one paging, cursor wrap, mixed-classification fairness, and
  convergence beyond one page without an unbounded `has_more` loop; and
- unchanged path-free engine/FFI projections.

The v49 clarification additionally requires tests proving the 64-plus-one
bound, exact category partition, deterministic ordering, strict malformed-data
failure, unavailable-current-context separation and exclusivity, path-free FFI
projection, independent Swift validation, and storage immutability. Source
boundary review must separately confirm that the census has no process-probe,
filesystem-enumeration, or recovery-task admission call path.

The repository's ordinary formatting, warnings-as-errors, policy, migration,
workspace, and platform build gates still apply. Verification totals belong in
the roadmap completion evidence, not in this architectural decision.

## Reconsideration triggers

Revisit this decision if DUX:

- supports a resumable scan format with durable traversal state;
- intentionally shares one writable store across hosts;
- gains a reliable Windows stable-host and boot-scope witness;
- changes the operating-system provenance inputs or their domain separation;
  or
- exposes claimed-row identity or provenance beyond the bounded aggregate
  categories accepted by the v49 clarification.

Any wider capability needs a separately versioned policy. It must not silently
reinterpret `interrupt_only` or backfill provenance into existing claims.

## Alternatives rejected

### Treat every non-current recovery scope as dead

Rejected. The combined version-1 scope cannot distinguish a reboot from a
foreign database, and PID or age does not repair that missing provenance.

### Backfill host provenance during migration

Rejected. Current host observations do not prove where or under which boot an
existing claim was created.

### Resume the prior scan

Rejected. Persisted scan ownership is insufficient to reconstruct traversal
state or publish a trustworthy snapshot.

### Remove its snapshot temp while interrupting history

Rejected. Process recovery does not prove physical quiescence or file identity.
The separately sealed terminal-temp protocol owns that later decision.

## Related decisions and contracts

- [ADR 0004: Shared Rust engine](0004-shared-rust-engine.md)
- [ADR 0005: UniFFI for the Swift/Rust transport](0005-uniffi-swift-rust-transport.md)
- [Roadmap Milestone 9](../../ROADMAP.md#milestone-9-cli-companion-and-production-distribution)
- [Security design](../../SECURITY_DESIGN.md)
- [Retention and maintenance](../RETENTION.md)
- [Binary snapshot format](../SNAPSHOT_FORMAT.md)
