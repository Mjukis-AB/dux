# DUX binary snapshot format and storage contract

Status: implemented format version 1, internal and crate-private.

This document freezes the durable full-tree snapshot contract implemented by
`dux-core::persistence::snapshot`. It is an implementation reference for the
completed-only fresh-scan converter and engine publication plus future
Explorer, migration, and retention work. It does not describe the legacy CLI
cache format.

## 1. Authority and trust boundary

A snapshot is sensitive, non-authoritative scan history. It may support UI
rendering, comparison, discovery, or a targeted rescan. It never proves that a
path still exists, identifies a current filesystem object, authorizes a cleanup
plan, or authorizes an effect.

SHA-256 detects accidental corruption and makes the SQLite reference exact. It
does not authenticate a same-user writer, encrypt path data, or make persisted
metadata safe to execute. Any future planner must obtain fresh lossless scan
provenance and live path/volume/rule witnesses independently.

All stored bytes are untrusted on read. The decoder checks bounds before
copying variable-length values, grows the node vector only as node records are
actually read, verifies the checksum, rejects trailing input, and then validates
the complete graph and aggregate semantics.

## 2. File naming and database reference

One completed scan has one deterministic file name:

```text
snapshot-<lowercase SHA-256 of the UTF-8 ScanId bytes>.duxsnapshot
```

The name is exactly 85 ASCII bytes and one path component. SQLite stores the
following tuple atomically on the `scans` row:

- positive snapshot format version;
- losslessly encoded relative file name;
- relative-name encoding tag;
- 32-byte snapshot digest.

The tuple must be entirely null or entirely present. It is valid only on a
`succeeded` scan. The decoded name must exactly equal the deterministic name for
that row's scan ID. A tuple is a reference, not evidence that its file is valid;
loads still open the file no-follow, verify storage identity and permissions,
decode the whole file, and compare its version, digest, and embedded scan ID.

## 3. Integer and checksum conventions

- Every integer is unsigned little-endian unless a field says otherwise.
- Fixed headers contain no native Rust enum discriminants or `usize` values.
- The file is `header || payload || digest`.
- The trailing digest is SHA-256 over the exact header and payload bytes. The
  digest does not cover itself.
- A reader must reach EOF immediately after the 32-byte digest.
- Reserved bytes must be zero.
- A format change that alters any encoded byte, field meaning, invariant, or
  accepted representation requires a new format version and explicit
  migration/invalidation behavior.

Golden encoded lengths and whole-file SHA-256 values are fixed in codec tests
for Unix and Windows so encoder and decoder drift cannot silently agree.

## 4. File header

The fixed header is 96 bytes.

| Offset | Size | Field |
|---:|---:|---|
| 0 | 8 | Magic `DUXSNAP\0` |
| 8 | 4 | Format version, currently `1` |
| 12 | 4 | Header size, exactly `96` |
| 16 | 8 | Payload byte length |
| 24 | 8 | Node count |
| 32 | 8 | Directory count |
| 40 | 8 | Regular-file count |
| 48 | 8 | Root logical-byte aggregate |
| 56 | 8 | Root allocated-byte aggregate or zero when absent |
| 64 | 8 | Capture time: seconds since Unix epoch |
| 72 | 4 | Capture time: nanoseconds, less than 1,000,000,000 |
| 76 | 1 | Aggregate presence flags; bit 0 means allocated bytes present |
| 77 | 1 | Root host encoding |
| 78 | 2 | Reserved zero |
| 80 | 4 | UTF-8 scan-ID byte length |
| 84 | 4 | Root host-value byte length |
| 88 | 8 | Reserved zero |

The payload starts with the scan-ID bytes and root bytes, followed by exactly
`node_count` node records and their optional names. The declared payload must be
exactly consumed.

Host encodings are:

- `1`: lossless Unix `OsStr` bytes;
- `2`: lossless little-endian Windows UTF-16 code units.

Files are host-specific. The current host must be able to reconstruct and
validate the root as an absolute path containing no `.` or `..` component. A
node name uses the same host encoding as the root and must reconstruct as one
non-empty normal component.

## 5. Node record

