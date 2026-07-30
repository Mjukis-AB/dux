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

Pressure episodes are retained with the capacity history and are not rolled
up or pruned by the raw-sample cadence. Each Warning or Critical episode keeps
its entry time, optional Healthy-recovery time, and policy revision. A
Warning→Critical change closes the Warning episode and opens a Critical one at
the same observation time; Unknown observations never open or close an
episode. Episode writes occur in the same transaction as the raw sample and
bounded readers reject malformed or overlapping rows. The public reader is
anchored to one accepted capacity observation, returns at most 64 newest-first
records, validates one additional lookahead row before truncation, and treats
an exit recorded after the anchor as still open at that observation. It also
requires every interval to remain within the referenced volume lifetime and
permits only the newest record to be open. An accepted anchor is provable as
either an exact durable raw sample or the volume's exact current `last_seen`
timestamp when hourly cadence suppressed that observation's raw row; an
arbitrary timestamp merely between `first_seen` and `last_seen` is rejected.
A later retention slice will define the episode horizon separately from raw
and daily samples.

Trend readers use the newest raw sample at or before the requested observation
time as the anchor. Twenty-four-hour and seven-day changes compare the newest
raw sample at or before each exact cutoff; if no baseline exists, the change is
unknown rather than extrapolated. The 30-day chart is bounded to one point per
UTC day, prefers the exact daily representative for completed days, and uses
the newest raw observation for the current day. These are presentation facts,
not cleanup estimates or execution authority.

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

## SQLite history-maintenance orchestration

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

FFI contract v17 (carrying the unchanged v5 maintenance API) and the native app
now own this idle boundary. UniFFI exposes
only opaque maintenance tasks and path-free typed aggregates. The in-process
scheduler starts after a 60-second grace, rechecks Low Power Mode and thermal
state, executes at most one batch at a time, completes one fair seven-kind cycle
with scan recovery ordered before terminal-temp reconciliation and one-minute
spacing, then waits six hours. `DeferredBusy`, `has_more`,
deterministic deferrals, retryable failures, and energy denial have separate
bounded delays. Wake/lifecycle signals coalesce but never erase startup grace
or a resource/failure backoff. This scheduling grants no new cleanup authority.

## Running-scan recovery orchestration

Schema v9 records an immutable private process-instance claim in the same
transaction that creates each new `running` scan. Claims are capped at 64 per
exact process owner; owner and reliable-scope indexes keep admission and
discovery bounded without allowing foreign or earlier-boot debt to block a new
owner. Existing v8 running rows remain unclaimed and are never guessed into an
owner.

Schema v15 adds a partial running-row index for a separate observation-only
diagnostic. One read inspects at most 64 unclaimed rows plus one lookahead and
returns only exact-pristine, unexplained, and inspected counts with
truncation. It does not probe a process, inspect a path or temporary folder,
claim or terminalize a scan, or infer owner death from age or PID. Legacy
external snapshot stages are intentionally excluded because their marker
cannot attribute them safely to one store.

Schema v16 adds an immutable nullable provenance tuple to claimed running
scans: separate 32-byte domain-separated stable-host and boot-scope digests
plus the sole `interrupt_only` policy. New claims store the tuple only when the
process owner and both bounded operating-system observations agree. Every
v9–v15 claim remains all `NULL`; migration never attributes old work to the
current host or boot. Partial tuples, wrong storage classes or lengths,
unknown policies, and owner/boot mismatches fail closed.

On macOS, a hardened runtime may deny the boot-session sysctl. New work then
retains an unscoped exact PID/start-token owner rather than failing persistence.
It also retains all-`NULL` execution provenance. It can confirm only an exact
live match: it has no recovery-scope key, is not recoverable, and cannot use
prior-boot interruption.

One idle-only `ScanRecoveryMaintenance` batch reads and fully validates one
global 64-row keyset page plus a lookahead. A complete same-host/current-boot
claim is probed only after releasing the SQLite guard, and only
`DefinitelyGone` is recoverable. A complete same-host/prior-boot
`interrupt_only` claim is history-interruptible without probing its old PID.
Foreign-host and unproven claims are non-executable; a migrated all-`NULL`
claim retains the old probe only when its combined recovery-scope value exactly
matches the current owner. An absent scope can confirm only `Alive` or
`Unknown`, never recovery.

The writer guard is reacquired and one complete pristine claim/scan tuple is
exact-CASed to `interrupted`; a concurrently completed or recovered row is
reported without overwrite. `Alive`, liveness `Unknown`, foreign-host,
unproven, malformed data, newer-schema races, and pre-start clocks never
produce a recovery write. Page cursors are process-local discovery hints, not
durable authority. `has_more` reports a sentinel page or additional recoverable
work in the inspected page, so retained foreign or unproven debt alone cannot
cause an endless immediate retry.

