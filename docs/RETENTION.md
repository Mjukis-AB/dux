# DUX retention contract

This document defines the retention policy and mutation boundary for DUX-owned
history and snapshot storage. Retention removes only bounded, validated DUX
artifacts. It is not cleanup of user data and grants no cleanup authority.

## Capacity history

DUX keeps raw capacity observations for 30 exact 24-hour periods. A raw sample
whose timestamp is exactly the cutoff remains eligible; only older samples are
pruned.

Daily history uses UTC buckets and is retained for 365 complete UTC days. A
daily row is not an average or estimate. It copies the last raw observation in
that UTC day exactly—total bytes, ordinary available bytes, optional important-
usage available bytes, and pressure—and uses the day's UTC midnight as its
bucket key. This preserves valid field relationships and keeps an unavailable
important-usage value as `NULL`. Later pressure episodes, rather than the daily
row, preserve intraday transitions and extrema.

Before maintenance prunes a raw sample whose day is still inside the daily
window, the exact daily representative must already exist or be created in the
same transaction. Thus every day is rolled up no later than its first raw row
aging out of the 30-day window. A future prompt-rollup producer may create the
same deterministic row earlier, but must use a source-row-bounded cursor so a
dense already-covered day cannot starve other retention work. An existing daily
row that differs from the deterministic representative is corruption;
maintenance rolls back instead of overwriting it. Raw observations older than
the daily window may be removed without creating an immediately expired
rollup.

Each maintenance invocation has fixed row and SQLite VM/deadline limits. It
returns whether more eligible work remains so an idle background caller can
schedule another batch. It never loops over an unbounded history in one writer
transaction.

## AI insight cache

AI insights are cache records, not history or authority. Their producer assigns
an expiration, initially 30 days from creation. Maintenance removes an insight
when `expires_at` is equal to or earlier than the observed maintenance time.
Changing the redacted input digest creates a different cache identity. A
separate explicit clear-cache action will remove unexpired insights later.

## History that automatic retention cannot delete

Automatic retention never deletes or rewrites cleanup sessions, cleanup items,
cleanup evidence or warnings, rule outcomes, candidate history, scan summaries,
or schedules. Cleanup history remains until a separate user-initiated clear-
history action is designed and confirmed. The SQLite maintenance authorizer
allows inserts/deletes only for capacity rows and deletes only from the AI cache
table; the module exposes no general SQL surface, and its sole AI delete
statement additionally requires the exact selected ID and expiration cutoff.

## Engine orchestration

`EngineHandle::start_history_maintenance` is the only public core entry point
for automatic SQLite retention. It is deliberately the engine's lowest-
priority task: a closed session returns a typed error, an already-active task
returns that task's exact ID, and a session with running or queued foreground
work returns `DeferredBusy`. These preflight outcomes do not touch SQLite or
allocate a task record. After an eligible preflight, submission rechecks both
current-schema write compatibility and idle admission so the earlier
observation cannot become authority through a race.

One engine task samples the maintenance time on its worker and invokes exactly
one bounded SQLite batch. The result contains only the canonical observation
time, mutation counts, and `has_more`; it contains no paths or retained row
identities. `has_more` is a hint for the app to request another task at a later
idle boundary. Core never self-enqueues a drain loop or extends one writer
lease across batches. Multiple engine sessions may request work against the
same store; the store coordinator and cross-process writer lease serialize the
individually bounded, idempotent transactions.

Cancellation and the `HistoryMaintenanceBatchApplying` event are ordered under
the task registry mutex. If cancellation or close wins that ordering, the task
terminates Cancelled without opening the retention transaction. If Applying
wins, it is the point of no return: a later cancellation remains visible as
intent, but the exact committed or failed store outcome determines the terminal
phase. A successful task publishes one finished event and immutable typed
result. Expected failures are stable, path-free categories; a panic becomes an
internal task failure and always releases exclusive admission.

The native app/FFI idle scheduler is not wired yet. It must treat `DeferredBusy`
and `has_more` as rescheduling hints, apply its own wake/energy policy, and must
not turn maintenance into foreground or cleanup authority.

## Snapshot policy

Full-tree snapshots retain the newest two complete snapshots for each exact
encoded scan root plus snapshots pinned by an active review. The default total
cap is 2 GiB. Old unpinned snapshots are eligible oldest-first; latest-two and
active pins remain protected even if protected bytes alone exceed the cap.

Terminal scan summaries and their original snapshot references are immutable.
Snapshot retention is therefore not allowed to clear or rewrite that historical
tuple. Before enabling referenced-snapshot eviction, a migration and accepted
design must add a separate, mutable availability registry or tombstone keyed by
the exact immutable snapshot identity. Retention must commit the availability
transition first, then unlink through a retained, revalidated handle. History
continues to explain which snapshot originally existed while loaders consult
the availability state before opening it. A crash between the registry commit
and unlink leaves a provable orphan; file-first deletion is forbidden.
Publication and retention share this lock order:

1. SQLite connection mutex;
2. cross-process writer/current-schema lease;
3. snapshot writer lock.

Snapshot retention is not enabled until active-review pins, the separate
availability representation and migration, retained-handle deletion, live
temporary-file leases, the typed size cap, and bounded marker-owned stage
scavenging all exist. A name prefix alone never proves that a temporary or stage
directory belongs to DUX.

## Failure and version behavior

Maintenance requires the current writable schema and the same private storage
validation as every history write. A newer schema is never mutated. Target rows
are fully type/range/enum validated before mutation. A transaction error rolls
back the complete batch. If storage validation fails after commit, DUX compares
the frozen postconditions while retaining the writer lease; an exact match is
adopted, otherwise the result is outcome-unknown and a later retry remains safe.
