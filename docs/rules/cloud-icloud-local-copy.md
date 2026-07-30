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
single-file probe serially; it never fans out or retries.

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