Recovery changes history only. It never opens or removes a snapshot, temp file,
candidate, cleanup plan, or user path. An exact snapshot-temp lease remains
attached after the parent becomes `interrupted`, so the independently sealed
terminal-temp batch owns the later physical-first reconciliation. Supported
macOS and Linux hosts can distinguish same-host prior-boot scan history from a
foreign database. Foreign-host observations remain unchanged, and Windows
remains unable to persist the reliable provenance required for prior-boot
interruption.

UniFFI v49 exposes a separate synchronous, read-only census of this claimed
bookkeeping. One call reads at most 64 claims in global claimed-time order plus
one lookahead and reports only the inspected total,
same-host/current-boot, same-host/prior-boot, foreign-host, stored-unproven, and
current-context-unavailable counts plus `has_more`. The five categories
partition the inspected page exactly. A full stored all-`NULL` tuple is
stored-unproven; a complete stored tuple that cannot be compared because this
process lacks complete current host/boot context is
current-context-unavailable. Partial, malformed, owner-inconsistent, and
unknown-policy tuples in the inspected page or lookahead fail the entire
census. A nonzero current-context-unavailable count can coexist with
stored-unproven claims but not with any of the three categories that require
current host/boot comparison.

This reader is distinct from both the v48 unclaimed-row census and
`ScanRecoveryMaintenance`. It does not probe a process, advance recovery
discovery, select a row, start a maintenance task, write history, enumerate a
filesystem, inspect a snapshot-temp lease, or remove anything. Settings
presents the two bounded censuses independently and does not describe any v49
category as alive, dead, abandoned, recoverable, or actionable. These counts
are DUX bookkeeping plus current operating-system provenance, not retention
victims, disk usage, reclaimable space, or an inventory of user or temporary
files.

## Snapshot-cap engine orchestration

`EngineHandle::start_snapshot_retention` is the sole typed engine edge into the
sealed snapshot-cap writer. It admits a `SnapshotRetention` task only when its
engine session is idle. A closed session returns a typed error, a duplicate
returns `SnapshotRetentionStartOutcome::AlreadyActive` with the exact task ID,
and foreground work or the other maintenance kind returns `DeferredBusy`.
Those preflight outcomes do not inspect SQLite or the snapshot store and do not
allocate a task. After the current-schema compatibility check, submission
repeats idle and duplicate admission under the registry lock.

One admitted task invokes `SnapshotRepository::enforce_retention_cap` exactly
once. It cannot provide a cap, inventory, candidate, scan identity, or path.
The repository rereads policy and rebuilds the complete retention proof while
holding the database guard before the snapshot writer lock, and it removes at
most one exact final. The engine never loops or self-enqueues; `has_more` is an
observation for a later explicitly scheduled idle request, and deferred
unstable accounting must not become a busy retry loop. Independently opened
engine sessions are not globally deduplicated, but their bounded repository
calls serialize through the shared database and snapshot leases and each
rebuilds fresh state after acquiring them.

The immutable `SnapshotRetentionResult` contains the canonical observation
time, cap, charged bytes before and after, `has_more`, and only a path-free
`SnapshotRetentionOutcome`: `UnderCap`, `DeferredUnstable`,
`DeferredNoEligibleSnapshot`, `RemovedTombstonedResidual { bytes }`, or
`TombstonedAndRemoved { bytes }`. The repository's selected scan identity is
discarded. These values report what one bounded DUX-store maintenance attempt
observed; they do not grant cleanup authority or identify user data.

Cancellation and `SnapshotRetentionBatchApplying` are ordered under the task
registry mutex. If cancellation or close wins, the repository is never called.
If Applying wins, the task owns the whole bounded attempt: later cancellation
remains visible intent, while the exact repository success or failure controls
the terminal phase. A successful attempt emits one
`SnapshotRetentionBatchFinished` event and retains one typed result. Failures
use stable path-free clock, schema, contention, unsafe-storage, budget,
corruption, incompatible-snapshot, unavailable, outcome-unknown, and internal
categories.

This core task is not itself scheduling. The native app/FFI owns explicit
review-lease acquire/renew/release/drop and the periodic, wake, energy, and
backoff policy described above; core still performs exactly one independently
revalidated batch for each admitted request and never self-enqueues.

## Snapshot policy

Full-tree snapshots retain the newest two physically present, logically
available succeeded snapshots for each exact encoded scan root plus snapshots
pinned by an active review. “Succeeded” is the terminal snapshot publication
fact; partial/limited scan coverage remains honest metadata but does not make a
published snapshot disappear from retention ranking. The default total cap is
2 GiB. Old unpinned referenced snapshots are eligible oldest-first; latest-two
and active pins remain protected even if protected bytes alone exceed the cap.

The effective cap is now a typed core setting stored only under the exact
`snapshot_retention` key. Value schema 1 is the canonical, deny-unknown JSON
object `{"cap_bytes":<u64>}`. A missing row means the versioned 2 GiB default
and causes no implicit write. Every `u64` value, including zero, is valid policy
input; it never weakens latest-two or active-pin protection. A malformed or
noncanonical current value is corruption, while a newer per-setting schema is
incompatible and is never overwritten. Unknown setting keys are untouched.
Explicitly setting 2 GiB stores that override and preserves user intent if a
later version changes its default; reset removes the row and restores default
provenance.