Nodes use depth-first pre-order. Every fixed node record is 112 bytes and is
immediately followed by its name bytes, except that the root has no name.

| Offset | Size | Field |
|---:|---:|---|
| 0 | 8 | Snapshot-local node ID |
| 8 | 8 | Parent node ID, or `u64::MAX` for the root |
| 16 | 4 | Depth |
| 20 | 1 | Node kind |
| 21 | 1 | Optional-field presence bits |
| 22 | 1 | Name host encoding, or zero for the root |
| 23 | 1 | Reserved zero |
| 24 | 8 | Logical bytes |
| 32 | 8 | Allocated bytes, or zero when absent |
| 40 | 8 | Descendant regular-file count, including self for a file |
| 48 | 8 | Immediate child count |
| 56 | 8 | Modification time seconds, or zero when absent |
| 64 | 4 | Modification time nanoseconds, or zero when absent |
| 68 | 4 | Name byte length, zero only for the root |
| 72 | 8 | Access time seconds, or zero when absent |
| 80 | 4 | Access time nanoseconds, or zero when absent |
| 84 | 4 | Scan-observation flags |
| 88 | 8 | Unix device number, or zero when absent |
| 96 | 8 | Unix inode number, or zero when absent |
| 104 | 8 | Reserved zero |

Optional-field presence bits are:

- bit 0: allocated bytes;
- bit 1: modification time;
- bit 2: access time;
- bit 3: Unix device/inode observation.

An absent optional field requires all of its storage fields to be zero. Access
time remains explicitly unreliable snapshot evidence.

Node kinds are:

- `1`: directory;
- `2`: regular file;
- `3`: symbolic link observation;
- `4`: other entry;
- `5`: error observation.

Scan-observation flags are a known-bit set:

- bit 0: inaccessible;
- bit 1: timed out;
- bit 2: hard-link duplicate;
- bit 3: mount boundary.

Unknown field-presence or scan-observation bits reject the file. A hard-link
duplicate flag is valid only on a regular-file node. A mount-boundary flag is
valid only on a directory node. Unix identity is optional historical evidence,
valid only in a Unix-encoded snapshot, and must not be reused as current object
identity. Windows snapshots currently represent hard-link accounting through
the finalized values and scan flag rather than a Unix identity tuple.

## 6. Canonical graph and aggregate invariants

The decoder accepts only one canonical tree representation:

1. The document contains at least one node.
2. Node IDs equal their zero-based record indexes.
3. Node zero is an unnamed depth-zero directory with no parent.
4. Every later node has exactly the active depth-first parent and a depth one
   greater than that parent.
5. Every directory's declared immediate-child count is consumed exactly.
6. Files, symlinks, other entries, and error entries have no children.
7. A regular file has `file_count == 1`; symlink/other/error nodes have
   `file_count == 0`.
8. Two children of one parent cannot have the same exact encoded name.
9. Directory logical bytes and file counts equal the checked sum of their
   immediate child aggregates.
10. Allocated bytes are canonical: all children known means `Some(exact sum)`;
    any unknown child means `None`; an empty directory is `Some(0)`.
11. The root aggregates equal the file-header totals.
12. Header directory and regular-file counts equal the actual node kinds.
13. All additions are checked for overflow.

This validation prevents a checksum-valid but ambiguous tree from creating
duplicate Explorer paths or inconsistent estimates. It still does not make the
tree current or actionable.

The private scanner adapter accepts only a type-state witness produced by a
fresh completed traversal; cached/client trees and failed or cancelled outcomes
cannot create one. Logical bytes count every observed pathname. Allocation for
regular files sharing one stable identity is assigned once to the exact
lossless lexicographically smallest path and all other observed links carry the
hard-link-duplicate flag with `Some(0)`. Conflicting logical size, allocation,
link count, or modification time makes the whole observed identity group's
allocation unknown. An absent identity cannot support deduplication, so a
multiply linked observation without identity is also allocation-unknown.

Unknown allocation is never replaced with logical size. A directory's exact
allocation is `None` when any child is unknown, while the legacy CLI tree keeps
the checked sum of known child allocation for useful display. Followed file and
directory symlinks are rejected by the v1 converter because the wire cannot yet
preserve their target provenance. Exact host path components, not lossy display
names, supply node names and deterministic depth-first IDs.

