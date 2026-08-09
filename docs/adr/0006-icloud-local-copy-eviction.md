# ADR 0006: iCloud local-copy eviction boundary

- Status: Accepted
- Date: 2026-07-30
- Scope: deterministic discovery and eventual non-destructive iCloud Drive local-copy eviction

## Context

DUX's Critical recovery order puts non-destructive cloud local-copy eviction
ahead of deletion-oriented recovery. The domain model already distinguishes
`SafeEvictable`/`EvictLocalCopy` and cleanup history already has an `Evicted`
outcome, but no production rule, provider probe, candidate, plan admission, or
effect exists.

Foundation exposes public ubiquitous-item resource values and
`FileManager.evictUbiquitousItem(at:)`. Apple documents that the operation
removes only the local copy and leaves the item in iCloud, from which it can be
downloaded again. The same documentation says not to wrap eviction in a
coordinated write. The uploaded resource value alone says only that data is
present in iCloud; it is not sufficient evidence that a local copy is current,
idle, conflict-free, or useful to evict.

Discovery has a separate limitation. Ubiquitous metadata-query scopes are
container- or prior-user-access scoped and are not proof that DUX can enumerate
the user's entire iCloud Drive. A pathname, Finder badge, extension, extended
attribute, or `Mobile Documents` component is not provider identity or upload
evidence. Other File Provider services have different APIs and semantics.

The primary app is not sandboxed under ADR 0003, but that does not grant iCloud
authority, bypass TCC, or weaken the shared-Rust ownership in ADR 0004.

## Decision

Implement iCloud local-copy eviction as a provider-specific, staged feature.
The first provider is Foundation's iCloud ubiquitous-item API. Do not present a
generic cloud-provider switch and never fall back to direct filesystem
deletion, Trash, shell commands, or provider-private metadata.

### Read-only probe boundary

The first implementation stage is observation-only:

1. A retained Rust Explorer review selects one exact non-root snapshot node.
2. Rust revalidates its no-follow path, ancestors, mount, type, identity, link
   count, and known nonzero local allocation.
3. Rust creates a one-shot request containing the exact path bytes. The request
   is consumed by the macOS adapter and cannot be replayed or retargeted.
4. Swift performs one fresh Foundation resource-value read and returns only a
   bounded raw fact record. It cannot return a path, candidate, plan, approval,
   or command.
5. Rust repeats the retained no-follow identity/ancestor check after the
   callback, then alone classifies the facts and returns a path-free
   assessment. Replacement during the metadata read fails closed.

The read-only assessment is not persistent execution evidence and cannot be
reused later. No candidate, plan, journal row, button, or effect is created by
this stage.

### Bounded observation source

Contract v44 adds a path-free source for an explicit multi-item review without
changing the probe or granting cleanup authority:

1. The user chooses a historical directory in Explorer and opens **iCloud
   Status**. Swift supplies only that retained snapshot node ID.
2. Rust walks only that snapshot subtree. It fails the whole query rather than
   publishing a partial source if traversal would exceed 200,000 descendants.
3. Rust keeps at most 32 complete regular-file observations with known nonzero
   allocation and no scan warning. It orders them by historical allocated
   bytes descending, logical bytes descending, then snapshot node ID
   ascending. Exact match and omission counts accompany the path-free rows.
4. Explorer loads the historical source without performing a live metadata
   check. Only the user's **Check iCloud status** action invokes the existing
   one-file v43 probe, serially in Rust-owned rank order.
5. Changed, invalid, or per-item metadata failures remain visible and permit
   the next bounded check. Unsupported, unavailable, or malformed systemic
   states stop the remaining checks.

The Foundation resource-value read is synchronous and has no truthful
mid-call cancellation or wall-time guarantee. **Stop after current check**
therefore prevents later calls and suppresses the in-flight result; it does not
claim to interrupt the system call. Code-owned single-flight state prevents a
cancel/restart cycle from overlapping or queueing a second batch. Navigation,
snapshot, content-mode, close, and retained-review generation changes discard
late results; row selection alone does not cancel the directory-scoped batch.

The source is not iCloud enumeration. Historical allocation nominates which
files to inspect but proves neither provider identity nor current allocation.
Results occur at different instants and are not atomic. Explorer does not sum
them, infer reclaimable capacity, persist them, send them to AI, add them to
Candidates, or expose a cleanup button. Every item must independently pass the
same before/after filesystem witness and Foundation fact policy as the
selected-file probe.

### Identity-capability probe