The shared engine exposes typed, path-free get, set, and reset operations.
Set/reset are explicit configuration writes and exact-reconcile ambiguous
commits; automatic history retention remains forbidden from mutating settings.
The retention inventory reads the effective setting while holding its
current-schema database guard and before acquiring the snapshot lock. A future
writer must repeat that read under its final database-to-snapshot boundary;
neither an engine settings DTO nor an earlier inventory is authority. UniFFI
v51 and native Settings expose only the effective cap, default/stored
provenance, update time, and idempotent set/reset result. Swift independently
validates those records and calls every synchronous core operation off the main
thread. Setting zero or any other value never invokes retention; the sealed
writer still rereads the cap and rebuilds its complete proof under the final
locks.

### Read-only DUX storage footprint

The public footprint observation is a bounded measurement of DUX's active
private database and snapshot stores, not a retention decision. It accepts no
path, name, scan ID, inventory, selector, cap override, or capability and
cannot start maintenance or authorize removal. The database-to-snapshot lock
order remains live for the whole observation. Both inventories are revalidated
before return, and an unsafe replacement, size drift, incompatible schema,
budget limit, or arithmetic inconsistency fails the complete observation
without returning partial totals.

Database usage includes only the retained SQLite main file, the exact
`-wal`/`-shm`/`-journal` sidecars when present, and stable marker, writer,
cleanup, cleanup-ready, and initialization controls. Snapshot usage reuses the
complete bounded retention inventory and separates controls, protected and
retention-eligible available finals, tombstoned residuals, physical orphans,
and active, quiescent, and unleased recognized temporary files. Logical length
and handle-derived allocation are reported independently; charged usage is the
checked sum of each file's `max(logical, allocated)`. Database plus snapshot
usage is the sole additive physical total. Directory-entry metadata and
unattributable interrupted provisioning stages are excluded.

Embedded AI-cache accounting is deliberately non-additive. One bounded,
allocation-constant SQLite pager sums the exact variable-length insight ID,
32-byte input digest, provider, adapter version, optional model label, and
output payload for every strictly validated record, including the expired
subset. It excludes integer fields and SQLite record, page, index, and
fragmentation overhead. The row population has no arbitrary display ceiling;
SQLite VM-work and elapsed-time budgets fail the whole read instead of
returning a partial count. These logical bytes are already inside database
usage, and deleting records does not imply an immediate reduction in the
database file or free-space increase.

The legacy CLI cache is also excluded. Its directory is selected by the caller
through the platform cache convention and contains no current DUX ownership
marker, so neither reporting nor future clearing may claim it until a separate
marker-owned cache migration proves ownership. UniFFI v52 and Swift revalidate
the complete path-free accounting algebra before presenting it. Native
Settings loads on entry and refreshes only on explicit request; a failed
refresh preserves the last complete observation. Its chart contains only the
two additive database and snapshot components, while AI content remains a
separate labelled row.

Terminal scan summaries and their original snapshot references are immutable.
Snapshot retention is therefore not allowed to clear or rewrite that historical
tuple. Schema v5 implements the prerequisite as an append-only
`snapshot_retention_tombstones` table keyed by scan ID and foreign-key-bound to
the complete immutable snapshot identity: succeeded status, completion time,
snapshot version, losslessly encoded relative name, and SHA-256 digest. It does
not backfill old rows. Absence means that the immutable reference may still be
opened; an exact tombstone means that it is logically unavailable even if its
bytes remain. Tombstones cannot be updated or deleted, and malformed or
mismatched rows are corruption rather than evidence of availability. DUX keeps
all trigger programs disabled with depth zero while inspecting an untrusted
database and enables depth one for exactly six migration-owned guards only
after the complete supported schema fingerprint has passed: the tombstone
update/delete guards, the schema-v6 review-pin update guard, and schema-v8's
temp-lease insert/update plus succeeded-scan-with-lease guards.
Unknown newer schemas remain trigger-disabled and read-only.

Every repository load now validates the current-schema database guard and
queries that exact tombstone before opening the snapshot file. A tombstone
returns a distinct unavailable result and wins over a missing or corrupt file,
while the terminal scan row continues to explain which snapshot originally
existed. The guarded retry path reuses its already-held database guard rather
than reacquiring the connection mutex. The production writer described below
can only append the complete exact row selected under the final locked policy
boundary; it exposes no update or delete operation.

Schema v6 defines an active review as an explicit expiring lease, never as a
selected candidate or a planned cleanup session. `snapshot_review_pins` binds
the exact succeeded snapshot identity to a canonical random 128-bit pin ID, a
strict process-instance owner, an `explorer` or `cleanup_review` purpose,
creation/renewal times, and an expiry exactly ten minutes after the last
renewal. Identity, owner, purpose, and creation cannot change. Equality at the
expiry boundary is inactive, and renewal cannot resurrect an expired row. V5
upgrades create no pins because historical state does not prove a live review.