## 7. Hard bounds

Version 1 rejects values outside these code-owned limits:

| Value | Limit |
|---|---:|
| Complete file, including digest | 2 GiB |
| Nodes | 5,000,000 |
| Depth | 4,096 |
| Scan ID | 128 UTF-8 bytes |
| Root host value | 65,536 bytes |
| One name component | 1,024 bytes |
| Reconstructed encoded path | 65,536 bytes |

Lengths are checked with overflow-safe arithmetic. A tiny truncated file that
claims the maximum node count fails on its first missing record and does not
reserve memory proportional to the unproven count. A legitimate maximum file
can still require substantial decoded memory. Generated Release fixtures now
measure exact balanced/wide 1M and balanced 5M publication/review behavior; see
`SNAPSHOT_PERFORMANCE.md`. The current decoded Explorer admits the measured 1M
shapes and deliberately rejects 5M before decode under its fixed memory budget.

The wire can encode unsigned 64-bit aggregates, but a completed scan prepared
for durable publication additionally requires every summary count to fit
SQLite's signed 64-bit integer domain. The converter and repository both enforce
that boundary before any snapshot bytes are staged or published.

## 8. Private snapshot store

The store is exactly `<SQLite database parent>/snapshots`. Engine configuration
rejects any other snapshot location. The SQLite owner remains alive while the
snapshot owner is used, retaining the database root's replacement guards.

Corrected first provisioning creates
`<SQLite database parent>/.dux-snapshot-stage-<32 lowercase hex>` inside that
same retained marker-owned root, then atomically publishes it without
replacement to the sibling `snapshots` entry. Pre-correction external stages may
remain at `<SQLite database parent parent>/.dux-snapshot-stage-*`. Their fixed
marker contains no target-root identity, so they are unattributable manual debt
and are never adopted or automatically removed.

The snapshot directory has independent permanent controls:

```text
.dux-snapshot-store
.dux-snapshot.writer.lock
```

Both have exact fixed markers. The directory inventory admits only those
controls, canonical final names, and a bounded set of recognized unique
temporary names. Unknown entries, symlinks/reparse points, special files,
multi-link files, ownership mismatches, unsafe permissions/DACLs, retained
identity changes, or over-budget inventory fail closed.

Unix directories/files are exact mode 0700/0600 even under a restrictive umask.
Snapshot staging occurs inside the exact private DUX root; on macOS that root
must have no extended ACL, and the deny-only publication-parent exception
applies only to initial SQLite-root provisioning.
Windows uses protected current-user-only DACLs,
handle-relative creation/publication, retained volume/file IDs, and no-delete-
sharing directory guards. Published files are closed and reopened read-only by
exact destination identity before higher layers receive them.

Read-only access never provisions or repairs a missing store. Therefore an
older engine that observes a valid newer SQLite schema performs zero snapshot
store writes.

## 9. Atomic publication and SQLite ordering

The global mutation lock order is:

```text
SQLite connection mutex
→ SQLite cross-process writer/compatibility lease
→ snapshot writer lock
```

Release is in reverse order. Snapshot retention must use the same order.

Publication proceeds as follows:

1. Under a current-schema SQLite guard, acquire the snapshot writer lock and
   reserve one unique recognized temp name containing the process ID and a
   128-bit random suffix.
2. Commit one immutable schema-v8 lease row binding that exact temp and final
   name to the running scan, then create the exclusive private file while both
   locks are still held. Row-before-file is deliberate: a crash may leave a
   row without a file, never a new v8 file whose creation was not preceded by
   its row.
3. Acquire an exclusive kernel lock on the staged file before releasing the
   snapshot and database locks. Retain that file lock while streaming the fully
   validated document plus checksum. No shared fixed `.tmp` name exists.
4. Reacquire the current-schema SQLite guard. If compatibility cannot be
   retained, abandon the recognized private temp close-only rather than
   mutating the store during drop or unwinding.
5. Under the snapshot writer lock, require the exact lease row, revalidate the
   whole store and exact retained temp, flush it, and publish with an atomic
   no-replace operation.
