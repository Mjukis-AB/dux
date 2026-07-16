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
neither an engine settings DTO nor an earlier inventory is authority. FFI and
Swift settings presentation remain unimplemented and must call these
synchronous core operations off the main actor.

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
database and enables depth one for exactly three migration-owned mutable guards
only after the complete supported schema fingerprint has passed: these
tombstone update/delete guards plus the schema-v6 review-pin update guard.
Unknown newer schemas remain trigger-disabled and read-only.

Every repository load now validates the current-schema database guard and
queries that exact tombstone before opening the snapshot file. A tombstone
returns a distinct unavailable result and wins over a missing or corrupt file,
while the terminal scan row continues to explain which snapshot originally
existed. The guarded retry path reuses its already-held database guard rather
than reacquiring the connection mutex. This checkpoint deliberately exposes no
production tombstone writer.

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
closes and the durable row expires naturally. Normal Explorer/review close will
call explicit release once those app/FFI surfaces exist. Process liveness is not
used to shorten a lease; expiry is the correctness boundary. Candidate status
and cleanup execution remain separate facts and gain no retention or cleanup
authority from this lease.

An expired lease object can still own its retained file handle even after
another process prunes the durable row. Its load and renewal always report
expiry, including after that prune, and release remains idempotent. The future
app/FFI owner must promptly release or drop the object after expiry or renewal
failure: logical retention may unlink the name, but storage blocks can remain
open until the retained handle closes.

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
not evidence that can be ignored. Recognized temps are reported with unknown
liveness and make accounting unstable: staging intentionally writes after
releasing the writer lock, so the file can still grow. No temp is called
abandoned or reclaimable until a durable live-temp lease/scavenging protocol
exists.

This physical-driven operation is bounded even when immutable history grows
without limit. It therefore cannot enumerate every old missing historical
reference. Exact snapshot loads still surface a requested missing file; a
future diagnostic history pager must be separately bounded and must never feed
cleanup authority without revalidation under the final locks.

Future retention must validate eligibility and commit the tombstone first,
then unlink through a retained, revalidated handle and durably flush the
snapshot directory. A crash between the tombstone commit and unlink leaves a
provable DUX-owned orphan; file-first deletion is forbidden.
Publication and retention share this lock order:

1. SQLite connection mutex;
2. cross-process writer/current-schema lease;
3. snapshot writer lock.

Snapshot retention is not enabled until app/FFI review-lease ownership,
retained-handle deletion, live temporary-file leases, and bounded marker-owned
stage scavenging all exist.
The implemented inventory ranks latest-two and reports cap observations but is
not authority. Tombstone insertion, the final pin/latest-two/cap eligibility
recheck, and retained-file acquisition must occur while holding the database
and snapshot locks in the order above. A prior inventory report is not
authority. A name prefix alone never proves that a temporary or stage directory
belongs to DUX.

## Failure and version behavior

Maintenance requires the current writable schema and the same private storage
validation as every history write. A newer schema is never mutated. Target rows
are fully type/range/enum validated before mutation. A transaction error rolls
back the complete batch. If storage validation fails after commit, DUX compares
the frozen postconditions while retaining the writer lease; an exact match is
adopted, otherwise the result is outcome-unknown and a later retry remains safe.