The sealed core repository admits at most 64 live leases for one stable owner
and 1,024 rows for one store. Acquisition generates bounded collision choices
before locking, inspects the complete bounded population with explicit SQLite
type, owner, parent, and tombstone validation, and prunes at most 64 expired
rows. It then validates the exact parent/tombstone state and opens the immutable
file while retaining the snapshot writer lock before committing the pin under
the already-held database fence. The returned non-cloneable object retains the
read-only file handle. Load verifies the exact unexpired row before decoding;
renew uses exact compare-and-swap, and explicit release is exact and idempotent
when pruning already removed that same pin. Acquire,
renew, and release adopt an ambiguous commit only when its complete frozen
postcondition matches.

Dropping a review lease performs no SQLite write because destruction can occur
during unwinding or while another persistence lock is held. The retained handle
closes and the durable row expires naturally. The app's actor-owned Explorer
controller now renews every five minutes and on wake/time change, explicitly
releases on review close and shutdown, and drops/releases on renewal failure.
Process liveness is not used to shorten a lease; expiry is the correctness
boundary. Candidate status and cleanup execution remain separate facts and gain
no retention or cleanup authority from this lease.

An expired lease object can still own its retained file handle even after
another process prunes the durable row. Its load and renewal always report
expiry, including after that prune, and release remains idempotent. The native
owner promptly releases or drops the object after expiry or renewal failure;
FFI engine close also invalidates renewal and attempts exact release for every
still-live registered session before core close. A failed durable release
expires naturally. Logical retention may unlink the name, but storage blocks
can otherwise remain open until the retained handle closes.

Schema v7 adds only the production lookup index needed to reconcile a bounded
snapshot-directory inventory with immutable scan history. The partial
`scans_by_snapshot_path` index orders by the lossless relative-name encoding,
relative-name bytes, and scan ID, and excludes rows with no snapshot reference.
It does not select retention victims, insert tombstones, unlink files, or grant
cleanup authority.

The sealed read-only inventory now acquires the current-schema database fence
before one snapshot writer lease and holds both through reconciliation. A
single bounded directory walk sequentially opens every accepted no-follow final
or recognized temp, captures identity plus logical length and filesystem
allocation, then closes that entry handle. After SQLite reconciliation it
sequentially reopens and identity-validates every name; immutable final usage
must still match exactly, while temp usage remains point-in-time. This avoids
making the 2,048-entry bound require 2,048 descriptors. Allocation comes from
the validated handle (`st_blocks * 512` on Unix and `FILE_STANDARD_INFO` on
Windows). The cap charges `max(logical, allocated)` because sparse,
compressed, cloned, and platform-specific files can make either observation
the larger conservative value. Both observations remain visible. The two
fixed controls count toward the store footprint; directory-entry metadata does
not. Checked arithmetic rejects overflow.

Each physical final is matched through the v7 index to zero or one strictly
decoded history row. Zero is a physical orphan. More than one, a malformed
row, a noncanonical encoded root, or an identity/path mismatch is corruption.
Grouping uses the exact stored `(root encoding, root bytes)` tuple and never
filesystem canonicalization. Available present rows receive deterministic
latest-two ranks by completion descending, start descending, then scan ID;
otherwise eligible observations are ordered completion/start ascending then
scan ID. Tombstoned-present bytes and orphans are reconciliation debt, not
normal eviction candidates.

The same observation strictly decodes all at most 1,024 review pins without
pruning. Expiry equality is inactive. An active pin protects its exact present
snapshot; an active pin with a tombstone or missing named file is corruption,
not evidence that can be ignored.

Schema v8 adds the durable coordination half of the temporary-file protocol.
The table holds at most 64 rows; each immutable row binds one running scan to
its deterministic final name, unique recognized temp name, random 128-bit lease
ID, strictly decoded process-instance observation, and creation time. The PID
embedded in the temp name and the stored process instance are identity checks,
never liveness or expiry authority. V7 migration creates no rows. A scan cannot
become `succeeded` while its row remains, but failed, cancelled, and interrupted
parents may retain an immutable row as explicit recovery debt.

Creation keeps the global database-before-snapshot lock order. Under a
current-schema database guard and snapshot writer lock, DUX reserves the exact
name and commits the row before creating the exclusive private file. Creation
finishes before either lock is released. The staged file then retains an
exclusive kernel lock while encoding continues outside the store-wide locks.
This makes row-without-file a valid crash residual and prevents a compliant
writer from creating that file later after a maintenance observer has acquired
both locks. Drop and unwinding close the handle only; neither enters SQLite nor
removes a name.

