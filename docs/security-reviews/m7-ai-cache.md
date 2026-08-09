# M7 sealed AI-cache security review

- Review date: 2026-08-09
- Reviewed commit: the commit containing this document
- Reviewed base: the completed uncached M7 authority-isolation checkpoint
- Scope: SQLite schema v19, sealed cache admission/lookup, FFI v61, native
  explicit-Explain integration, and the Storage & Privacy full-cache clear
- Result: approved for non-authoritative AI presentation caching only

## Relationship to the earlier review

[`m7-ai-authority-isolation.md`](m7-ai-authority-isolation.md) remains the
historical review of the uncached M7 flow. It correctly records that the prior
checkpoint was memory-only and did not approve persistence. This review covers
the later cache-specific edges and does not rewrite that evidence.

## Conclusion

The implemented cache preserves the earlier one-way authority graph. It stores
only bounded canonical output that Rust already accepted as inert presentation,
binds every interpretation-relevant revision, and treats the stored document
as untrusted on every hit. Lookup occurs only after the existing explicit
**Explain selection** action. A hit performs no credential access or network
request and is reparsed, revalidated, and remapped through the fresh retained
privacy proof. Matching corruption fails closed. Cache writes are best-effort
and cannot change provider success or trigger a retry.

The explicit clear is a separate Settings-only storage operation over the
complete cache population. It has no target selector, no provider edge, and no
candidate, plan, cleanup, Trash, scheduler, CLI, or filesystem authority. It
does not compact SQLite and makes no reclaimed-space claim.

## Durable schema and identity

Checksummed migration 19 drops and recreates only `ai_insights`. The former
16-MiB reserved shape never had an admitted producer, and its rows are
deliberately discarded instead of being trusted or upgraded. Every non-AI
table remains intact. The new table is strict and constrains:

- a 32-byte input digest;
- positive privacy-policy, input-schema, input-digest, output-schema, and
  adapter revisions;
- non-empty bounded provider, adapter identity, and exact model revision;
- a non-empty canonical output payload of at most 64 KiB; and
- creation/expiration milliseconds with expiration exactly 2,592,000,000 ms
  (30 days) after creation.

The unique lookup identity contains the digest and every revision/provider/
adapter/model field above. A changed field is a miss; it cannot reinterpret an
older row under a new contract. An unexpired exact identity is first-writer
stable. Only the sealed writer can replace the expired exact identity.

The stored payload is the canonical validated inner output document. It is not
the provider envelope and contains no credential, request header, endpoint,
source path/name, source scan/node identity, freshly projected snapshot node
ID, cleanup target, command, or execution capability. Variable-length cache
accounting remains a non-additive logical subset of the SQLite footprint.

## Lookup and fresh projection

Preparing or viewing the metadata disclosure does not read the cache. The
native explanation session calls the exact local lookup only after the user
presses **Explain selection**, while the same retained preview remains current.
The binding is derived inside Rust from that preview and fixed adapter
constants; callers cannot provide a digest, revision, provider, model, cache
key, row ID, or selector.

On a hit, Rust parses and completely validates the stored canonical document
against the live preview's sealed privacy proof. Request-local group IDs are
resolved through that fresh proof to the current snapshot node IDs. No stored
source or projected ID can become authority. FFI returns only the freshly
projected inert explanation and honest creation/expiration times. The native
path checks the same trusted provider/revision/digest/review binding before
presentation.

The lookup occurs before constructing the orchestrator. A hit therefore reads
no DUX provider Keychain item, creates no request, and starts no network work.
An ordinary miss may continue to the already reviewed one-shot provider path.
A matching malformed row is `CorruptData`, not a miss; it fails closed and
does not fall through to a provider request.

## Best-effort write

The consume-once Rust output validator returns the inert projected result and,
only after complete validation, the canonical inner bytes used to construct a
sealed record. Invalid wall-clock state, read-only/unavailable storage, writer
contention, or persistence failure cannot discard or alter the validated
presentation result. Insert failure is ignored after validation and never
causes another provider request. The cache is therefore a performance and
local-history convenience, not a correctness or availability dependency.

## Full-population clear

Storage & Privacy exposes **Clear cached AI explanations**, not a generic
database clear and not a row-level cache browser. Core prepares one opaque,
engine/store-bound, consume-once preview over the complete current validated
AI-cache population, including expired rows. Public preview facts are limited
to total/expired record counts, total/expired logical content bytes, preparation
time, and an exact two-minute expiration. No row ID, digest, provider, model,
path, cache key, or victim selector crosses FFI.