6. Flush the snapshot directory, close the writable temp handle, reopen the
   final name read-only, and require its identity to equal the published source.
7. Decode and compare the retained winner's full document and digest. A name
   collision is idempotent only when those facts are exact.
8. Retain the snapshot writer lock while the same SQLite guard atomically
   deletes the exact temp lease and compare-and-sets the running scan to its
   terminal summary and four-field reference, including the optional complete
   candidate evaluation in that transaction.
9. Revalidate the retained file and current schema after the database commit,
   then release the snapshot lock and SQLite guard in reverse order.

The file is always durable before SQLite can reference it. Failure between file
publication and database commit may leave an unreferenced immutable orphan;
retention may later remove only a proven unreferenced orphan. SQLite must never
reference an unpublished file. A potentially ambiguous post-commit storage
failure is reconciled by exact scan ID, status, canonical completion time,
counts, version, name, and digest rather than by retrying with changed facts.

Schema v5 adds an append-only logical-retirement tombstone without changing the
v1 snapshot wire or the immutable scan reference. Before opening a referenced
file, the repository holds a current-schema database guard and performs one
bounded exact-ID lookup. A matching tombstone returns the distinct
`SnapshotUnavailable` repository result before filesystem access. An invalid or
identity-mismatched tombstone is corruption; absence permits the ordinary
retained-file validation and full decode. The sealed cap writer can append only
the complete exact tombstone selected under the final database-before-snapshot
lock boundary; no production path updates or deletes one.

Schema v6 adds an explicit cross-process review lease without changing the v1
wire. A pin binds the exact succeeded snapshot tuple to a stable process owner,
Explorer/cleanup-review purpose, and fixed ten-minute expiry. The sealed
repository acquires the database fence before a locked snapshot open, commits
the pin while both boundaries remain held, and returns the retained read-only
file handle. Load verifies the exact live lease before decoding. Renewal cannot
resurrect expiry; explicit release is exact and idempotent after bounded expiry
pruning; implicit drop performs no database work and relies on expiry. An
expired object must still be dropped to close its retained handle. Candidate
and cleanup-session state never implies an open review. The cap writer rechecks
latest-two and these explicit active leases under the
same lock order before committing a tombstone, keeps the digest-validated
handle live through identity-revalidated deletion, and flushes the directory.

Schema v7 adds a partial SQLite lookup index over the lossless snapshot-name
encoding, snapshot-name bytes, and scan ID for rows with a snapshot reference.
It changes neither the v1 snapshot wire nor immutable scan history and grants
no tombstone or unlink authority.

Schema v8 adds an immutable, bounded temporary-file lease relation without
changing snapshot v1 bytes. A row records one running scan, exact final and
recognized temp names, a random 128-bit lease ID, a strictly decoded process
instance, and creation time. Stored PID/process data is identity evidence only.
Only the kernel lock retained on the staged file establishes current writer
liveness. The insert guard enforces the 64-row ceiling and a running parent;
rows cannot be updated, and a scan cannot transition to `succeeded` until the
exact row is deleted. Failed, cancelled, or interrupted scans may retain a row
as explicit crash debt.

Schema v16 does not change snapshot v1 bytes or the temp-lease relation. It can
classify a separately claimed pristine running scan as same-host/prior-boot
from complete immutable host/boot provenance and change only that scan and its
process claim to interrupted. This history transition deliberately leaves the
temp lease and any physical temp untouched. The terminal-temp batch below must
still independently prove parent state, lease identity, physical quiescence,
and storage facts before it can reconcile that debt.

