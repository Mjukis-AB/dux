# iCloud Drive local-copy eviction research

- Rule ID: reserved for a later reviewed catalog revision; no production rule
  exists in this slice.
- Provider: Apple's Foundation ubiquitous-item API only.
- Proposed category: `cloud_file`.
- Proposed safety/action: `safe_evictable` / `evict_local_copy`.
- Proposed scheduling: never eligible.
- Research date: 2026-07-30.

This review is independent DUX research. It does not copy Mole source, rule
lists, tests, paths, or wording.

## Supported behavior

Apple documents
[`FileManager.evictUbiquitousItem(at:)`](https://developer.apple.com/documentation/foundation/filemanager/evictubiquitousitem%28at%3A%29)
as removing the local copy of an item stored in iCloud without removing the
item from iCloud. The item can later be downloaded again. Apple also says not
to perform the operation inside a coordinated write.

Foundation exposes separate resource values for ubiquitous identity, upload
completion/activity/error, unresolved conflicts, download activity/error and
status, and sync exclusion. The
[`current`](https://developer.apple.com/documentation/foundation/urlubiquitousitemdownloadingstatus/current)
download status means a local copy exists and is the most up-to-date version
known to the device. `downloaded` is explicitly stale, while `notDownloaded`
has no local copy.

The
[`ubiquitousItemIsUploaded`](https://developer.apple.com/documentation/foundation/urlresourcevalues/ubiquitousitemisuploaded)
value alone is insufficient. DUX requires it together with current local state,
idle upload/download state, no reported transfer error, no unresolved conflict,
no requested download, sync inclusion, an exact regular-file witness, and
known nonzero local allocation. Missing facts fail closed.

## Identity capability

Contract v45 separately asks whether the identity facts needed by a future
durable eviction flow are available and stable during one read. The native
probe brackets two complete resource-value samples with current-account and
current-file-version observations. It reports account, container, item
generation, and file version only as stable, unavailable, changed during read,
or unsupported. Comparison archives are bounded, never decoded, never shown,
and never persisted. Matching archive bytes mean only equality inside the one
bracketed read; they are not treated as a canonical or durable identity
encoding. Shared-item and sync-paused values are independent tri-state facts.

Apple's
[`ubiquityIdentityToken`](https://developer.apple.com/documentation/foundation/filemanager/ubiquityidentitytoken)
identifies the current iCloud account but does not connect an app to ubiquity
containers. Apple's
[`url(forUbiquityContainerIdentifier:)`](https://developer.apple.com/documentation/foundation/filemanager/url(forubiquitycontaineridentifier:))
addresses containers declared for that app. Neither API supplies a stable
container identity for an arbitrary iCloud Drive file selected in Explorer.
DUX therefore reports the production container fact as unsupported and does
not infer it from a path, display name, metadata-query result, or private
provider state.

Sync eligibility and identity readiness are deliberately independent. A file
may have favorable current upload, download, conflict, and exclusion metadata
while identity readiness remains blocked. The v45 capability result is
point-in-time, memory-only information; it creates no durable evidence,
candidate, plan, approval, journal/history row, provider command, cleanup
button, retry, or effect. A supported stable container witness and the
read-only real-device protocol must be completed before designing persistence
or candidate admission.

## Initial scope and exclusions

The live policy still admits only one manually reviewed, regular, single-link
iCloud file at a time. Explorer may nominate a bounded set for those individual
checks, but DUX does not initially admit:

- directories or packages;
- symlinks or any symlink ancestor;
- multiply linked files;
- stale or remote-only placeholders;
- active uploads or downloads;
- upload/download errors;
- unresolved conflicts;
- items excluded from sync;
- unknown or zero local allocation;
- Dropbox, OneDrive, Google Drive, or another File Provider implementation.

A `Mobile Documents` path component, Finder badge, extension, placeholder
filename, or extended attribute is not evidence of provider identity or upload
completion. Metadata queries are not assumed to enumerate the user's entire
iCloud Drive; Apple's
[search-scope documentation](https://developer.apple.com/documentation/foundation/metadata-query-search-scopes)
describes container- and prior-access-scoped behavior.

## Safety boundary

The current implementation stage is read-only. Rust chooses and revalidates an
exact retained Explorer node. Swift reads raw Foundation facts. Rust classifies
them and returns a path-free observation. The result creates no candidate,
plan, journal entry, approval, schedule, AI input, callback, or effect.

Explorer exposes this boundary only as an explicit check for the currently
selected regular file or as **Check iCloud status** for a Rust-owned bounded
directory source. Contract v44 walks at most 200,000 descendants of the exact
retained snapshot directory and returns at most 32 complete regular-file rows,
ranked by historical allocated bytes, logical bytes, and stable node ID. A
source load performs no Foundation reads. The manual batch invokes the existing
single-file probe serially; it never fans out or retries. Contract v45 enriches
each manual item result only with bracketed identity-capability facts; it does
not widen the v44 source or batch authority.

This is not iCloud Drive enumeration. Allocation is only a nomination signal,
not provider evidence. DUX does not probe automatically, sum observations, or
publish results into the Candidates view. A stopped batch finishes the current
synchronous Foundation read before stopping; no truthful mid-read
cancellation or timeout exists. Navigation, content-mode, snapshot,
presentation, and review-generation changes clear the source and fence late
replies, while row selection does not cancel an otherwise current
directory-scoped batch. The displayed allocation is labeled as the historical
value observed in the scan; it is not live allocation, a reclaimable total, or
verified capacity change.

Before a later effect can ship, ADR 0006 requires a separate manual,
journal-fenced, one-shot executor with fresh filesystem and provider
revalidation immediately before the supported API, no automatic retry, and
outcome-unknown quarantine after any ambiguous API entry.

## Required product copy

Any future review must say “Remove local copy,” “Stays in iCloud,” and
“Requires a network connection to download again.” Estimated recovery is known
local allocation, not cloud logical size, and is not verified freed capacity.
The current inspector includes all three disclosures as future-action
explanation and states that no cleanup action is available yet.