Inventory now classifies recognized temps by the complete bounded row set and
a nonblocking kernel-lock probe. A row-bound file whose lock is contended is
`active`; one whose lock was acquired at observation is
`quiescent_at_observation`; a physical recognized temp with no row is
`unleased`; and a row with no physical file is reported separately as a
residual lease. Active and unleased files make accounting unstable. Quiescence
is only a point-in-time observation, not cleanup authority. All three physical
classes are charged and excluded from normal snapshot victims.

The publication path's physical temp reconciliation is an exact same-scan
retry. While retaining the current-schema database guard and then snapshot
writer lock, it rejects an active file as busy. For a quiescent row-bound file it
reopens the exact name, requires the observed identity, acquires the kernel lock
nonblockingly, revalidates the retained file and name, unlinks it, and flushes
the directory before deleting the exact row. A row-without-file residual may
have its exact row deleted because row-before-file creation cannot still be
pending after both locks are acquired. An unleased temp is never adopted or
removed by this path. Normal abort is also physical-first: it removes and
flushes the retained current-call temp before consuming the row. Successful
publication instead makes the immutable final durable first, then atomically
deletes the exact temp lease with the succeeded scan summary and optional
evaluation while the snapshot writer lock remains held. A failed transaction
retains both the running scan and lease input; exact post-commit reconciliation
never treats a conflicting row as success.

A staged handle still retained by its creating call is a narrower rollback
capability than inventory. If its row is missing or conflicting under an exact
current-schema guard, publication is forbidden, but DUX removes only that
handle's identity-matched, kernel-locked temp, leaves any conflicting row
unchanged, and returns corruption. This does not adopt or sweep an observed
unleased temp. If the database/current-schema guard cannot be established, the
handle is closed without mutation instead.

This publication protocol is not a general scavenger. The separate bounded
terminal-row reconciler below can consume failed, cancelled, or interrupted
row-bound debt, and the independent unleased-temp reconciler below can remove
one fully proven physical-only item. None of these paths settles a running scan
or touches `.dux-snapshot-stage-*` provisioning siblings. Native Windows
runtime coverage of the kernel-liveness and retained-temp removal paths is
still required; the implementation does not claim that verification from Unix
tests.

This physical-driven operation is bounded even when immutable history grows
without limit. It therefore cannot enumerate every old missing historical
reference. Exact snapshot loads still surface a requested missing file; a
future diagnostic history pager must be separately bounded and must never feed
cleanup authority without revalidation under the final locks.

### Terminal snapshot-temp reconciliation contract

Terminal-temp maintenance is a separate sealed capability for row-bound crash
debt. One batch holds the current-schema database guard and then the snapshot
writer lease while it decodes the complete at-most-64-row immutable lease
population and one bounded physical inventory. Aggregate counts classify rows
from the lease query's joined parent status and are observations only. Before
any effect, the first actionable row's exact parent is fully decoded and must
be `failed`, `cancelled`, or `interrupted` with no snapshot reference.
`running`, missing, queued, succeeded, malformed, or snapshot-bearing selected
parents grant no mutation. Stored owner identity, PID, creation time, age, and
apparent quiescence never terminalize a scan or prove writer death.

Rows are considered in deterministic immutable order. An active row-bound temp
is counted but skipped so it cannot starve a later actionable row; if every
terminal residual is active, the batch returns `DeferredActive` without
mutation. For the first actionable row, a row-without-file residual may consume
only its exact lease after the locked directory state is durably confirmed.
Row-before-file creation cannot still be pending once both locks are held. A
quiescent physical temp is reopened no-follow, required to match the observed
identity and exact logical/allocation usage, kernel-locked nonblockingly again,
and name/private-object revalidated. Checked count and charged-byte
postconditions are frozen before physical removal.

For a physical residual, removal always precedes row deletion. The
delete-capable handle is consumed and closed before the snapshot directory is
synced, including on Windows where POSIX disposition takes effect at handle
close. A known pre-unlink error leaves both file and row intact. Once unlink
succeeds, directory-durability uncertainty is `OutcomeUnknown` and the exact
row remains retry debt. After durable removal, failure to prove the exact row
deletion also becomes `OutcomeUnknown`; a later batch can safely converge from
a row-only residual. No scan, final, tombstone, pin, candidate, cleanup,
evaluation, or unrelated temp row is changed.

The repository API is
`SnapshotRepository::reconcile_terminal_snapshot_temp_residual`. The typed
idle-only engine starts through
`EngineHandle::start_snapshot_terminal_temp_maintenance`, publishes
`SnapshotTerminalTempMaintenanceBatchApplying` and
`SnapshotTerminalTempMaintenanceBatchFinished` events, and exposes the typed
`SnapshotTerminalTempMaintenanceResult` through
`EngineHandle::snapshot_terminal_temp_maintenance_result`. Applying is the
cancellation point of no return, one task performs exactly one repository call,
and core never loops or self-enqueues. Results distinguish
`NoTerminalResidual`, `DeferredActive`, `ReconciledRowOnly`, and
`RemovedTemp { bytes }`, with only canonical time, terminal/active counts,
charged bytes, and `has_more`. Scan IDs, lease IDs, owners, names, roots, and
paths stay private. Native scheduling must apply backoff to active-only debt.