The read-only retention inventory uses that index only after a single bounded
physical directory pass sequentially opens exact no-follow file handles,
captures identity and handle-derived usage, then closes each entry. After the
indexed SQLite match it sequentially reopens every name and requires immutable
final identity and usage to remain exact. This avoids an entry-count-sized file
descriptor requirement. It reports logical length and platform allocation
separately and conservatively charges their maximum; it does not decode full
snapshot bodies or treat these bytes as content validity. The v8 lease
population plus a nonblocking kernel-lock probe classifies recognized temps as
row-bound active, row-bound quiescent-at-observation, or unleased; row-only
residuals are reported separately. Active and unleased temps keep accounting
unstable, and every physical class remains charged and non-evictable.
Quiescence is not unlink authority. Latest-two/cap results are observations
that the production writer recomputes under the final database and snapshot
lock boundary.
The effective cap comes from the typed `snapshot_retention` database setting;
absence means 2 GiB. Inventory reads it under the current-schema database guard
before taking the snapshot lock. This does not change snapshot v1 bytes, and a
cached settings value never grants retention authority.

One production cap batch removes at most one final. It rebuilds the complete
inventory while retaining the current-schema database guard and snapshot
writer lease. Existing tombstoned physical residuals are selected first. A new
victim is considered only when charged bytes exceed the fresh cap and
accounting is stable; active or unleased temps defer the operation. Latest-two
per exact encoded root, active pins, controls, temps, physical orphans, and
already tombstoned bytes are never normal candidates. Available candidates are
ordered oldest completion, oldest start, then scan ID.

Before either fresh retirement or residual retry, the final is reopened from
the locked observation, required to keep exact filesystem identity and
logical/allocation usage, fully decoded, and matched to the immutable scan ID
and body digest. For a new victim the complete append-only tombstone commits
first and any commit-adjacent failure is adopted only after an exact row match.
The writer then reopens the same observed identity with deletion access,
rechecks retained handle, name, and usage, unlinks it, and syncs the directory.
A schema race after commit leaves a logically unavailable physical residual.
A later batch repeats identity, usage, and full-content validation, so changed
or same-name replacement bytes are not removed. Scan and tombstone history are
never deleted.

Physical-orphan reconciliation is a separate sealed one-final batch, not a cap
fallback. Under the current-schema database guard and then the snapshot writer
lease, it builds a complete bounded typed-final/catalog match and selects only
the deterministic first final with zero exact snapshot references. It retains
the observed identity and logical/allocation usage, fully decodes and
checksum-validates the body, requires the decoded scan ID to derive the exact
filename, and requires an existing parent with the same lossless root and no
snapshot reference. Only `running`, `failed`, `cancelled`, or `interrupted`
parents are admissible; missing, queued, succeeded, referenced, malformed, or
root-conflicting parents leave the final untouched.

The reconciler keeps the validated read handle live, performs a second
delete-capable name/identity/usage validation, applies checked accounting before
mutation, removes at most that final, and syncs the snapshot directory. It
creates no tombstone, changes no scan or temp row, and does not use temp state,
cap policy, latest-two ranking, or review pins as authority. Known pre-unlink
failures are retryable storage failures; uncertainty after unlink is
`OutcomeUnknown`. The typed idle-only engine task accepts no path or candidate,
publishes path-free aggregate observations, invokes one batch, and never
self-enqueues.

An exact same-scan retry is the publication path's temp reconciliation. With
the database guard held before the snapshot writer lock, it returns busy for a
contended kernel lock. It may reopen, identity-revalidate, nonblockingly lock,
unlink, and directory-flush only the quiescent temp named by that scan's exact
row, then delete the row. A compliant creator completes row-before-file
creation before it releases those same locks, so creation cannot still be
pending once maintenance holds both and a row-only residual can be deleted. It
never adopts or removes an unleased temp. Normal abort also removes and flushes
its retained current-call temp before exact row consumption. Unleased-temp
removal uses the separate physical-only boundary below; provisioning stages use
the independent root-local boundary described later in this section. Legacy
external stages remain manual debt.

A separate bounded terminal-temp batch classifies the complete immutable lease
population through joined parent status, but those aggregate observations grant
no mutation. It holds the current-schema database guard before the snapshot
writer lease, inspects that bounded population and physical inventory, skips
active row-bound files, and selects at most the first deterministic row-only or
quiescent residual. The selected row's fully decoded exact parent must be
`failed`, `cancelled`, or `interrupted` with no snapshot reference. `running`
rows, unleased temps, provisioning stages, PID/owner/age evidence, cap state,
pins, and final-snapshot policy cannot authorize this operation.

