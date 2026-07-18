# `developer.rust.target` rule review

Status: bundled revision 2, discovery-only, unschedulable, and non-executable.

Reviewed: 2026-07-18 against Cargo 1.96.0 and the current Cargo Book. This is
an independent DUX review. It does not derive policy, code, fixtures, or wording
from Mole or another cleanup product.

## Vendor evidence

The Cargo Book provides the two policy facts used by this rule:

- [Build cache](https://doc.rust-lang.org/cargo/reference/build-cache.html)
  defines the target directory as Cargo build output and says its default is
  `target` at the workspace root. It also documents that configuration,
  environment, or `--target-dir` can relocate that output.
- [`cargo clean`](https://doc.rust-lang.org/cargo/commands/cargo-clean.html)
  describes its subject as generated target-directory artifacts and says the
  no-option command removes the entire target directory.

Cargo has emitted `CACHEDIR.TAG` in target directories since Cargo 1.46
([Cargo changelog entry #8378](https://doc.rust-lang.org/cargo/CHANGELOG.html)).
The changelog also records that Cargo 1.97 refuses an explicit target directory
that does not look like a Cargo target, preventing accidental deletion. A local
Cargo 1.96.0 fixture confirmed that a new default target contains a regular
`CACHEDIR.TAG` beginning with the standard cache-directory signature.

These sources establish that an actual Cargo target directory is generated and
may be rebuilt. Rebuilding can still cost substantial time, CPU, and network
access, and an output binary may be convenient or temporarily difficult to
reproduce. DUX must disclose that cost; “regenerable” does not mean disposable
without review.

## Revision 2 discovery evidence

The immutable scan classifier requires all of the following:

1. an exact directory component named `target`;
2. a direct regular, non-symlink sibling named `Cargo.toml`;
3. a direct regular, non-symlink child named `CACHEDIR.TAG`; and
4. no followed or unfollowed symlink component in the target ancestry.

The cache-tag filename and kind are supporting snapshot evidence only. The
scanner does not read its signature, resolve Cargo configuration, or prove that
this is the workspace's current target directory. Custom, relocated, or shared
target directories are intentionally not discovered by revision 2. A stale
member-local target may still be discovered when it carries Cargo's tag; that
is useful historical evidence but not live authority.

The rule carries `SafeRegenerable` and
`RemoveKnownRegenerableContents` as proposed policy, but every production
candidate still has `ProtectedPath`. It is not selectable, cannot construct a
cleanup plan, is not schedule eligible, crosses no FFI cleanup boundary, and
cannot reach a filesystem effect. The stricter marker also narrows the legacy
CLI's existing artifact classification; it adds no new CLI deletion authority.

## Adversarial coverage

Core projection and evaluator tests cover:

- both exact markers and lossless evidence paths;
- either marker missing, wrong case, wrong kind, or wrong location;
- a symlinked marker, target, or ancestor;
- verified nested targets with deterministic outer-root suppression;
- marker removal followed by a fresh projection;
- scan/coverage binding and stable catalog digest;
- durable publication, reopen, and path-free summary policy; and
- rejection by candidate selection and the sealed test-only plan constructor.

The catalog digest is checked during build and load. Only this exact rule ID and
revision may carry the safe-regenerable action pair; the other bundled rules
remain informational and reveal-only.

## Live default-layout witness

A later 2026-07-18 checkpoint adds a sealed, crate-private Unix planner witness
without changing the rule or its blockers. It accepts only the exact revision-2
candidate, source scan, policy, three evidence facts, unschedulable flag, and
sole unresolved `ProtectedPath` blocker. It then validates the current scan
root, direct target directory, direct `Cargo.toml` sibling, and direct
`CACHEDIR.TAG` child without following symlinks; requires both marker files to
be regular, single-link objects on the root volume; and proves their parent
identities match the target layout.

The tag is opened through a nonblocking retained descriptor and compared with
path identity observations before and after a bounded read. DUX requires the
same exact 43-byte prefix
[Cargo's own target-directory validator](https://doc.rust-lang.org/beta/nightly-rustc/src/cargo/ops/cargo_clean.rs.html#146-175)
reads:

```text
Signature: 8a477f597d28d172789f06886806bc55
```

Trailing bytes are accepted because the cache-directory tag standard permits
comments and Cargo's own validator checks only this prefix. The signature is a
generic cache marker that anyone can forge; it does not authenticate Cargo.

This witness cannot be cloned, serialized, converted into a cleanup plan, sent
across FFI, or used to perform an effect; it cannot clear `ProtectedPath`. It
deliberately does not parse the manifest or run Cargo, so `.cargo/config`,
environment overrides, custom target directories, outer workspaces, and
`package.workspace` remain unresolved. Windows also remains unsupported at
this boundary until ancestor traversal is handle-relative.

## Required before executable use

Removing `ProtectedPath` requires a separate reviewed implementation that
proves, at minimum:

- trusted home, selected-volume, canonical ancestry, and mount identity;
- a stable code-owned protected-root boundary grant;
- trusted Cargo metadata/config resolution authoritatively binding the
  workspace manifest and target directory to the already implemented live
  tag/layout identities;
- current target kind, symlink, link-count, mount, and descendant policy;
- inactive Cargo/rustc state and changed-since-scan revalidation;
- overlap resolution, exclusions, current reviewed plan, expiry, and approval;
- handle-relative executor-time revalidation and durable journal fencing; and
- exact-path UI disclosure, global permanent-cleanup disablement, capacity
  verification, history, and recovery.

Absence of a textual denial or presence of revision 2 metadata proves none of
those witnesses.