Contract v45 keeps the feature read-only while making the missing execution
evidence explicit. One manual item check brackets two complete Foundation
resource-value samples with:

1. an account-token observation before sample A;
2. sample A and its current file-version observation;
3. sample B and its current file-version observation; and
4. a second account-token observation.

Account identity, item generation, and current file version are each reported
only as `stable`, `unavailable`, `changed during read`, or `unsupported`.
Archives are bounded transport-local comparison material: DUX does not decode,
display, or persist them, and equality means only that the bounded keyed-archive
bytes matched within this one bracketed read. It is not a documented canonical
cross-process, cross-OS, or durable identity representation. Shared-item and
sync-paused facts remain separate tri-state policy inputs. Failure to obtain
either complete resource sample fails the entire metadata read rather than
publishing a mixed-time record.

Foundation's `ubiquityIdentityToken` is an opaque identity for the current
iCloud account, and Apple documents that it does not connect the app to
ubiquity containers. `url(forUbiquityContainerIdentifier:)` resolves only a
container declared for the app; it does not provide a stable identity for the
arbitrary user-selected iCloud Drive container containing an Explorer item.
The ubiquitous-container display name is presentation text, not identity.
Therefore the production v45 probe reports container identity as
`unsupported`. It must not synthesize identity from a pathname, display name,
metadata-query scope, Finder state, or provider-private data.

Point-in-time sync eligibility and identity readiness are independent:
favorable upload/download/conflict/exclusion facts may still support a
read-only review, while any unavailable, changed, or unsupported identity fact
blocks identity readiness. In particular, the unsupported container fact means
the production v45 result cannot become durable provider evidence, a candidate,
or an effect input. Contract v45 adds no persistence schema, rule, candidate,
plan, approval, journal/history row, provider call, retry, cleanup button, AI
input, CLI edge, notification, schedule, or filesystem effect.

### Public File Provider identity observation

Contract v58 supersedes only v45's production `unsupported` container
capability result. On macOS 14 and later, the public
`NSFileProviderManager.getIdentifierForUserVisibleFile(at:)` API can return the
opaque File Provider item identifier and domain identifier for the exact
user-visible URL. The adapter brackets the existing complete read with two of
these observations:

1. account token A;
2. File Provider item/domain observation A;
3. complete Foundation resource sample A and current file version A;
4. complete Foundation resource sample B and current file version B;
5. File Provider item/domain observation B; and
6. account token B.

Domain/container identity and provider-item identity are independent facts.
Each is reported only as `stable`, `unavailable`, `changed during read`, or
`unsupported`. An API error, missing component, five-second observation
timeout, empty identifier, or UTF-8 value larger than 4 KiB becomes
`unavailable`; a non-ubiquitous file becomes `unsupported`. The raw opaque
strings stay inside the Swift reader and are never transported, displayed,
logged, persisted, hashed into authority, or sent to AI. Contract v58 adds a
separate provider-item field and blocker rather than conflating it with item
generation or file version. Rust and Swift require the fixed order account,
domain/container, provider item, generation, version, shared state, then sync
paused state.

A completely stable v58 observation may make the path-free in-memory identity
readiness flag true. That means only that the public capabilities were present
and unchanged during this one read. It is not durable evidence, does not prove
cross-launch or cross-OS stability, and cannot be replayed. Contract v58 still
adds no schema, raw identifier transport, rule, `Candidate`, plan, approval,
journal/history row, provider command, cleanup button, retry, effect, AI input,
CLI edge, notification, or schedule. Candidate admission remains closed until
the isolated real-device protocol validates the public identifiers and a
separate contract binds opaque durable provider/account/domain/item/version
evidence to fresh filesystem and sync proofs.

The real-device gate uses a separate non-shipping qualification target rather
than weakening that production boundary. Only when
`DUX_ICLOUD_READ_ONLY_QUALIFICATION` is compiled for the unhosted test bundle
may the same reader derive domain-separated HMAC-SHA256 comparison tags from
the five bounded identities. The exact 32-byte key and tag reference are
private `0600` operator state outside iCloud Drive and DUX application data.
The public evidence record contains only stability and continuity enums; raw
identities, keys, tags, paths, filenames, content, hashes, and error text are
forbidden. This keyed state characterizes equality across test-process and
host transitions only. It is not the separately versioned durable production
evidence anticipated above, cannot cross FFI, and cannot create a rule,
candidate, plan, journal, provider command, or effect. Ordinary app Debug,
Release, and cleanup-qualification targets do not compile this condition.

