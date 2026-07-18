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

## Bounded Cargo resolution witness

A second 2026-07-18 Unix checkpoint consumes the live default-layout witness
and accepts no caller-supplied manifest, working directory, command arguments,
or expected result paths. It invokes one observed canonical regular file named
`cargo`; all symlink launchers, including the common rustup proxy, are rejected
because a project `rust-toolchain.toml` can otherwise redirect execution before
Cargo processes `--offline`, according to
[rustup's override precedence](https://rust-lang.github.io/rustup/overrides.html).

Observation binds a single-link executable's full bounded SHA-256 and the exact
reviewed Cargo 1.96.0 `--version --verbose` digest. Other and future Cargo
releases fail closed until their metadata behavior is reviewed and the policy
revision is updated. Resolution repeats those checks around a fixed command run
from the current manifest parent:

```text
cargo metadata --format-version 1 --no-deps --locked --offline --quiet \
  --color=never --manifest-path <exact-live-Cargo.toml>
```

The [Cargo metadata reference](https://doc.rust-lang.org/cargo/commands/cargo-metadata.html)
defines these flags and output fields. The
[Cargo configuration reference](https://doc.rust-lang.org/cargo/reference/config.html)
specifies that configuration discovery begins at the process working
directory, so the manifest parent is intentional. The child receives a minimal
environment containing only canonical home, Cargo-home, temporary-directory,
locale, and forced offline/color values; ambient target-directory, rustup,
compiler-wrapper, proxy, credential, dynamic-loader, and `PATH` inputs are not
inherited. `PATH` is set to the fixed non-directory `/dev/null` sentinel. The
result must be one bounded format-version-1 JSON document with
`resolve: null`, the exact manifest parent as `workspace_root`, and the exact
already witnessed directory as `target_directory`. Stdout, stderr, runtime,
and JSON are bounded; timeout or output overflow terminates the original
process group and reaps the direct child. Raw output is not retained. The
returned witness preserves the exact executable digest/identity and canonical
environment observations that produced the accepted metadata.

The manifest is now fully read through a retained nonblocking descriptor and
SHA-256-bound before/open/after, so an in-place same-inode rewrite also rejects
the observation. The original test-only witness can still accept a deliberately
unsigned fake Cargo to exercise bounded subprocess failures, but it cannot be
called by production planning authority. The sealed enrolled production entry
described below requires exact durable trust and macOS static-code evidence.

The production metadata entry now admits only an exactly negative Cargo 1.96
file-configuration closure. It checks both the legacy extensionless `config`
and `config.toml` at every `.cargo` directory Cargo would discover from the
process cwd through its ancestors, followed by the exact Cargo home. Any
present file rejects before metadata execution, which makes recursive
`include` resolution empty without reimplementing Cargo's TOML semantics. The
observation is bounded to 64 cwd ancestors, 132 unique watched directories,
and 64 KiB of native path material and records a revisioned SHA-256 closure
digest. Canonical identities are revalidated around a macOS kqueue vnode fence;
any event is terminal even when a created config is immediately removed.

The project cwd is also opened no-follow and retained. On macOS the production
runner installs it with `posix_spawn_file_actions_addfchdir_np`; the descriptor
is explicitly inherited only for that file action and closed before exec.
Descriptor and pathname identities are revalidated around launch. This removes
pathname resolution from child cwd selection. Every config lookup directory
must be local APFS; other, remote, virtual, and unprobeable filesystems reject.
Kqueue is strong reviewed-filesystem change inference, not direct evidence of
the exact config inode Cargo read. Positive configs and included files, and
every workspace-member manifest, therefore remain unattested. Either witness
is still supporting evidence only: neither can clear `ProtectedPath`,
construct a plan, cross FFI, schedule, or execute.

## Suspended macOS launch and selected-running-code continuity

The admitted macOS production path now closes the executable swap/restore race
without claiming an fd-based exec primitive that Darwin does not provide. DUX
first opens vnode-event descriptors for the exact enrolled executable and each
canonical ancestor through the filesystem root. Every watched object must be
on local APFS. It then revalidates the full executable SHA-256, identity,
single-link shape, cwd, and negative config closure before directly calling
the exact path with `posix_spawn`; it never uses `posix_spawnp` or `PATH`.

The spawn attributes require `START_SUSPENDED`, a new process group,
`CLOEXEC_DEFAULT`, an empty signal mask, and default dispositions for all
catchable signals. Standard input is `/dev/null`; bounded stdout and stderr are
the only pipe descriptors. Before DUX sends `SIGCONT`, it proves through
kernel process records that this is still its direct stopped child, with the
expected process group, credentials, process start instant, and retained cwd.
Security.framework then validates the dynamic kernel guest and returns its
selected Code Directory hash. That hash must be one member of the exact sorted
all-architecture Code Directory set stored by enrollment. The process record,
executable digest/identity, cwd/config guards, and all vnode fences are checked
again before resume. Executable and config fences remain polled during bounded
output collection and are revalidated after the exact child is reaped.

Resolution-policy revision 3 records launch-policy revision 1 and a SHA-256 of
the raw running Code Directory hash in the sealed witness. The raw process
output and raw hash are not persisted or exposed as authority. The selected
hash proves the architecture that actually ran; the enrollment's strict static
record and the full-file digest bind the complete universal executable.

This is swap/restore-resistant selected-code continuity on the reviewed local
filesystem, not pathname-independent execution or sandbox confinement. A
same-UID external process may send `SIGCONT` to the stopped child, and kqueue
events are inference rather than a formal proof of the kernel's exact pathname
open. Scripts are not admitted as Cargo executables. These limitations keep
the result non-authoritative and `ProtectedPath` remains unchanged.

## Durable discovery-source binding

A third 2026-07-18 checkpoint removes the earlier forgeable planner input of a
caller-paired scan ID and path. Production live validation can now begin only
from a sealed, non-cloneable source loaded under one store coordinator. The
loader requires an exact succeeded scan with complete coverage and an available
immutable snapshot; the exact evaluation must have succeeded with the current
evaluator revision, catalog schema and SHA-256, context format and recomputed
context digest, plus the scan's exact snapshot version and digest. The selected
candidate must still be `Discovered`, retain the exact revision-2 policy and
three evidence facts, and match the deterministic candidate ID recomputed from
the current bundled rule and lossless target path.

Acquisition holds an exact `CleanupReview` snapshot lease, decodes the
checksummed retained snapshot through the existing charged review-memory
budget, and locates the root, complete ancestor chain, direct `target`, sibling
manifest, and child cache tag in the snapshot graph. Each must have the expected
kind and a Unix device/inode observation. A second complete source read must
equal the first after decoding, and the lease is checked with fresh clock reads
immediately before the source is returned. The live validator then requires
the current root, ancestor chain, and three terminal objects to retain those
scan-time observations. Ordinary replacements with different observed
device/inode values are therefore rejected, but Unix inode reuse remains
possible; this witness does not prove unbroken object continuity.
The source is owned by the live witness and, transitively, the Cargo witness;
both repeat lease and source-history validation around their filesystem work.
Holding the value alone does not renew its bounded pin, so an expired or removed
lease fails closed. Acquisition failures, consuming release, and witness drop
all make a best-effort exact pin release; unresolved database failure remains
safe and expires naturally.

This binding proves exact persisted provenance and changed-since-scan identity,
not cleanup authority. It does not prove current descendants, authenticate
Cargo or its configuration inputs, grant protected-root or volume authority,
establish process inactivity, remove `ProtectedPath`, or expose a
plan/FFI/execution conversion. Final authority still needs retained-handle or
generation evidence strong enough to address inode reuse.

## Exact snapshot evaluator replay

A fourth 2026-07-18 checkpoint now replays the complete current candidate batch
from the decoded retained snapshot before the Rust-target source can be
returned. It does not reconstruct a second full-path tree. Instead, one
depth-first enumeration caches each open directory's bounded marker summary,
aggregate known allocated bytes, directory/file newest modification time, and
any-classified-ancestor state. Candidate and evidence paths are materialized
only for the bounded output and charged incrementally against the same
conservative 32 MiB durable-batch budget before retention. Working memory is
therefore O(depth + accepted candidate payload); enumeration is O(nodes +
edges), with additional work proportional to the materialized path bytes. A
wide directory is never rescanned once per artifact-named child.

The replay uses the same catalog pattern declarations and final candidate
policy as fresh evaluation while independently enumerating snapshot nodes. It
reproduces marker preference, native-byte evidence sorting, suppression by any
classifiable outer artifact (including a different rule), known-byte estimates
when allocation is partially unknown, and the exact global 4,096-success versus
4,097-failure boundary. Symlink, other, and error-node mtimes remain excluded,
matching the fresh tree.

Every replayed candidate is compared by ID against the complete durable
evaluation: source scan, rule/revision, category, ordered paths and evidence,
estimated bytes, newest mtime, safety, action, schedule eligibility, and
ordered blockers must all match. Missing, injected, or modified candidates and
any change to ordered child fields fail closed. Mutable review status is not
evaluator output; the existing source loader and acquisition sandwich continue
to require the selected candidate to be `Discovered` and reject any source
drift.

Replay runs while the charged `CleanupReview` document and exact pin are held,
before snapshot identity binding and the second complete source read. Its only
result is success or a path-free error. Failure drops both decoded-memory
accounting and the auto-releasing pin. No replayed candidate or capability can
escape this boundary, and the checkpoint still cannot clear `ProtectedPath`,
create a plan, cross FFI, schedule, or execute.

## Explicit direct-Cargo enrollment and macOS static-code evidence

A fifth 2026-07-18 checkpoint adds an explicit local trust decision for the
direct Cargo executable. It does not search `PATH`, invoke a rustup proxy, infer
trust from an installed toolchain, or choose a Cargo binary automatically. The
macOS core API accepts one canonical absolute single-link regular file named
`cargo` and separates the operation into inspection and commit. Inspection is
read-only and does not execute the selected file. Its non-cloneable preview can
be committed only through the exact engine/store that created it.

Inspection binds all of the following bounded evidence:

- the lossless canonical executable path and current filesystem identity;
- a full SHA-256 of the executable, capped at 256 MiB;
- the canonical `HOME`, `CARGO_HOME`, and temporary-directory identities used
  by the future scrubbed subprocess environment; and
- static-code policy revision 1: signature flags, sorted unique Code Directory
  hashes, signing identifier, optional team identifier, optional SHA-256 of the
  serialized designated requirement (capped at 64 KiB), and whether the
  signature is CMS or ad-hoc.

Static-code inspection calls Security.framework directly. It requests strict
validation for all architectures, disables network access, and uses
single-threaded validation. It does not shell out to `codesign` or `spctl` and
does not treat Gatekeeper assessment as cleanup authority. A valid ad-hoc
signature authenticates no publisher; it only lets macOS validate the embedded
Code Directory against the current bytes. DUX accepts it only because the user
explicitly enrolls that exact observed path, bytes, version output, and signing
evidence. A CMS team identifier is retained as evidence, not elevated into a
global publisher allowlist.

The shared settings database stores one canonical, deny-unknown-fields v1 value
under `developer_rust_target_cargo_enrollment`. The row is bounded to 96 KiB and
contains a monotonic revision plus a canonical millisecond timestamp. The
inspection preview also freezes the complete setting that existed before and
after inspection. Commit first proves that setting is still current and repeats
the executable, environment, and signature checks. Consuming the preview is the
explicit authorization point that executes the selected file with the bounded
scrubbed `--version --verbose` command. Only an exact reviewed Cargo 1.96.0
release is accepted; its complete output SHA-256 joins the enrollment identity.
The setting is then conditionally written only if its prior state remains exact.
An identical enrollment is an idempotent no-op. A changed identity advances the
revision. Revocation advances it again and retains a field-free
`revoked` tombstone, so a preview created before enrollment, replacement, or
revocation cannot recreate prior trust. Post-commit uncertainty is reconciled
only when the complete exact expected state can be reread; otherwise the result
is outcome-unknown.

The sealed production metadata entry no longer accepts a Cargo path from its
caller. It obtains the exact enrollment from the same store coordinator owned
by the durable Rust-target source, statically observes that enrolled path again,
requires its path, full digest, and signature to match, and rereads the complete
enrollment before executing any bytes. Only then may it run bounded version
validation, require every stored release/version fact, and continue into the
existing enrollment rereads around the fixed metadata command. The resulting witness
retains the enrollment revision and signature observation. Missing, revoked,
corrupt, newer-schema, changed, unsigned, malformed, stale-preview, or
foreign-engine state fails closed.

This checkpoint deliberately exposes only the Rust core API. FFI and Swift
settings UI remain future work, so the application cannot ask a user to enroll
Cargo yet. That UI must clearly disclose that confirmation executes the exact
statically previewed binary for bounded version validation. More importantly,
enrollment establishes only local executable
provenance for discovery. Positive `.cargo/config`, legacy extensionless
config, and recursive `include` inputs are not directly attested; projects
containing them now reject. The exact negative lookup closure, retained cwd,
and suspended selected-code checkpoint cover the currently admitted direct-
executable case, but the launch is still path-based and workspace-member
manifests are not yet attested. Direct-read or generation evidence is still
required before promotion. `ProtectedPath` therefore remains untouched.

## Required before executable use

Removing `ProtectedPath` requires a separate reviewed implementation that
proves, at minimum:

- trusted home, selected-volume, canonical ancestry, and mount identity;
- a stable code-owned protected-root boundary grant;
- reviewed acceptance or mitigation of the suspended selected-code
  checkpoint's path-based and same-UID signaling limitations, direct positive
  Cargo config/include identity provenance if configured projects are ever
  admitted, and every workspace-member manifest;
- current target kind, symlink, link-count, mount, and descendant policy;
- inactive Cargo/rustc state and post-witness change revalidation;
- overlap resolution that consumes the still-exact replayed full batch,
  exclusions, current reviewed plan, expiry, and approval;
- handle-relative executor-time revalidation and durable journal fencing; and
- exact-path UI disclosure, global permanent-cleanup disablement, capacity
  verification, history, and recovery.

Absence of a textual denial or presence of revision 2 metadata proves none of
those witnesses.