This authority never adopts an unleased temp, touches a provisioning stage, or
itself recovers a `running` scan. The preceding scan-recovery boundary must
first prove exact same-scope process death and terminalize the parent without
touching the lease; terminal-temp maintenance then retains sole residual-removal
authority.

### Unleased snapshot-temp reconciliation contract

Unleased-temp maintenance is a separate sealed physical-only capability. It
does not infer a parent or adopt a temp into SQLite. One batch holds a
current-schema database guard before the snapshot writer lease, inspects the
complete at-most-64-row immutable temp-lease population, and reconciles it with
one complete bounded physical inventory from the retained marker-owned private
snapshot store. A physical entry is a candidate only when its whole name
matches DUX's generated
`.snapshot-<64 lowercase hex>.<canonical nonzero u32 PID>.<32 lowercase hex>.tmp`
grammar and that exact case-sensitive name is absent from every inspected lease
row. Prefix, PID, owner/process identity, age, mtime, and quiescence observed by
an earlier pass are never authority, individually or together.

The full removal proof additionally requires a fresh no-follow open of the
same retained name, exact filesystem identity and logical/allocation usage,
the private regular-file mode or protected owner-only DACL, exactly one link,
name-to-handle identity, and a second nonblocking exclusive kernel lock. The
database guard, snapshot writer lease, complete row set, and bounded physical
inventory remain live through that proof and effect. Any missing, malformed,
over-limit, row-bound, renamed, replaced, resized, relinked, non-private, or
lock-contended evidence fails closed.

Candidates use exact lexicographic name order. One call skips active entries so
they cannot starve a later quiescent item and removes at most the first
quiescent candidate. An empty set returns `NoUnleasedTemp`; active-only debt
returns `DeferredActive`, sets `has_more`, and requires scheduler backoff.
Before the OS effect, the batch checks the exact after-count and charged-byte
subtraction. It then consumes and closes the locked delete-capable handle before
syncing the snapshot directory. This ordering includes Windows POSIX
disposition, where closing the handle applies the name removal. A failure known
to precede unlink is a no-effect storage failure; uncertainty after unlink or
during directory durability is `OutcomeUnknown`. Successful removal updates
only the retained physical inventory. No SQLite row is inserted, updated, or
deleted; no temp is adopted or mapped to a scan; and no scan, lease, final,
tombstone, pin, provisioning stage, or unrelated history is changed.

The pre-v8/version-skew boundary is explicit. Once the current-schema fence has
won, an older DUX writer cannot pass its later schema/name revalidation and
publish a final or history reference. On Windows, pre-v8 writable staging
handles also denied delete sharing, so an actually live old writer prevents the
delete-capable reopen even though it did not participate in the v8 kernel-lock
protocol. Unix permits unlinking an open inode: an old live writer without the
v8 advisory lock can continue writing only its detached handle, and its later
publication fails because the name and current schema no longer match. That is
an acknowledged same-user availability race, not authority to corrupt a final
or SQLite. The store's 0700/0600 ownership checks or Windows protected
owner-only DACL exclude other users; they do not defend against a malicious or
incompatible process running as the same user. Operators must not run pre-v8
and current DUX binaries concurrently against the same private store when that
availability guarantee matters.

The repository API is
`SnapshotRepository::reconcile_unleased_snapshot_temp`. The typed idle-only
engine starts through
`EngineHandle::start_snapshot_unleased_temp_maintenance`, publishes
`SnapshotUnleasedTempMaintenanceBatchApplying` and
`SnapshotUnleasedTempMaintenanceBatchFinished`, and exposes its immutable
result through
`EngineHandle::snapshot_unleased_temp_maintenance_result`. Applying is the
cancellation point of no return. One task performs exactly one repository
call, accepts no name, PID, scan, lease, identity, path, inventory, or
candidate, publishes only canonical time, aggregate unleased/active counts,
charged bytes, `has_more`, and `NoUnleasedTemp`, `DeferredActive`, or
`Removed { bytes }`, and never loops or self-enqueues.

Focused tests cover exact row-bound exclusion from physical candidates,
deterministic one-item removal, exact accounting, active skip/deferral and
convergence, stage exclusion, rejection of noncanonical PID name forms, invalid
time, pre/post-effect classification, exact-temp-only storage mutation,
path-free engine outcomes and redaction, explicit rescheduling,
idle/cross-maintenance admission, cancellation/close, stable failure mapping,
and independently opened sessions. Separate lease-table tests prove the
complete 64-row bound. Unix executes the unlink path; native Windows
compile/runtime mutation-path verification remains open.

This capability does not scavenge provisioning stages, recover a `running`
scan or its row-bound temp, or clear the store. The app/FFI scheduler can only
request this sealed one-batch operation; it cannot choose or widen its target.

### Snapshot provisioning-stage reconciliation contract