The initial deterministic policy admits only a regular, single-link file with
known nonzero local allocation when every relevant fact is known:

- the item is ubiquitous;
- the item is uploaded;
- no upload is in progress and no upload error is present;
- no unresolved conflict is reported;
- the local download state is exactly `current`;
- no download has been requested;
- no download is in progress and no download error is present;
- the item is not excluded from sync.

Unknown values fail closed. A stale `downloaded` copy, a `notDownloaded`
placeholder, a directory, package, symlink, hard link, zero/unknown allocation,
conflict, active transfer, or excluded item is ineligible. Directory eviction
is deferred because a directory-level status does not prove every descendant is
uploaded, current, and conflict-free.

The product may describe a passing probe only as an observed iCloud local copy
whose known facts currently support review. It must not claim that a concurrent
writer is impossible, that bytes are guaranteed reclaimable, or that an effect
has been authorized.

### Eventual effect boundary

An eviction effect may be added only after the read-only probe is proven. It
must use a separate Rust-owned planner/journal executor modeled on, but not
shared with, the Trash effect:

- manual explicit selection only; no AI, CLI, notification, or schedule can
  approve it;
- a fresh plan with `SafeEvictable`, `EvictLocalCopy`, and
  `CleanupMode::EvictLocalCopy`;
- exact rule revision, scan, provider/account/container, item-version, path,
  filesystem identity, type, link-count, allocation, and exclusion bindings;
- a final no-follow filesystem revalidation and fresh provider fact read after
  a durable `effect_started` receipt and immediately before the API;
- one synchronous `evictUbiquitousItem(at:)` call from a core-issued one-shot
  request;
- no automatic effect retry;
- success recorded as `Evicted`, never removed, deleted, or trashed;
- any error or panic after API entry recorded as outcome-unknown and held in
  the existing process-lifetime cleanup quarantine;
- persistence-only restart reconciliation that never invokes eviction again.

Do not use `NSFileCoordinator` to wrap the eviction call. If public API
semantics and destructive real-device race tests cannot prove that pending
local changes are protected at the final boundary, the effect remains
unreachable.

Estimated recovery uses known local allocated bytes, not logical cloud size.
API success means “local copy evicted,” not “this many bytes verified freed.”
Post-effect volume capacity is separate telemetry and may lag.

### Presentation

The review and confirmation copy must say:

- **Remove local copy**;
- **Stays in iCloud**;
- **Requires a network connection to download again**.

It must distinguish unknown, changed, uploading, downloading, conflicted,
already remote-only, failed, and outcome-unknown states. Accessibility must
carry the same disclosure. No cloud path or document content is sent to AI.

## Consequences

Positive:

- eviction uses Apple's supported non-destructive API;
- Rust retains nomination, eligibility, planning, journaling, and retry
  prevention;
- native code supplies platform facts and the eventual platform primitive
  without becoming a second planner;
- missing or stale metadata cannot silently become eligibility;
- the initial scope avoids unsafe recursive directory and cross-provider
  assumptions.

Negative:

- the first useful scope is individual regular files and may recover less space
  than Finder's broader provider management;
- the bounded snapshot source is not global iCloud discovery and may nominate
  ordinary local files that the live probe then rejects;
- one synchronous Foundation metadata read may still stall despite serial
  scheduling and stop-after-current behavior;
- real iCloud account/device tests are required before the effect can ship;
- the public File Provider identity call is asynchronous and may be unavailable
  or time out for a particular item, while Foundation resource reads retain no
  truthful wall-time guarantee;
- provider/account/domain/provider-item/item-version persistence and restart
  reconciliation still require real-device qualification and a later durable
  schema revision.

## Alternatives considered

### Delete provider-managed files directly

Rejected. Deletion can remove cloud data everywhere and violates the roadmap's
non-destructive semantics.

### Treat `isUploaded == true` as sufficient

Rejected. It does not establish current local state, idle transfer state,
conflict absence, sync inclusion, allocation, or exact target identity.

### Infer iCloud from a known filesystem path

Rejected as authority. A path may be used only as bounded discovery scope after
separate validation; provider identity and eligibility come from supported
Foundation facts.

### Use `NSMetadataQuery` as authoritative enumeration

Rejected. Query scope and results are access-, container-, Spotlight-, and
timing-dependent. A future query may provide hints, but the retained exact item
must still pass the full live probe and execution gates.

### Support directories in the first version

Rejected. Directory state does not prove a bounded complete descendant set or
atomic safety when children change.

### Coordinate the eviction write

Rejected. Apple's eviction documentation explicitly warns against coordinated
write wrapping.