For a quiescent residual, exact name, identity, logical/allocation usage, and a
second nonblocking kernel lock are revalidated before checked accounting and
physical removal. The delete handle closes before directory sync; post-unlink
durability uncertainty is `OutcomeUnknown` and retains the row. Only after
durable physical removal is the exact row consumed. A row-only residual first
durably confirms the locked directory state, then consumes only its row. The
batch never changes its parent scan or any final, tombstone, pin, or unrelated
history. Its idle-only engine task accepts no identity or path, exposes only
aggregate counts/bytes, performs one batch, and never self-enqueues.

The separate bounded unleased-temp batch owns no database row and fabricates no
scan relationship. While retaining the current-schema database guard before
the snapshot writer lease, it subtracts the complete at-most-64-row immutable
lease-name population from one bounded physical inventory of the marker-owned
private store. A candidate must match the whole generated
`.snapshot-<64 lowercase hex>.<canonical nonzero u32 PID>.<32 lowercase hex>.tmp`
grammar and have no exact case-sensitive row match. Prefix, PID, owner/process
identity, age, mtime, and an earlier quiescent observation are not authority.

Entries are considered by exact lexicographic name. Active entries are skipped
without starving a later quiescent entry; one call removes at most the first
quiescent item. Before removal, the exact name is reopened no-follow and must
repeat its retained identity, logical/allocation usage, private regular-file
protection, one-link count, name-to-handle identity, and a second nonblocking
kernel lock. Counts and charged-byte subtraction are checked before effect.
The delete-capable locked handle is consumed and closed before directory sync;
uncertainty after unlink is `OutcomeUnknown`. The operation performs no SQLite
mutation or adoption and never changes a scan, row-bound lease, final,
tombstone, pin, provisioning stage, or unrelated history.

`SnapshotRepository::reconcile_unleased_snapshot_temp` is reachable only from
the typed idle-only `SnapshotUnleasedTempMaintenance` engine task through
`EngineHandle::start_snapshot_unleased_temp_maintenance`. Its Applying and
Finished events and result getter expose only canonical time, bounded
unleased/active counts, charged bytes, `has_more`, and `NoUnleasedTemp`,
`DeferredActive`, or `Removed { bytes }`. They expose no name, PID, identity,
path, scan inference, or mutation input; one task invokes one batch and never
self-enqueues.

Pre-v8 writers require a platform-specific version-skew qualification. The
current-schema fence prevents an older writer from publishing after v8 wins.
Windows pre-v8 writable temp handles denied delete sharing, so a live older
writer prevents the deletion reopen. Unix permits unlink of an older writer's
open, non-v8-locked inode; that writer may continue only on the detached handle
and later fails name/current-schema publication checks. Avoiding this same-user
availability race requires not concurrently running old and current binaries
on the same private store. The 0700/0600 or protected owner-only DACL boundary
excludes other users but does not make a malicious or incompatible same-user
process part of the threat model.

The retained current-call handle remains narrow rollback authority over its
own exact identity even if its row is concurrently deleted or replaced under
an otherwise valid current schema. Such a mismatch forbids publication; DUX
removes only that retained temp, preserves conflicting metadata, and reports
corruption. Without a valid database/current-schema guard it closes the handle
without mutating the store. This current-call rule is not inventory authority
and cannot be used to adopt an unleased name.

Initial snapshot-directory provisioning uses a private marker-complete
root-local stage and same-parent atomic no-replace directory publication. A
racing winner is reopened and fully validated. The database-root inventory
tolerates at most 64 exact canonical private stage directories within fixed
total-entry and 256-KiB aggregate-name budgets plus sampled elapsed-time checks
against 250 ms. A current-user-owned Unix stage whose mode is a stricter subset
of 0700 can be interrupted creation debt; tolerating it is not cleanup
authority.

