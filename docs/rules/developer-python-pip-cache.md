# `developer.python.pip_cache` rule review

Status: bundled revision 1, discovery-only, unschedulable, and non-executable.

Reviewed: 2026-07-29 against pip's official caching documentation. This is an
independent DUX review. It does not derive policy, code, fixtures, or wording
from Mole or another cleanup product.

## Vendor evidence

The [pip caching documentation](https://pip.pypa.io/en/stable/topics/caching/)
states that pip maintains an HTTP response cache and a locally built wheel
cache. It documents `~/Library/Caches/pip` as the default cache location on
macOS, advises users not to rely on the internal directory structure, and
documents `pip cache purge` as removing all items from the wheel and HTTP
caches.

These sources establish that pip's cache is derived package/download material
that pip can fetch or build again. Rebuilding may need network access, source
toolchains, time, and bandwidth, and a future install can behave differently
if its upstream artifact has changed. “Regenerable” is therefore a cost and
review label, not a promise that removal has no consequence.

## Revision 1 discovery evidence

The immutable targeted-scan evaluator requires all of the following:

1. the engine selected its code-owned `UserCacheDirectory` evaluator scope for
   the exact known-cache targeted scan;
2. an exact, case-sensitive direct child directory named `pip` exists
   immediately below the scanned current-account `Library/Caches` root;
3. the candidate itself is not a symlink;
4. the candidate and every recorded regular file or directory below it have a
   representable modification time; and
5. the newest such observation is at least seven inclusive days old at the
   evaluator's exact persisted instant.

DUX deliberately matches the provider-owned top-level cache rather than any
of pip's undocumented internal subdirectories. A nested or differently cased
`pip`, a regular file, and a selected/configured-root scan do not match. The
historical aggregate is labeled an observed estimate and is not summed across
visual groups as if it were verified reclaimable space.

Coverage is candidate-local. Unknown coverage or an issue at the cache root,
an ancestor, or within `pip` prevents the minimum-age fact. An unrelated issue
below another cache does not. Any recorded symlink in the candidate subtree
adds `SymlinkBoundary`.

The seven-day fact does not prove inactivity. DUX does not observe running pip
processes, package-manager locks, an environment-specific `pip cache dir`
result, in-progress downloads, or ownership of every descendant. Every
candidate therefore retains `MissingOrIncompleteEvidence` and `ProtectedPath`
regardless of age.

## Policy and authority boundary

The catalog carries `SafeRegenerable` and
`RemoveKnownRegenerableContents` as proposed presentation policy. Revision 1
is blocked from selection, unschedulable, unable to construct a cleanup plan,
and has no command or filesystem executor. No path, candidate, provider
command, approval, or effect authority is accepted through the targeted-scan
FFI. AI cannot clear either blocker.

A future promotion would require a separately reviewed provider-aware live
witness, active-process and lock checks, exact descendant policy, no-follow
identity and hard-link validation, protected-root and volume proof,
changed-since-scan revalidation, explicit review, and central executor
admission. None is inferred from this discovery rule.

## Adversarial coverage

Core evaluator and immutable-snapshot replay tests cover exact direct matching,
wrong case/kind/location, selected-scope isolation, inclusive age, missing and
recent timestamps, candidate-local partial coverage, unrelated sibling
exclusions, descendant symlinks, exact scope/scan-ID binding, durable replay,
and stale evaluator replacement. Positive candidates remain unschedulable and
blocked by `MissingOrIncompleteEvidence` plus `ProtectedPath`.