## Validation criteria

The read-only probe stage requires:

- one favorable fact combination and independent negative coverage for every
  false, missing, stale, active, error, conflict, exclusion, type, link, and
  allocation state;
- exact retained-node/path/ancestor replacement races;
- replacement during the platform metadata callback;
- one-shot path consumption and callback panic/error containment;
- FFI and Swift rejection of unknown enum values and malformed records;
- source-boundary checks proving there is no eviction or deletion call.

The v45 identity-capability stage additionally requires:

- exact account-before/resource-A/version-A/resource-B/version-B/account-after
  ordering;
- bounded comparison archives and fail-closed unavailable, changed, and
  unsupported states;
- independent sync-eligibility and identity-readiness results;
- an explicit unsupported production container fact, without fallback
  inference;
- read-only real-device characterization under the protocol in
  [`docs/testing/icloud-local-copy-real-device.md`](../testing/icloud-local-copy-real-device.md).

The v58 File Provider identity stage additionally requires:

- exact account-A/provider-A/resource-A/version-A/resource-B/version-B/
  provider-B/account-B ordering;
- independent bounded domain and provider-item stability states;
- error, missing, timeout, empty, oversized, changed, and non-ubiquitous
  refusal without exposing either raw identifier;
- fixed Rust/FFI/Swift blocker order and strict native response validation;
- proof that a fully stable result remains memory-only capability evidence and
  creates no candidate or effect edge; and
- a default-skipped, wrapper-only, three-consecutive-read qualification target
  whose closed report and private keyed state cannot expose identities or
  reach application/engine persistence; and
- the macOS 14 plus newest-supported-macOS characterization matrix before any
  identifier can enter durable evidence.

The effect stage additionally requires:

- destructive tests in an isolated real iCloud account with disposable files;
- local edit, active writer, upload, conflict, account change, target
  replacement, API error, crash, and restart-reconciliation races;
- proof that every pre-call rejection makes zero platform calls;
- proof that every post-entry ambiguity makes at most one call and never
  retries;
- correct history, accessibility, capacity-accounting, and redownload copy.

Reconsider this ADR if Apple deprecates ubiquitous-item eviction, documents a
stronger File Provider abstraction suitable for iCloud Drive, or real-device
tests show that local-only changes can be discarded despite the final required
facts.

## References

- [ROADMAP §7.4, §7.5, §8, §13.3, and Milestone 6](../../ROADMAP.md)
- [SECURITY_DESIGN](../../SECURITY_DESIGN.md)
- [ADR 0001: Native SwiftUI macOS application](0001-native-swiftui-macos-application.md)
- [ADR 0003: Primary build without App Sandbox](0003-primary-build-without-app-sandbox.md)
- [ADR 0004: Shared Rust engine](0004-shared-rust-engine.md)
- [ADR 0005: UniFFI transport](0005-uniffi-swift-rust-transport.md)
- [Apple: `FileManager.evictUbiquitousItem(at:)`](https://developer.apple.com/documentation/foundation/filemanager/evictubiquitousitem%28at%3A%29)
- [Apple: `URLUbiquitousItemDownloadingStatus`](https://developer.apple.com/documentation/foundation/urlubiquitousitemdownloadingstatus)
- [Apple: `ubiquitousItemIsUploaded`](https://developer.apple.com/documentation/foundation/urlresourcevalues/ubiquitousitemisuploaded)
- [Apple: metadata query search scopes](https://developer.apple.com/documentation/foundation/metadata-query-search-scopes)
- [Apple: `FileManager.ubiquityIdentityToken`](https://developer.apple.com/documentation/foundation/filemanager/ubiquityidentitytoken)
- [Apple: `FileManager.url(forUbiquityContainerIdentifier:)`](https://developer.apple.com/documentation/foundation/filemanager/url(forubiquitycontaineridentifier:))
- [Apple: `URLResourceValues.generationIdentifier`](https://developer.apple.com/documentation/foundation/urlresourcevalues/generationidentifier)
- [Apple: `NSFileVersion.persistentIdentifier`](https://developer.apple.com/documentation/foundation/nsfileversion/persistentidentifier)
- [Apple: `URLResourceValues.ubiquitousItemIsShared`](https://developer.apple.com/documentation/foundation/urlresourcevalues/ubiquitousitemisshared)
- [Apple: `URLResourceValues.ubiquitousItemIsSyncPaused`](https://developer.apple.com/documentation/foundation/urlresourcevalues/ubiquitousitemissyncpaused)