The separate provisioning-stage reconciler holds the current-schema database
guard while completely inventorying that retained root under its fixed
2,048-entry bound and the same 256-KiB aggregate-name, 64-stage, and 250-ms
bounds. It considers stages in exact
ASCII lexical order and removes at most one canonical 0700/protected-DACL stage
whose complete child set is either the exact 16-byte store marker alone or that
marker plus the exact 16-byte writer marker. Marker and writer controls must be
private, regular, single-link, no-follow objects whose retained identities still
match their names. An empty or stricter-mode stage is reported as unproven and
never starves a later proven stage. Wrong or partial markers, writer-only,
unknown/extra children, links/reparse points, broader permissions, unsafe DACLs,
or a 65th stage fail the whole batch before effect. Legacy external stages are
never inventoried or adopted.

Removal is non-recursive and ordered writer control, stage-directory sync,
store marker, stage-directory sync, empty stage, then retained-root sync.
Counts and logical/allocation-derived charged control bytes are checked before
the first namespace mutation; directory allocation is deliberately not claimed.
A failure before the first unlink/disposition is retryable, while any failure
after it is `OutcomeUnknown`. A crash after marker removal can leave an empty
unproven directory that future automatic maintenance preserves; eliminating
that bounded debt would require a durable deletion journal. The typed idle-admitted
engine task accepts no root, stage name, path, identity, or victim, performs one
batch, exposes only canonical time and aggregate before/after observations, and
never self-enqueues. `running` scan/temp-lease rows remain separate debt.

Inventory-observed unleased temps may now be removed only through the
independent bounded physical-only boundary above; they are never adopted.

## 10. Compatibility and failure behavior

- Unknown snapshot versions return a distinct incompatible-version result.
- Invalid magic, length, checksum, semantic data, limits, and I/O remain
  separate path-free internal categories.
- Engine startup opens/migrates SQLite first, then validates or provisions the
  snapshot owner before publishing workers.
- A current-to-newer schema race is rechecked under the SQLite writer lease
  before snapshot provisioning and every later mutation.
- Root-local canonical stage-name tolerance is not ownership proof. The sealed
  reconciler additionally requires the retained root, exact marker bytes,
  bounded complete child set, private retained identities, and fresh name
  validation. Missing, partial, wrong markers or unknown children cannot
  authorize removal, and legacy external stages are neither migrated nor
  adopted.
- Existing successful summaries retry only when every frozen completion fact
  and the fully decoded referenced document match exactly.
- A schema-v8 temp row is created before its file and consumed atomically with
  successful scan completion. Kernel-lock contention is live-writer evidence;
  PID, age, and a quiescent observation are not deletion authority.
- A schema-v5 tombstone makes the reference logically unavailable before file
  open; malformed tombstones and missing or corrupt available files fail
  closed. History remains an observation and is not silently rewritten.
- Engine scan admission refreshes current-schema write authority and excludes
  overlapping canonical roots within one engine session. It creates the random
  durable scan ID only after dequeue, exact-reconciles the start, and publishes
  through this repository only after the scanner's completed terminal claim.
  Queued cancellation has no durable row; cancelled, failed, and interrupted
  work has no snapshot reference. This is not a cross-process scan lease.

Future formats must retain read compatibility or explicitly invalidate old
files. An older writer must never occupy a deterministic final name after a
newer database schema has won.

## 11. Deliberately separate future work

This checkpoint does not implement:

- cross-process overlapping-root scan leases or recovery of an unclaimed
  legacy-v8 engine scan left `running`; schema v9 same-scope claimed recovery
  and schema v16 same-host/prior-boot history interruption are implemented
  without snapshot authority;
- unclaimed legacy recovery of `running` temp-lease parents; schema v9
  same-scope and schema v16 prior-boot recovery preserve the exact lease for
  terminal-temp reconciliation, and legacy external provisioning stages remain
  manual debt;
- explicit user clear-data actions;
- native Windows temp/final/provisioning-stage removal and
  sparse/compressed-allocation runtime verification plus bounded accounting
  probes for slow filesystem drivers (Windows stage regressions are compiled
  but have not run on this host);
- a future streaming/indexed representation if interactive Explorer review of
  a measured 5M-node snapshot is required; the current v1 format remains
  persistable while its decoded review is refused before allocation;
- migration from or hardening of the legacy CLI cache.

Those items remain separate roadmap work. None may weaken the immutable
publication, current-schema fence, non-authoritative data boundary, or lock
ordering defined here.