Provisioning-stage maintenance is a separate sealed physical-only capability.
The repository retains the current-schema database guard that excludes a
compliant concurrent provisioner, then completely inventories raw/native names
under the retained marker-owned database-root handle. Every entry counts toward
fixed total-entry, 256-KiB aggregate-name, 64-stage, and 250-ms limits without
requiring unrelated host names to be UTF-8. Only exact
`.dux-snapshot-stage-<32 lowercase hex>` names are considered, in exact ASCII
lexical order. Pre-correction external siblings are outside this root and are
never adopted or removed.

A proven stage is an exact current-user-owned 0700 Unix directory (and, on
macOS, has no extended ACL), or a protected current-user-only Windows DACL
directory with no reparse shape, whose complete
bounded child set is the exact 16-byte store marker alone or that marker plus
the exact 16-byte writer marker. Each control must be a private, single-link,
no-follow regular file; its retained identity must still match its name and its
marker bytes and logical/allocation usage must be exact. Empty stages and Unix
owner-owned modes stricter than 0700, including mode 000 creation crashes, are
unproven and deferred without starving a later proven stage. Partial/wrong
markers, writer-only or extra children, links/reparse points, broader
permissions/DACLs, changed identities, or exceeded/incomplete inventories fail
the entire batch before effect. Name, prefix, PID, age, mtime, owner, and
private permissions alone never authorize mutation.

One call removes at most the first lexical proven stage and never recurses.
Checked before/after counts and exact marker/writer charged bytes are frozen
before effect; no directory allocation is claimed. Marker-complete removal is
ordered writer control, stage sync, store marker, stage sync, empty directory,
then retained-root sync. Marker-only removal begins at the marker step. Delete
handles close before their durability flush. A known failure before the first
namespace mutation is `BeforeEffect`; any failure afterward is
`OutcomeUnknown`. A crash after marker removal can leave an empty unproven
stage that later automatic batches intentionally preserve; a durable deletion
journal would be required to reclaim that state safely.

The repository API is
`SnapshotRepository::reconcile_snapshot_provisioning_stage`. The typed
idle-admitted engine starts through
`EngineHandle::start_snapshot_provisioning_stage_maintenance`, publishes
Applying/Finished events, and exposes its immutable path-free result through
`EngineHandle::snapshot_provisioning_stage_maintenance_result`. It accepts no
root, stage name, path, identity, inventory, scan, lease, cap, or victim;
performs exactly one repository call; exposes only canonical time, aggregate
before/after counts and control bytes, `has_more`, and `NoStage`,
`DeferredUnproven`, `RemovedMarkerOnly`, or `RemovedMarkerComplete`; and never
loops or self-enqueues. Applying is the cancellation/close point of no return.
Native Windows deletion/DACL/reparse/cap regressions are present and
cross-compiled, but this checkpoint has not run them on a Windows host.

Production cap enforcement is a sealed, one-final-per-call repository batch.
It acquires the current-schema database guard before the snapshot writer lease,
freshly loads the cap, and rebuilds the complete physical/history, pin, and temp
lease inventory inside that mutation boundary. A pre-existing physically
present tombstoned residual is selected before any new victim, even when the
store is now below cap. Otherwise a new tombstone is considered only when the
charged store total exceeds the cap and accounting is stable: an active or
unleased temp defers the batch. The deterministic oldest available observation
is eligible only when it is outside the latest two for its exact losslessly
encoded root and has no active review pin. Protected, orphan, temporary,
control, and already tombstoned bytes are never normal victims. One batch never
removes more than one final.

The final inventory observation is still not sufficient by itself. Before
retiring either a fresh victim or an existing tombstoned residual, DUX reopens
the exact observed final read-only, requires the same filesystem identity and
logical/allocated usage, fully decodes the bounded snapshot, and matches its
scan ID and digest to immutable history. For a fresh victim it then prepares
the complete succeeded parent tuple and commits one append-only tombstone. A
commit-adjacent failure is adopted only when the complete stored row is exact;
schema uncertainty returns outcome-unknown without touching the file. Only
after an exact durable tombstone does DUX reopen the observed identity with
deletion access while keeping the digest-validated read handle live, repeat
identity, name, and usage checks, use descriptor-relative name unlink on Unix
or handle disposition on Windows, and durably flush the snapshot directory.
File-first deletion is forbidden.

A crash or failure after tombstone commit leaves a logically unavailable
physical residual. A later batch gives such debt priority but again requires
the freshly observed identity, unchanged usage, and full snapshot digest before
unlink. A same-name replacement after validation or a changed body therefore
remains untouched by that batch; a later batch must observe and fully validate
the then-current exact bytes before it can remove them. Malicious same-user
name substitution during the final Unix syscall window is outside the private-
store isolation guarantee documented in `SECURITY_DESIGN.md`.
The tombstone and immutable scan history are retained permanently; successful
physical removal never deletes either row. If unlink succeeds but directory
sync is uncertain, the durable tombstone still prevents future loads and a
later inventory safely retries only if the name is present again.
Publication and retention share this lock order:

1. SQLite connection mutex;
2. cross-process writer/current-schema lease;
3. snapshot writer lock.

Core cap enforcement, retained-handle final deletion, exact tombstoned residual
handling, typed one-batch engine invocation, app/FFI review-lease ownership,
same-scope hard-process-death scan recovery, and native periodic idle scheduling
with bounded backoff are implemented.
Physical-orphan reconciliation is the separate implemented capability below,
and terminal row-bound temp reconciliation is the separate implemented
capability above. Unleased physical-temp and exact-marker-owned root-local
provisioning-stage reconciliation are separate implemented capabilities above.
Broad/unproven stage scavenging and explicit clear-data actions remain future
maintenance capabilities. A prior
inventory report is never authority; each writer
recomputes every proof under the lock order above. A name prefix alone never
proves that a temporary or stage directory belongs to DUX.

### Physical-orphan reconciliation contract

A physical orphan is an accepted typed final name with no matching snapshot
reference in the current database. That zero-match classification is only a
bounded observation. Orphan maintenance is separate from cap enforcement:
orphans are never normal cap victims, latest-two or pin policy does not grant
their removal, and a malformed cap, pin, or temp row must not become deletion
authority.

One sealed orphan batch may select only the deterministic first typed orphan
from a complete bounded physical-final/catalog reconciliation. It holds the
current-schema database guard before the snapshot writer lease continuously
through proof and mutation. It then retains the exact observed identity and
usage, fully decodes the snapshot wire and checksum, and requires the decoded
scan ID to derive the exact observed filename. The exact scan parent must
exist, its lossless root must match the decoded root, and it must have no
snapshot reference. `running`, `failed`, `cancelled`, and `interrupted` parents
are admissible; `queued`, `succeeded`, missing, referenced, malformed, or
root-conflicting parents are corruption and leave the final untouched.

Allowing a `running` parent does not preempt a live publication. Publication
retains the same snapshot writer lease through its database compare-and-set,
so orphan maintenance cannot own that lease while a compliant publisher is
between final publication and reference commit. A later retry must reacquire
the locks and can recreate or exactly validate its final. Active, quiescent,
and unleased temporary files therefore neither authorize nor defer removal of
a separately proven orphan final. The batch never consumes a temp lease,
settles a scan, writes a tombstone, or changes any history row.

After a checked pre-mutation accounting update, the batch keeps the
digest-validated read handle live, reopens the exact observed identity with
deletion access, repeats name/identity/usage validation, removes at most that
one final, and durably syncs the snapshot directory. A failure known to precede
unlink remains a retryable storage failure. Once unlink has succeeded, any
remaining directory-durability uncertainty is reported as `OutcomeUnknown`,
never as success or a definite no-effect failure. A later bounded inventory
converges from the physical namespace; no tombstone may be fabricated for an
unreferenced final.

The engine entry point is likewise separate and idle-only. It accepts no path,
scan ID, filename, root, cap, inventory, or candidate. Public events and results
discard the private scan ID/name and expose only canonical time, bounded orphan
counts/charged bytes, removed bytes, and whether another explicit idle request
may be useful. Core never loops or self-enqueues. The opaque FFI task is part of
the native seven-kind idle-maintenance scheduler; that scheduler supplies only
cadence and does not broaden this repository authority.

The sealed repository entry point is
`SnapshotRepository::reconcile_physical_orphan`. The corresponding engine API
is `EngineHandle::start_snapshot_orphan_maintenance`; its
`SnapshotOrphanMaintenanceBatchApplying` event is the cancellation point of no
return, and `SnapshotOrphanMaintenanceBatchFinished` publishes either
`NoOrphan` or `Removed { bytes }`. The immutable result also reports the
canonical observation time, before/after orphan counts and charged bytes, and
`has_more`. Admission is session-local and duplicate-safe and succeeds only
when no foreground or maintenance work is queued or running at the observed
idle boundary. Later storage mutations remain serialized by repository locks;
the task does not block subsequently submitted foreground work. One task
performs exactly one repository call.

## Failure and version behavior

Maintenance requires the current writable schema and the same private storage
validation as every history write. A newer schema is never mutated. For
database-mutating history-retention and snapshot-cap batches, target rows are
fully type/range/enum validated before mutation, a transaction error rolls back
the complete transaction, and a commit-adjacent failure is adopted only after
the frozen durable postconditions match exactly under the retained writer
lease. Otherwise the result is outcome-unknown and a later retry remains safe.
Physical-orphan reconciliation performs no SQLite transaction and changes no
row; its sole mutation is one proven final unlink, with guaranteed pre-effect
failures kept distinct from post-unlink `OutcomeUnknown`.
Unleased-temp reconciliation likewise performs no SQLite mutation: the exact
absence of its generated name from the complete bounded lease population is
re-proven under the retained database and snapshot locks, and its only effect
is one identity/usage/private/name/one-link/kernel-lock-revalidated temp unlink.