Native runtime acquires its AI drain barrier before consuming the preview, so
active explanation/presentation work cannot race the confirmed clear. The
writer reloads and fingerprints the complete population under its bounded
guard and rejects any drift before effect. Its SQLite authorizer permits only
the reviewed `ai_insights` delete. Consumption is one-shot across success and
every error. A proven pre-effect mismatch is `ChangedSincePreview`; a possible
post-commit ambiguity is `OutcomeUnknown`. Native code invalidates the old
footprint, measures owned storage once, and never retries deletion.

The result reports records and logical content removed. It does not run
`VACUUM`, compact SQLite, or claim that the database file, allocated bytes, or
volume free space decreased. Credentials, capacity/history, scans, candidates,
cleanup journals, snapshots, settings, managed scan cache, legacy cache data,
and user files are outside the authorizer and the public capability.

## Authority-isolation audit

The cache introduces no reverse edge into the prior compiled graph:

```text
explicit Explain selection
    -> exact local lookup from retained privacy proof
    -> canonical output revalidation + fresh ID projection
    -> inert presentation

validated one-shot provider output
    -> inert presentation
    -> best-effort sealed cache insert

explicit Storage & Privacy confirmation
    -> two-minute full-population witness
    -> AI-table-only delete
    -> one storage remeasurement

candidate / plan / approval / Trash / cleanup / executor
    <- no cache or AI edge
```

The cache result uses the same compiler-isolated presentation DTOs as a fresh
result. It cannot construct Browser's opaque Trash confirmation, a Rust plan
review, cleanup confirmation, dry-run task, schedule, rule, candidate, or
filesystem request. Clearing rows is application-private data retention, not
user-file cleanup, and does not make model prose authoritative.

## Threat cases reviewed

1. **Reuse output after a contract/privacy/provider change.** Full identity
   binding makes the row a miss.
2. **Persist unvalidated or oversized provider bytes.** Only the validator's
   canonical inner output reaches the sealed constructor; schema and Rust both
   enforce the 64-KiB cap.
3. **Reuse stale snapshot IDs.** Stored bytes contain request-local IDs only;
   every hit revalidates and projects through the fresh retained proof.
4. **Hide matching corruption as a miss.** A matching malformed row returns a
   typed corruption failure and cannot trigger a provider fallback.
5. **Use a hit to read credentials or contact the provider.** Lookup precedes
   orchestrator construction; the hit returns immediately.
6. **Let cache availability control AI correctness.** Record construction and
   insert are best effort after validation; failure preserves the provider
   result and never retries.
7. **Clear selected rows or another table.** The public clear has no selector,
   its witness covers the complete population, and its authorizer admits only
   AI-table deletion.
8. **Replay or race a confirmation.** The preview is consume-once,
   engine-bound, expires after two monotonic minutes, drains AI work, and
   rejects complete-population drift.
9. **Retry an uncertain deletion.** Outcome unknown is terminal; native code
   remeasures once without another delete.
10. **Advertise cache clearing as disk recovery.** UI and result report logical
    cache content only and explicitly disclaim `VACUUM`, database-file shrink,
    and volume free-space change.

## Verification evidence

All focused verification was credential-free and made no provider request or
user-file cleanup effect.

- `cargo test -p dux-core ai_insight_cache --locked`
  - 10/10 cache persistence and engine tests passed, including exact binding,
    30-day/bounds, first-writer stability, corrupt-hit refusal, drift,
    authorizer confinement, ambiguity reconciliation, and clear expiry.
- `cargo test -p dux-core populated_v18_upgrade_discards_only_the_never_admitted_ai_reservation --locked`
  - passed; legacy AI rows were discarded and a non-AI Settings row survived.
- `cargo test -p dux-core v19_ai_cache_schema_enforces_revision_identity_and_payload_bounds --locked`
  - passed.
- `cargo test -p dux-ffi ai_insight_cache --locked`
  - 2/2 passed, covering exact round trip, cross-engine rejection,
    consume-once clear, and close-time preview release.
- Native tests in `DuxOwnedStorageFootprintTests` cover the clear barrier,
  remeasurement/no-retry outcomes, two-minute envelope, lease release,
  operation exclusion, shutdown joining, and explicit copy/accessibility.
- The remote-transport architecture suite contains the cache module in its
  deny-by-default dependency audit and remains credential/network-free.

## Explicit caveats

- No live API key was read, stored, or verified for this review.
- No Anthropic or other provider request was made.
- No signed/notarized release, update feed, or live-provider qualification is
  claimed.
- No generic provider/model picker, automatic AI request, scheduler/CLI AI,
  provider fallback, or retry was approved.
- No AI output, cache row, or cache-clear result can become cleanup authority.
- Deleting cache rows is not evidence of reclaimed database bytes or volume
  free space.
