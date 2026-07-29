# `developer.rust.target` rule review

Status: bundled revision 2 with deterministic reviewed core/FFI execution,
unschedulable and not yet executable from the shipped native UI.

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

The exact pinned implementation review also covers
[`--no-deps` workspace-package output](https://github.com/rust-lang/cargo/blob/30a34c6821b57de0aaec83a901aca39f88f6778c/src/cargo/ops/cargo_output_metadata.rs#L35-L59),
[package/dependency serialization](https://github.com/rust-lang/cargo/blob/30a34c6821b57de0aaec83a901aca39f88f6778c/src/cargo/core/package.rs#L221-L237),
[dependency local-path serialization](https://github.com/rust-lang/cargo/blob/30a34c6821b57de0aaec83a901aca39f88f6778c/src/cargo/core/dependency.rs#L169-L195),
[path-source null encoding](https://github.com/rust-lang/cargo/blob/30a34c6821b57de0aaec83a901aca39f88f6778c/src/cargo/core/source_id.rs#L625-L634),
and [recursive workspace path-dependency discovery](https://github.com/rust-lang/cargo/blob/30a34c6821b57de0aaec83a901aca39f88f6778c/src/cargo/core/workspace.rs#L850-L990).
Those exact-commit sources, not current Cargo behavior in general, support the
bounded accepted profile below.

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
cannot reach a filesystem effect. The stricter marker also narrows the
read-only CLI's artifact classification; the CLI has no cleanup authority.

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

The catalog digest is checked during build and load. This exact rule ID and
revision, together with the independently reviewed Python `__pycache__`
revision 2 rule, may carry the safe-regenerable action pair; the other bundled
rules remain informational and reveal-only. Both proposed safe rules remain
blocked by `ProtectedPath` and unschedulable.

## Private promotion checkpoint

The planner now has a private, non-Clone `RustTargetPromotion` token as an
intermediate authority join. Admission requires the exact revision-2 policy,
one target path, the three immutable marker facts, the deterministic candidate
ID, the source scan and live target, and a complete immutable-body comparison
against the retained durable candidate record. A candidate-bound Cargo grant
must also revalidate its Cargo/read-set, process-quiescence,
descendant-policy, home-volume, protected-rule, and target evidence.

The token deliberately keeps the original `ProtectedPath` blocker and has no
blocker-removal, plan, approval, journal, FFI, scheduling, or filesystem-effect
method. Generic exact review and cleanup-plan construction continue to reject
blocked candidates. A later typed plan-facts boundary must consume this token
before any actionable representation can exist. The promotion can now be
consumed into a private `RustTargetPlanFacts` capability after immediate grant
revalidation; it retains the blocked candidate and canonical scan-root/target
witnesses and still cannot construct a cleanup plan. The next boundary must
join these typed facts to a plan without exposing blocker removal to callers.
That private facts-to-plan join now produces a permanent-safe domain plan plus
the retained authorization after repeating plan-shape validation. It is not
yet wired to exact review, approval, journal, scheduling, FFI, or effects. A
private handoff can now pair that plan with its retained authorization in the
existing reviewed-plan wrapper only after exact item/path matching and another
grant revalidation.
The private handoff can also pass through the existing expiry-checked approval
capability, which revalidates the plan and grant again while remaining before
journal, FFI, scheduling, and effects. The approved private handoff can now
persist and claim the planned session through the existing owner/generation
fence with canonical timestamps. The engine now consumes that claimed session
through the existing descriptor-relative executor, including its journal,
identity, cancellation, and terminalization fences. This bridge remains
crate-private and has no production evaluator, FFI, UI, scheduler, AI, or CLI
caller.

The first production-shaped evaluator-to-planner pipeline now acquires an exact
successful scan/evaluation source, rehydrates its domain candidate from the
current bundled catalog, compares every immutable field against durable
history, and returns the non-cloneable live witness together with that
candidate. The EngineHandle entry point remains crate-private and stops before
Cargo/protected-root grants, plans, approval, journal, FFI, UI, scheduling,
AI, or effects.

That live input can now be consumed by the enrolled-Cargo metadata,
current-account home/mount, protected-root, process-quiescence, and
empty-descendant-policy joins. The resulting private promotion token retains
the unresolved `ProtectedPath` blocker and has no plan, approval, journal, FFI,
UI, scheduling, AI, or filesystem-effect operation.
The promotion can now be consumed with its canonical scan-root/target witnesses
into `RustTargetPlanFacts`; this repeats grant validation but still does not
create a plan ID, review, approval, journal claim, FFI value, schedule, AI
request, or effect.

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

The production metadata entry now admits a bounded positive Cargo 1.96
file-configuration observation. It reproduces cwd-ancestor `.cargo` discovery
in Cargo's order and accepts at most one of the legacy extensionless `config`
or `config.toml` at each non-Cargo-home lookup. Dual names deliberately reject
instead of relying on Cargo's legacy-name preference. Cargo-home configuration
also remains unsupported. DUX parses only top-level `include` declarations
with exact `toml` 1.1.2 and leaves all other values to Cargo. Required and
present optional includes are followed in declaration-order depth-first;
missing optional includes reject because their absence namespace is not yet
fenced.

Every root and include must be a canonical UTF-8, control-free, single-link
regular file with no symlink or hard-link alias. Full bytes and SHA-256 come
from one retained no-follow descriptor with before/open/after identity checks.
The closure is bounded to 64 unique files, 128 include edges, 16 include
levels, 1 MiB per file, 16 MiB total bytes, and 128 KiB of path material.
Policy revision 3 binds lookup choice, root/read order, file identity/length/
digest, include declarations, and exact directory-watch semantics.

DUX requires exact reviewed commit
`30a34c6821b57de0aaec83a901aca39f88f6778c`, whose source has a fixed trace
event immediately before its
[configuration-file read](https://github.com/rust-lang/cargo/blob/30a34c6821b57de0aaec83a901aca39f88f6778c/src/cargo/util/context/mod.rs#L1370-L1404).
Both metadata passes set the fixed
`CARGO_LOG=cargo::util::context=debug`; stderr is already bounded. DUX accepts
only the pinned DEBUG record grammar and compares the complete ordered path
sequence and domain-separated digest with its independently captured closure.
Missing, reordered, extra, malformed, duplicated, or newline-spoofed records
reject. This is positive path-intent evidence, not kernel proof that Cargo read
the captured inode.

On macOS, close-on-exec vnode watches retain every exact configuration file
plus deduplicated ancestry through the filesystem root. Exact files and direct
parents treat write/replace events as terminal; higher ancestors retain
rename/delete continuity without failing on unrelated sibling writes. Higher
ancestors whose `.cargo` entry is absent and Cargo home use exact before/after
absence so unrelated directory/cache writes do not reject. A transient
create-remove in those absent locations remains an explicit inference limit.

The project cwd is also opened no-follow and retained. On macOS the production
runner installs it with `posix_spawn_file_actions_addfchdir_np`; the descriptor
is explicitly inherited only for that file action and closed before exec.
Descriptor and pathname identities are revalidated around launch. This removes
pathname resolution from child cwd selection. Every config lookup directory
must be local APFS; other, remote, virtual, and unprobeable filesystems reject.
Kqueue is strong reviewed-filesystem change inference, not direct evidence of
the exact config inode Cargo read. Either witness is still supporting evidence
only: neither can
clear `ProtectedPath`, construct a plan, cross FFI, schedule, or execute.

## Workspace-member manifest closure

Cargo reports member manifests only after parsing the workspace, so DUX does
not treat the first metadata document as authority. It validates that
discovery document, requires a non-empty one-to-one relation between at most
256 opaque `workspace_members` IDs and local `packages`, and rejects duplicate
or unknown member/default IDs, extra packages, non-null package sources, and
manifest aliases. The root `Cargo.toml` is always included, including for a
virtual workspace where it is not a package.

DUX then captures the declared root/member manifests as canonical descendants
of the witnessed workspace. Every manifest must be an exact single-link
regular file named `Cargo.toml`. The closure is bounded to 257 manifests,
4 MiB per file, 64 MiB total contents, 256 KiB of native paths, and 512 unique
watched directories. Its revision-1 digest binds root/member role, opaque
package ID, native path bytes, filesystem identity, byte length, and full-file
SHA-256. On macOS, exact manifest descriptors plus deduplicated ancestry from
each member through the filesystem root must be local APFS and remain behind
vnode fences while DUX runs the identical command again. A descriptor-budget
preflight includes currently open descriptors and reserves 128 more for the
app and launch; insufficient `RLIMIT_NOFILE` rejects rather than silently
dropping coverage. Configuration and workspace guards are rechecked before
spawn and resume, throughout bounded output, after exact-child reaping, and
before evidence extraction. Only a byte-identical, independently parsed second
result is accepted. Current resolution-policy revision 11 records the manifest policy,
member and retained-manifest counts, closure digest, accepted-output digest,
and configuration root/file/edge/byte plus read-intent evidence.

Before the discovery pass, DUX also captures Cargo 1.96's complete potential
ancestor-manifest candidate list in nearest-to-farthest order, with the pinned
`target/package` and Cargo-home stop behavior. Policy 2 admits at most 64
canonical UTF-8 candidates and binds every directory identity plus each
present direct `Cargo.toml`; present manifests must be single-link regular
files and are fully hashed under 4 MiB per file, 64 MiB aggregate, and native-
path bounds. On macOS, the guard captures an FSEvents cursor before
observation and replays a short-lived, bounded history stream after arming the
farthest absent-candidate directory. Replay must reach history-done with
monotonic event IDs and the same volume UUID; dropped/coalesced, wrapped,
unknown, or incomplete coverage fails closed. Exact `Cargo.toml` matching
honors the mounted volume's case-sensitivity semantics. Present manifest file
events and candidate-directory delete/rename/revoke events are terminal;
directory entry writes request exact replay, and a create/remove of an absent
candidate is terminal while unrelated sibling activity is ignored. Counts,
present bytes, and the domain-separated ordered closure digest are retained in
resolution policy 12.

The strict document also requires every package's serialized dependency list.
Across at most 4,096 declarations and 256 KiB of aggregate local-path text,
path-dependency policy 1 requires a local source and path to occur together,
requires each path to be absolute, normalized, and control-free, and requires
the exact `path/Cargo.toml` to equal one reported package manifest. Registry
and Git sources must not carry a local path. Duplicate declarations, including
across dependency kinds, remain repeated owner-to-target edges while the unique
target count remains separate. A domain-separated digest binds those sorted
rows; the accepted-output digest separately binds their complete JSON fields.
Because every admitted target is a reported package, its manifest is already
covered by the workspace guard throughout the accepted second pass. Malformed
graphs and unreported targets reject before that pass can produce a witness.

The strict document now also requires every package's complete serialized
`targets` array. Target-namespace policy 1 admits at most 256 packages and
4,096 targets, with at most 16 kind labels per target, 512 KiB of target text,
2 MiB of native path material, and 16,384 namespace records. Every reported
`src_path` must be an absolute normalized canonical single-link regular file
inside the witnessed project. Symlink, hard-link, missing, special-file, and
external source aliases reject.

For every reported package root, DUX independently captures Cargo 1.96's
finite target auto-discovery namespace: `src`, `src/lib.rs`, `src/main.rs`,
`src/bench.rs`, implicit `build.rs`, the complete direct entries of `src/bin`,
`examples`, `tests`, and `benches`, and `main.rs` below every direct child
directory. It also captures the edition-2015 `src/<target-name>.rs` fallback
for every reported target. Package roots, present entries, absence, entry kind,
filesystem identity, single-link file shape, target declarations, and ordered
native names are bound into one domain-separated digest.

On macOS, every present object and the complete ancestry of reported source
paths must be on local APFS and fit the descriptor budget. Vnode fences make
writes to every discovery directory terminal, including a create/remove that
restores the prior entry set; exact file replacement, link, attribute,
rename, delete, and revoke events are also terminal. DUX replays the entire
observation before evidence extraction. The target guard is polled with the
configuration, ancestor-manifest, and workspace-manifest guards throughout
the accepted second Cargo pass. Resolution policy 12 retains package, target,
namespace-record, and closure evidence beside the earlier provenance rows.

Before either metadata pass, workspace-glob policy 1 separately captures the
generation that produces those reported members. It parses the exact
single-link root manifest with TOML 1.1.2, distinguishes absent and empty
member/default-member declarations, applies pinned glob 0.3.3 component
semantics to `members` and `default-members`, and preserves Cargo's literal
prefix handling for `exclude`. Raw files match before Cargo's final directory
filter and therefore do not trigger its zero-match fallback; recursive `**`,
leading-dot names, `*`, `?`, and classes use the reviewed Cargo behavior.
Literal components use native targeted lookup (including case-insensitive APFS
resolution), retain the selected present/missing state, and fence the parent
generation without enumerating unrelated siblings. Ordered duplicate explicit
default-member rows and distinct recursive derivations are preserved.
Canonical workspace-root components with glob metacharacters are outside the
confined profile only when a non-empty declaration invokes glob expansion,
because Cargo then interprets those components as part of its absolute glob.

The admitted profile permits at most 256 strings per array and 768 total, 4
KiB per string, 256 KiB total text, 64 components/depth, 4,096 directories,
65,536 namespace entries and raw matches, 8 MiB across retained native-path
copies, 262,144 traversal states, and 2,097,152 comparisons. Direct entries are
charged before storage. The conservative observation rejects selected
symlinks, special or unreadable entries, escapes, invalid patterns, and every
over-bound case.

The root manifest and every consulted local-APFS directory are armed before
the discovery pass and replayed after arming. Root-file changes and directory
writes, including create/remove restoration, are terminal throughout both
commands. Reported-membership-consistency policy 1 requires non-excluded
expansion and an eligible root package to seed the reported packages, verifies
reachability through Cargo's validated serialized dependency edges, and
reproduces explicit defaults or Cargo's virtual-all/package-root fallback
exactly.

Dependency-manifest policy 1 then independently parses the exact retained
manifest bytes with TOML 1.1.2. It enumerates local `path` entries from normal,
development, build, and target-specific tables, including values inherited
from `[workspace.dependencies]`. Direct paths are relative to the declaring
manifest and inherited paths to the workspace root. Normalization must produce
one of the canonical single-link manifests already owned by the workspace
guard. Duplicates across tables remain distinct, and the complete sorted
owner-to-target multiset must equal Cargo's reported local edge multiset.
The profile shares the 4,096-declaration and 256-KiB path-text limits;
malformed values, escapes, unsupported inheritance, invented or omitted rows,
and unreported targets reject. The guard revalidates around parsing and policy
1 binds counts, unique targets, and a domain-separated closure. Resolution
policy 11 binds the workspace namespace, reported graph, and independent
manifest graph together. These remain observations and cannot clear
`ProtectedPath`.

Package-metadata policy 1 also derives every package's `readme` and
`license_file` output from those exact bytes. Direct strings, `readme = true`
and `false`, and values inherited from `[workspace.package]` are reproduced,
including Cargo's member-relative rebasing. If direct `readme` is absent, DUX
captures all three ordered implicit candidates—`README.md`, `README.txt`, and
`README`—and selects the first single-link regular file while retaining every
missing/directory observation. Symlinks, hard links, special entries, absolute
paths, escapes, malformed values, and out-of-workspace targets reject. Limits
are 256 packages, 4 KiB/value, 256 KiB total text, 2 MiB native paths, 2,048
namespace records, 64 components, and 4,096 watched objects. Exact manifests
and package roots are local-APFS fenced, and root directory writes are terminal
across the accepted pass. Explicit README/license targets are not opened by
the pinned metadata command, so this policy deliberately attests only their
declaration-derived path, not existence or contents. Resolution policy 12
binds its package/declaration/probe/selection counts and domain-separated
closure without adding authority.

This proves the exact reported root/member manifest bytes remained stable
under the reviewed path-based inference model and excludes unreported local
dependency declarations from accepted witnesses. It does not attest those
unreported manifests. Cargo can read an external manifest during discovery
before DUX rejects the document, while standalone or excluded path
declarations can be conservatively rejected even when Cargo did not read the
target manifest. Cargo 1.96's exact
`metadata --no-deps` code path deliberately does not load or create
`Cargo.lock`; real-Cargo tests include a malformed lockfile to pin that
version-specific behavior. DUX still does not prove Cargo's full read set or
remote dependency attributes,
transient
absent ancestor create/remove, attestation of safe unreported path
dependencies, external discovery-manifest stability, or kernel-level read
identity. It is not fd-based Cargo reads;
kqueue remains event inference and same-UID/post-witness changes still require
later guards. `ProtectedPath` and every authority edge remain unchanged.

## Suspended macOS launch and selected-running-code continuity

The admitted macOS production path now closes the executable swap/restore race
without claiming an fd-based exec primitive that Darwin does not provide. DUX
first opens vnode-event descriptors for the exact enrolled executable and each
canonical ancestor through the filesystem root. Every watched object must be
on local APFS. It then revalidates the full executable SHA-256, identity,
single-link shape, cwd, and config closure before directly calling
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

## Cargo-bound planning grant

The next private join consumes the complete Cargo/read-set boundary together
with the exact source scan, candidate, target snapshot, current-account
home/mount boundary, and requested/canonical protected-root assessment. The
grant repeats every retained Cargo and filesystem fence and fails closed on
foreign identities, target replacement, manifest/read-set drift, policy drift,
or boundary changes. Production cannot mint a Rust-target scope token without
this Cargo provenance. It remains non-actionable: the candidate retains its
`ProtectedPath` blocker and the grant cannot create a plan, approval, schedule,
FFI value, or cleanup effect.

The scope token consumes two narrower private grants. The home-volume grant is
revisioned and consumes the current-account `TrustedHomeMountWitness`. The
protected-rule grant binds the exact rule/revision, a code-owned stable
boundary key, the scan-root/target identities, and both requested and
canonical `NoTextualMatch` policy revisions. These grants repeat their own
evidence and fail closed on key, policy, volume, or target drift; they do not
remove the candidate's `ProtectedPath` blocker.

Before a Cargo boundary can be retained, DUX also captures exact inactive
process-name guards for `cargo` and `rustc` through the bounded macOS libproc
provider. The guard set is code-owned, never shell-derived, and is revalidated
at every boundary reuse. Active, incomplete, malformed, PID-replaced, or
wrong-guard observations fail closed. Proof revision 2 validates the complete
bounded PID/name table and requests start-time/executable identity only for
exact guarded names. An inaccessible matching `cargo` or `rustc` process is
still a refusal; unrelated processes do not need to disclose executable paths.
A zero-length or full PID buffer rejects as incomplete/possible truncation,
and failed PID inspection is treated as disappearance only when libproc returns
`ESRCH` or a second
independently complete PID table proves that exact PID absent. This prevents
cleanup while the known writers are active but does not claim that an
already-open descriptor cannot exist, and it does not clear `ProtectedPath`.

The current rule declares no protected or excluded descendants, so the Cargo
boundary still carries an explicit empty descendant-policy witness. A supplied
non-empty selector set is rejected rather than silently ignored. Once a rule
declares selectors, it will need its own code-owned selector policy and
descriptor-relative executor proof before it can become actionable.

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

The later private planner-to-journal handoff retains the discovery
`ProtectedPath` fact rather than deleting it. Checksummed schema v12 records a
revisioned active-claim seal only after the trusted Rust-target capability has
matched the exact candidate, session, and item ordinal. Planned and active
recovery loads require that seal and independently re-check the complete
candidate body with exactly the sole `ProtectedPath` blocker. Ordinary claims,
forged or moved seals, blocker drift, and unsealed pre-v12 trusted work cannot
recover this exception. Terminal claim settlement cascades the seal away. This
remains crate-private and grants no FFI, UI, CLI, AI, or scheduled cleanup
route.

UniFFI contract v27 and native Settings now expose this exact enrollment
lifecycle without exposing the planner. The user selects one direct `cargo`
file as lossless bytes; static inspection executes nothing and returns one
engine-bound, consume-once preview whose bounded path, digest, and static-code
records are display observations only. The explicit final confirmation
discloses that DUX will execute those exact inspected bytes with its fixed
bounded version command. Each engine admits at most one live preview, commit
checks ownership before consuming it, close releases it, and status/revoke
accept no path or reconstructed identity. Ad-hoc signing is presented as local
integrity rather than publisher authentication. This surface still establishes
only local executable provenance for discovery. Positive `.cargo/config`,
legacy extensionless
config, and recursive `include` inputs are not directly attested; projects
containing them now reject. The bounded negative lookup observation, retained cwd,
suspended selected-code checkpoint, and guarded two-pass root/member manifest
closure cover the currently admitted direct-executable case, but launch and
manifest reads remain path-based and the complete Cargo read/namespace set is
not attested. Direct-read or generation evidence is still required before
promotion. The FFI surface cannot create a candidate, plan, approval, journal
claim, schedule, AI request, or cleanup effect, and `ProtectedPath` therefore
remains untouched.

UniFFI contract v28 and the native Explorer can now prepare a short-lived,
observation-only preview after this complete deterministic chain succeeds for
one exact revision-2 candidate. The request supplies only the candidate ID
through the candidate's retained Explorer review. Rust derives the current
target, source scan, plan ID, permanent-safe mode, estimate, ordered warnings,
rule facts, and effective expiry; the returned opaque child supports only
`info` and release. It creates no approval, durable plan/session, claim,
journal row, schedule, callback, AI request, or filesystem effect, and the
candidate remains `Discovered` with its `ProtectedPath` blocker.

The preview is bound to one exact parent-session identity, not merely its scan
ID. Parent release, drop, or expiry invalidates it; parent renewal cannot
extend the frozen child deadline. Exact current-path bytes are display-only
and cannot be supplied back to Rust. Controls, invalid UTF-8, backslashes, and
the pinned Unicode-16 format/default-ignorable set use a deterministic escaped
display that Swift rederives and compares byte-for-byte. Explorer shows that
current path separately from historical discovery evidence, along with the
estimate, warnings, and expiry, and offers only prepare/check-again/close. It
states explicitly that no cleanup was approved or performed.

UniFFI contract v31 adds one consuming edge from this exact opaque review to
the existing engine-owned permanent-safe task. A foreign FFI engine is rejected
before the review changes. Any owning-engine start attempt consumes the child
once before core revalidation, including when an information read is already in
flight or core later refuses admission. The request cannot carry a path,
candidate or plan ID, timestamp, approval flag, callback, AI result, command,
or retry token. The opaque task exports only explicit cancellation and
strictly validated, path-free polling with durable history correlation.
Dropping it neither cancels nor retries the effect.

This transport now reaches an internal native execution checkpoint, not shipped
product authorization. `EngineService`, the app model, and Explorer bind the
complete visible preview to a consume-once v31 start, independently validate
generation-fenced path-free task state, disclose changed-since-plan and unknown
outcomes, and preserve observation across window closure. The confirmation and
start action compile only with `DUX_INTERNAL_PERMANENT_SAFE_CLEANUP` in Debug;
Release exposes no execution action and the release pipeline rejects that
condition. AI, CLI, schedules, history, and reconstructed display values cannot
enter the consuming call, and every release gate below remains open.

## Required before native executable use

Native executable use requires the remaining product gates to continue proving:

- trusted home, selected-volume, canonical ancestry, and mount identity;
- a stable code-owned protected-root boundary grant;
- reviewed acceptance or mitigation of the suspended selected-code
  checkpoint's path-based and same-UID signaling limitations, direct positive
  Cargo config/include identity provenance if configured projects are ever
  admitted, and the documented manifest read-set/namespace limitations;
- current target kind, symlink, link-count, mount, and descendant policy;
- inactive Cargo/rustc state and post-witness change revalidation;
- overlap resolution that consumes the still-exact replayed full batch,
  exclusions, current reviewed plan, expiry, and approval;
- handle-relative executor-time revalidation and durable journal fencing; and
- exact-path UI disclosure, global permanent-cleanup disablement, capacity
  verification, history, and recovery.

Absence of a textual denial or presence of revision 2 metadata proves none of
those witnesses.
