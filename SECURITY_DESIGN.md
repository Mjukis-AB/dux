# DUX Security Design

Status: normative design and implementation gate

Last reviewed: 2026-07-30

Applies to: `dux-core`, `dux-cli`, `dux-ffi`, and the direct-download macOS app

## 1. Purpose and reading rules

DUX scans storage and will eventually move, remove, or evict filesystem data.
That combination makes filesystem identity, user intent, privacy, and truthful
reporting product correctness requirements rather than optional hardening.

This document defines the security boundary for those capabilities. It is the
normative companion to the product roadmap and the architecture decisions in
`docs/adr/`. When implementation and this document disagree, new/public macOS
cleanup remains disabled until the code and document are reconciled in the same
reviewed change.

Accepted architecture decisions are binding implementation defaults. A
material change to one of those decisions requires a superseding ADR rather
than a silent rewrite.

The terms **MUST**, **MUST NOT**, **SHOULD**, **SHOULD NOT**, and **MAY** are
normative. A control described as **implemented** exists in the current tree. A
control described as **required** is a shipping gate, not a claim about current
code.

This document deliberately separates:

- observations from authorization;
- deterministic policy from AI interpretation;
- review records from approval;
- estimated reclaimable bytes from verified capacity change; and
- read-only client presentation from the private reviewed-plan executor.

## 2. Security objectives

DUX has six primary security objectives, in priority order:

1. Never mutate a path that was not selected by the user or a shipped,
   deterministic, revisioned rule.
2. Never turn stale, ambiguous, lossy, AI-produced, or text-only evidence into
   cleanup authority.
3. Keep every mutation inside one reviewable executor boundary with current
   filesystem revalidation.
4. Preserve recoverability by default and state permanent or non-destructive
   behavior accurately before execution.
5. Keep scan results, paths, history, and provider payloads private and local
   unless the user explicitly invokes a disclosed AI provider flow.
6. Report partial access, partial execution, and capacity outcomes truthfully;
   uncertainty must never be rendered as zero usage, success, or safety.

Availability and performance matter, but they do not override these objectives.
Under memory pressure, timeouts, cancellation, malformed input, inaccessible
paths, provider failure, or database failure, DUX MUST fail closed for cleanup.

## 3. System and authority boundaries

The intended authority flow is one-way:

```text
filesystem observations + trusted platform facts + signed rule catalog
                              |
                              v
                     deterministic candidates
                              |
                              v
                  validated, expiring review plan
                              |
                    explicit approval / schedule gate
                              |
                              v
                serialized executor + live revalidation
                              |
                              v
                 operation journal + capacity sample

AI metadata ----------------> explanations and presentation groups only
Swift/CLI UI ---------------> requests and approval only
cache/history --------------> display and discovery hints only
```

No reverse edge may grant authority. In particular:

- AI output MUST NOT create candidates, paths, modes, plans, approvals, rule
  revisions, or executor inputs.
- Swift views and CLI interaction state MUST NOT perform filesystem mutation.
- Cached trees and history MUST NOT be treated as current path identity.
- A lexical success or protected-policy `NoTextualMatch` result MUST NOT be
  treated as an allow decision.
- A cleanup plan MUST NOT be treated as approval or as proof that its evidence
  is still current.
- Full Disk Access, unsandboxed execution, POSIX access, or successful opening
  of a path MUST NOT be treated as cleanup permission.

### 3.1 Target component responsibilities

`dux-core` MUST own portable scanning facts, deterministic rule evaluation,
candidate construction, planning, path and protected-root validation,
execution policy, operation records, and shared persistence semantics.

Platform adapters MUST own only narrowly scoped operating-system effects. On
macOS this includes capacity sampling, Trash, supported cloud-local-copy eviction,
notifications, and access guidance. Adapters MUST NOT decide that a target is
safe; they consume executor-owned requests and return typed results.

`dux-ffi` MUST expose coarse immutable records and opaque engine/task/plan handles.
It MUST NOT expose arbitrary-path mutation, raw filesystem handles, Rust
references, or an AI-to-plan bridge. Panics MUST remain contained and expected
failures MUST cross as stable typed errors.

Swift MUST own presentation, user interaction, settings, and provider process
configuration. Generated UniFFI APIs MUST remain contained in `EngineService`.
Views and `AppModel` MUST NOT import destructive capabilities.

`dux-cli` MUST become a client of the shared engine. Its temporary core-owned
legacy deletion adapter is migration debt, not a second approved architecture
or the production executor.

## 4. Threat model

### 4.1 Protected assets

DUX protects:

- user documents and irreplaceable data;
- credentials, keychains, tokens, browser profiles, messages, mail, notes,
  cloud documents, password-manager data, and security-tool state;
- system integrity, applications, other user profiles, mounted volumes, and
  operating-system metadata;
- the accuracy of rule safety tiers, execution modes, approvals, and history;
- full path listings, scan snapshots, settings, exclusions, provider payloads,
  and operation journals;
- signing, update, release, and bundled-rule integrity; and
- the user's ability to distinguish estimates, actions, and verified outcomes.

### 4.2 Threats in scope

The design assumes ordinary users can make mistakes and that filesystem state
can be adversarial or change concurrently. It addresses:

- empty variables, relative paths, traversal, repeated separators, control
  characters, invalid host-native encoding, path-prefix confusion, Unicode and
  case ambiguity, and very long paths;
- symlinks, reparse points, firmlinks, bind mounts, hard links, mount changes,
  aliasing, target replacement, ancestor replacement, and plan staleness;
- misleading cache-like names, forged or changed marker files, nested matches,
  protected descendants, active applications, incomplete cloud upload, and
  parent/child candidate overlap;
- corrupt, oversized, unknown-version, or malicious rule, cache, database,
  FFI, CLI JSON, and AI inputs;
- prompt injection embedded in file or directory names;
- provider hangs, excessive output, malformed output, child-process escape,
  ambient TCC authority, and secrets in stderr or environment variables;
- accidental introduction of raw destructive calls outside the executor;
- partial failure, cancellation, process exit, crash, concurrent clients, and
  stale operation journals; and
- compromised dependencies, mutable CI actions, mismatched generated bindings,
  unsigned rules, tampered downloads, and unsafe updater configuration.

### 4.3 Assumptions

DUX runs as the signed-in user. The first production app has no `sudo`, root
helper, privileged daemon, or launch daemon. The primary app is not App
Sandboxed, but TCC, SIP, ACLs, POSIX permissions, file flags, and filesystem
rules still apply.

User approval of an unintended item does not bypass protected roots, sensitive
categories, action compatibility, freshness, or explicit mode. The CLI is now
read-only rather than retaining a weaker exception. An advanced confirmation
is never a universal policy bypass.

The signed application bundle and embedded deterministic rules are trusted
only after Developer ID verification, Hardened Runtime signing, notarization,
and the release gates in this document succeed.

### 4.4 Non-goals

The first design does not claim to defend against:

- a fully compromised operating system, kernel, or DUX process;
- a malicious user intentionally replacing and running a modified DUX binary;
- recovery from secure deletion or guaranteed erasure of filesystem blocks;
- cleanup that requires root, disabling SIP, changing system protections, or
  executing arbitrary vendor commands; or
- containment of an arbitrary local AI command solely through flags,
  environment cleanup, or an empty working directory.

These non-goals do not weaken path protection. Even advanced or scheduled
flows MUST NOT gain a generic “ignore safety” switch.

## 5. Current implementation status

As of the review date, the macOS application exposes separately confirmed
Explorer Trash. One exact Rust-target permanent-safe reviewed-plan chain is
integrated only behind a Debug compile condition; Release contains no start
action and release automation rejects that condition. The CLI is a read-only
scan/navigation/history/reveal companion. Selected-file and bounded-directory
iCloud metadata review exists only as path-free discovery observation.
Contract v45 independently reports bracketed account, item-generation, and
file-version stability plus shared/sync-paused facts. The production container
identity fact is explicitly unsupported because the public Foundation surface
does not expose stable container identity for an arbitrary user-selected
iCloud Drive item. Favorable sync eligibility therefore remains separate from
and cannot imply identity readiness. The v45 result is memory-only and creates
no persistence, candidate, plan, journal, provider command, or effect;
cloud-eviction effect, AI-provider, and scheduled-cleanup authority remain
absent.

Contract v47 completes the subordinate read-only snapshot comparison for one exact
Explorer review. Rust alone selects the immediately preceding retained
snapshot with the same lossless root and non-null root-identity digest, retains
both exact leases, matches hierarchical host bytes, and publishes bounded
union pages and magnitude treemaps with growth and shrinkage accounted
separately. Snapshot-local node IDs and lossy display names are never matching
keys. The comparison carries historical logical/allocation observations and
coverage only; it exposes no live path, filesystem identity, candidate,
reclaimability, plan, approval, AI input, schedule, driver, or effect. The
v47 records expose both optional historical node kinds so native validation can
prove one-sided presence, replacements, and directory descent rather than
trusting the selected display kind. The native Changes mode prepares lazily,
fences every asynchronous publication to the exact parent and child
generations, releases the child before its parent on every invalidation path,
and keeps ordinary Browse state intact when comparison is unavailable. Its
table, treemap, and inspector remain path-free and contain no cleanup action.

Contract v48 adds a separate bounded observation of unclaimed durable
`running` scan records. Schema v15's partial index lets core inspect at most 64
rows plus one lookahead without traversing terminal history. The result
contains only inspected, exact-pristine, and unexplained counts plus
truncation; it omits row identity, root, path, time, age, PID, owner, scope,
bytes, and every recovery or mutation input. Neither an unclaimed row nor its
shape proves abandonment, owner death, recoverability, or reclaimable space.
The native Settings disclosure loads this observation explicitly and cannot
invoke the separate scan-recovery maintenance task.

The schema-v16 checkpoint changed only the private Rust running-scan recovery
policy. New immutable scan claims may bind separate domain-separated
stable-host and boot-scope digests with the sole `interrupt_only` policy. A
complete same-host/prior-boot tuple can authorize one exact pristine
scan-history transition to `interrupted` without probing an old PID;
foreign-host and unproven claims cannot. That checkpoint added no transport
field, path, selector, byte claim, filesystem operation, or cleanup authority
and did not change the v48 unclaimed-row census.

Contract v49 separately exposes one bounded, path-free aggregate census of
claimed `running` scans. It reports only the inspected total,
same-host/current-boot, same-host/prior-boot, foreign-host, stored-unproven, and
current-context-unavailable counts plus truncation. Stored all-`NULL`
provenance is not conflated with complete stored provenance that cannot be
compared because this process lacks complete current host/boot context.
Malformed provenance in the inspected page or lookahead fails the census. The
query exposes no identity, digest, policy, process fact, timestamp, path, byte
estimate, or selector; it performs no liveness probe, recovery admission,
filesystem traversal, or mutation and cannot invoke scan-recovery maintenance.

Schema v17 adds a separate bounded scan-scope exclusion registry shared by
engine-backed app work and uncached progressive CLI scans. Acquisition starts
only from a freshly captured canonical root and occurs before queue publication
or terminal takeover. Under the connection mutex, current-schema writer lock,
and one immediate SQLite transaction, core completely loads at most 64
immutable lease rows, conservatively removes only leases proven stale, checks
every retained root plus every legacy `scans.status = 'running'` root, and
inserts one random 128-bit token. Exact, ancestor, and descendant component
scopes conflict; siblings remain independent. Windows comparison uses
locale-free ordinal case folding over lossless UTF-16 components.

The scope token is observation exclusion only. It has no scan-history,
snapshot, candidate, plan, approval, cleanup, AI, recovery, or effect
authority. Engine cancellation and close detach queued closures under the task
registry but release their tokens only after that lock is gone. A running
scanner retains its token through quiescence and durable terminal settlement;
panic unwinding follows the same drop boundary. Release is an exact
compare-and-delete by token, root, owner, acquisition time, and provenance,
with post-commit reconciliation. If acquisition or release cannot revalidate
storage/schema state, the durable row remains a process-lifetime availability
quarantine rather than being removed speculatively.

New leases bind the exact process instance, optional reliable recovery scope,
and—when available—separate stable-host and boot-scope digests under the sole
`release_only` policy. Same-host prior-boot evidence or a reliable
same-scope exact-process `DefinitelyGone` result can reclaim a stale lease.
Age, PID alone, foreign-host evidence, unproven or unavailable scope, malformed
rows, and an unclaimed legacy running scan never authorize release. Migration
from v16 creates an empty registry and preserves all scan/claim bytes. The
running-row transition fence prevents an already-running v16 engine scan from
being overlapped after migration, while current-schema revalidation turns an
older waiting writer read-only. A pre-v17 CLI binary that never opened DUX
storage cannot be retroactively fenced, so updating every installed CLI is the
one-time protocol boundary.

`dux-core` has typed rule, candidate, cleanup-plan, lexical validation,
filesystem evidence, protected-root policy, dangerous-path tests, durable
journaling, Trash admission, and one private Rust-target permanent-safe
executor chain. Public Release cleanup is still gated because the complete
trusted home/volume/rule/platform evidence, mode, history, and verification
requirements in §17.3 are not yet satisfied.

The shared engine now runs one deterministic discovery evaluator for every
fresh successfully completed scan. Its exact bundled catalog is checked during
the crate build and again before engine storage or workers are published.
Production evaluation accepts only the same completed-scan type-state witness
used to create the immutable snapshot plus the engine-selected evaluator scope.
The current catalog contains selected-scan-root developer-artifact
observations backed by the independently implemented M0 marker projection and
two exact user-cache-root observations. Nine rules remain
Informational/RevealOnly.
The independently researched `developer.rust.target` revision 3 rule may
propose `SafeRegenerable`/`RemoveKnownRegenerableContents` only when a direct
regular `Cargo.toml` sibling and direct regular `CACHEDIR.TAG` child were both
present in the completed snapshot and complete file/directory modification-time
coverage proves the newest observation was at least seven inclusive days old at
the evaluator's exact persisted, context-bound scheduled instant. Recent or future observations and
missing required timestamps fail closed. The cache-tag filename is supporting
snapshot evidence, not proof of its standard signature or a live Cargo target.
A second independently researched `developer.python.pycache` revision 2 rule
may propose the same SafeRegenerable/RemoveKnownRegenerableContents pair only
when a direct regular, non-symlink `.py` sibling and symlink-free candidate
ancestry were present in the completed snapshot. Python cache relocation,
active writers, descendant contents, and protected-root authority remain
unresolved.
Two independently reviewed revision-1 user-cache rules match only exact direct
`Homebrew` and `pip` directory children of the engine's Rust-derived
current-account `Library/Caches` targeted root. They require candidate-local
coverage plus complete inclusive seven-day newest-mtime observations. This
recency is prioritization evidence, not provider-inactivity or ownership
proof: both rules always retain `MissingOrIncompleteEvidence` and
`ProtectedPath`, add `SymlinkBoundary` for an observed descendant symlink,
remain unschedulable, and have no planner or executor promotion. Snapshot
replay requires the exact user-cache scope; pending evaluation recovery refuses
to reconstruct that scope without a fresh OS-account root witness.
A separate sealed, crate-private planner checkpoint can now accept only that
exact candidate shape while it still carries only the `ProtectedPath` blocker.
Its production entry is bound to an exact complete durable source: one current-
identity succeeded candidate evaluation, its exact succeeded complete-coverage
scan and snapshot reference, a fully decoded checksummed retained snapshot,
and a fresh bounded `CleanupReview` lease. Acquisition rereads the complete
source after snapshot decode. Before that reread, a snapshot-native evaluator
replays the complete current catalog result without allocating a second
full-path tree. One bounded enumeration plus result-path materialization uses
per-directory marker summaries to preserve exact marker preference and
cross-rule ancestor suppression; bottom-up frames reproduce known allocation
estimates and directory/file newest mtimes. Candidate payload is charged
incrementally against the same 32 MiB durable-batch budget, and the global
4,096/4,097 boundary fails rather than truncates. The ID-keyed replay must
match every immutable field of every durable candidate, so missing, injected,
or modified candidates and changed ordered fields fail closed. Replay returns
no candidate or capability and releases decoded-memory and pin ownership on
failure. Acquisition also recomputes the deterministic Rust candidate ID and
retains scan-time device/inode observations for the root, complete ancestor
chain, target, manifest, and tag. The live witness owns this non-cloneable
source and rejects source-row drift, pin expiry, or different observed
device/inode objects. Inode reuse remains possible, so this is not proof of
unbroken object continuity and cannot grant authority.
On Unix it reconstructs current no-follow root, target, manifest, and tag
identities, requires direct default-layout parent relationships and single-link
regular markers, and reads Cargo's exact 43-byte standard cache-tag signature
through a nonblocking retained descriptor with before/open/after identity
checks. The ephemeral witness has no clone, serialization, plan conversion,
blocker-removal, FFI, or execution operation. Windows fails closed at this
planner boundary.
A second sealed Unix checkpoint consumes that witness and derives every Cargo
path and expected result from it. It rejects symlink launchers, including the
usual rustup proxy, and observes one canonical single-link executable named
`cargo` by full bounded SHA-256 plus the exact reviewed Cargo 1.96.0
verbose-version digest, and the returned witness retains that executable plus
canonical environment evidence. It then invokes only fixed format-version-1,
no-dependency, locked, offline metadata arguments from the exact manifest
parent. The child receives a cleared minimal environment;
nonblocking stdout/stderr, runtime, and JSON are bounded; timeout and overflow
terminate the original process group, and raw output is neither persisted nor
exposed. Only `resolve: null`,
the exact project workspace root, and the already witnessed target directory
are accepted. The manifest is now read in full through a retained descriptor
and SHA-256-bound before/open/after, detecting in-place content changes that
identity alone misses.
On macOS, a fifth sealed checkpoint now derives production Cargo observation
only from one explicit revisioned setting in the same durable store as the
Rust-target source. Read-only inspection binds the direct Cargo path, full
digest, canonical scrubbed environment, and strict all-architecture/no-network
Security.framework static-code evidence without executing selected bytes.
Ad-hoc signatures are classified as integrity only, not publisher identity.
Trust comes from a consuming user commit of one non-cloneable preview bound to
its exact engine/store and prior setting revision. Commit repeats the static
checks, then explicitly authorizes bounded execution to bind the reviewed
verbose-version digest before conditionally writing; exact retries are no-ops,
while replacement and revocation advance the revision. Revocation persists a
field-free tombstone so stale previews cannot recreate earlier trust. The enrolled
metadata entry statically matches the stored bytes/signature and rereads the
complete enrollment before any automatic execution, then retains the same
guard before and after the fixed Cargo command. That core-only checkpoint
exposed no blocker-removal, plan, FFI, scheduling, or effect edge.

UniFFI contract v27 exposes only the explicit discovery-enrollment lifecycle to
the native Settings surface. The request contains one lossless, bounded,
control-free UTF-8 Unix path selected by the user and cannot contain command
text, `PATH` lookup, arguments, environment, expected output, signatures,
digests, candidates, plans, or cleanup modes. Static inspection runs no
selected bytes. Its result stays inside one opaque engine-bound preview;
display DTOs are observations only and are never accepted back as authority.
An engine admits at most one live preview, ownership is checked before
consumption, commit consumes the preview before any fallible version work, and
explicit release, engine close, or object drop destroys the retained
capability. Status is read-only and revoke takes no path or identity.

The confirmed commit is a bounded synchronous settings operation that may run
the exact inspected bytes only with the core-owned verbose-version command. It
runs on `EngineService`'s utility executor, not the Swift main actor. There is
no cancellation token in this core boundary: after explicit confirmation the
UI truthfully shows that enrollment is finishing, offers no false Cancel
action, and ordered shutdown waits behind the serialized engine operation.
Swift task cancellation only generation-fences presentation. A stale, failed,
or outcome-unknown commit is never retried from the consumed preview; the app
must acquire fresh status and require a fresh inspection for another attempt.
The confirmation action carries the exact generation and full evidence shown,
so it cannot consume a replacement preview. A native mutation whose returned
record is malformed or does not correlate to that operation is treated as
outcome-unknown, followed by one observation-only status reload. Settings
visibly blocks another mutation and exposes an explicit reload action until an
authoritative read succeeds. Closing Settings releases even a late-arriving
inspection preview; it does not interrupt a confirmed mutation. Once core has
reported a successful mutation, any failure to project its transport record is
also outcome-unknown; typed core errors are preserved only before that
successful boundary. Runtime shutdown installs one shared task before its
first suspension, so reentrant callers cannot duplicate the stop/close
pipeline while a confirmed mutation is finishing.
This narrow lifecycle is discovery provenance only and cannot clear
`ProtectedPath`, build or approve a plan, create a journal claim, schedule
work, invoke AI, or perform a filesystem cleanup effect.

UniFFI contract v33 carries the separate reviewed-plan child for the exact
revision-3 Rust-target candidate. Its only request value is a
candidate ID; the exact retained Explorer parent supplies engine ownership and
source scan identity, while Rust derives the current path, cleanup mode, plan
identity, estimate, warnings, rule facts, newest observed modification time,
exact seven-day requirement, and time. The core child retains the real reviewed
capability and exposes information, consuming release, and the separately
gated consume-once task transition. Displayed values cannot reconstruct that
authority.

Preparation is split into exact-parent admission, unlocked deterministic live
work, exact-parent validation, unlocked reviewed-plan materialization, and
post-validation before registry publication. A non-reusable per-parent
identity plus a liveness token rejects same-scan parent substitution, release,
drop, and expiry. One engine admits at most one in-flight preparation or
published child. Child and parent deadlines are frozen to the earliest
relevant authority horizon; equality is expired, parent renewal never extends
the child, and capacity admission consumes stale children. Preparation,
information reads, and registry publication perform expensive work without
holding parent, child, engine, or registry mutexes. Potentially blocking parent
lease teardown can retain that parent's mutex, but runs outside engine and
registry locks on tracked bounded cleanup work. Close can therefore deny new
work immediately and preserve its bounded quiescence result.

The exact current path is a display observation, never an accepted input.
Transport carries bounded lossless platform bytes and a display
deterministically derived from those bytes. Non-UTF-8 bytes, backslashes,
controls, and the pinned Unicode-16 format/default-ignorable union are
byte-escaped. Swift reconstructs that projection and compares UTF-8 bytes
rather than canonically equivalent `String` values. It also independently
requires an absolute normalized Unix path whose final component is exactly
`target`, along with the exact rule/revision/candidate/one-item/one-path/
warning/lifetime shape. The native controller owns the opaque child, refreshes
its immutable observation on a short cadence, releases children before
parents, and generation-fences every snapshot, mode, candidate, expiry,
cancellation, and shutdown transition. Explorer provides only
prepare/check-again/close controls and states that the preview is not approval
and changed no files.

UniFFI contract v29 exposes one exact cleanup-session history observation
without exposing the journal capability that produced it. Its only request
field beyond the record version is a bounded stable session ID copied from the
recent path-free summary feed. Core runs the complete bounded journal decoder;
FFI independently checks record versions, stable identifiers, lifecycle/time
shape, complete-versus-legacy policy shape, ordered item ordinals, item/path/
evidence totals, status counts, warning uniqueness, and bounded error
categories before returning immutable presentation data. The signed verified
capacity delta remains distinct from estimated bytes.

The v29 records omit paths, evidence payloads, candidate IDs, execution owners
and generations, claims, receipts, and mutable journal state. The session ID is
only a history selector: no clear, cleanup retry, recovery, approval, callback,
scheduler, AI, or executor operation accepts it. Swift repeats the version,
identifier, lifecycle, legacy/complete shape, ordinal, aggregate, status-count,
checked estimate-sum, derived-warning-order, duplicate-session-ID, and
bounded-category checks before publishing app-owned models. AppModel
generation-fences selection, summary refresh, read retry, close, and shutdown.
The native drill-down can only read the same observation; its retry control
reloads history and cannot repeat an effect.

UniFFI contract v30 adds one separate Settings-only metadata-clear boundary.
Callers cannot supply a session, row, candidate, path, plan, approval, AI
result, cleanup instruction, or effect. Core prepares an opaque, engine/store-
bound, consume-once preview over the exact validated terminal history graph
and exposes only its count, oldest/newest start times, preparation time, and
expiry. The graph witness covers raw rows in `cleanup_sessions`,
`cleanup_items`, `cleanup_item_paths`, `cleanup_item_evidence`, and
`cleanup_plan_warnings`. Validation, hashing, and deletion use 64-session
keyset pages plus fixed per-page/per-session SQLite progress budgets; the
two-minute expiry is monotonic and does not compare durable journal time to
the current wall clock. Active/recovering rows and their claims are not
selected. Outcome-unknown effects remain active recovery evidence, while a
forged terminal parent with unfinished items or a live claim fails closed.

Commit consumes the preview before entering core, repeats the witness under
cleanup exclusion → current store/writer lease → immediate transaction, and
installs an authorizer that permits only reads and child-first deletes against
the five history tables. No external selector reaches those statements.
Changed terminal history rejects before deletion. A commit-observation failure
returns success only if reconciliation proves no terminal graph remains,
returns the original error only if the exact witness remains, and otherwise
returns outcome-unknown without retry. Swift independently validates the
preview and correlated count, binds destructive confirmation to those exact
facts, fences concurrent/stale history publication, releases unconfirmed
authority, waits through confirmed clearing at shutdown, and performs exactly
one read-only refresh after a terminal response. Settings says this is a local
metadata privacy action: no filesystem cleanup, snapshot/scan/candidate
deletion, setting/exclusion/capacity/AI deletion, compaction, capacity sample,
or free-space claim occurs.

The admitted macOS metadata case now also has bounded positive Cargo 1.96
file-configuration provenance. DUX reproduces Cargo's cwd-ancestor lookup
order, admits one unambiguous `config` or `config.toml` at each non-Cargo-home
lookup, and independently follows only top-level `include` declarations using
the exact TOML 1.1.2 parser generation used by the enrolled Cargo. Every
admitted root/include must be a canonical UTF-8, control-free, single-link
regular file and is read and SHA-256-bound through the same retained descriptor.
Configuration policy 3 binds lookup selection, root/read order, file identity
and bytes, include edges, and watch semantics under 64-file, 128-edge,
16-level, 1-MiB/file, 16-MiB aggregate, and path-material bounds. Cargo-home
configs, dual config names, cycles, aliases, symlinks, missing optional
includes, and ambiguous paths reject.

Both fixed metadata passes require the exact reviewed Cargo commit
`30a34c6821b57de0aaec83a901aca39f88f6778c` and set only its pinned
`CARGO_LOG=cargo::util::context=debug` tracing target. Cargo 1.96 emits one
fixed DEBUG record immediately before each configuration-file read; DUX parses
only exact bounded records, rejects malformed/spoofed sequences, and requires
the full count, order, paths, and intent digest to equal the independent
closure. Identity-bound close-on-exec kqueue watches cover every exact config
file and its ancestry on local APFS throughout both passes. Higher absent
`.cargo` lookups and Cargo home still use exact before/after absence without
treating unrelated ancestor/cache writes as terminal, so a transient
create-remove there remains an explicit inference limit. The child enters the
retained no-follow project directory through `fchdir`; descriptor and pathname
identities bracket launch. Other, remote, virtual, and unprobeable filesystems
reject. This proves exact path intent plus strong reviewed-filesystem byte and
identity stability, not which kernel file descriptor Cargo opened.
The admitted macOS runner now closes the executable swap/restore race with a
direct suspended-launch checkpoint. It arms local-APFS vnode fences for the
exact enrolled executable and every canonical ancestor, then uses only direct
`posix_spawn` with a new process group, fixed signal state,
`START_SUSPENDED`, and `CLOEXEC_DEFAULT`. The retained cwd is selected by file
action. Before `SIGCONT`, kernel records must show the expected direct stopped
child, process generation, credentials, group, and cwd; Security.framework
must validate that live guest and return a selected Code Directory hash found
in the enrolled all-architecture static record. Full executable
digest/identity and cwd/config guards are rechecked before resume and after
exact-child reaping; executable/config fences are polled during bounded
collection. Resolution
policy 3 retains launch-policy revision 1 and a digest of the running hash.
This is selected-running-code continuity, not fd-based/pathname-independent
execution or confinement: macOS exposes no supported fd exec here, a same-UID
actor may externally resume the child, and kqueue is event inference.
The metadata witness now also binds reported workspace-member manifests with a
two-pass protocol. A strict first document declares at most 256 unique local
member IDs/packages; the virtual or package root and every member `Cargo.toml`
are captured as canonical single-link descendants under explicit file, path,
and aggregate bounds. A revisioned digest binds role, opaque member ID, native
path, identity, length, and SHA-256. Exact local-APFS manifest and complete
ancestry-through-root vnode fences remain polled while an identical second
command runs; a current-open-descriptor plus 128-slot reserve preflight rejects
insufficient process limits instead of reducing coverage. Only that
independently parsed second output is accepted. Resolution policy 8 retains
manifest policy 1, member/manifest counts, closure digest, launch evidence, and
accepted-output digest together with configuration-policy-3 file/edge/byte and
read-intent evidence. Before discovery, a separate policy-1 closure reproduces
Cargo 1.96's at-most-64 ordered ancestor-manifest candidate namespace and
binds candidate presence, directory identities, and full bounded hashes of
present single-link manifests. Local-APFS manifest and candidate-directory
delete/rename/revoke events are terminal; directory entry writes replay the
exact observation, rejecting persistent changes without treating unrelated
restored high-ancestor activity as a manifest change. Cargo 1.96's exact
package serialization is also required to expose a closed path-dependency
graph. Path-dependency policy 1 admits at most 4,096 declarations and 256 KiB
of aggregate local-path text, requires the exact local-source/path invariant,
and maps every normalized absolute local target to one reported package
`Cargo.toml`. Its domain-separated evidence binds total/local/unique counts
and duplicate-preserving sorted owner-to-manifest edges. Every admitted target
is consequently behind the workspace-manifest guard throughout the accepted
second pass; an unreported external, excluded, or standalone target rejects
without producing a witness. This is deliberately conservative: discovery
may already have read a rejected external manifest, while Cargo may not read
some standalone or excluded targets that DUX still rejects. Cargo 1.96's exact
`metadata --no-deps` path deliberately
does not load or create `Cargo.lock`; an executable malformed-lock regression
pins that reviewed version-specific behavior. This remains path-based stability
evidence for reported and potential ancestor manifests, not proof of Cargo's
complete reads. Workspace-glob generation is addressed separately below.

Target-namespace policy 1 additionally requires every package's bounded
serialized `targets` array and every reported source path to be a normalized,
canonical, single-link regular descendant. Across at most 256 packages and
4,096 targets, it binds Cargo 1.96's conventional `src/lib.rs`, `src/main.rs`,
`src/bench.rs`, implicit `build.rs`, `src/bin`, `examples`, `tests`, and
`benches` probes, complete direct directory entries, child `main.rs` probes,
and edition-2015 `src/<target-name>.rs` fallbacks. The closure is limited to
16,384 records, 2 MiB of native paths, and 512 KiB of target text. Local-APFS
vnode fences make discovery-directory writes terminal, so transient
create/remove cannot restore accepted state unnoticed; exact observation
replay brackets the accepted pass. Resolution policy 8 binds its package,
target, namespace, and digest evidence without adding authority.

Workspace-glob policy 1 now closes the workspace-member generation that
precedes those reported-package observations. Before the first metadata pass,
DUX reads the exact canonical single-link root `Cargo.toml` under a 4 MiB bound,
parses it with the pinned TOML 1.1.2 generation, and preserves root-package,
workspace, absent-versus-empty member/default-member, and literal exclude
facts. Member and default-member patterns use pinned glob 0.3.3 behavior,
including leading-dot matches, `*`, `?`, classes, recursive `**`, raw-file
filtering, and the zero-raw-match literal fallback. `exclude` remains Cargo's
literal normalized prefix test rather than another glob expansion, including
the raw-member-prefix override.

The positive profile admits at most 256 declarations per array and 768 total,
4 KiB per declaration, 256 KiB aggregate declaration text, 64 components and
traversal depth, 4,096 consulted directories, 65,536 namespace entries and raw
matches, 8 MiB across retained native-path copies, 262,144 traversal states,
and 2,097,152 entry comparisons. Directory enumeration applies its N/N+1 check
before storage. Metacharacter components observe complete frontiers. Literal
components instead use Cargo's targeted native lookup, retain the selected
present/missing state, and watch the parent generation without enumerating
unrelated siblings; ordered duplicate default-member rows and distinct
recursive derivations are preserved.
Escapes, malformed patterns, selected symlinks or special entries, unreadable
consulted namespaces remain outside the conservative profile. Canonical root
components with glob metacharacters are rejected only when a non-empty member
or default-member declaration invokes Cargo's absolute glob expansion.

On macOS the exact root manifest and every retained directory must be on local
APFS and fit the current-descriptor plus 128-slot reserve. All watches are
armed and the complete observation is replayed before Cargo starts. Root-file
mutation and directory write/delete/attribute/link/rename/revoke events are
terminal, so a create/remove that restores the same glob result cannot survive
either metadata pass. Reported-membership-consistency policy 1 rejects a
disconnected final document: non-excluded expanded members and an eligible
root package seed the reported set, every other package must be reachable
through Cargo's validated serialized local path-dependency graph, and explicit
or implicit defaults must reproduce the reported IDs exactly.

Dependency-manifest policy 1 independently derives the admitted local path
graph from the exact retained workspace-manifest bytes. The pinned TOML 1.1.2
parser enumerates direct and workspace-inherited `path` values from normal,
development, build, and target-specific dependency tables. Paths are
normalized relative to the declaring package or workspace root, must resolve
lexically to one of the already captured canonical single-link manifests, and
retain duplicate owner-to-target rows. The independently derived multiset must
exactly equal Cargo's reported local multiset under 4,096-declaration and
256-KiB path-text bounds. The workspace guard is revalidated before and after
parsing. This closes fabricated, omitted, and redirected local edges for the
admitted profile; it does not independently reproduce remote dependency
attributes or prove which descriptors Cargo read.

Package-metadata policy 1 independently derives each reported package's
README and `license_file` values from the same exact manifest generation.
Direct strings, `readme = true/false`, and `[workspace.package]` inheritance
are reproduced with exact package-relative output. When direct `readme` is
absent, DUX observes Cargo's ordered `README.md`, `README.txt`, then `README`
namespace, distinguishing missing, directory, and single-link regular-file
state. Symlinks, hard links, special entries, absolute paths, malformed text,
escapes, and out-of-workspace targets reject. At most 256 packages, 4 KiB per
value, 256 KiB aggregate text, 2 MiB native paths, 2,048 records, 64 path
components, and 4,096 watched objects are admitted. On local APFS, package-root
directory writes are terminal, so implicit-name create/remove restoration
cannot survive the accepted pass. Pinned Cargo does not require or read the
file named by an explicit README or license declaration during metadata; those
values therefore prove path derivation only, never file existence or content.

Resolution policy 12
binds root, pattern, namespace, match, seed, excluded, reachable, default, and
reported/independent dependency plus package-metadata and ancestor-replay
closure evidence. No authority edge is added.

Attestation and discovery stability for unreported path dependencies, kernel
actual-read identity, and post-witness mutations remain outside the proof.
Those limits and the remaining authority grants keep `ProtectedPath` intact.
Every rule remains unschedulable, and every emitted candidate retains
`ProtectedPath` because kernel-level actual-read identity and complete Cargo
manifest/namespace provenance, protected-root, authoritative volume/mount,
process, descendant,
approval, and executor-time authority are deliberately unresolved. These rows
and all planner witnesses are observations, not cleanup authority. The durable
binding exposes no replayed candidate, blocker-removal, planning, FFI,
scheduling, or effect edge.

The CLI no longer offers filesystem deletion. The temporary
`dux-core::cleanup::legacy_cli` adapter, its raw `remove_file` and
`remove_dir_all` effects, confirmation/progress modes, worker orchestration,
and destructive-call exceptions were removed on 2026-07-29. Pressing the
former `d` shortcut while browsing is inert. The CLI remains a supported
read-only companion for scanning, navigation, selection, computed views,
history, and reveal.

This retirement closes the arbitrary-descendant authority path rather than
claiming it was migrated. Cached tree names remain lossy, non-authoritative
observations and cannot nominate a cleanup target. Any future CLI cleanup MUST
consume the same current, unexpired, reviewed-plan executor used by the native
product and MUST NOT restore a CLI-specific adapter, raw path request, or
client-owned worker.

The private core permanent-safe driver and macOS Trash bridge remain the only
product cleanup effect boundaries. Public Release exposure remains closed by
§17.3; removing the CLI exception does not satisfy the remaining trusted
home/volume/rule, mode, history, platform, or verification gates.

Scanner workers now return bounded typed coverage and issue observations with
their tree and an authoritative terminal state. Access failures, explicit
policy exclusions, depth limits, symlink omissions, mount boundaries,
cancellation, probe timeouts, and pool exhaustion qualify totals instead of
silently becoming zero. Unknown filesystem-boundary classification is itself a
partial-coverage fact; it preserves scan availability rather than proving
locality. Fresh CLI results display that status, while the legacy cache cannot
carry it and therefore reloads as explicitly unknown. Relaxed scan flags and a
`Complete` observation remain non-authoritative and MUST NOT imply cleanup
authority.

The legacy binary scan cache is atomic and checksummed, but its checksum detects
corruption rather than malicious tampering and its writer does not enforce the
future 0700-directory/0600-file permissions. Cached paths are therefore
sensitive, non-authoritative data. Production callers no longer read or write
that format. The managed TUI cache uses an unrelated SHA-256-bound format and
accepts only the fixed marker-owned `Dux/scan-cache-v1` child. It preflights
header/file/node bounds before body allocation, validates every tree and
root/config semantic, requires private handle-derived storage facts, and never
falls back to legacy decoding. The conventional outer directory and every
legacy or unknown sibling remain excluded from ownership, accounting, and
clearing.

The separate application snapshot store has bounded semantic decoding, SHA-256
references, private permissions, current-schema-fenced atomic publication, and
platform storage tests. Shared engine scan tasks write it only from a private
completed-only converter that carries lossless fresh facts, validates the
document, and deduplicates hard-linked allocation. Cancelled, failed,
queued-cancelled, or panic-interrupted work receives no snapshot reference
through the engine or history. A completed traversal can leave an unreferenced
immutable orphan if file publication wins but its SQLite completion cannot be
reconciled; later bounded maintenance owns that case. The engine result,
history row, and snapshot remain observations and
cannot authorize anything. Neither snapshot implementation can grant cleanup
authority, so public app cleanup remains blocked.

The compatibility-only legacy loader reads the complete cache before structural
validation, has no retention cap, and accepts same-user replacement as ordinary
input. A forged legacy cache cannot itself mint target identity, but it can
affect displayed names, estimates, and selection and can consume memory if a
future caller reintroduces it. Production code MUST continue using the bounded
managed decoder and treating all persisted bytes as untrusted.

The following implemented controls are safe foundations, not shipping approval:

- crate-private strict lexical and live no-follow path evidence;
- revisioned, crate-private, text-only protected-root tables;
- immutable non-executable cleanup-plan records;
- a deny-unknown rule-catalog schema and private loader for test fixtures;
- a versioned dangerous-path corpus, cross-platform properties, and an isolated
  filesystem-free fuzz target; and
- a private, versioned SQLite migration boundary with path-free compatibility
  status, transactional checksummed upgrades, read-only newer-schema handling,
  and hardened owned-store provisioning; and
- release dependency, checksum, action-pinning, and packaging gates for the
  existing Rust artifacts; and
- a fail-closed local macOS release workflow with reviewed empty entitlements,
  explicit inside-out Developer ID signing, two-stage app/DMG notarization and
  stapling, Gatekeeper and identity verification, immutable versioned output,
  and retained notarization evidence.

The source tree configures Hardened Runtime for the app spike, but no public
Developer ID-signed/notarized artifact exists yet. The local workflow cannot run
without a clean exact release tag and explicit production bundle, team, signing,
and Keychain-backed notarization inputs; it rejects the temporary spike identity
instead of inventing release values. Swift rejects an incompatible
explicit DUX FFI contract before engine work, and the implemented SQLite
settings/history surfaces remain bounded observations as described below.
Launch at Login and notification authorization Settings exist, but the former
still requires production-identity validation and the latter has no delivery
API. AI, Trash, eviction, scheduled cleanup, transition notifications, and typed
TCC coverage remain unimplemented.

## 6. Cleanup authority chain

Sections 6 through 18 describe target shipping requirements unless a paragraph
is explicitly labeled as current implementation status.

Every future mutation MUST pass all stages below. A later stage cannot repair a
missing earlier witness, and no stage may infer a positive witness from absence
of a denial.

### 6.1 Fresh observation

Discovery starts from a live scan with a unique scan identity, lossless path
provenance, selected volume identity, structured coverage, and captured object
facts. Permission errors, skipped roots, timeouts, and probe-pool exhaustion are
recorded as issues. Inaccessible storage is unknown, not zero.

A binary snapshot MAY accelerate display or identify areas to rescan. It MUST
NOT directly supply actionable path identity. Any candidate originating from a
cached snapshot requires a fresh targeted observation before planning.

### 6.2 Deterministic candidate

Only code-owned deterministic evaluation of a validated, bundled, revisioned
rule may construct a rule candidate. Arbitrary Explorer selections form a
separate review-required input and default to Trash; they do not become
safe-regenerable candidates.

Each rule candidate binds at least:

- stable rule ID and nonzero revision;
- source scan ID and target volume identity;
- lossless requested and observed path forms;
- exact marker, age, size, application, process, cloud, and exclusion evidence;
- safety tier and compatible proposed action;
- every blocker and coverage limitation; and
- independently derived target identity and estimated allocated bytes.

Candidates are observations. A “no known blockers” state is not approval.

Current implementation checkpoint: evaluator revision 3 binds the exact
catalog bytes, selected scan root, source scan, persisted scheduled instant,
lossless path bytes, and structured coverage into deterministic IDs and a
version-2 context digest.
It validates every declared catalog matcher array and policy field, then relies
on the existing marker classifier's separately tested, sometimes stronger
evidence checks. It orders output independently of arena insertion order and
fails at the first match beyond 4,096 without truncation or unbounded result
materialization. The production entry point is crate-private and requires a
fresh `CompletedScanArtifact`; public/cached trees cannot mint persisted
results. Because authoritative volume identity, canonical ancestry, and a
protected-root grant are not yet present, every current finding retains
`ProtectedPath` even when scan coverage is complete. The Rust target and Python
`__pycache__` rules may carry a safe-regenerable proposed policy, but both
remain unscheduled, unselectable, and unable to enter the current cleanup
planner.

The sealed core candidate-grouping boundary now computes deterministic,
presentation-only overlap facts over one scan's complete candidates. Groups
are keyed by category, safety tier, and proposed action. Exact duplicate
observations coalesce only when their rule revision and every other immutable
fact match, with the lexical candidate ID as the input-order-independent
tie-break. A same-rule parent may own a child only when it covers every child
path; blocked candidates, mixed rules or policies, conflicting facts, and
internal candidate overlaps remain unresolved and produce zero actionable
bytes. Relative/traversal paths, duplicate IDs, and mixed scans fail closed.
The result retains indices into the original observations and cannot construct,
persist, approve, schedule, cross FFI, or execute a plan. The existing private
`CleanupPlan` constructor therefore continues to reject unresolved overlap;
trusted volume, protected-root, process, descendant, and executor witnesses
remain required before any blocker can be removed.

The next sealed planner boundary now binds a selected candidate-group result to
one code-owned canonical scan-root observation for exact-path review. It first
rechecks cleanup policy and mode compatibility, then validates each selected
path as a lossless strict descendant and captures requested spelling, canonical
spelling, relative location, volume/object identity, target kind, hard-link
count, and ordered no-follow ancestor identities. Missing, symlinked,
special, changed, out-of-scope, or multiply-linked permanent targets fail
closed; all work is capped at 64 candidates and 256 paths with checked byte
totals. Protected-root and volume grants are not yet available, so every path
retains an explicit unresolved protection disposition and the review reports
`is_actionable == false`. The non-`Clone`, non-serializable result cannot
construct a plan, clear blockers, persist, cross FFI, approve, schedule, or
mutate; future planner work must revalidate it under authoritative policy and
executor witnesses.

Exact review can now consume one matching trusted rule-scope authorization for
each selected target into a crate-private permanent-safe `CleanupPlan`. The
candidate facts retained by review are the only plan input; mode, candidate
policy, path overlap, and domain validation are rerun by the plan constructor.
Dry-run/Trash modes and missing, duplicate, or mismatched authorizations fail
closed. The resulting plan retains the authorization tokens for a future
executor-time revalidation and exposes no approval, journal, FFI, scheduling,
or effect operation.

The Rust-target authorization path is now bound to the exact Cargo planning
boundary rather than to location and textual policy alone. A private,
non-cloneable grant consumes the retained Cargo/read-set witness, exact source
scan and candidate identities, the canonical target snapshot, current-account
home/mount evidence, and a fresh requested/canonical protected-root
`NoTextualMatch` assessment. Its revalidation repeats those checks and rejects
foreign candidates, target replacement, manifest/read-set drift, boundary
changes, or policy changes with path-free errors. Production cannot mint a
Rust-target scope token without this Cargo join. The grant remains observation
only: `ProtectedPath` is retained, and no plan, approval, scheduling, FFI, or
effect capability is added.

A separate private `RustTargetPromotion` checkpoint now consumes that grant
only after validating the exact revision-3 rule policy, unscheduled action,
sole `ProtectedPath` blocker, four required marker/age facts, deterministic
candidate ID, source scan, and live target. The Cargo boundary compares every
immutable candidate fact (including estimates, modification time, evidence,
and blockers) with the retained durable discovery record; an ID/path match
alone is insufficient. The non-Clone token retains the original blocked
candidate and exposes only a later revalidation/release seam. Generic exact
review and `CleanupPlan` still reject blocked candidates, and this checkpoint
cannot clear blockers, create plans, approve, persist, cross FFI, schedule, or
mutate. The promotion token can now be consumed into a private
`RustTargetPlanFacts` capability only after an immediate grant revalidation;
the facts retain the original blocked candidate and canonical scan-root/target
witnesses. This capability still has no plan, approval, journal, FFI,
scheduling, or effect API. The next join must construct plan facts through a
typed authority boundary rather than a caller-controlled blocker flag. That
boundary now builds the permanent-safe domain plan from an internal facts
projection and returns the retained authorization as a pair; it repeats mode,
duplicate, source-scan, overlap, and byte validation. It remains disconnected
from exact review, approval, journal, FFI, scheduling, and effects until the
next orchestration join. A private Rust-target handoff now verifies the single
plan item and exact requested target against the retained grant, revalidates
again, and stores the pair in the existing `TrustedReviewedCleanupPlan`
wrapper. It remains disconnected from approval, journal, FFI, scheduling, and
effects until the generation-fenced orchestration join. A private Rust-target
entry now passes the approved capability through the existing canonical-time
planned-session persistence and owner-fenced journal claim path; no executor
effect is started by this handoff.
The same private path now passes through the existing expiry-checked approval
capability; approval revalidates the retained plan and grant and still exposes
no journal claim, FFI, schedule, or effect.

The reviewed-plan-to-journal join now has one insertion-only candidate-status
coupling for that exact Rust-target capability. It never removes or rewrites
the durable `ProtectedPath` fact. Instead, planned-session insertion requires
the sole candidate to remain revision-3 `developer.rust.target`,
`DeveloperArtifact`, `SafeRegenerable`,
`RemoveKnownRegenerableContents`, unscheduled, and blocked only by
`ProtectedPath`; it also compares the full frozen source-scan, path, byte,
modification-time, evidence, and review-state body before atomically moving the
candidate to `planned`. Only the private trusted-plan constructor can mint the
in-memory coupling. Schema v12 preserves that distinction during an active
reopen with a separate revisioned seal bound to the exact candidate, session,
and item ordinal. The decoder reconstructs the special coupling only while the
seal, ordinary candidate claim, frozen one-item/one-path permanent-safe plan,
complete candidate body, and sole `ProtectedPath` blocker still agree.
Ordinary claims must retain an empty blocker set. Forged or moved seals,
missing seals, blocker drift, and unsealed pre-v12 trusted recovery fail
closed. Terminal candidate-claim settlement cascades the seal away, so
completed history retains no active coupling. All other blocked candidates
continue to fail closed, including candidates that merely imitate the safe
policy fields. The production discovery witness remains strict while the
candidate is unclaimed. After exact planned-session insertion atomically moves
that candidate from `Discovered` to `Planned`, the core rebinds the retained
source to that sealed session and item before an owner/generation claim can
make the journal active. Binding failure therefore leaves no active owner.
This claimed form is accepted only through the complete schema-v12 journal
decoder: the trusted seal, ordinary claim, planned-or-active lifecycle,
one-item permanent-safe plan, source scan, complete immutable candidate body,
sole blocker, current evaluation, and retained snapshot reference must all
still agree. The source repeats this joined validation before every downstream
Cargo, grant, approval, and effect check; no other status transition is
normalized or ignored.

Revision 3 adds time-validity evidence to every stage of this chain. Discovery
and snapshot replay use the exact context-bound evaluation instant and require
complete file/directory mtime coverage with the newest value at least seven
inclusive days old. Before initial preview preparation, again at final child
materialization, and immediately before `effect_started`, one shared
descriptor-relative, no-follow validator inventories the target root and every
descendant twice under fixed entry/name/depth bounds and requires every
representable timestamp to remain at or before the freshly derived cutoff. The
concrete driver inventories before and after its final Cargo/read-set
revalidation, then compares each regular file's identity, type, link count,
logical size, and mtime before unlink. Missing, future, recent, linked, special,
symlinked, reordered, added, removed, or changed entries fail closed; a
pre-effect refusal unlinks nothing. The schema-v12 decoder accepts sealed
revision-2 rows only so recovery can terminalize historical state after an
upgrade. New minting, plan review, effect admission, and driver authority all
require current revision 3, so decode compatibility cannot revive old cleanup
authority.

### 6.3 Protected-root and sensitive-category policy

Denies override every allow or rule match. Protection evaluation covers the
requested and canonical forms of scan root and target. It also requires the
complete root-to-scan ancestry and an authoritative mount location, not merely
matching device numbers.

Textual protected-root policy returns only:

- denied;
- a requirement for a specific, stable rule-boundary grant; or
- no textual match.

No disposition means safe. A guarded subtree can be entered only through the exact
code-owned rule scope and grant revision for that boundary. Caller booleans,
rule data, AI output, a user-selected scan root, or execution mode cannot mint a
grant.

The path-validation layer now also retains a crate-private, repeated filesystem
boundary observation for a canonical scan root. It captures the complete
no-follow root-to-scan ancestry and platform mount identity (descriptor-bound
`fstatfs` on macOS and descriptor-relative `statx` mount identity on Linux;
Windows remains unsupported). The bounded witness can be revalidated, but it is
still observation only: it does not issue a trusted volume/location grant,
change a `ProtectedPath` disposition, or cross the planner, FFI, journal, or
executor boundary. Unix/macOS textual registry construction now obtains the
current account home from the OS account database, rejects real/effective UID
ambiguity and non-absolute or non-UTF-8 homes, captures the no-follow root
twice, and requires final-directory ownership by that account. This remains
policy input rather than a trusted home/profile grant; APFS firmlink semantics,
Windows known-folder/reparse evidence, and stable rule grants remain separate
gates.

The current-account home observation now retains a descriptor-bound owner and
mount sample alongside the no-follow root ancestry. The macOS-only
`TrustedHomeMountWitness` consumes that OS-account evidence and a canonical
scan-root witness, requiring an equal-or-descendant path relationship, the home
identity in the scan ancestry, and identical mount identity. Revalidation
rereads the account record/home boundary and both retained boundaries, so an
account relocation, owner change, root replacement, mount change, or ancestry
drift fails closed. Linux bind-mount semantics and Windows handle/reparse
evidence remain unsupported for this positive profile. This is still a
location observation, not a trusted protected-root rule grant or cleanup
capability.

Exact-path review now consumes this registry for every selected target. It
assesses requested and canonical forms independently, rejects any hard deny,
and retains specific-rule or no-textual-match dispositions as explicit
non-authoritative metadata. Failure to construct or assess the registry fails
the review closed; no textual result is an allow decision.

Exact-path review also captures the boundary witness before selected-path
validation and retains the complete no-follow ancestry and platform mount
identity in its non-cloneable evidence. It revalidates that boundary before
publishing the review, so ancestry or mount drift fails closed. This is still
observation only: it does not establish a trusted volume/location grant,
remove `ProtectedPath`, or create planning, approval, FFI, scheduling, or
effect authority.

The next boundary is represented by a separate crate-private
`TrustedVolumeLocationWitness`. It is minted only from a `CanonicalScanRoot`
and validates repeated root ancestry plus non-zero kernel filesystem identity.
macOS additionally requires an absolute kernel mount path and an exact
requested/canonical spelling (a conservative refusal of alias/firmlink-like
ambiguity); Linux requires the kernel mount ID so a device number alone cannot
bless a bind mount. Windows and other unsupported platforms fail closed. The
witness is non-cloneable, path-private, revisioned, and limited to
revalidation or comparison with the already-captured boundary. It is not yet
the trusted grant that can change a protected-root disposition; rule scope,
account/known-folder provenance, process/descendant guards, and executor-time
revalidation remain separate gates.

The bounded Cargo metadata witness now retains its complete read-set fences
after publication: configuration, ancestor-manifest probes, workspace globs,
workspace manifests, package metadata, target/source/build namespace, the
descriptor-retained project directory, the exact executable/version
observation, the optional enrollment guard, and the filesystem boundary. Its
private revalidation repeats these fences, the enrolled verbose Cargo version,
live Rust-target evidence, and boundary identity. This closes the stale-
evidence gap without making Cargo output authoritative: the witness remains
path-private, non-cloneable, blocked by `ProtectedPath`, and unable to create
a plan, approval, FFI transport, schedule, or effect.

The retained Cargo provenance can now be consumed into a sealed
`RustTargetRuleBoundaryEvidence` join with the macOS home-mount witness. The
join compares the exact retained scan-root boundary, repeats all Cargo/read-set
and account/home/mount revalidation, and carries the unresolved `ProtectedPath`
marker forward. The next private rule-scope layer promotes those observations
into a revisioned home-volume grant and a code-owned protected-rule grant. The
protected grant binds an exact rule/revision and stable boundary key, the
scan-root/target identities, and both requested and canonical
`NoTextualMatch` policy revisions; every grant revalidates before reuse. These
are still provenance and scope evidence only: `ProtectedPath` remains, and no
plan, approval, FFI, scheduling, or effect conversion is available.

Rust-target boundary admission also requires a code-owned process-quiescence
witness. On macOS it enumerates the bounded libproc table directly and accepts
only a complete observation with both `cargo` and `rustc` absent. Active,
incomplete, malformed, guarded-PID-replaced, or wrong-guard observations fail
closed; the exact guard set and fresh process table are revalidated on each
reuse. Proof revision 2 validates the bounded full PID/name table but retains
start-time and executable-path identity only for exact guarded names. Failure
to read a matching `cargo` or `rustc` identity is therefore still a refusal;
an unrelated process whose image path macOS withholds cannot by itself make
the guard unavailable. A zero-length or filled PID buffer is incomplete or
possible truncation and rejects. A failed PID detail read counts as
disappearance only for `ESRCH` or when a second independently complete PID
table proves that exact PID is absent;
still-listed, truncated, permission, transient, and otherwise unprovable
failures reject. This is a safety precondition, not proof that an already-open
descriptor is gone, and it does not remove `ProtectedPath`.

The same boundary always carries descendant-policy coverage. For the current
Rust-target catalog this is an explicit revalidated empty selector witness;
non-empty protected/excluded selectors are rejected until a rule-specific
allowlist and matching executor semantics exist. Missing, overlapping,
symlinked, multiply-linked, or changed selector evidence fails closed.

The process-activity seam is now staged as a private, non-cloneable witness.
On macOS it reads the bounded libproc table directly, never through a shell,
and retains private PID, start-time, executable-path, and process-name identity
only for exact guarded names. Exact process-name guards require complete,
duplicate-free matching records; active, malformed, truncated, inaccessible
guarded identities, and unsupported observations fail closed. Unrelated
processes require a valid bounded PID/name record but not readable executable
identity. Bundle-identifier guards do not fall back to process names and remain
unsupported until a signed bundle-identity provider can be reviewed. A fresh
revalidation rejects newly active guards and PID/image replacement, but it is
still defense-in-depth: a same-user process can retain an already-open file
descriptor or race after the check. The Cargo rule-boundary evidence may carry
this witness and revalidate it, while the current catalog declares no activity
guards, so no blocker, plan, schedule, approval, or effect authority changes.

A bounded exact descendant-policy seam is also staged. It retains validated
protected/excluded selector entries as no-follow path snapshots, including
ancestor identities, entry kind, and regular-file link count. Selector paths
are component-aware and duplicate/overlapping selectors are rejected so a
future effect cannot infer precedence. Missing, symlinked, multiply-linked,
replaced, removed, or otherwise changed entries fail closed on revalidation.
This is deliberately not recursive ownership proof: arbitrary descendant
enumeration, descriptor-relative child effects, and rule-specific protected
subtree semantics remain open. The optional witness is path-private and does
not clear `ProtectedPath`, create a plan, cross FFI, schedule, approve, or
mutate.

The first trusted deterministic-rule scope token is now staged as a separate
planner boundary. It allowlists only the revision-3 Rust-target and revision-2
Python `__pycache__` rules, consumes the macOS current-account home/mount witness and
the code-owned protected-root registry, and binds one exact no-follow target
whose requested and canonical assessments are both `NoTextualMatch`. The
non-cloneable token repeats account, mount, ancestry, boundary, policy, and
target identity evidence before release. It has no path getter, plan,
approval, persistence, FFI, scheduling, or effect operation; current
candidates therefore retain `ProtectedPath` until process/descendant,
plan, and executor joins are complete.

The next boundary consumes that fenced witness into a path-private planning-
provenance token. Creation binds the exact source-scan and candidate IDs,
witness and resolution-policy revisions, and the unresolved protected-path
marker. Token revalidation repeats those bindings and delegates to every
retained Cargo/filesystem fence; explicit release consumes its snapshot lease.
The token exposes no path, candidate detail, plan, approval, FFI, scheduling,
or platform-effect operation, and therefore cannot make a rule actionable by
itself.

Sensitive categories are a separate deny layer. Credentials, keychains,
tokens, browser profiles, messages, mail, notes, password managers,
security/management software, active VM/container disks, unknown cloud state,
and other protected categories remain informational unless a separately
reviewed narrow design says otherwise. A cleanup rule cannot weaken them.

### 6.4 Plan construction

The planner accepts only current typed candidates or an explicit user-selected
Trash request after fresh validation. It resolves parent/child overlap,
deduplicates targets and hard-linked accounting, freezes exact evidence, and
creates an immutable plan with a unique ID and short expiry, initially 15
minutes.

Every item states one proposed effect:

- dry run of Trash, permanent-safe removal, or local-copy eviction;
- move to platform Trash;
- permanent removal of known regenerable contents; or
- supported cloud-local-copy eviction.

The plan records warnings and estimates. It contains no mutable approval bit
and exposes no direct filesystem method. Changing mode, targets, evidence,
rules, exclusions, or expiry creates a new plan.

### 6.5 Approval

Manual execution requires review of exact targets, mode, estimate age,
recoverability, important warnings, exclusions, coverage gaps, and rule
provenance. Approval binds the current plan ID and its immutable digest; it is
not transferable to a regenerated plan.

Explorer selections default to Trash. Manual permanent-safe cleanup requires
explicit per-run confirmation, and the first global enablement requires the
product's typed confirmation sentence. Settings MUST provide a global switch
that disables all permanent
cleanup. That setting cannot be overridden by a plan, rule, schedule, CLI flag,
AI response, or low-disk state.

### 6.6 Executor admission

The centralized executor is the only product cleanup mutation boundary. Its
mutating entry point accepts an unexpired reviewed-plan witness, not arbitrary
paths. Dry-run types MUST be structurally unable to reach mutation.

Before the first item, the executor creates and durably starts an operation
session. For every item it revalidates, in order:

1. plan identity, expiry, approval, source, and requested mode;
2. rule ID/revision, action compatibility, schedule eligibility when relevant,
   and global settings;
3. selected volume and mount-location identity;
4. lexical scan-root and target binding using raw host-native units;
5. requested and canonical protected-root policy;
6. complete no-follow ancestry and object identity;
7. rule markers, excluded/protected descendants, age, process, application,
   cloud-upload, and other live guards; and
8. cancellation state immediately before the operating-system effect.

The Explorer Trash boundary implements admission and the explicit platform-call
half of this protocol. A non-cloneable core capability claims the store journal, checks
that the reviewed lexical path equals the frozen journal row, repeats the
no-follow root/ancestor/object witness, records a fenced `effect_started`
receipt, and revalidates that receipt immediately before a future adapter call.
Missing, changed, unsupported, or unbound evidence is recorded as a typed
rejection/changed/unavailable result. The capability has a consuming
pre-effect-cancellation path and carries no FFI path or platform primitive. A
private synchronous driver seam now repeats the target/receipt fence immediately
before the call and settles `Trashed`, `Failed`, or `OutcomeUnknown` in the
journal while the claim is held; its recording tests perform no filesystem
mutation and cannot retry. The only production caller is the explicit,
confirmation-gated Explorer action; permanent-safe, AI, CLI, and scheduled
cleanup remain separate gates.

The macOS side has an internal, dependency-injected adapter contract around
`FileManager.trashItem(at:resultingItemURL:)`. It accepts no caller path or
plan state; production calls arrive only through the core-owned synchronous
callback after the retained review lease, fixed Trash plan, journal claim,
and final no-follow revalidation. Fake tests assert that a Foundation throw
becomes `OutcomeUnknown` without a retry.

The UniFFI v21 callback contract is active only for the explicit Explorer
selection method. `TrashEffectRequest` still has no public constructor and is
consumed only once; it carries a bounded target kind, encoding, and exact
ephemeral path bytes, never a plan ID, approval, journal receipt, or arbitrary
caller path. `TrashPlatformDriver` is synchronous and returns only bounded
outcomes. The engine rejects foreign or closed review leases, creates the
fixed review-required plan internally, and invokes the driver while its
journal claim is held. Swift validates the request before constructing a URL
and consumes target metadata before the one-shot bytes; malformed or
non-round-tripping bytes never reach Foundation. AI, CLI, and scheduled
cleanup code have no call path to this method.

Any ambiguity returns a typed rejection, skip, or `ChangedSincePlan`. The
executor MUST NOT silently refresh a target and proceed; changed evidence needs
a new plan and approval.

### 6.7 Mutation and result

Mutation is serialized per engine and guarded by a cross-process cleanup lease
for the affected store and volume. The lease has an owner identity, bounded
renewal, crash recovery, and journal reconciliation; it cannot be broken merely
because a second UI wants to proceed. App, bundled CLI, standalone CLI, and any
future scheduler MUST NOT execute overlapping plans concurrently.

The initial implementation uses one permanent store-wide cleanup lock, which
safely dominates per-volume serialization until plans carry an authoritative
stable volume scope. Its operating-system lock is non-expiring and MUST NOT be
stolen because a heartbeat is old. "Bounded renewal" means a bounded cadence
for owner-and-generation-fenced journal heartbeats before new effects, not a
wall-clock lease expiry. Recovery additionally requires process-instance
liveness evidence proving the previous owner is definitely gone; PID alone,
heartbeat age, lock age, and unknown liveness are insufficient.

The implemented private liveness prerequisite uses a strict versioned owner
identity no longer than the schema's 128-byte bound. It binds PID and the OS
process-start token to a random claim nonce and, where reliable, a hashed boot
or namespace scope. The nonce prevents accidental claim reuse but is never
liveness evidence. A tri-state probe reports `Alive`, `DefinitelyGone`, or
`Unknown`; only a same-scope absence or changed start token can prove death.
Foreign/changed scope, malformed or partial platform data, permission failure,
and unsupported proof remain `Unknown`. macOS normally uses its boot-session
UUID plus `proc_pidinfo`; if a hardened runtime denies the boot-session sysctl,
it retains only the exact PID/start observation. That unscoped identity can
confirm an exact live match but can never prove death or enter scoped recovery.
Linux uses boot ID plus the current PID-namespace identity and
`/proc/<pid>/stat`. Windows retains a process handle across creation-time and
nonblocking exit checks, but without a reliable host/boot scope it can prove
only an exact live match and otherwise returns `Unknown`. The codec and probe
are crate-private evidence only: they do not read heartbeat age, mutate a
journal, claim a generation, obtain the cleanup lock, or authorize recovery or
an effect.

The version-1 process token still intentionally exposes only a combined scope,
so it cannot by itself distinguish a reboot from a copied database. Schema v14
therefore adds separate nullable
`execution_host_identity_v1_sha256`,
`execution_boot_scope_v1_sha256`, and `execution_recovery_policy` columns to
new cleanup claims. The two digests are fixed 32-byte, domain-separated
observations; the only accepted policy is currently `resumable`. Runtime
admission requires the complete tuple and rejects partial or malformed
provenance. Every row migrated from v1–v13 keeps all three values `NULL` and
remains explicitly unproven. These stored observations are not liveness
evidence, a recovery claim, or effect authority.

macOS derives the host observation from two matching bounded `gethostuuid`
reads around the boot-session UUID read, then requires that boot digest to
match the owner token's scope. Linux uses the bounded canonical machine ID for
the host digest and the existing boot-ID/PID-namespace scope for the boot
digest. Windows and other unsupported platforms emit no provenance. Hostnames,
usernames, network addresses, database paths, database-local randomness, PIDs,
and timestamps are never substitutes. Unavailable or conflicting OS evidence
therefore leaves a new claim unproven and makes later recovery unavailable;
it does not fabricate provenance.

The cleanup-owner classifier can distinguish complete same-host/same-boot
provenance from complete same-host/prior-boot and foreign-host observations.
Only same-host/same-boot provenance may continue to the existing process
liveness and exact-CAS recovery checks. Prior-boot, foreign-host, migrated,
partial, malformed, and otherwise unproven observations produce typed
non-executable results and leave the complete journal graph unchanged.
Cross-reboot cleanup-journal reconciliation remains absent.

Milestone 5 fixes the cleanup-journal policy for that missing proof: changed
boot/foreign scope and Windows non-live observations are recovery refusals, not
timeouts and not resumable cleanup. `try_recover` returns no
`CleanupJournalClaim` and performs no write, so owner/generation, session,
item, path, ordinary candidate-claim, and trusted Rust-target-claim rows remain
byte-for-byte unchanged. PID, heartbeat age, cleanup-lock availability, and
plan time are never fallback authority. Only same-reliable-scope
`DefinitelyGone` can enter the existing exact-CAS recovery state machine.
The version-1 combined scope remains insufficient for migrated claims and must
not reconcile their history. Schema v14 supplies separate provenance only for
new claims; it does not implement reconciliation. Any future prior-boot
reconciliation capability must be separately versioned, non-resumable, and
unable to validate, resume, or start an effect.

The implemented storage primitive retains one exact private
`<database>.cleanup.lock` separately from the SQLite writer marker. A
filesystem-first upgrade flushes the lock and `.cleanup.lock.ready` control
before durably advancing the retained root-ownership marker from layout v1 to
v2. That non-recreatable anchor permits writer-locked legacy upgrade, while v2
with either cleanup control missing or malformed fails closed and never creates
a replacement inode.
Acquisition is bounded but the held OS lock has no expiry. It revalidates the
retained file and pathname identity before returning; future executors must
repeat that check immediately before each effect. Windows retains both cleanup
controls without delete sharing so a locked file cannot be displaced. Unix
continues to rely on identity revalidation and the documented same-user storage
boundary. This guard is storage exclusion only—not a lease owner, approval,
recovery witness, plan capability, or effect capability. The private journal
lease now retains and revalidates it while separately proving owner/generation;
it remains uncoupled from every executor.

Where the platform supports it, permanent
removal uses descriptor-relative or handle-relative operations tied to the
validated parent. A path-based fallback requires an explicit platform review
and must not claim to close races it cannot close.

Disappearance is `Skipped`, not `Removed`. An unsupported Trash or eviction
operation is `Rejected`, not emulated with a less safe effect. Partial success
is recorded per item and never collapsed into whole-plan success.

The operation journal records the terminal state before publishing it to UI.
After execution, DUX samples filesystem capacity. It reports item outcomes and
capacity change separately.

## 7. Path and filesystem safety

### 7.1 Raw lexical validation

Validation occurs before lossy display conversion or lexical normalization. It
MUST reject:

- empty and relative paths;
- invalid host-native encoding;
- NUL and all control characters;
- current-directory and parent-traversal components;
- repeated and trailing separators;
- filesystem roots, the scan root itself, and targets outside the scan root;
- paths exceeding supported encoded-unit limits; and
- unsupported platform forms.

Accepted lexical evidence preserves the exact path units and proves only that
the target is an absolute, non-root, strict normal-component descendant of the
bound scan root. It is not filesystem evidence or cleanup authority.

### 7.2 Live identity and alias checks

The validator inspects every ancestor without following symlinks or reparse
points, captures stable volume/object identity and entry type, and compares two
snapshots to detect change during validation. The requested path and canonical
scope are retained separately; canonicalization does not erase the spelling
the user reviewed.

Future shared cleanup MUST reject:

- a symlinked or reparse cleanup base;
- an intermediate symlink/reparse point;
- a canonical target outside the selected scope;
- any transition away from the selected scan volume;
- special entries the operation does not support;
- unexpected hard-link state;
- identity or type change between observations; and
- any inability to inspect the complete chain.

Permanent and deterministic rule cleanup reject a final symlink or reparse
point. Ad hoc Trash uses a separate no-follow final-link witness that identifies
and trashes the link object itself while still rejecting a symlinked base or
intermediate ancestor.

The new boundary observation is not a grant: it must still be joined to trusted
account/profile policy and rule scope before planning. On Windows, both the
boundary witness and current validator remain non-actionable until
handle-relative reparse and mount evidence exists. On Linux, descriptor-relative
`statx` mount identity supplements (but does not replace) authoritative mount
location review; equal device identity alone never distinguishes a bind mount.
On macOS, `fstatfs` identity and mount location remain insufficient to settle
APFS firmlink semantics without dedicated integration tests. No current rule or
plan may authorize a volume crossing. Selecting an external volume as its own
validated scan root is a distinct operation, not an exception to this rule.

### 7.3 Protected roots

Per-platform protected policy covers system roots, structural anchors, user and
profile containers, selected home roots, other profiles, mounted-volume
containers, package/service roots, and guarded application-data areas. Exact
anchors are denied even when descendants may later be reachable by a specific
rule.

Protection comparison is component-aware. Static ASCII system components use
the platform's conservative case policy without inventing Unicode
normalization. A prefix lookalike such as `/Systematic` is not `/System`, while
case or alias uncertainty never becomes permission.

Environment variables MAY add conservative protected paths but MUST NOT remove
or relocate trusted protection. Home and known-folder roots used for positive
planning must come from trusted OS account/platform discovery.

### 7.4 Hard links and recursive targets

Fresh scan accounting deduplicates hard-linked allocation within its declared
scope, preserves conflicting or unidentified multiply linked allocation as
unknown, and still remains non-authoritative. Planning MUST independently
record and revalidate link counts and overlap decisions. A
permanent regular-file candidate with a link count greater than one is rejected unless a future rule
type explicitly proves ownership of every link and explains that removing one
name may not reclaim allocation; no such rule exists initially. Ad hoc Trash
may trash one reviewed link name but reports the link operation, never the full
shared allocation as freed. Recursive permanent cleanup rejects any encountered
multiply linked regular file unless its separately reviewed rule defines and
tests that condition. A rule that removes a
directory MUST define whether it owns all descendants and MUST enumerate
protected/excluded child behavior. Directory identity alone is insufficient to
authorize a recursively changing subtree.

The executor SHOULD enumerate or operate relative to retained validated
handles so a descendant cannot be replaced or redirected between validation
and removal. Until that contract exists and is tested, the new app MUST NOT
offer recursive permanent cleanup.

## 8. Rules and deterministic evidence

Rules are versioned data shipped inside the signed app bundle or bound to the
exact standalone CLI package through a separately verified catalog digest.
Unsigned standalone binaries do not gain production rule-loading authority
merely from a filesystem-relative resource. Production cleanup remains disabled
for a distribution channel until its release/package verification binds the
catalog bytes, schema version, and binary version. Rules contain
no shell fragments, commands, arbitrary glob programs, or executable code.
They use a deny-unknown schema with explicit fields. Any future variable syntax
requires an explicit recognized-variable allowlist and unknown-variable rejection.

Rule catalogs MUST pass build-time and load-time validation. Schema validity
does not prove origin or safety. Production loading additionally requires a
trusted bundled source, catalog identity/digest, reviewed rule provenance, and
tests. Synthetic `fixture.*` catalogs are test-only.

Every active rule has:

- a stable ID and nonzero revision;
- independently researched provenance, preferably vendor/platform sources;
- exact scope and component matching;
- positive, negative, nested, symlink, and changed-after-plan fixtures;
- explicit safety/action compatibility;
- required and forbidden evidence;
- protected and excluded descendants;
- age, size, process, app, and cloud guards where applicable; and
- explicit schedule eligibility.

Input order never defines precedence. Deny/protection results override cleanup
matches. A revision changes whenever an edit could alter targeting, evidence,
safety, action, or user explanation. Plans and history retain the revision.

Remote rules do not exist in the initial design. If introduced, they MUST use a
separately reviewed signed envelope, pinned verification keys, expiry and
rollback policy, atomic installation, last-known-good recovery, and rejection
of unsigned or unknown-schema data. HTTPS alone is insufficient.

## 9. Execution modes and truthful UI

### 9.1 Dry run

Dry run executes the same discovery, planning, protection, and live
revalidation decisions as the proposed real action but has no mutation
capability. Each item retains its proposed effect so UI can distinguish Trash,
permanent removal, and eviction. A dry-run success means “would currently pass
validation,” not “space was freed.”

The first production implementation is the core-owned revision-3 Rust-target
dry run. Only the exact engine-bound opaque reviewed-plan child can enter it.
Core consumes that child, projects its frozen permanent-safe plan to
`DryRun`, recomputes the mandatory warnings, and retains its exact items,
proposed removal action, authorizations, and deadline. Permanent execution and
dry run share an inert internal observation validator for the complete plan,
Cargo/read-set and process evidence, home/mount/protected scope, target
identity, seven-day subtree recency, manifest digest, and cache tag. Only the
permanent branch, inside the planner module, can convert that observation to an
effect witness; the dry-run capability has no approval, target export,
capacity, driver, or effect method.

The engine serializes the dry run with Trash and permanent cleanup, but the
permanent-cleanup opt-in does not authorize or block observation. While holding
the same cleanup lock used by exclusion writers, persistence converts only an
otherwise successful observation to durable `user_excluded`; cancellation and
more specific validation failures keep their own outcome. It then atomically
inserts an uncoupled terminal `DryRun` graph. That ownerless graph never enters
running or recovery state and has no execution owner, heartbeat, candidate
claim, trusted-rule claim, effect-start timestamp, capacity delta, or
removed-byte result. Schema v12 still requires terminal path attempt ordinal
`1`; this is only row-shape metadata and is not an execution owner or effect
receipt.

The task closes cancellation under the engine registry mutex immediately
before durable recording. Requests accepted before that boundary can convert
an otherwise successful observation to `Cancelled`; requests after it report
that terminalization has already begun and are not presented as accepted.
Ambiguous database completion can retry only exact graph reconciliation while
retaining the lease. If the one retry cannot prove the graph, the task reports
`HistoryUnresolved` and releases the non-authoritative lease; it does not enter
the process-lifetime filesystem-cleanup quarantine. The failure is never
filesystem `OutcomeUnknown`, because no filesystem call is reachable.

UniFFI contract v35 and the native Explorer preserve that separation instead
of presenting dry run as a mode of permanent cleanup. The exported start
method accepts exactly one opaque `RustTargetPlanReviewSession`; foreign-engine
objects are rejected before consumption and every owning-engine attempt
consumes the review once. It returns a distinct `RustTargetDryRunTask` whose
path-free result contains only record version, a strict
`cleanup:rust-target-dry-run:<32 lowercase hex>` history correlation, and one
allowed terminal status. The boundary has no path, candidate/plan/session
selector, Boolean mode, approval, driver, callback, capacity sampler, AI input,
or permanent-policy gate. Rust and Swift independently reject wrong task
kinds, malformed identifiers/statuses, impossible phase/result/failure
envelopes, revision regression, and cancellation rollback.

The Release Explorer surface consumes the exact displayed review and labels
the observation as a point-in-time **dry check**. Every terminal presentation
states **No files changed · 0 B freed**, never “reclaimed” or “removed,” and
requires a fresh preview before any later dry run or cleanup. Closing Explorer
does not cancel an accepted core task; explicit cancellation and app shutdown
remain the only cancellation edges. A dry-run result carries no reusable
authority and cannot chain into permanent cleanup. The separately typed
permanent action remains confirmation-gated and compiled out of Release behind
`DUX_INTERNAL_PERMANENT_SAFE_CLEANUP`.

UniFFI contract v36 adds a separate observation-only pressure-history query.
The request is restricted to one validated stable startup-volume identity, one
exact accepted capacity anchor, and a limit from 1 through 64. Rust validates
that the anchor is an exact durable raw observation or the current exact
`last_seen` observation represented without a raw row by hourly cadence; a
timestamp merely inside the volume lifetime is not accepted evidence. Rust
also validates
the full selected page plus one lookahead row against the referenced volume
lifetime, strict newest-first order, non-overlap, and the rule that only the
newest episode may remain open. The response contains only Warning/Critical
level, entry time, optional exit time, policy revision, and a truncation bit.
Swift independently rejects record, identity, anchor, timestamp, ordering,
interval, open-state, and truncation contradictions before presentation.
Explorer may draw capacity points and pressure ribbons, and the menu may show
an active period only when volume, anchor, and current pressure all match.
Neither surface receives a path, candidate, recommendation, plan, approval,
effect witness, driver, callback, AI input, or mutation command.

### 9.2 Trash

Arbitrary Explorer cleanup defaults to Trash. On macOS, the platform executor
uses `FileManager.trashItem(at:)`; it MUST NOT manually move data into
`~/.Trash`. A symlink selection trashes the link itself, never its target, and
the review UI says so.

The core now has a separate, crate-private Trash review witness that resolves a
retained Explorer node to a fresh no-follow target snapshot. It keeps the
selected final symlink as a link object while rejecting symlinked roots or
intermediate ancestors, special entries, and identity changes. This witness is
not the existing read-only live-target record, is not serializable or cloneable,
and cannot approve or construct a plan. Journal admission now owns a private
synchronous driver seam and terminal effect outcome recording, but no Swift/FFI
caller or real macOS adapter is connected. Approval, journal admission, the
centralized executor, and the core-owned callback into the macOS adapter remain
required before any mutation.

Linux implements the XDG Trash specification with correct mount behavior or
refuses Trash. Windows uses an independently reviewed recoverable shell API or
refuses. No platform may silently substitute permanent deletion.

“Moved to Trash” is not “space freed.” DUX never empties all Trash as a side
effect of another operation.

### 9.3 Permanent-safe removal

Permanent-safe mode is limited to shipped deterministic rules whose current
evidence proves known regenerable contents and whose action is compatible with
`SafeRegenerable`. It is disclosed as irreversible and reports only observed
item removal until a capacity sample shows change.

The current core contains a private, tested Rust-target contents driver staged
behind the approved-plan and cleanup-journal witnesses. It inventories the
entire target before mutation, preserves the direct `CACHEDIR.TAG` marker, uses
descriptor-relative no-follow operations, rejects unsafe or multiply-linked
descendants, and records cancellation or unknown outcomes conservatively. A
private production-core acquisition pipeline now joins only an exact succeeded
scan/evaluation source to the current bundled catalog, rehydrates and compares
the full immutable candidate body, and returns a lease-backed live witness with
the domain candidate. It is crate-private and stops before Cargo/protected-root
grants, plans, approval, journal, FFI, UI, scheduling, AI, or effects. A
subsequent private authority join consumes that witness through enrolled Cargo
metadata, current-account home/mount, protected-root, process-quiescence, and
empty-descendant-policy evidence, returning only a non-cloneable promotion
token that retains `ProtectedPath` and still cannot form a plan or effect. A
private facts handoff now consumes that token with its canonical scan-root and
target witnesses, repeats grant validation, and retains the unresolved blocker
without creating a plan ID, review, approval, journal claim, or external
authority. A
crate-private engine bridge now accepts only the non-cloneable approved
session, selects that driver, and proves journal settlement and cleanup history
in a project-local fixture. A private session orchestrator now consumes the
same capability in deterministic item/path order, continues after durably
settled failures, stops on outcome-unknown recovery, and terminalizes only
after all paths settle. Cancellation requests interrupt only the remaining
planned work. Its journal validation is durable before live evidence is
rebuilt, so changed, stale, or unavailable targets leave `Planned` only
through an explicit bounded validation outcome and never enter the effect
phase; after the validation transition, a terminal prefix is allowed but the
exact target must be `validating` and every later path must remain `planned`.
A sub-millisecond start-time mismatch is canonicalized before persistence and
claiming. This bridge is not exported through FFI, not registered with Swift,
and has no product caller. The typed Rust-target facts and journal request
now have one additional crate-private engine bridge into this same executor;
it accepts no paths, callbacks, AI output, CLI request, FFI value, or UI input,
and performs no effect outside the already-claimed session capability. Its
macOS regression fixture now traverses the production durable-source,
enrolled-Cargo, home/mount, protected-root, process-quiescence,
descendant-policy, facts, reviewed-plan, schema-v12 claim, and
descriptor-relative executor chain. It verifies that only target contents are
removed, the Cargo marker, manifest, lockfile, and source remain, candidate and
history state settle, and active claims disappear. The former test-only facts
constructor and Cargo-revalidation bypass no longer exist. A companion
manifest read-set drift fixture fails during the planner/journal handoff before
the planned row is written, with the target left untouched. These tests prove
the private production-core chain only; they do not make it a product caller.
The durable candidate continues to carry `ProtectedPath`; an insertion-only
typed Rust-target coupling admits that retained fact only after the complete
trusted facts/review/approval chain and exact candidate-body comparison.
Generic blocked candidates remain unable to create a planned session. The
private production bridge derives one capacity scope from every revalidated
rule authorization in the approved plan and refuses mixed scopes. That scope
carries the kernel filesystem ID, mount location, platform mount discriminator
(mount path on macOS; mount ID where available), and filesystem type without
exposing a caller path or effect capability. A core-owned macOS
`statfs` sampler accepts observations only while the exact ID, mount location,
and type still match. It captures one pre-effect and one post-settlement
observation around fresh system-clock effect boundaries; canonical journal
timestamps cannot select or widen that window. After pre-sampling, the
executor reads the authority clock again immediately before rebuilding every
live effect witness, so telemetry cannot extend an expired approval. Only
matching timing, total-capacity, headline-source, and availability-shape
evidence can persist a signed available-space delta. Missing or conflicting
samples leave the delta null, and outcome-unknown sessions remain in recovery
without terminalization.
The core engine now owns the first product-shaped permanent-safe task boundary
for that exact opaque review. Admission is engine-affine and consume-once:
foreign, closed, full, cleanup-busy, or quarantined starts return the original
review rather than extracting authority. Acceptance consumes and approves the
exact child synchronously before returning. The consuming transition performs
its final authorization revalidation, uses an acquire of parent liveness as
the consume/release linearization point, then samples the clock and requires
that later sample to precede both parent and child deadlines. The parent may
then release without racing authority transfer into a queued worker. The
queued closure retains the approved capability and store, not its
`EngineInner`; final-handle drop can therefore close the registry and cancel
queued/running work. Only the worker derives session identity, trigger,
journal claim, volume sampler, or effect witness. Queued cancellation drops
approved authority before any journal write. One engine reservation serializes
the task with synchronous Explorer Trash, and known durable terminal cleanup
states are distinct from task orchestration failure.

Capability-changing ambiguity remains fail-closed throughout this task. A
generation-one claim failure retains its exact lease, approved plan, session
identifier, owner, and canonical claim time; retry may repeat only that claim
and adopts only its exact matching post-state. A still-unproven claim or a live
claim whose frozen-plan comparison cannot be proven is retained with the
cleanup lock. Explorer Trash applies the same rule after claim: a known
pre-effect refusal must finish validation and terminalize the session before
the claim is released; any unproven validation, effect-start, cancellation, or
terminalization transition retains the exact claim and any minted receipt.
Quarantine is keyed by the process-unique coordinator for the physical store,
retains every ambiguous capability, and lasts for the process lifetime. A
same-process engine reopen therefore remains blocked; only a process restart
can hand the still-durable owner to conservative recovery. The permanent-safe
platform call and Explorer Trash callback are caught inside their receipt/claim
scopes: a panic after `effect_started` becomes durable `outcome_unknown`, not
an ordinary task panic. If either settlement cannot be reconciled, a
non-cloneable capability retains the exact receipt, intended journal
transition, completion time, observed platform result, and owning
claim/session. Its retry has no effect witness or platform callback and can
repeat only persistence. Continued ambiguity never triggers automatic cleanup
retry. Failed permanent tasks retain the exact path-free session identifier
with a `Recovering` observation so history lookup does not depend on recency.
The FFI, Swift, CLI, scheduler, and AI surfaces cannot supply the sampled
volume or a capacity value. The native service/controller/browser integration
is available only in explicitly internal Debug builds; user-facing Release
execution and the remaining release gates are still required before
permanent-safe cleanup is reachable from the shipped app.

The app-facing v28 boundary observes one fully admitted Rust-target review.
Contract v31 adds exactly one consuming edge from that opaque object to the
engine-owned task. Foreign engines are rejected without touching the child;
once the owning FFI engine attempts start, the child cannot be inspected,
released into reuse, or tried again even when an information read was already
in flight or core later refuses admission. The start request has no path,
identifier, timestamp, approval Boolean, callback, AI output, command, or
retry token. Its opaque observer exposes only cancellation plus strictly
validated path-free task state and durable session correlation. The generated
Swift binding carries this transport. The native adapter calls it only from
the exact controller-owned child. That controller stores the complete
immutable information displayed in confirmation, compares every field at
start, and removes the child before suspension. Explorer adds a separate
generation/UUID confirmation fence and a path-free observer that never
retries, survives ordinary window dismissal, and is explicitly cancelled and
awaited during app shutdown. The destructive action remains compiled only
under the Debug-only `DUX_INTERNAL_PERMANENT_SAFE_CLEANUP` condition; the
public Release UI cannot start it, and release automation rejects that
condition in resolved Release settings. Preview alone still creates no cleanup
session, claim, approval, schedule, or journal row.

Arbitrary-path advanced permanent removal is excluded from the first production
authority graph. Adding it later requires a separate threat model and revision
of this design; it remains subject to protected roots, sensitive categories,
path identity, exact review, and global disable and cannot become safe,
schedule-eligible, or a generic force flag.

### 9.4 Cloud-local-copy eviction

Eviction is neither deletion nor Trash. It is allowed only through a supported
provider/platform API after fresh provider facts and filesystem witnesses pass
the reviewed fail-closed policy. Point-in-time metadata alone does not prove
that no unflushed or concurrent local change exists. UI states that the item
remains in cloud storage and requires network access to download again.
Provider-managed files are never freed by direct filesystem deletion.

ADR 0006 stages that authority. UniFFI contracts v43-v44 implement only the
read-only iCloud Drive probe and its bounded path-free observation source. A
retained Explorer review resolves one exact
non-root snapshot node, revalidates its no-follow ancestors, identity, regular
single-link kind, and known nonzero allocation, and creates a consume-once
path request. Swift may read Foundation ubiquitous-item metadata for that path
and return only versioned tri-state upload, download, conflict, exclusion, and
local-copy facts. It cannot return a path, provider, kind, allocation, clock,
eligibility decision, candidate, plan, approval, command, or effect.

For an explicit directory-level review, v44 lets Rust walk at most 200,000
descendants of the exact retained historical directory and return at most 32
complete regular-file rows with known nonzero allocation and no scan warning.
Ranking is historical allocation descending, logical size descending, then
snapshot node ID ascending. The source contains names and bounded parent
context for display, but no current or absolute path and no provider
conclusion. Loading it performs no live metadata read. The user must start a
serial batch that sends each row independently through the existing v43
before/after witness and policy.

One Foundation resource-value read is synchronous and cannot truthfully be
cancelled or timed out mid-call. The app therefore offers stop-after-current
semantics, prevents overlapping batch ownership, suppresses a cancelled
in-flight result, and schedules no later read. Context generations fence
navigation, snapshot, content-mode, close, and review replacement. Results are
non-atomic point-in-time observations; they are neither summed nor persisted
and cannot enter AI, Candidates, planning, approval, journaling, history,
notification, scheduling, or execution.

Rust repeats the retained identity and ancestor validation after the callback,
then fixes iCloud Drive as the provider, stamps the observation time, retains
the core-owned kind/allocation witness, and requires every favorable fact:
ubiquitous, fully uploaded, idle upload/download, no transfer errors, no
unresolved conflicts, exactly current local state, no pending download, and
not excluded from sync. Missing facts fail closed. The path-free passing
assessment is discovery metadata only; Apple's current-local-copy status does
not prove absence of an unflushed or concurrent writer. It cannot enter the
candidate, emergency-recovery, planner, journal, history, AI, notification,
schedule, or executor graph.

No eviction API is called in this stage. A later effect must separately bind
durable provider/account/container/item-version evidence, re-probe immediately
before one journal-fenced `FileManager.evictUbiquitousItem(at:)` call, never
wrap it in a coordinated write, never retry automatically, and quarantine any
post-entry ambiguity as outcome-unknown. Until the ADR's isolated real-iCloud
race suite proves this boundary, effect admission remains unreachable.

## 10. Automation

Automation is absent and disabled by default. It ships only after manual plan,
executor, history, dry-run, and recovery behavior is mature.

A scheduled operation requires the same technical safety witnesses as a manual
operation, but its authorization is the user's explicit saved schedule rather
than an exact-path dialog on every run. The engine still creates and reviews a
fresh immutable plan against current policy before each run. Schedule consent
never approves a stale or changed target.

Scheduled execution additionally requires:

- a shipped `SafeRegenerable`, permanent-safe, schedule-eligible rule;
- at least two recent successful manual executions of the same rule/scope;
- no failure or protected descendant in the last two runs;
- current age, size, process, evidence, and coverage checks;
- user-configured cadence, limits, exclusions, and notification policy;
- the menu bar app running without `sudo`; and
- one current revalidation after wake, with at most one missed run.

Schedules cannot use AI, arbitrary paths, Trash-emptying, advanced overrides,
remote unsigned rules, or hidden defaults. Low disk pressure changes priority,
not safety. Maximum bytes per run is enforced before and during execution.
Cancellation, app quit, sleep, thermal state, and competing manual cleanup have
defined journaled outcomes. A safety failure pauses the affected schedule until
the user reviews it; repeated failures do not silently retry forever.

Settings MUST provide a global automation kill switch plus per-schedule pause
and delete controls. Defaults are disabled, monthly, a 30-day minimum age, a
25 GiB maximum per run, and notification before each of the first three runs.
DUX does not run automation during an active manual scan, under high thermal
pressure, or on battery below a documented conservative threshold.

## 11. AI boundary

AI is optional, off by default, and outside the cleanup authority graph. DUX
must remain fully functional for deterministic scanning, explanation, and
cleanup with AI disabled.

Allowed AI results are summaries, labels, presentation-only groups referencing
existing input node IDs, questions, uncertainties, and suggestions for future
human rule research. They are visibly labeled with provider and never become
DUX safety badges.

AI MUST NOT:

- receive or return an actionable path;
- create or mutate a candidate, rule, safety tier, blocker, action, plan, mode,
  approval, schedule, exclusion, or operation result;
- call DUX filesystem, FFI, planner, executor, shell, MCP, or provider tools;
- inspect the disk directly; or
- trigger manual or scheduled cleanup.

### 11.1 Input privacy and prompt injection

Default input is bounded structured metadata, never file content. Persisted
scans and history stay local. A user-requested AI explanation may transmit only
the disclosed, bounded, redacted payload derived for that one request. Absolute
paths are shortened to home-relative labels unless the user explicitly enables
full paths for a disclosed request. Protected categories and secrets are
excluded before provider selection, so provider-specific code cannot opt them
back in.

File and directory names are hostile data. They stay JSON values and are never
concatenated into instructions. Requests declare omitted/aggregated children,
content inclusion, schema version, and a digest of the redacted payload. The UI
lets the user inspect exactly what will be sent.

### 11.2 Provider process gate

The only provider allowed before the security spike passes is disabled/no
provider. A local Claude, Codex, or custom-command adapter is not confined merely
because DUX launches it with fixed flags.

Any local adapter requires a separate adversarial TCC review on every supported
macOS release. It must use a user-selected or canonical probed executable,
reject unknown major versions, avoid a shell, use adapter-owned arguments, a
minimal environment, an empty temporary working directory, bounded JSON stdin,
separate redacted stderr, timeout, cancellation, output limits, process-tree
termination, tool disabling, and strict output-schema validation.

Those controls are defense in depth. If a subprocess can retain the unsandboxed
app's ambient filesystem or Full Disk Access authority, it MUST NOT ship. Use a
metadata-only remote API or another architecture with real confinement instead.

Provider credentials belong in the provider's approved credential storage and
are never copied into DUX logs, history, caches, or model input. If a future
remote API requires a DUX-managed credential, it MUST use macOS Keychain through
a separately reviewed adapter; plaintext settings are forbidden. A generic raw
shell command string is forbidden.

### 11.3 Output validation

AI output is byte-bounded, deny-unknown, schema-versioned, and treated as
untrusted. Unknown node IDs invalidate a group. Unknown fields, paths, actions,
tool requests, malformed Unicode, excessive nesting, or schema mismatch reject
the response. Cached insights bind the digest of redacted input and adapter
version; they do not gain freshness or cleanup authority.

## 12. Privacy and local data

Scans, history, settings, and operation journals remain local by default. Core
operation requires no telemetry. Future diagnostics export is explicit,
previewable, and excludes secrets by default.

### 12.1 Data classification

DUX treats these as sensitive local data:

- full paths and directory/file names;
- scan trees, aggregate history, capacity episodes, and coverage issues;
- cleanup candidates, exclusions, plans, operation journals, and errors;
- configured project roots and provider executable paths;
- AI request/response payloads and input digests; and
- installed CLI/update state that can reveal usernames or home layout.

### 12.2 Storage requirements

The target macOS stores are `~/Library/Application Support/Dux/` for SQLite,
snapshots, AI insights, and logs, and `~/Library/Caches/Dux/` for cache-only
data. The current CLI instead uses the platform cache directory returned by the
Rust `dirs` crate with a `dux` child. Migration MUST identify both locations,
preserve version checks, and never imply the legacy cache already meets target
permissions or retention.

Application-support, cache, snapshot, temporary-provider, and history
directories MUST be created with user-only access, target mode 0700 on Unix.
Every file beneath DUX Application Support and cache roots MUST be created
atomically with target mode 0600 on Unix. Code MUST
not rely on the process umask to tighten an initially broad file. Existing
permissions are checked and safely repaired or the store fails closed.

These permissions protect against other local accounts, not malicious code
running as the same user or a compromised unsandboxed DUX process. DUX relies
on macOS account isolation and, when enabled by the user, FileVault for storage
encryption. The first design does not add application-layer encryption; adding
it requires a key-custody, backup, migration, and recovery design. DUX MUST NOT
describe 0600 files as encrypted.

SQLite uses restrictive permissions before opening, transactional migrations,
and one coordinated writer model. Application snapshots use unique private
temporary files, SHA-256/version validation, durable flush where required, and
atomic no-replace publication; published handles are reopened read-only by
exact identity. The legacy cache still uses CRC. Both checks detect accidental
corruption, not authenticity or confidentiality.

Raw capacity history is non-authoritative local telemetry inside that SQLite
boundary. Its stable volume ID is supplied as opaque evidence and is never
derived from mutable mount paths, display names, or filesystem labels. The
typed writer accepts only a positive SQLite-representable total, required
ordinary available capacity, optional important-usage available capacity,
bounded metadata, a losslessly encoded absolute mount path, and a canonical
nonnegative millisecond timestamp. Ordinary and important-usage availability
remain distinct and each is bounded by total capacity. If the UI can obtain
important-usage capacity but not ordinary availability, that observation is
shown ephemerally and is not persisted; DUX does not invent ordinary capacity
to make a row fit.

Capacity writes use the same connection-mutex → cross-process writer and
current-schema lease → immediate-transaction order as other history. Volume
metadata advances only with a non-stale observation. The persistence layer
stores at most one routine raw sample per UTC hour but records a genuine change
from the latest stored pressure immediately. A same-key retry is idempotent
only after retained storage and the current schema revalidate, every sample fact
matches, and its timestamp remains inside the volume's first/last-seen interval,
including after an ambiguous commit. Differing collisions and out-of-order
samples fail closed. Cadence-suppressed observations advance the durable
last-seen bound without retaining their capacity facts; after a session restart,
an observation at that exact timestamp is therefore rejected rather than
guessed to be an exact retry, and older ephemeral observations are superseded
against the same bound. Latest and cursor history queries have fixed page, SQLite-
operation, and elapsed-time limits and validate storage classes, byte lengths,
enum values, paths, times, volume-observation bounds, and integer ranges before
returning data. These observations do not prove filesystem identity, scanned
coverage, cleanup safety, or bytes freed.

The native startup-volume path now routes versioned Foundation observations
through the capacity call introduced in FFI v6 and carried unchanged by v7.
The startup request/status remain record v1 beside separate typed policy
get/set/reset records; a caller cannot
supply policy with an observation. For persistable input, the engine serializes the
bounded session baseline, loads the newest durable pressure inside the final
writer-leased transaction, selects the newest valid policy baseline, evaluates
the Rust-owned threshold and recovery-hysteresis policy, chooses routine versus
transition admission against durable history, writes, commits, revalidates, and
reconciles ambiguity. Important capacity is preferred for pressure; ordinary
availability is the explicit fallback and is the only basis for used-byte
accounting. Missing UUID evidence, incomplete optional metadata, or
important-only input may be classified for display but cannot insert or advance
either persistence table. One timestamp-fenced, engine-session-only slot carries
their pressure and policy revision forward without synthesizing durable identity.
A policy revision change ignores older-revision hysteresis, forces one raw
baseline even inside the same UTC hour, and never rewrites historical
classifications. A durable or session baseline claiming a revision newer than
the effective policy fails as corruption. Policy mutation never acquires the
capacity-session mutex; the observation lock order remains session then store.
Returned status
is path-free and carries an explicit history disposition; this telemetry creates
no cleanup authority.

The implemented SQLite boundary provisions a previously absent DUX directory
in an unpredictable private sibling stage. It creates and durably writes a
fixed-length, versioned ownership marker plus an empty database before atomically publishing the
directory without replacement. A racing winner is re-probed and never
overwritten. An existing unmarked directory or database is not claimed,
repaired, or populated, and a marker-owned missing database is not recreated.
The ownership entry and identity are permanent. Its exact legacy-v1 content
may advance once, in place and under its own writer lock, to layout v2 only
after both cleanup controls are durable; every other rewrite, downgrade, or
unknown value fails closed. This one-way layout boundary intentionally makes
older binaries reject the upgraded store. After a successful current-schema
migration and WAL setup DUX durably creates a separate private initialization
sentinel, distinguishing an interrupted first provision from a previously
initialized database later truncated to zero. The SQLite layer
accepts only the allowed database/control/sidecar and reserved app-support
entries (on macOS, another spelling is tolerated only when it resolves to the
same filesystem object reached by an allowed canonical name), plus at most 64
lexically canonical private
`.dux-snapshot-stage-<32 lowercase hex>` directories in the final DUX
directory. That root walk also has fixed total-entry and 256-KiB aggregate-name
budgets plus sampled elapsed-time checks against 250 ms. The SQLite layer
validates only canonical stage naming and a current-user-owned, no-follow directory
whose Unix mode is 0700 or a stricter subset left before mode repair. It does
not inspect the stage marker or children and gains no cleanup authority. The
separate implemented provisioning-stage maintenance boundary independently
proves the retained root, canonical name, exact marker, bounded known child
set, and fresh identity/security state.

On Unix, stage children are created relative to a retained directory handle;
the final no-replace rename is relative to a retained current-user parent that
is not writable by group or others. Linux and macOS expose no equivalent of
Windows' source-handle-bound directory rename, so the unpredictable stage name
and retained-identity checks detect but cannot prevent a malicious same-user
process substituting that source name immediately before publication. Such
same-user code is outside the storage-isolation guarantee stated above; DUX
fails closed on the post-publication identity mismatch. The final DUX directory and SQLite files
must have exact 0700/0600 modes and current ownership; regular files must have
one link. macOS additionally rejects every extended ACL on final DUX objects,
while permitting deny-only ACLs on the publication parent so the normal system
`everyone deny delete` entry on `~/Library/Application Support` remains usable.
Linux currently relies on ownership and mode checks and does not inspect POSIX
ACLs. On Windows, the stage children and no-replace publication are bound to
retained handles, final objects receive protected current-user-only DACLs, and
a final-root handle that denies delete sharing prevents a parent
`DELETE_CHILD` grant from swapping the directory while SQLite uses its path.
SQLite-created Windows sidecars inherit private access and are immediately
repaired to the exact protected file DACL before further use.

The implemented application snapshot owner independently validates the exact
reserved `snapshots` sibling. New provisioning creates
`<SQLite database parent>/.dux-snapshot-stage-<32 lowercase hex>` inside the
same retained, marker-owned database root and atomically publishes it without
replacement to the sibling `snapshots` entry. It never uses the outer
application-support parent. Its fixed-marker 0700/0600 or protected-DACL store
uses bounded inventory, exclusive random temporary files, durable no-replace
publication, single-link/no-follow identity checks, and read-only reopened
final handles. The frozen v1 depth-first wire validates exact sibling
names, graph structure, aggregates, scan flags, optional times and Unix
identity observations, and a trailing SHA-256 digest before returning any
document. Snapshot bytes remain non-authoritative. Mutation order is SQLite
connection mutex, SQLite writer/compatibility lease, then snapshot writer lock;
publication retains the last lock through the exact SQLite terminal-scan CAS
and post-commit file revalidation. A newer schema therefore fences an older
snapshot writer before provisioning and every later mutation. File-first
failure may leave an unreferenced immutable orphan, never a database reference
to an unpublished file. The exact wire and failure contract are documented in
[`docs/SNAPSHOT_FORMAT.md`](docs/SNAPSHOT_FORMAT.md).

Migration SQL is embedded, sequential, SHA-256 checked, denied embedded
transaction/savepoint control, applied in one runner-owned `BEGIN IMMEDIATE`
transaction, and recorded in a contiguous ledger paired with an exact schema
fingerprint. Stored path observations use a frozen lossless
codec: tag 1 is UTF-8 host-path bytes and tag 2 is little-endian UTF-16 host
units; tag 0 is reserved for UTF-8 non-path aggregate keys. Path fields accept
at most 32,768 host units, including the corresponding 65,536-byte UTF-16
representation. Semantic status, pressure, tier, mode, trigger, and category
values are constrained self-describing text rather than implicit enum ordinals.

Schema v2 rebuilds candidate and cleanup history into explicitly versioned
record formats. Migrated v1 rows remain format-1 summaries with every original
value preserved and every unavailable fact left `NULL`; no loader may
reconstruct them as complete candidates or plans. Format-2 candidate child
tables preserve ordered accepted-host path observations, typed evidence, and
blockers. Cleanup sessions and item/path journals freeze the source scan, exact
plan lifetime, policy/action, warnings, and proposed effect even when the mode
is dry-run. Recovery fields distinguish owner generations, heartbeats,
`effect_started`, and `outcome_unknown`; process death alone MUST NOT mark a
session successful, failed, or interrupted. The private mutable-journal layer
claims a new generation transactionally and compare-and-sets every journal
write against owner plus generation while retaining the cleanup lock. All
persisted facts remain historical
observations: they are not canonical path witnesses, current evidence,
approval, or executor capabilities, and they do not add an AI/history-to-plan
authority edge.

The implemented typed candidate-history API prepares all bounded data before
locking, atomically inserts a format-2 parent and contiguous ordered children,
and requires a durably succeeded source scan. Its exact-ID reader shares the
bounded SQLite progress budget across parent and child queries, checks storage
types and byte lengths before materialization, and returns either a dedicated
complete observation or an explicit legacy summary. It rejects legacy child
pollution, missing/gapped/over-limit children, dangling scans, malformed
evidence, incompatible policy, incomplete cloud-upload facts, and complete
candidates attached to running or non-successful scans. The only standalone
status writer is a typed cleanup-review projection: exact `discovered`↔
`selected`, source-specific transition to `dismissed`, and explicit
`dismissed`→`discovered` restore after full format-2 validation; direct
dismissed-to-selected transition is absent. Selection requires a cleanup-
capable blocker-free observation but is never approval; dismissal is neither a
protected-path rule nor authority over an already frozen plan. Legacy rows and
evaluator/planner/journal-owned statuses
cannot enter this API. Exact post-commit target state is adopted only while the
retained store and current schema still validate under the writer lease. It
cannot construct a domain `Candidate`, create a plan, or execute an effect.

A second status boundary is sealed inside persistence for a future deterministic
evaluator. Its source-typed commands can mark `discovered`, `selected`, or
`dismissed` observations unavailable, then conclusively mark any of those
states or `unavailable` stale. Unavailable can be conclusively refined to stale
with a fresh exact-source command, never the reverse; both states are terminal
for that scan-bound observation and cannot re-enter review, planning, or
journal state. The command performs the same full bounded candidate and
succeeded-source-scan validation, exact compare-and-set, writer-lease ordering,
and storage-gated ambiguity reconciliation as review state. It is not currently
constructible by engine, FFI, Swift, CLI, AI, or any evaluator module. This
prevents a generic ID-only lifecycle setter from becoming an authority edge.

Schema v4 adds a strict one-to-one candidate-evaluation ledger for succeeded
scans. Each row binds the evaluator revision, exact catalog schema and SHA-256,
versioned context digest, and exact immutable snapshot version and digest. A
terminal success count must equal its complete bounded candidate set; pending
and typed failure states must contain none. The normal engine path first makes
the snapshot file durable, then one `BEGIN IMMEDIATE` transaction changes the
scan to succeeded, inserts its evaluation identity, inserts the entire candidate
batch or no batch, and writes the terminal evaluation state. A failure at any
later candidate rolls back the scan transition and every earlier insert, so a
normal engine crash cannot leave a succeeded scan without its discovery result.
Historical scans are not backfilled with fabricated evaluation rows.

The bounded loader uses SQL-limited set-based parent and child reads under a
dedicated fixed maximum-batch VM/time budget, and revalidates storage classes,
record format, time ordering, succeeded-scan and exact-snapshot binding, status
shape, count, IDs, and every complete candidate child. Exact retry and
ambiguous-commit adoption compare the
full scan completion, request identity, normalized terminal time, failure kind,
and candidate set. A standalone legacy candidate insert is rejected whenever
the source scan already owns an evaluation, preventing post-terminal batch
drift. Pending-only primitives remain sealed and are consumed only by the
bounded recovery seam. The first bounded core recovery seam selects at most one oldest
pending row with a two-row sentinel, revalidates the exact succeeded scan and
snapshot tuple, decodes the retained immutable snapshot into a bounded review
index, and replays the deterministic evaluator without touching the live
filesystem. Evaluator/catalog/context drift becomes a terminal typed discovery
failure; malformed, missing, unavailable, or incompatible snapshot state never
becomes replay authority. The seam returns only a candidate count and bounded
`has_more` hint. Contract v32 now admits it only as an eighth idle-only
maintenance task: Rust supplies the clock and all replay inputs, while FFI and
Swift can select only the maintenance kind and observe path-free terminal
state. No UI, AI, CLI, or cleanup caller can select the pending row, snapshot,
evaluator, catalog, candidate, plan, or effect.

The production engine exposes that ledger only through an exact-scan,
path-free discovery-history projection. A missing scan is distinct from an
existing scan with no evaluation; pending, succeeded, and typed failed states
retain their bounded lifecycle facts. Before any payload-bearing child query,
the store preflights row counts and encoded bytes against a conservative 32 MiB
aggregate decoded-materialization budget. Evaluation write preparation applies
the same charge so a newly accepted batch remains safely reloadable; the
separate SQLite VM/time bound still applies. A valid evaluator graph over that
budget is recorded as typed discovery `LimitExceeded` while the completed scan
and snapshot remain successful. The store then validates the complete
parent/child graph before projection.

A successful summary may expose identifiers, rule/category, estimates, policy,
scheduling eligibility, path count, evidence kinds, blockers, creation time,
and historical status, but never accepted-host paths or evidence payloads.
Candidate identifiers are stable pseudonyms derived partly from path bytes, not
privacy-safe tokens for remote disclosure. Persisted selected, planned,
completed, failed, stale, or unavailable state is not current validation or a
planner witness. The projection cannot reconstruct a domain `Candidate` or
`CleanupPlan`, mutate review state, approve cleanup, or reach an effect.

The separate candidate-detail boundary is an explicit local disclosure keyed
by both scan and candidate ID. Path and evidence pages have a code-owned maximum
of 64, strict immutable cursors, and exact totals. Before product-callable exact
candidate loading can copy a child payload, a scalar-only preflight validates
parent and child storage classes, counts, encoded lengths, claim coupling, and
the same conservative 32 MiB aggregate charge. Standalone writes use that
charge too. Page paths expose a display string and lossless accepted-host UTF-8
or little-endian UTF-16 bytes, not SQLite codec tags or a planner `PathBuf`.
Every evidence variant is copied into a distinct presentation DTO. These are
sensitive local observations: they are not AI/remote-safe by default and no
detail value is accepted back as validation or execution input.

Review mutation likewise remains semantic and non-authoritative. The only core
commands are Select, ClearSelection, Dismiss, and Restore; none accepts a path.
The source scan and complete candidate are validated inside the immediate
writer-leased transaction, and Dismiss resolves discovered versus selected in
that same transaction. Selection remains restricted to blocker-free cleanup
policy. Dismissal is UI intent, not an exclusion, and cannot revoke a frozen
plan claim. Planner-, evaluator-, and journal-owned states reject review.
Exact compare-and-set and post-commit reconciliation remain mandatory. No
command constructs a plan, approves cleanup, reaches an effect, or crosses FFI.

Schema v3 implements planner/journal candidate coupling without a lossy status
overwrite. Every newly inserted plan is coupling version 2 and atomically
freezes its complete graph, compare-and-sets only an exact `discovered` or
`selected` candidate to `planned`, and inserts a strict claim binding that
candidate to one session/item plus its prior review state. Candidate and item
foreign-key ownership edges are delete-restricted. A planned candidate is a
valid observation only while exactly one bounded format-2 claim points to a
coupled `planned`, `running`, or `recovering` session and its matching item;
every non-planned status must have no claim. This projection and stored graph
remain history, not execution authority.

Schema v12 adds one narrower active-lifecycle relation for the private trusted
Rust-target exception. Its row is revisioned and binds the exact candidate ID,
session ID, and item ordinal already owned by the ordinary plan claim. It is
inserted atomically only after the full retained-blocker candidate comparison
and is deleted by cascade when that claim settles. Active decoding rechecks
the frozen one-item/one-path rule shape and the complete candidate, including
exactly one `ProtectedPath` blocker; a seal is not independently sufficient.
Upgrades create an empty relation and never infer prior trust.

Terminal journal derivation settles each candidate and deletes its exact claim
inside the same transaction as the immutable terminal session. A mode-correct
real effect becomes `completed`. Dry-run and fully effect-free cancellation
restore the claim's exact `discovered` or `selected` value. Every other terminal
item becomes `failed`; partial sessions project each item independently.
Running, recovering, effect-started, and outcome-unknown work retain `planned`
and the claim. Once a claim is settled, restored review state may legitimately
change again through review or evaluator commands, so immutable terminal
history never depends on the candidate's later mutable projection.

Pristine expiry is settled only by a cleanup-lock-owned history operation. The
exact expiry boundary rejects every item/path with `plan_expired`, projects all
candidates to `failed`, removes all claims, and records generation-one terminal
provenance atomically; it does not create a running claim or effect receipt.
The persisted millisecond is rounded upward so the terminal observation cannot
appear earlier than the nanosecond expiry proof, while an observation even one
instant before expiry is refused. Ambiguous commits retain the lease and adopt
only the exact terminal owner/time/graph. Schema-v2 format-2 sessions migrate as
explicit coupling version 1 with no fabricated claims: pristine rows cannot be
newly claimed, while already-active rows may recover or terminalize without a
retroactive candidate projection.

The implemented typed cleanup-history boundary likewise remains
non-authoritative and currently accepts only an immutable domain plan for a
`planned` journal. It prepares and bounds the full plan before locking, then
requires the referenced scan and every format-2 candidate observation to
exist and match the frozen source, rule, category, paths, estimates,
nanosecond times, evidence, policy, scheduling flag, and absence of blockers.
One immediate transaction stores the parent, contiguous items and paths, all
evidence and warnings, and each proposed effect even in dry-run mode. Its
bounded exact-ID reader returns either a complete planned observation or an
explicit format-1 legacy summary; it checks storage types and lengths before
allocation and rejects child pollution, gaps, limits, relative format-2 paths,
overlap, incomplete cloud facts, mode/action mismatches, dangling dependencies,
and time/warning/estimate drift. It cannot reconstruct `CleanupPlan`, claim an
execution owner, transition or recover a journal, or reach an effect.

The core engine exposes that storage only through two path-free historical
projections. `recent_cleanup_history` is a 1..=64 opaque-keyset feed. Before it
publishes a summary, scalar preflight bounds every selected graph, validates
format-specific parent/item shapes, child ownership and ordinals, warning and
claim relationships, and charges a conservative 64 MiB per-graph read envelope.
A journal-owned scalar pass then reuses the execution state machine's lifecycle,
generation/time, path-shape, derived-item, mode/action-success,
active/recovering, and terminal-status rules without selecting target paths,
evidence payloads, or candidate payloads. Format-2
`cleanup_session_history` performs the same preflight and then runs the
complete bounded journal decoder before projecting item summaries and warnings.
Format-1 exact lookup uses the bounded legacy-summary loader and remains explicitly
`LegacyIncomplete`; missing source, lifetime, policy, evidence, warning, and
trusted error-category facts are never fabricated.

Both projections omit target paths, evidence payloads, candidate IDs,
owner/generation/heartbeat, claims, prior review state, and execution fences.
Their status, policy, and prior outcome fields are presentation observations,
never current validation, approval, planner input, recovery permission, or
executor authority. They are local history and are not AI/remote-safe by
default. Reads retain the compatibility/writer guard across the multi-query
observation but acquire no cleanup OS lock and are not described as a separate
SQLite transaction snapshot. The 64 MiB read envelope has no matching write-
admission guarantee; a schema-valid historical graph may later return
`QueryLimitExceeded` rather than being partially published.

The separate rule-outcome boundary is also a derived, read-only presentation
query; it never reads or writes the legacy `rule_outcomes` table. It accepts
only an exact cleanup-session ID and returns exactly one path-free typed state
per validated v2 journal item. Eligibility requires a terminal Completed or
PartiallyCompleted permanent-safe session, a successfully removed
SafeRegenerable `RemoveKnownRegenerableContents` item, complete removed paths,
and an exact source candidate whose current-policy succeeded evaluation
completed no later than plan creation. Source and follow-up scans must be
succeeded snapshot observations with complete coverage, the same non-null
schema-v13 root-identity digest, the same evaluation scope, and compatible
evaluator, catalog, and context-format identities. The scan path is compared
losslessly inside persistence and never projected.

Candidate absence is not a zero observation. `LaterSizeObserved` means the
first explicit compatible observation was nonzero; it is not called regrowth.
`ZeroBaselineObserved` requires an explicit zero candidate, and `Regrown`
requires a later compatible nonzero candidate after that zero. Evaluator
revision 5 makes this possible for Rust targets by subtracting only the
preserved exact direct regular non-symlink `CACHEDIR.TAG`; unknown allocation
for any reclaimable descendant rejects evaluation/replay rather than
fabricating zero. Follow-ups are ordered by completion time with deterministic
ties, and otherwise-comparable overlapping closed scan intervals are ignored
inclusively. A later permanent-safe, successfully removed path that overlaps
by native path components supersedes any nonterminal attribution; scan/effect
boundary equality is not accepted as pre-effect evidence. Trash, dry run,
failed/unknown effects, and textual-prefix-only paths do not supersede.

The outcome query has fixed scan/journal cardinality, SQLite VM/deadline,
materialization, and pure-Rust work limits and returns
`QueryLimitExceeded` without a partial result. A full follow-up evaluation is
reduced immediately to at most one optional byte observation per source item
and dropped. A full intervening journal updates at most one earliest
superseding timestamp per source item and is dropped before the next journal.
The result exposes only item ordinal, `RuleRef`, typed ineligibility/state,
observation times, observed bytes, and derived duration. It contains no path,
candidate ID, evaluator or snapshot digest, journal owner, plan, approval,
schedule, AI input, filesystem witness, driver, or cleanup capability.
Capacity deltas remain session-level telemetry and are never attributed to a
rule by this boundary.

UniFFI contract v41 transports that exact-session derivation as a distinct
bounded batch with a separate typed error domain. Projection requires one
contiguous outcome per immutable cleanup-history item and independently
validates record versions, stable rule tokens, nonzero revisions, chronology,
bytes, epoch conversion, and exact derived duration. Strictly ordered
`SystemTime` values that would collapse into the same projected millisecond are
rejected rather than emitting a DTO the native boundary cannot accept. Swift
then repeats exact session/count/ordinal/rule/revision and state-shape checks
before creating app-owned values.

Native outcomes are dynamic presentation state, not part of the immutable
cleanup-session detail. They own a separate cancellable task and generation;
failure leaves the base history visible, and selection, summary refresh, close,
clear, shutdown, or any newer request fences late publication. Successful
home, subtree, and pressure-triggered targeted scan observations invalidate and
re-read an open outcome once. The loaded UI reports when it read the derivation
and permits an explicit read-only refresh without scanning or repeating
cleanup. Its six-state chart and item copy preserve the distinction between
absence, first later nonzero size, explicit zero, confirmed regrowth, and
supersession. Loading, failure, legacy-unavailable, and loaded accessibility
states remain distinct. Neither transport nor UI contains a conversion into a
candidate, plan, approval, scheduler input, callback, driver, or effect.

The recurring-storage ranking is a separate bounded, read-only aggregate over
that exact rule-outcome derivation. It considers only the newest 32 Completed
or PartiallyCompleted schema-v2 permanent-safe sessions and reads one additional
ID solely to disclose that older qualifying history was omitted. Complete
journals are grouped by deterministic rule ID across observed revisions. A rule
counts at most once per cleanup session, and only when every matching item
proves a complete successful `SafeRegenerable`
`RemoveKnownRegenerableContents` effect. Only exact `Regrown` outcomes add
observed bytes or zero-to-nonzero duration. Missing candidates,
`LaterSizeObserved`, `ZeroBaselineObserved`, `AwaitingComparableScan`,
`Superseded`, and `NotEligible` never contribute a fabricated zero or growth
rate.

Groups are ordered using the exact aggregate observed-bytes/duration fraction
without overflow, followed by successful cleanup count, confirmed cycle count,
latest regrowth time, and rule ID. At most 12 groups cross the engine boundary,
while the full ranked-rule count and older-session sentinel keep truncation
visible. The bytes-per-day presentation value is separately floored and carries
an explicit saturation flag. Manual cleanup sessions and manual regrowth cycles
remain distinct from CLI, low-disk, and scheduled triggers. The latest observed
rule revision reaches only an `automation_history_threshold_met` observation
after at least two successful Manual sessions of that revision and one
Manual-origin confirmed regrowth cycle. That value is necessary historical
evidence, not current automation eligibility: it does not inspect a current
candidate, satisfy the remaining §15 gates, create a schedule, enable one, or
enter a planner or executor.

The ranking reuses the outcome query's fixed scan/journal cardinality,
materialization, SQLite VM/deadline, elapsed-time, and pure-Rust work limits.
Every full journal/evaluation is validated under those limits, and any
corruption or exhaustion rejects the entire ranking without a partial result or
durable write. The legacy `rule_outcomes` table remains deliberately excluded.
The path-free result contains only window/group counts, truncation, stable rule
identity/revision counts, cleanup/cycle counts, aggregate observed bytes and
duration, the derived rate/cap state, observation times, and the historical
threshold bit.

UniFFI contract v42 transports this ranking through a distinct typed error
domain. Core projection revalidates source/group limits, unique stable rule
IDs, counts, the derived display rate/cap pair, exact fraction order, timestamps,
and threshold prerequisites. The FFI adapter independently repeats the bounds,
identity, count, duration, exact-order, epoch, and aggregate-threshold checks.
Swift repeats the bounded response-shape, uniqueness, duration, timestamp,
truncation, exact overflow-free fraction order, derived rate/cap pair, and
aggregate-threshold checks before creating app-owned values. Cleanup History
loads the ranking lazily in independent
cancellable, generation-fenced state, reports its
read time and bounded source coverage, keeps an earlier valid ranking visible
when refresh fails, and offers an explicit read-only refresh that neither scans
nor cleans. Successful scan observations refresh only an already requested
ranking; history refresh/clear, close, shutdown, cancellation, and newer
generations fence stale publication. The accessible ranked bars carry visible
rate and count text, explicit empty/loading/failure/truncated states, and copy
that distinguishes observed estimated regrowth from verified capacity change.
No layer exposes a path, candidate ID, evaluation or snapshot identity, plan,
approval, AI input, scheduler command, callback, platform driver, or effect.

The storage layer also implements the permanent store-wide cleanup exclusion
primitive used by execution-state journaling. Its immutable lock and ready
control are exact root entries, provisioned for legacy owned stores only while
the writer lock is held and flushed before the root-ownership marker's durable
layout-v2 transition. Both are retained with private no-follow identity
evidence and never recreated after that transition.
The cleanup and writer locks remain independent so the sealed journal lease
performs short fenced transactions in the required cleanup-before-writer order;
the future executor must preserve that ordering beneath its engine cleanup
mutex. Same-process and subprocess tests prove bounded contention and release;
native Windows handles deny delete sharing to prevent path replacement. No
history, FFI, CLI, Swift, AI, plan, or effect API can obtain this guard; only
the sealed mutable-journal lease couples it to an owner-generation claim.

The global permanent-cleanup opt-in is a separate revisioned typed setting. A
missing row means disabled by the versioned default; malformed or newer rows
are errors, never an implicit enable. Value schema v2 stores that default
explicitly. Its version-aware decoder preserves a schema-v1 `Stored(true)` as
the user's prior explicit consent, preserves `Stored(false)`, and strengthens a
legacy `Default(true)` epoch to disabled because the old default did not prove
first-enable confirmation. Reset rewrites legacy state as canonical v2
`Default(false)`. Every write takes the same store-wide cleanup exclusion used
by the journal. A permanent-safe claim observes the setting while holding that
exclusion and durably rejects a disabled path before an effect receipt; the
final `effect_started` transition rechecks it under the same exclusion.
Therefore a setting write cannot race admission, and disabled state reaches no
platform driver call. The setting contains no target, path, plan, approval, or
executor authority.

User cleanup exclusions are a separate bounded, lossless settings value. Each
entry is an absolute lexical host path with no `.` or `..` components; entries
are sorted and deduplicated before canonical JSON storage, and malformed or
newer values fail closed. The journal reloads the set while holding the
store-wide cleanup exclusion and rejects a matching path prefix before writing
`effect_started`. Exclusions are deny-only: they cannot authorize a path,
weaken protected-root policy, replace live identity validation, or be supplied
by AI as an execution instruction.

Configured project roots are a separate bounded, lossless discovery setting.
The rowless versioned default is empty; an explicitly stored empty set remains
distinguishable until reset. At most 16 absolute, normalized, non-root paths
are stored in deterministic host-byte order. Duplicate, nested, overlapping,
control-containing, oversized, malformed, or newer-schema values fail closed.
Windows accepts only ordinary drive and UNC prefixes; verbatim and device
namespaces are rejected independently by core and FFI. An uncertain write or
post-write projection failure prevents another native edit until an
authoritative read succeeds, so stale presentation cannot resurrect a root.
The registry is not a capability: saving a root does not open it, start a scan,
grant macOS access, bypass coverage/protected-root checks, create a candidate
or plan, approve cleanup, or provide an AI/executor input. The pressure runner
rereads this setting at every path-free ordinal admission, retains a no-follow
root identity through snapshot publication, and accepts only a root proven on
the pressured startup volume. On macOS, an observed startup mount of `/` may
also match the fixed `/System/Volumes/Data` device; no arbitrary lexical or
mounted-volume equivalence is inferred. Warning and Critical require their own
current open episode and exact latest durable capacity anchor. A root receives
at most 50,000 nodes and one pass at most 200,000 nodes, sequentially. Only a
qualifying succeeded targeted scan with a retained snapshot and terminal
candidate evaluation can be reused. Every result remains observation behind
the normal planner gates and carries no cleanup or AI authority.

The separate process-instance module supplies only the liveness evidence
described in §6.7. Its native Unix subprocess regressions distinguish an exact
live owner from both graceful and abrupt death in one reliable boot scope;
pure tests keep PID reuse, scope mismatch, malformed identities, and macOS/
Windows unscoped non-live observations fail closed. The mutable-journal lease now
couples that evidence to the held cleanup lock and stored owner only after
dropping all database locks: `DefinitelyGone` supplies a one-use in-memory
permit whose stale phase, owner, generation, heartbeat, and cancellation bit
must all match again in the recovery transaction. `Alive` and `Unknown` leave
the journal unchanged. Executable recovery remains same-host and same-boot
only. Schema v14 binds new cleanup claims to separate stable-host and boot-scope
provenance, so a changed relationship is classified as `ForeignHost` or
`PriorBoot` rather than treated as owner death. Both classifications are typed
journal no-ops, and migrated, partial, malformed, or unsupported provenance is
`Unproven`; no prior-boot reconciliation handle exists.

Schema v9 separately binds each newly started scan to an immutable private
process-instance claim in the same transaction as the pristine `running` row.
Schema v16 extends new claims with nullable
`execution_host_identity_v1_sha256`,
`execution_boot_scope_v1_sha256`, and `execution_recovery_policy` fields. The
only accepted policy is `interrupt_only`; its two observations are exact
32-byte domain-separated digests bound to the owner boot scope. The tuple is
all `NULL` or complete. Every v9–v15 claim migrates all `NULL`, and unavailable
or conflicting OS proof never causes fabricated provenance.

One batch validates a global 64-row keyset page plus one lookahead under a
current-schema guard. Complete same-host/same-boot claims and migrated
all-`NULL` claims whose combined recovery-scope value exactly matches the
current owner retain the existing liveness path: SQLite locks are dropped
before probing each exact owner, and only `DefinitelyGone` is recoverable. An
absent scope can confirm only `Alive` or `Unknown`. A complete same-host/prior-
boot `interrupt_only` claim is separately history-interruptible without a PID
probe. Foreign-host and other unproven claims are non-executable.

The batch reacquires the writer guard and changes at most one exact pristine
claim/scan to `interrupted`. Its compare-and-set binds owner, combined scope,
start time, record format, complete provenance tuple and policy, and the
pristine parent. Normal completion consumes that same exact tuple. `Alive`,
`Unknown`, foreign, unproven, malformed, schema-raced, pre-start-clock, and
legacy-v8 unclaimed rows never grant a write. A process-local cursor and global
time/scan-ID index let repeated bounded batches pass non-executable rows
without starvation. A snapshot-temp lease is not consumed by recovery; the
independent terminal-temp boundary owns any later physical reconciliation.

Schema v15 adds only a partial index over durable `running` scan rows. A
separate bounded reader selects at most 64 rows without a process claim plus
one lookahead, validates every selected scalar, and classifies exact-pristine
versus unexplained shapes. It performs no liveness probe or write and exposes
only path-free counts. Time and PID are intentionally absent, so neither the
query nor its Settings presentation can infer death, fabricate an owner, or
turn legacy debt into recovery authority. External snapshot stages remain
outside this census because their legacy marker cannot attribute them to one
store safely.

UniFFI v49 adds a distinct claimed-row provenance reader over the same global
claimed-time order used by bounded recovery. One synchronous call inspects at
most 64 claims plus one lookahead and partitions every inspected row into
exactly one of five aggregate categories: same-host/current-boot,
same-host/prior-boot, foreign-host, stored-unproven, or
current-context-unavailable. The partition separates an all-`NULL` stored tuple
from complete stored evidence for which this process cannot obtain complete
current host/boot context. Partial, malformed, owner-inconsistent, and
unknown-policy tuples in the inspected page or lookahead fail the whole query.
A successful result exposes only counts and `has_more`, requires their exact
arithmetic, and rejects a nonzero current-context-unavailable count alongside
any of the three comparable host/boot categories. It never returns the claim
or scan identity, owner, scope, digests, policy, time, root, path, bytes, or a
row selector.

This diagnostic observes bounded current execution provenance only to derive
the aggregate relationship. It never probes the stored process owner, obtains
a recovery permit, advances the recovery cursor, starts a maintenance task,
enumerates a filesystem, or writes storage. Native Settings validates the
complete response independently and presents claimed and unclaimed censuses
separately. Its refresh copy identifies DUX bookkeeping and current
operating-system provenance rather than claiming a database-only read, and no
category is described as alive, dead, abandoned, recoverable, or actionable.

The implemented execution-state journal remains crate-private and performs no
effect. Its non-cloneable, non-shareable lease acquires and revalidates the
cleanup lock before the connection mutex, writer lock, and each short immediate
transaction, then generates the only owner identity accepted for a new claim.
A pristine, unexpired plan becomes generation one; every child update is also
fenced through its exact parent owner and generation. Heartbeats are monotonic
progress evidence, never expiry authority. Cancellation request, settlement,
and terminal derivation are distinct, and terminal rows retain immutable
owner-generation provenance. Failed or ambiguous capability-changing calls
retain their lease/claim and reconcile exact typed post-state before retry.

Before a future OS call, `effect_started` must commit durably and an exact typed
receipt must be issued after that commit or ambiguity-reconciled, then
revalidated against the complete journal graph, the absence of cancellation,
and the retained cleanup control. The receipt
uses the database's millisecond timestamp canonically while retaining its finer
ordering instant, so completion cannot precede intent. Cancellation that wins
this final boundary records a known no-call interruption and clears effect
provenance. Commit ambiguity never authorizes retrying an OS effect. Recovery
increments the generation in the same transaction that claims the stale row,
resets `validating` paths to pristine `planned`, maps `effect_started` paths to
`outcome_unknown` with their effect time preserved, and enters a recovery-only
phase that forbids new validation or effects. Unknown outcomes must be
explicitly reconciled before remaining planned work can resume. This is journal
ordering evidence only: the future centralized executor must still supply
current plan approval, trusted target witnesses, live safety revalidation, and
the immediately following reviewed effect call.

Compatibility inspection and migration have both SQLite-VM-operation ceilings
and deadlines sampled every 1,000 VM operations by SQLite's progress callback.
Schema/ledger storage types and byte lengths are checked before Rust
materialization, fingerprint input has per-value and aggregate caps, and direct
Rust-side deadline checkpoints cover row validation and hashing.
Startup performs bounded full quick/foreign-key integrity inspection; live
presentation status performs only bounded ledger/version/fingerprint
compatibility inspection rather than rescanning all stored rows. Exceeding a
budget returns a distinct path-free limit category and never asserts
corruption. Every compatibility refresh holds the process coordinator mutex
and then the stable cross-process writer lease. A newer valid schema is opened
strictly read-only and is never downgraded. Marker-owned recovery sidecars may
be opened read-write only under that lease for SQLite recovery; a resulting
newer schema is immediately reopened read-only.

An interrupted or losing first provision can leave a private sibling named
`.dux-stage-*` containing only the fixed marker and empty database. This is a
small availability/footprint debt, not a published store or cleanup authority.
Future retention maintenance must scavenge only bounded, identity-validated,
code-owned stages; current code deliberately does not recursively delete an
unproven path during error recovery.

Corrected snapshot provisioning can leave a root-local private
`.dux-snapshot-stage-<32 lowercase hex>` directory in empty, marker-only, or
marker-complete form beside `snapshots`. The implemented maintenance boundary
can attribute only a root-local canonical private stage with the exact
snapshot-store ownership marker and a complete bounded child set consisting of
that marker alone or that marker plus the exact writer control. Empty or
stricter-mode stages remain unproven and untouched; partial/wrong markers,
writer-only, malformed, linked/reparse, extra-entry, broader-permission, and
unsafe-DACL stages fail the batch before effect. Name, prefix, age, PID, or
private permissions alone are never removal authority, and stage cleanup never
recurses. Pre-correction external stages may remain outside the database root.
Their globally fixed marker contains no root identity, so two databases sharing
that outer parent cannot attribute them; every legacy external stage remains
manual debt even when its marker bytes are exact.

Schema v8 now commits a bounded immutable row before creating each production
`.snapshot-*.tmp` and retains a kernel file lock while its writer is live. A
crash can therefore leave a row-only residual or a row-bound quiescent temp.
Recognized pre-v8 or otherwise unleased temps remain possible and are never
adopted. Startup validates at most 64 physical temps or 64 rows; a 65th makes
the relevant inventory unavailable, and an individual temp may be large.
Exact same-scan retry, the separate bounded terminal-row reconciler, the
separate bounded physical-only unleased-temp reconciler, and exact-marker-owned
root-local provisioning-stage reconciliation described below are implemented.
Schema-v9 claimed running scans whose exact same-scope owner is proven gone can
be terminalized to `interrupted` without filesystem effect. Schema v16 also
permits that history-only transition for a complete same-host/prior-boot
`interrupt_only` claim, without probing the old PID. Foreign-host,
Windows-unproven, all-`NULL` out-of-scope, and legacy-v8 unclaimed running rows
remain explicit debt, as do unproven or legacy external stages. None of these
observations grants permission to inspect or delete an unproven path.

Symlinked storage roots, ownership mismatch, unsupported schema versions, and
unsafe permissions block writes. Older clients fail read-only rather than
downgrade or corrupt shared state. The internal SQLite coordinator provides a
stable cross-process lease whenever an `EngineHandle` opens the store. The
application snapshot owner now takes that compatibility lease before its own
writer lock and retains snapshot exclusion through the exact scan-summary CAS.
Schema v17 additionally coordinates app/engine scans with uncached progressive
CLI traversal through the bounded scan-scope registry described above.
Cache-only CLI review performs no traversal and acquires no scope lease.

### 12.3 Retention and deletion

Retention is explicit per store. Hourly disk samples are retained for 30 days
and daily rollups for one year. Full-tree snapshots retain the latest two
complete snapshots per root plus any snapshot referenced by an active cleanup
review. The snapshot directory has a default 2 GiB total cap and evicts the
oldest unreferenced snapshots first. AI insights default to 30 days, are
user-clearable, and are invalidated when the redacted-input digest changes.
Operation history is retained until the user explicitly clears it so interrupted
and failed cleanup remains explainable.

UniFFI v53 exposes one input-free, bounded observation of storage owned by
DUX's active private database, snapshot, and managed scan-cache stores. It is deliberately
path-free and authority-free: no path, filename, scan or candidate ID,
inventory token, selector, cleanup capability, or mutation command crosses the
boundary. The core keeps the database writer/current-schema guard before the
snapshot writer lease and the final managed-cache writer lock, reuses the
complete bounded inventories, and revalidates all retained stores before
returning. Replacement, usage drift, unsafe ownership/permissions,
incompatible schema, query-budget exhaustion, or checked-arithmetic failure
rejects the complete observation.

The database component counts only the retained SQLite main file, exact
SQLite sidecars, and stable marker/lock/initialization controls. The snapshot
component separates controls, protected and retention-eligible available
finals, tombstoned residuals, physical orphans, and recognized active,
quiescent, or unleased temporary files. Per-file charged usage is
`max(logical, allocated)` and checked sums preserve logical, allocated, and
charged totals independently. Directory-entry metadata and unattributable
interrupted provisioning stages are excluded. The managed-cache component
separates fixed ownership controls, published entries, and recognized
quiescent temporary remnants. Its conventional outer `Dux` directory and every
legacy or unknown sibling remain excluded: ownership begins only at the exact
marker-validated `scan-cache-v1` child.

AI reporting is a logical subset of the SQLite component, never another
physical total or a reclaimable-space promise. One scalar, allocation-constant
pager validates and sums insight IDs, fixed input digests,
provider/adapter/model labels, and output payloads, including an expired
subset. Integer fields and SQLite record/page/index/fragmentation overhead are
not counted. There is no arbitrary row ceiling; fixed VM-work and elapsed-time
budgets fail closed without partial results. Deleting an AI row does not imply
that SQLite immediately shrinks or that free space increases. Swift repeats
the complete accounting algebra, loads off the main thread, preserves the last
valid observation after failure, and charts the additive database, snapshot,
and managed-cache components with text/symbol/pattern accessibility.

Managed-cache clearing is a separate, narrow operation rather than a general
retention or filesystem API. The core mints one engine-bound, consume-once,
path-free preview of the exact current entry and recognized-temporary
population. It expires after two monotonic minutes. Final admission reacquires
the writer lock and rejects any inventory drift before the first unlink. The
operation accepts no path, filename, key, selector, AI output, or caller-owned
effect and cannot remove ownership controls, outer/legacy siblings, embedded
AI, database/history, snapshots, settings, or user files. A post-effect
durability or revalidation ambiguity becomes outcome unknown. Swift invalidates
the earlier footprint, remeasures once, and never retries deletion.

UniFFI v54 adds a separate explicit **Clear older snapshots** boundary. It is
not a caller-directed retention API: the engine alone freezes the exact
bounded population of retention-eligible available finals and already
tombstoned physical residuals into a two-minute, consume-once, engine-bound
preview. Latest-two finals per exact encoded root and every active review pin
remain protected. Orphans, every temporary category, residual temporary
leases, provisioning stages, controls, history, the managed cache, AI content,
settings, legacy cache data, and user files remain outside the operation.
Preparation rejects unstable temporary accounting. Only scalar counts, exact
logical/allocated/charged aggregates, protected and excluded aggregates,
active-review count, and timestamps cross FFI; roots, scan IDs, names, digests,
pin identities, selectors, inventories, and victim capabilities do not.

Consumption reacquires the permanent database-before-snapshot lock order,
rebuilds the complete private witness, and rejects any drift before effect.
Every selected final is decoded against its immutable reference before one
atomic transaction appends the complete set of new tombstones. No available
file can be removed before its tombstone is exact. Exact retained-handle
removal then clears newly retired finals and pre-existing residuals while
preserving immutable history and tombstones. A pre-effect mismatch is
`ChangedSincePreview`; after the first possibly committed tombstone or unlink,
any uncertainty is `OutcomeUnknown`. Swift consumes the lease before invoking
the effect, invalidates the earlier footprint, measures once, and never retries
the mutation. Charged usage is conservative accounting, not a free-space
promise.

The SQLite schema records raw and daily-rollup capacity samples as distinct,
constrained kinds so retention cannot infer a sample's lifetime from timestamp
shape. Scan coverage likewise records a constrained status separately from its
optional quantitative estimate: unknown coverage remains `NULL`, never a
misleading zero, while complete coverage is exactly 1000 permille.

Typed raw-sample and bounded scan-coverage persistence exist today. A private,
writer-leased SQLite maintenance operation now enforces the capacity windows in
bounded transactions: it copies the exact last raw tuple of a completed UTC day,
keeps the exact 30-day raw and 365-complete-day daily boundaries, requires an
exact daily representative before pruning still-relevant raw data, and removes
only expired AI cache rows. It validates every selected row before mutation,
fails a mismatched rollup or malformed target atomically, uses a SQL authorizer
that can mutate only `disk_samples` and delete from `ai_insights`, and keeps the
sole private AI delete statement ID- and expiration-bound. It reconciles frozen
postconditions after commit ambiguity. Fixed row, VM-operation, and
deadline budgets return `has_more` rather than extending a writer transaction.

The shared engine now owns a typed, path-free, idle-only history-maintenance
task. Closed, duplicate, and foreground-busy calls resolve before SQLite
access; an eligible call rechecks current-schema compatibility and admission,
then runs exactly one bounded transaction. Cancellation and its Applying point
of no return are linearized under the registry lock, so an earlier cancellation
mutates nothing and a later request cannot falsify a committed outcome. The
engine never self-enqueues from `has_more`. Native app/FFI idle scheduling and
Explorer review-lease ownership are now implemented without adding any
authority-bearing input. Native app/FFI capacity presentation and default-policy
sampling, custom threshold controls, and anchored pressure-episode presentation
are also implemented. Pressure history is path-free telemetry and never enters
the planner or executor. Broad/unproven stage scavenging and explicit user
clear-data actions remain unimplemented. Exact-marker-owned root-local stage
reconciliation has its own implemented boundary below.

Schema v5 now records snapshot logical retirement separately from immutable
scan history. Its append-only tombstone duplicates and foreign-key-binds the
exact succeeded snapshot identity; v4 upgrades create none. Snapshot loads
consult one bounded exact-ID tombstone under the current-schema database guard
before opening a file. A matching row is unavailable even when bytes remain,
and a malformed or mismatched row fails as corruption. Trigger programs remain
disabled with depth zero while untrusted schemas are inspected; only the exact
supported fingerprint activates depth one for the six migration-owned guards:
the v5 tombstone update/delete guards, the v6 review-pin update guard, and
schema-v8's temp-lease insert/update plus succeeded-scan-with-lease guards.
Newer schemas stay trigger-disabled and read-only. The sealed production writer
uses the final database-before-snapshot lock boundary described below; it can
only append the complete succeeded identity selected under that boundary and
never updates or deletes a tombstone. Physical removal uses an exact retained
file identity followed by a directory flush. This preserves the rule that a
crash can leave only a logically retired, revalidatable DUX residual, never a
history row that silently points at file-first deletion. The complete policy
and remaining scheduling/scavenging prerequisites are in
[`docs/RETENTION.md`](docs/RETENTION.md).

Schema v6 adds the separate active-review fact needed by that future decision.
A pin is an explicit ten-minute lease over the exact succeeded snapshot tuple,
owned by one strictly decoded process instance and scoped to Explorer or
cleanup review. It is never inferred from candidate or plan state. Immutable
identity/owner/purpose/creation guards, monotonic renewal, idempotent exact
release, no migration backfill, 64-per-owner and 1,024-per-store limits,
complete bounded hostile-row inspection, and 64-row expired pruning keep
coordination finite.
Acquisition holds the current-schema database fence before the snapshot writer
lock, retains the exact read-only file handle, and commits the pin before
releasing either boundary. Acquire, renew, and release reconcile only exact
postconditions after ambiguity. Drop performs no persistence work and leaves
the conservative row to expire; process liveness does not shorten it. The API
is sealed inside core until Explorer/FFI owns its acquire/renew/release
lifecycle. After expiry, that owner must also close the lease object promptly
because its retained file handle outlives the durable row and can defer
physical block reclamation after a logically valid unlink.

Schema v7 adds only a partial lookup index over lossless snapshot-relative-path
encoding, bytes, and scan ID for rows that carry a snapshot reference. This is
a bounded inventory-reconciliation prerequisite; it does not choose victims,
create tombstones, unlink files, or grant cleanup authority.

Schema v8 adds at most 64 immutable `snapshot_temp_leases` rows. Each binds a
random 128-bit lease ID, one running scan, its deterministic final name, one
unique recognized temp name, a strictly decoded process-instance observation,
and creation time. V7 upgrades fabricate no liveness. The insert/update guards
enforce the bounded immutable facts and a scan cannot become `succeeded` while
its row remains. Failed, cancelled, and interrupted parents may retain a row as
explicit crash debt. PID, process-instance identity, and age never prove that
the staging writer is live; only contention on the staged file's kernel lock
does.

Creation follows SQLite connection mutex → cross-process
writer/current-schema lease → snapshot writer lock. DUX reserves the exact
name and durably commits its row before exclusive file creation, acquires a
kernel-exclusive file lock, and completes creation before releasing the two
store-wide locks. Encoding then continues with only the file/kernel lease.
Drop and unwinding close only. A compliant creator can therefore never still
be waiting to create a row-only temp after a maintenance observer acquires the
database and snapshot locks in that order.

The sealed inventory acquires the current-schema database fence before one
snapshot writer lease, sequentially opens/captures/closes every accepted
physical entry in one bounded walk, and performs at most one indexed exact
history lookup per final. It then sequentially reopens every name, revalidates
identity, and requires immutable final/control usage to remain exact. It
strictly validates the complete bounded pin population without pruning and
groups latest-two by exact encoded root bytes, never canonicalized display
paths. Logical length and handle-derived allocation are both reported; the cap
charges their maximum with checked totals. Active-pin inconsistencies,
duplicate/hostile rows, unsafe objects, over-budget work, and arithmetic
failure yield no eligibility evidence. Tombstoned residuals, physical orphans,
and recognized temps are distinct debt. The bounded v8 row population and a
nonblocking kernel-lock probe classify a row-bound temp as active or
quiescent-at-observation, a physical temp without a row as unleased, and a row
without a file as separate residual metadata. Active and unleased files keep
accounting unstable; all physical classes remain charged and non-evictable.
Quiescence is only an observation, so this inventory cannot authorize
scavenging or cap enforcement.

An exact same-scan retry is the publication path's temp mutation outside normal
publication/abort. Under database then snapshot exclusion, a contended kernel
lock returns busy. A quiescent temp must match the row's exact name and prior
identity, be reopened and locked nonblockingly, pass retained/name validation,
then be unlinked and followed by a directory flush before the row is consumed.
A row-only residual can have its exact row removed under the same locks because
row-before-file creation can no longer be pending. The retry neither adopts nor
removes an unleased temp. Normal abort is likewise physical-first. Successful
publication makes the immutable final durable first, then atomically deletes
the exact lease with the succeeded scan summary and optional evaluation while
snapshot-writer exclusion is retained. A rollback preserves the running scan
and lease; only exact post-commit facts may reconcile ambiguity.

A still-live staged handle is a separate, narrower current-call rollback
capability over only its retained identity. Under an exact current-schema
guard, a missing or conflicting row forbids publication; DUX removes that one
kernel-locked temp, leaves conflicting metadata untouched, and returns
corruption. If the guard cannot be established, the handle is close-only. This
does not turn a physical unleased observation into adoption or scavenging
authority.

Terminal snapshot-temp maintenance is a separate sealed one-row authority.
Under a current-schema database guard followed by the snapshot writer lease, it
classifies the complete bounded immutable lease population through joined
parent status; those aggregate observations are not mutation authority. Before
any effect, it fully decodes the selected actionable row's exact parent and
accepts only `failed`, `cancelled`, or `interrupted` with no snapshot reference.
`running`, missing, queued, succeeded, malformed, or snapshot-bearing selected
parents grant no mutation. Stored PID, process instance, creation time, age,
and a prior quiescent observation never prove writer death or authorize scan
settlement.

The batch counts all terminal row-bound residuals, skips active files so they
cannot starve later actionable debt, and selects at most the first deterministic
row-only or quiescent row. A row-only residual may consume only its exact row
after the locked directory state is durably confirmed. A physical temp must be
reopened no-follow, identity- and exact-usage-matched, kernel-locked
nonblockingly again, and name/private-object revalidated. Checked accounting is
frozen before its physical-first removal. The delete-capable handle closes
before directory sync; a known pre-unlink failure is distinct from post-unlink
`OutcomeUnknown`, which retains the row. Only after durable removal may the
exact row be consumed. The parent scan and every final, tombstone, pin,
candidate, cleanup, evaluation, and unrelated temp row remain unchanged.

The corresponding typed idle-only engine task accepts no scan, lease, owner,
name, root, path, inventory, or candidate. It exposes path-free aggregate
outcomes/counts/bytes, invokes one repository batch, linearizes cancellation at
Applying, and never self-schedules. Native scheduling must back off active-only
debt. Running-row recovery, unleased temps, and provisioning stages remain
outside this authority.

Unleased snapshot-temp maintenance is an independent physical-only authority,
not an extension of the terminal-row capability. The full authority conjunction
is: a retained marker-owned private snapshot store; current-schema database
guard acquired before and held with the snapshot writer lease; the complete
at-most-64-row immutable temp-lease population; one complete bounded physical
inventory; the whole generated
`.snapshot-<64 lowercase hex>.<canonical nonzero u32 PID>.<32 lowercase hex>.tmp`
grammar; absence of that exact case-sensitive name from every lease row; and a
fresh no-follow proof of unchanged retained/name identity, logical/allocation
usage, private regular-file protection, one link, and a second nonblocking
exclusive kernel lock. Prefix, embedded PID, owner/process identity, age,
mtime, and an earlier quiescent observation are never authority.

The batch orders candidates by exact ASCII name, skips active entries so they
cannot starve later quiescent debt, and removes at most the first quiescent
item. An empty set reports `NoUnleasedTemp`; active-only debt reports
`DeferredActive` and requires later backoff. Exact after-count and charged-byte
subtractions are checked before effect. The locked delete-capable handle is
consumed and closed before the snapshot directory is synced, including Windows
POSIX disposition. A known pre-unlink error is no-effect; any uncertainty after
unlink is `OutcomeUnknown`. Success updates only the retained physical
inventory. The batch performs no SQLite mutation, never adopts or maps the temp
to a scan, and changes no scan, temp row, final, tombstone, pin, provisioning
stage, or unrelated history.

Pre-v8/version-skew protection is platform-qualified. The current-schema fence
prevents an older writer from passing its later publication revalidation after
v8 wins. Windows pre-v8 writable temp handles denied delete sharing, so the
delete-capable reopen fails while such an older writer is actually live. Unix
can unlink an open inode and pre-v8 writers did not hold the v8 advisory lock;
an old writer may therefore continue writing only its detached handle, then
fails name/current-schema publication and cannot create a final or database
reference. Preventing that same-user availability race requires not running
old and current binaries concurrently on one store. The retained 0700/0600
owner checks or protected owner-only Windows DACL exclude other users; they are
not a defense against a malicious or incompatible process with the same user
identity.

Only the typed idle-only `SnapshotUnleasedTempMaintenance` task may invoke this
batch through `EngineHandle::start_snapshot_unleased_temp_maintenance`. Its
Applying/Finished events and immutable result getter expose only canonical
time, bounded unleased/active counts, charged bytes, `has_more`, and
`NoUnleasedTemp`, `DeferredActive`, or `Removed { bytes }`. The task accepts no
name, PID, identity, scan, lease, path, inventory, or candidate; invokes one
repository batch; linearizes cancellation at Applying; and never loops or
self-schedules.

Provisioning-stage maintenance is a separate physical-only, non-recursive
authority. `SnapshotRepository::reconcile_snapshot_provisioning_stage` first
retains the current-schema database guard that excludes a compliant concurrent
provisioner, then completely inventories the retained marker-owned database
root. The raw/native walk counts every root entry against fixed total-entry,
256-KiB aggregate-name, 64-stage, and 250-ms bounds without requiring unrelated
names to be UTF-8. Only exact
`.dux-snapshot-stage-<32 lowercase hex>` names are candidates, ordered by exact
ASCII bytes. A candidate must be a freshly name/identity-revalidated exact
current-user-owned 0700 Unix directory (and, on macOS, have no extended ACL),
or the equivalent protected current-user-only Windows DACL with no reparse
shape. Owner-owned Unix modes that are strict
subsets of 0700, including mode 000 left between `mkdirat` and mode repair, are
opaque unproven debt and are not opened.

The complete stage inventory must contain only the exact 16-byte store marker,
optionally followed by the exact 16-byte writer marker. Each control is opened
no-follow with deletion access and must be a private, single-link regular file
whose retained identity matches its name; its exact 16-byte marker and
point-in-time logical/allocation usage are validated during the complete
inventory. Empty stages defer without starving later proven debt. A missing,
partial, wrong or writer-only marker set, any unknown/extra child, a link or
reparse point, unsafe permissions/DACL, changed identity, exceeded bound, or
incomplete inventory fails the entire batch before effect. Legacy external
siblings are outside the retained root and are never considered, adopted, or
removed. The batch mutates no SQLite row and does not infer a scan, lease,
owner, PID, age, or liveness relationship.

At most the first lexical marker-owned stage is removed. Checked before/after
stage counts and marker/writer charged bytes are frozen first; directory
allocation is not reported. Marker-complete removal orders writer disposition/
unlink, stage sync, marker disposition/unlink, stage sync, empty-directory
removal, then retained-root sync. Marker-only removal begins at the marker step.
Handles are consumed and closed before the corresponding directory flush.
Anything known to fail before the first namespace effect is `BeforeEffect`;
anything uncertain afterward is `OutcomeUnknown`. A crash after marker removal
may leave an empty unproven directory that later batches deliberately preserve;
fully automatic recovery of that state requires a durable deletion journal.

Only the typed idle-admitted `SnapshotProvisioningStageMaintenance` task may
invoke this boundary. It accepts no root, stage name, path, identity, inventory,
cap, scan, lease, or victim; calls one repository batch; and publishes only
canonical time, aggregate before/after counts and charged bytes, `has_more`, and
the redacted `NoStage`, `DeferredUnproven`, `RemovedMarkerOnly`, or
`RemovedMarkerComplete` outcome. Applying is the cancellation/close point of no
return, and core never loops or self-enqueues. Native Windows regression cases
are compiled for CI, but this checkpoint has not executed the Windows mutation
path on a Windows host.

The v8 temp protocol itself is not a broad scavenger or retention-victim
decision. The repository's separate cap writer may unlink only an exact
tombstoned final; it gains no authority over temp rows or names. App/FFI
ownership and scheduling plus same-scope hard-process-death recovery for
schema-v9 claimed running rows and schema-v16 same-host/prior-boot history
interruption are implemented without granting snapshot authority. Legacy-v8
running-row policy, unproven/legacy stage handling, and native Windows runtime
verification of the lock/removal paths remain future gates.

The snapshot cap itself is a typed, exact-key setting. The canonical
value-schema-v1 `snapshot_retention` object contains only `cap_bytes`; absence
means 2 GiB without an implicit write. Strict bounded decoding rejects
malformed/noncanonical current values, reports newer per-setting schemas as
incompatible, and leaves every unknown key untouched. Core engine get/set/reset
return path-free default/stored provenance and exact-reconcile ambiguous
commits. Inventory rereads the effective value under its current-schema
database guard before the snapshot lock. Neither a settings DTO nor that
read-only report grants tombstone, unlink, or cap-enforcement authority. The
sealed writer rereads the setting and repeats the complete proof under its final
locks. UniFFI v51 exposes only versioned, path-free policy records and
get/set/reset operations; it never accepts a snapshot identity, path, victim,
inventory, or retention command. Native Settings independently validates the
record version, source/time relationship, and exact echoed value before
publication. Its generation-fenced model serializes operations, preserves the
last confirmed policy on failure, and requires an authoritative read after
`OutcomeUnknown` rather than retrying a write. A zero cap remains policy input,
not a clear-data action, and no cap mutation invokes the sealed retention
writer.

One core cap-enforcement batch removes at most one final. It first rebuilds the
bounded physical/history inventory under the current-schema database guard and
snapshot writer lease. Existing tombstoned physical residuals have priority.
For a new retirement, active or unleased temps defer the decision, latest-two
per exact encoded root and active review pins remain protected, and only the
deterministic oldest available entry may be selected while the freshly charged
total exceeds the cap. Orphans, controls, temps, protected snapshots, and
already tombstoned bytes are never normal victims.

Before either fresh retirement or residual removal, the writer reopens the
observed final read-only, requires its exact identity and logical/allocation
usage, fully decodes the bounded snapshot, and matches its scan ID and digest
to immutable history. A fresh victim's complete tombstone is committed and
exact-reconciled before any file mutation. Only then is the same observed
identity reopened with deletion access while the digest-validated read handle
remains live. Identity, name, and usage are checked again before Unix performs
descriptor-relative name unlink or Windows uses handle disposition, followed
by a durable directory flush. Commit/schema uncertainty leaves the name
untouched. A later residual retry repeats identity, usage, and full-content
validation, so changed bytes or a post-validation replacement are not removed
by that batch. The documented malicious same-user Unix substitution window
remains outside the private-store isolation guarantee. Immutable scan and
tombstone rows are never deleted.

The sole engine edge into that sealed writer is the typed, idle-only
`SnapshotRetention` task. Closed, duplicate, foreground-busy, and
other-maintenance-busy states already visible at initial preflight resolve
before storage access; after the compatibility refresh, lifecycle, duplicate,
and idle admission are rechecked before publication. The task accepts no cap,
inventory, victim, scan identity, or path, invokes exactly one batch, and never
self-schedules. Its path-free result is only an aggregate observation of the
repository decision; it cannot identify user data or reach cleanup planning or
execution. Cancellation and close are linearized with
`SnapshotRetentionBatchApplying`: an earlier request prevents the repository
call, while a later request remains intent and cannot rewrite the exact result.
Native app/FFI review-lease ownership and periodic idle scheduling are
implemented outside this cap authority: they can retain an exact validated
review or request one sealed batch, but cannot select or alter a victim.
Clear-data remains unimplemented. Provisioning-stage, physical-orphan,
terminal-temp, and unleased-temp maintenance have separate implemented
boundaries.

Physical-orphan reconciliation is a separate, one-final authority and never a
cap-policy fallback. A current-schema database guard and then snapshot writer
lease must remain held while DUX builds a bounded exact-path catalog
reconciliation, selects only its deterministic first zero-reference typed
final, retains its exact identity and logical/allocation usage, and fully
decodes its checksum-validated body. The decoded scan ID must derive the exact
filename and identify an existing parent whose lossless root matches and whose
snapshot reference is absent. Only `running`, `failed`, `cancelled`, or
`interrupted` parents are admissible; missing, queued, succeeded, referenced,
malformed, or root-conflicting parents grant no mutation. A live publication
cannot be in the file-first/DB-commit interval while the reconciler owns the
same snapshot writer lease.

The reconciler keeps the validated read handle live through a separate
delete-capable identity/name/usage recheck, removes at most that exact final,
and requires snapshot-directory durability before success. A known pre-unlink
failure is distinct from post-unlink durability uncertainty; the latter is
`OutcomeUnknown`. It creates no tombstone because there is no succeeded
snapshot tuple to retire, and it never settles the scan, consumes a temp lease,
or rewrites history. Temporary files and provisioning stages neither supply
authority nor become targets of orphan reconciliation. Its separate idle-only engine task accepts no
authority-bearing input and publishes only path-free aggregate observations;
it never self-schedules. Native scheduling, scan-row recovery, clear-data, and
native Windows mutation verification remain separate; provisioning stages use
their own implemented boundary above.

Clearing DUX data removes only DUX-owned stores after the same storage-root and
symlink checks. It does not empty system Trash, provider caches, or user data.
Secure erasure is not promised on copy-on-write or SSD storage.

### 12.4 Logs and diagnostics

Normal logs avoid absolute paths, environment dumps, credentials, provider
stderr, file contents, and raw AI payloads. Stable error categories are logged
separately from redacted technical detail. Diagnostic export shows the exact
payload before saving and uses a user-selected destination.

No path or provider text becomes a metric label. Telemetry, if ever proposed,
requires a separate opt-in privacy design and is never required for cleanup.
Crash reporting is telemetry: it is opt-in, disabled by default, and MUST redact
paths, provider data, environment values, and operation payloads before data
reaches a third-party crash service.

## 13. Target macOS access and process authority

The primary public app will be distributed outside the App Store without App
Sandbox. This improves observable scan coverage but increases compromise
impact. It does not
bypass TCC, SIP, ACLs, POSIX permissions, flags, or filesystem protections and
does not authorize mutation.

DUX MUST begin with the access already available to the user. Full Disk Access is
optional and is never requested as a first-launch prerequisite. macOS exposes
no authoritative public “Full Disk Access enabled” query, so DUX reports
observed access evidence and structured scan coverage, never a fabricated
boolean or a claim of complete disk knowledge.

The app MUST provide useful capacity and accessible-root analysis with limited
coverage. A first-run explanation may persist only its acknowledgement and MUST
NOT scan, probe, sample capacity, invoke FFI, or request permission. The app MUST
offer System Settings guidance only after a concrete scan-coverage gap and an
explicit broader-analysis request. Any observed-access check MUST be bounded to
opening a fixed reviewed set of directories without enumerating names or reading
contents, and presentation receives only path-free aggregate counts. Returning
active MUST re-probe exactly once only when opening that guidance armed the
check; ordinary activation MUST do nothing. Re-probing MUST NOT start a scan or
claim that coverage changed. It MUST never ask for administrator credentials,
disable platform protections, or install a privileged helper to gain coverage.

The public app MUST use Hardened Runtime, library validation, the smallest
reviewed entitlement set, and signed bundled code. `SMAppService.mainApp`
remains opt-in launch at login, not a privilege boundary. The native setting
uses macOS status as its sole source of truth, performs no registration at
startup, serializes user-requested changes, and re-reads status before claiming
an outcome. Approval-required is not described as enabled. The service adds no
helper, daemon, entitlement, filesystem access, engine call, or cleanup
authority, and automated tests never mutate the host's Login Items. Real
enable/disable and sign-in-cycle validation waits for a signed stable app with
the frozen production identifier. The separately launched CLI has its own
process and TCC context; the app MUST NOT imply that GUI access transfers to it
or use the CLI as an access bypass.

Conditional menu-bar visibility is presentation policy, not storage or cleanup
policy. It may consume only the app's cached effective startup-volume capacity;
changing its validated Swift-owned preference MUST NOT trigger a sample, scan,
FFI call, notification, or filesystem effect. Missing or failed capacity MUST
keep the control surface visible, and an explicit reopen MUST reveal it for the
remainder of the process session without persisting derived insertion or reveal
state. This policy MUST NOT override or impersonate Rust-owned disk-pressure
classification.

## 14. Target FFI and client boundary

The UniFFI boundary is private and versioned independently from product,
database, snapshot, rule, and CLI JSON schemas. App startup MUST verify the DUX
contract version and MUST retain UniFFI checksum checks.

FFI values MUST be owned, immutable, coarse-grained, paginated or budgeted, and
use fixed-width scalar types. The boundary does not export `PathBuf`, `SystemTime`,
arena references, platform handles, unbounded trees, or localized strings as
authority.

The engine MUST own one task registry, bounded workers, cancellation tokens,
immutable published snapshots, serialized cleanup, and coordinated persistence.
Swift MUST invoke blocking calls off the main actor, convert generated values to
application-owned `Sendable` DTOs, and treat ARC release as neither
cancellation nor shutdown. Explicit close and cancellation are idempotent.

The current core checkpoint owns one registry per engine session with fixed
worker, queue, event, terminal-record, and input bounds; the application
architecture owns one lazily opened engine session. Startup validation and
migration run on the app's utility queue before workers are published and
expose no raw SQL or domain persistence. Typed production work currently includes read-only
formatting, durable full scans with deterministic candidate evaluation, one
exact-scan path-free durable candidate-discovery history projection, and one
bounded recent plus exact-session path-free cleanup-history projection, and one
idle-only bounded batch for each of the eight maintenance kinds. These history projections are
Rust-core only and cannot reconstruct a candidate, cleanup plan, or execution
fence. Maintenance resolves
duplicate or busy preflight without SQLite, rechecks compatibility before
admission, and publishes only path-free counts and `has_more`; it has no AI
inference or classification, plan, or user-data cleanup authority.
The native eight-kind rotation requests scan recovery, then pending
candidate-evaluation recovery, before terminal-temp reconciliation. Neither
recovery task accepts a scan, owner, lease, path, candidate, or victim.
Cancellation intent is recorded separately from the operation-
reported outcome so a late request cannot falsely claim completed effects were
rolled back. UniFFI contract v32 retains the real engine handle and carries v7's
opaque, non-reconstructable maintenance tasks, versioned path-free aggregate
poll results, cancellation, and Explorer-purpose review sessions acquired by
validated scan ID. Review objects expose no path, digest, filename, handle,
candidate, inventory, or cleanup input. The Swift owner uses per-scan
generations, five-minute renewal, wake renewal, explicit release, and ordered
shutdown. FFI close rejects renewal, attempts exact release for every still-live
registered review, then closes core; a failed durable release expires naturally.
Concurrent close callers observe one bounded quiescence result. Engine `Closed`
means every worker has quiesced or the fixed five-second wait reported that it
did not.

Contract v5 added one bounded startup-volume request/response operation.
Both records are versioned. It fixes `/` inside the Rust adapter, accepts no
caller path, returns no path, and exposes only capacity facts, pressure
boundaries, prior durable pressure, and a history disposition. Swift invokes it
off the main actor and uses the Rust result as render state; it does not duplicate
threshold policy or derive used bytes when ordinary availability is absent.

Contract v6 adds path-free versioned pressure-policy get/set/reset operations.
Rust remains the only semantic validator and evaluator. The exact-key canonical
JSON setting distinguishes a missing Default revision 0, explicit Stored values
including values equal to defaults, and persisted Default reset epochs. Revisions
advance monotonically only for a real source/value change. The Swift adapter
retains editable strings until exact integer GiB/basis-point conversion succeeds,
maps every typed error, preserves last-good state, and signals the capacity
scheduler once only for a changed result. These calls cannot express a path,
scan, candidate, notification, schedule, plan, or cleanup action.

Contract v7 adds one opaque read-only scan task. Its only path-bearing field is
a versioned, nonempty, absolute, control-free discovery root bounded to 32 KiB;
the root is an observation scope and is never returned, persisted as transport
authority, or accepted by any planner or executor operation. Core performs all
filesystem-kind, symlink, identity, overlap, schema, and task-admission checks.
The production Swift adapter supplies the current user's Home directory only;
arbitrary injected roots are test-only. Poll and cancel return authoritative
phase, coarse stage, cancellation intent, revision, optional cumulative
aggregate counts, sticky truncation evidence, typed path-free failure, and a
path-free terminal summary. Missing progress remains unknown. Neither the task
nor its result contains a node, current path, candidate detail, review command,
plan, approval, execution fence, or cleanup capability. The app requests scan
cancellation and quiesces its generation-fenced publication driver before the
maintenance/review/engine shutdown chain.

Contract v22 introduced path-free global permanent-cleanup get/set/reset
operations. Contract v34 changes their semantic default without adding fields:
Rust maps missing/default state to disabled, preserves only versioned explicit
stored consent, and rejects impossible `Default(true)` projections. Swift
receives only bounded enabled/provenance observations and typed storage errors;
it cannot supply a target, plan, approval, callback, or executor input.
EngineService rejects malformed status shapes, while the generation-fenced
AppModel requires an authoritative loaded disabled policy and the exact
sentence `ENABLE PERMANENT CLEANUP` before the sole product enable call.
Disabling and reset-to-disabled are immediate protection-strengthening
operations and require no enable confirmation. Shutdown invalidates pending
setting work. The path-bearing exclusion setting remains a separate boundary
and is not included in this contract.

Notification authorization is a separate Swift-owned observation boundary.
Settings reads `UNUserNotificationCenter` status and may request Alert and Sound
authorization only after an explicit action from a confirmed Not Determined
state. The adapter rechecks that state before requesting, and the app re-reads
the authoritative status afterward. Its protocol exposes no notification
request, content, delivery, removal, delegate, or deep-link operation; opening
Settings or returning active can read status but MUST NOT prompt. Authorization
grants no scan, candidate, plan, AI, scheduling, or cleanup capability.
Transition delivery, pressure-episode deduplication, cooldowns, and deep links
remain absent until the Milestone 6 boundary is implemented and reviewed.

Emergency recovery ordering is a separate read-only observation boundary under
UniFFI contract v40. Rust owns the revisioned fixed order and accepts only the
exact retained Critical pressure proof plus targeted-root catalog stamp. It
privately reselects exact retained scans and current terminal candidate
evaluations, excludes evidence more than one hour older than the capacity
anchor, and repeats the pressure/catalog checkpoint after projection. Revision
1 emits only stale safe-regenerable observations, guided exploration, and
permission/coverage gaps. Unsupported cloud, Trash, installer/archive, and
large-file lanes are absent rather than zero or fabricated. An unavailable
root is a typed permission/coverage count; it never receives a fake scan ID,
hidden-byte estimate, or action.

The ordering and every child are path-free and bounded. They MUST NOT contain a
candidate ID, reclaim forecast, aggregate byte total, cleanup mode, plan,
approval, AI input, schedule, callback, provider command, platform driver,
filesystem handle, or executor token. Existing Homebrew and pip observations
remain review-only behind their mandatory evidence/protection blockers.
Core, FFI, and Swift independently validate completeness arithmetic, ranks,
fixed lane order, semantic uniqueness, lane payloads, source ordinals,
timestamps, and the unavailable-root permission group. Swift consumes its
retained proof only after the complete response validates. Explorer and menu
actions may navigate only to the exact read-only Candidates, Browse, or
Coverage view. Critical pressure MUST NOT weaken any planner or executor rule.

Native scheduler shutdown invalidates its generation, requests cancellation for
the current opaque task, and awaits the driver before review release or engine
close proceeds. Results returning from a suspended energy check, maintenance
admission, or task poll must revalidate that generation before changing state.

Every expected error is typed. Rust panics are defects and MUST NOT become UI
text or unwind through Swift. Callback/event tests cover retention, completion,
cancellation, reentrancy, stale generations, and cycle avoidance before real
engine work crosses the boundary.

The implemented `dux status`, scan `history`, `scan-detail`, `candidates`,
`review-state`, and `cleanup-history` JSON surfaces use shared bounded engine
observations. Status/history are path-free and validate selected parents plus
complete coverage children under the fixed recent-history SQLite budget.
Scan detail omits stored relative components and reports only global/root/
descendant scope. Candidate summaries omit paths and evidence payloads;
candidate IDs remain sensitive local pseudonyms rather than anonymous
remote-safe identifiers. Cleanup history omits targets, evidence, candidates,
claims, owners, generations, and effect fences. None of these observations is
current validation, a plan, approval, retry instruction, recovery capability,
or executor authority.

`review-state` is the only mutation in this CLI contract. It retains the exact
snapshot-review lease while applying one of four semantic, scan/candidate-bound
review commands. It accepts no path or arbitrary status, reports explicitly
that no cleanup occurred, and treats an ambiguous commit as non-retryable until
the caller reloads candidate history. The CLI adapter prepares missing standard
platform parents on first use, but core independently validates the exact
publication parent and owns private-store staging/publication. Every JSON object
has a schema version; output explicitly declares no path disclosure; runtime
errors use stderr and nonzero status; and golden tests prevent accidental
contract drift. A newer database reports compatibility but does not query
unknown layout. No JSON command accepts a cleanup target, plan, approval,
executable AI output, or arbitrary permanent-cleanup path. The normative
serialization/null/error/cursor contract is `docs/CLI_JSON.md`.

### 14.1 Optional CLI installation

The app installs its bundled universal CLI only through
`CLIInstallerService`, at the single account-derived destination
`~/.local/bin/dux`. `HOME` and arbitrary destinations are not accepted as
input. Read-only inspection and every mutation traverse HOME, `.local`, and
`bin` through directory descriptors with `O_NOFOLLOW`; owners, permissions,
file type, link count, executable mode, and a 256 MiB ceiling are checked. Only
missing per-user `.local`/`bin` directories may be created. The app never
executes the destination.

The bundle contains one arm64/x86_64 macOS-14 CLI and one exact canonical
manifest. The manifest binds record/product/version, database schema, snapshot
format, ordered architectures, and the SHA-256 of the signed executable. The
hidden metadata command accepts no arguments and emits no path or storage
content. Packaging verifies both slices, deployment target, full-byte hash,
Hardened Runtime static signature, signing identifier, and the compatibility
tuple before and after Xcode embedding. Local/CI builds require the reviewed
ad-hoc development identities; production binds the CLI identifier
`<app-bundle-id>.cli` and Team ID to the signed outer application. A valid hash
alone never substitutes for a valid code-signature relationship.

An installed file is managed only when all filesystem checks pass and its
bounded `com.mjukis.dux.cli-installation` xattr has the exact DUX
record/product/source, semantic version, and full-byte hash. Its Mach-O slices
and static signature must still match the current bundled CLI. A missing marker
is unmanaged; a malformed marker or changed managed evidence is unsafe. DUX
never adopts, executes, replaces, or removes either category.

Install and reinstall/upgrade copy from an already observed bundled descriptor
into a UUID sibling created with `O_EXCL`. The service writes all bytes, applies
mode `0755`, writes the create-only managed marker, `fsync`s, reopens, hashes,
parses, signature-checks, and path-revalidates that stage. A new install uses
`RENAME_EXCL`. Upgrade uses `RENAME_SWAP`; the old inode remains under the
private stage name until it exactly matches the confirmed managed evidence,
then alone is unlinked. Any mismatch swaps the entries back. Uninstall first
creates and fully observes a random non-executable sentinel, atomically swaps it
with the confirmed destination, and proves both the displaced managed inode
and published sentinel before either unlink. Observation failure or mismatch
restores the swap. Final directory `fsync` and observation are required;
post-mutation ambiguity is `outcomeUnknown`, never success.

The actor permits one prepared operation at a time. Confirmation is one-shot
and binds the exact bundled and target observations; the platform re-observes
both under an exclusive directory lock immediately before mutation. Downgrade
of a newer managed CLI is refused. After a changed or uncertain outcome,
Settings clears potentially stale status and disables further changes until an
authoritative reload. Dismissing Settings discards an unaccepted confirmation;
shutdown cancels observation/preparation but waits for an already-confirmed
mutation to return before closing.

The installer MUST NOT escalate privileges, write a system directory, read or
edit shell startup files, or imply that the CLI inherits the app's TCC access.
Settings gives manual PATH guidance only. App uninstall, data clearing, and
Sparkle never change the separately installed CLI; a new app version can only
offer another explicit confirmed upgrade. Standalone Homebrew/crates.io
releases remain unmanaged, and schema-version skew follows the
read-only/fail-cleanly rules above.

## 15. Target concurrency, cancellation, and recovery

Shared cleanup execution MUST be serialized even when scans use bounded
parallelism. A cleanup session MUST be durably recorded before its first item.
Each item MUST transition through valid journal states in a transaction or
equivalent durable protocol.
On restart, an incomplete session is reported as `RecoveryRequired` or
`OutcomeUnknown` until live inspection reconciles it; DUX never assumes an
in-flight syscall succeeded. A terminal `interrupted` journal state is written
only by explicit reconciliation, never merely because a process disappeared.

Cancellation is a request, not proof that an operating-system call stopped.
The executor checks cancellation between items and before effects, drains or
records active work, and reports the difference between cancellation requested
and execution quiesced. Graceful app quit waits for or cleanly cancels executor
work according to the reviewed operation contract. A force-quit warning cannot
make a partial recursive effect atomic.

Database and snapshot updates occur after filesystem results are durable, not
before. Failure to update a display cache does not roll back a filesystem
effect; the journal remains the recovery source. Cache/tree removal happens
only after confirmed success. Retry creates a new plan when evidence or expiry
requires it and never retries a protected or changed target automatically.

## 16. Supply chain, release, and updates

Public app releases are universal, Developer ID signed, Hardened Runtime
enabled, notarized, and stapled. Nested Rust code and the optional CLI are
signed before the outer app. CI verifies architecture, entitlements, designated
identity, signature, notarization log, staple, Gatekeeper assessment, and
cryptographic checksums.

Release CI uses full-commit action pins, read-only default permissions, locked
dependencies, vulnerability/license/source policy, isolated protected signing
secrets, and a fine-grained token scoped only to the Homebrew tap for the final
ordered job. Pull requests cannot access signing, notarization, publishing, or
tap credentials.

Generated Swift, headers, module maps, Rust archives, and the app come from the
same reviewed source and locked UniFFI graph. CI regenerates committed bindings
and rejects drift. Published artifacts are immutable; a correction receives a
new version.

The local release workflow MUST use only a valid Developer ID Application
identity matching the explicit Team ID and credentials referenced through a
`notarytool` Keychain profile. It MUST NOT accept an Apple ID password or API
private-key path as an argument. The reviewed release entitlement file is an
empty dictionary until a separate security review changes it. Unknown nested
code bundles fail before signing. The app is notarized and stapled before it is
placed into the signed DMG; the DMG is then separately notarized and stapled.
Both Apple logs are retained and checked for accepted status and errors. Private
staging and final version output share the validated repository-owned
filesystem so publication is one same-filesystem rename. Output is published
only after signature, timestamp, Hardened Runtime, exact identity,
architecture, deployment target, entitlements, image integrity/layout, staple,
Gatekeeper, and checksum checks pass. Actual Developer ID execution remains
blocked until Milestone 9 freezes the production identity.

Automatic updates use Sparkle 2 only after the production bundle identity and
Developer ID signing lane are stable, as specified by ADR 0002. The app embeds
only the EdDSA public key; its private key MUST remain outside the repository,
application, artifact host, and public pull-request environment. Protected
release CI publishes a signed HTTPS appcast only after the immutable enclosure
passes Developer ID, Hardened Runtime, notarization, staple, architecture,
deployment-target, checksum, version-monotonicity, and compatibility gates.
Both the signed feed and signed enclosure are verified. Invalid, stale,
downgrade, identity-drifted, or incompatible updates fail closed. Rollback is a
higher-version corrective release, never a silent downgrade. Sparkle updates
only the application bundle and MUST NOT mutate a separately installed CLI.
DUX MUST NOT download or execute an unsigned replacement.

## 17. Verification and enforcement

### 17.1 Destructive-call boundary

CI MUST reject cleanup-capable removal, relocation, Trash/eviction, selected
direct truncation, and process-launch calls outside the executor and narrow
platform-effect adapters. The scan includes Rust removal/truncation APIs, shell
`rm` and `truncate`, Swift `FileManager` removal/eviction and direct URL writes,
Python destructive calls/write modes, process execution that can mutate, and
equivalent new wrappers. Tests and internal atomic-temp cleanup require a
reviewable annotation that names the locally constructed scope; an annotation
is not permitted for user-controlled cleanup. This lint is not a general proof
that arbitrary persistence writes cannot overwrite data: app-owned stores also
require the ownership, permissions, bounded-input, and semantic gates in §11.

This boundary is implemented in two layers. Workspace Clippy configuration
denies compiler-resolved Rust filesystem mutation and child-process methods, so
imports, aliases, re-exports, and formatting do not bypass the Rust rule. The
standard-library repository scanner covers Rust defense in depth plus Swift,
generated Swift, C-family sources, shell and workflow blocks, PowerShell, batch,
and Python. It scans tracked and untracked non-ignored source plus executable,
shebang, Makefile, and `.command` inputs; an unknown executable language fails
closed. Its self-tests exercise multiline calls, aliases and callable references,
quoted/split shell tokens, PowerShell, cloud eviction, process strings,
truncation, generated/untracked inventory, stale annotations, and intentional
negatives. Compiler-resolved Clippy runs on each supported OS so target-gated
Rust is checked, and both policy tests and scanning are repeated for release tags.

Every exception has one stable registered ID bound to an exact path, detected
rule/primitive, and where needed an enclosing symbol or test context. It is
adjacent to one matched statement and includes a specific reason. Unknown,
duplicate, malformed, misplaced, copied, unused, and multi-call annotations
fail, while registered IDs missing from source are stale and fail repository
scanning. The former three `legacy-adapter-delete-*` exceptions and their
architecture allowlist were removed with the CLI adapter. Repository policy
now rejects any attempt to reintroduce that module, symbol family, or raw
recursive-delete surface. Internal cache exceptions own only `create_new`
temporary files and their atomic destination. Build exceptions own only
`mktemp -d` staging paths or the exact repository-generated XCFramework path;
the builder rejects its former caller-selected output path and any symlinked or
non-physical output parent before invoking any tool. The Finder exception
launches a fixed non-shell `open -R` argument vector.

The lint is defense in depth, not authority. Adding an allowed wrapper still
requires typed executor admission, tests, and this document to be updated.

### 17.2 Required test layers

Before app cleanup ships, CI covers:

- lexical, protected-root, alias, mount, symlink/reparse, hard-link, encoding,
  length, and component-boundary unit/property tests on supported platforms;
- a reviewed versioned dangerous-path corpus and retained fuzz regressions;
- rule positive, negative, protected-descendant, nested, active-process,
  symlink, cloud-state, and changed-evidence fixtures;
- plan expiry, overlap, dry-run parity, approval binding, global-disable, and
  action-compatibility tests;
- executor replacement races, permission/mount change, disappearance, partial
  failure, retry, cancellation, process interruption, and journal recovery;
- macOS Trash on an isolated account or temporary volume and refusal/fallback
  tests on other platforms;
- pre/post capacity reporting that never substitutes estimated sizes. The
  cleanup boundary accepts a signed available-space delta only when stable
  volume identity, ordered effect timing, bounded pre/post sample skew, total
  capacity, headline source, and ordinary/important availability shape all
  match. Missing or conflicting telemetry produces no verified delta and cannot
  authorize, imply, or substitute for a filesystem effect. The private
  production Rust-target bridge now derives a kernel filesystem ID, mount
  location, mount ID, and filesystem type only from every revalidated
  rule-scope authorization in the approved plan, using the mount path on macOS
  and a kernel mount ID only where the platform supplies one. Its core-owned
  macOS sampler issues `statfs` for that exact mount and rejects an ID,
  location, or type
  mismatch; no FFI, Swift, CLI, scheduler, AI, or generic capacity DTO can
  select the sampled volume. Effect start and completion are fresh system-clock
  observations around mutation, independently from canonical journal
  timestamps. A separate fresh authority-clock read after pre-sampling and
  immediately before every live witness rebuild refuses an approval that
  expires during telemetry. The current app plan-review route still cannot
  approve or invoke this private bridge;
- private storage permissions, symlinked store roots, migrations, corruption,
  unsupported versions, and concurrent clients;
- FFI panic/error/lifetime/callback/cancellation tests and bounded payloads;
- hostile AI filenames, redaction, provider timeouts, output limits, process
  termination, malformed schemas, and proof that AI cannot reach plan APIs;
- destructive-call linting and a mutation detector proving dry run performs no
  filesystem effect; and
- signed clean-install release verification plus dependency audit/license gates.

Fuzz adapters are bounded, deterministic, filesystem-free, and absent from
normal builds. A fuzz crash is promoted to the reviewed corpus before the fix;
regressions are not deleted merely to make CI pass.

### 17.3 Cleanup release gate

No public macOS build may expose cleanup until all of these are true:

- the centralized executor and forbidden-call lint exist;
- every mutation originates from an unexpired reviewed plan;
- trusted home, profile-container, selected-volume, mount-location, rule-scope,
  and platform executor witnesses are implemented;
- current platform ancestry/reparse gaps for that mode are closed;
- every implemented/exposed mode is visibly distinct, and unavailable Trash,
  permanent-safe, or eviction modes remain absent or disabled;
- operation history records planned, dry-run, trashed, removed, evicted,
  skipped, rejected, failed, changed, cancelled, and interrupted states;
- private data permissions and migrations pass;
- AI is absent from the authority graph and the app works with it disabled;
- limited-access operation shows truthful partial coverage; and
- the complete safety suite passes on every supported platform for exposed
  behavior.

Unsupported platforms or incomplete modes fail closed. Passing one platform's
tests does not authorize another platform.

## 18. Security maintenance and incident response

Update this document in the same change when a trust boundary, cleanup mode,
validator class, protected category, rule source, provider architecture,
entitlement, persistence format, updater, privileged component, or destructive
API changes.

Every macOS major release triggers a review of protected roots, app/container
layout, TCC behavior, SIP, APFS/firmlink/mount behavior, Trash and cloud APIs,
capacity reporting, rule validity, entitlements, and supported provider
confinement. Protected registries have explicit revisions and independently
authored tests.

A cleanup safety incident requires:

1. disable or revoke the affected rule/mode in the next safe release path;
2. preserve local evidence without transmitting paths automatically;
3. add a minimized regression to the appropriate fixture/corpus;
4. identify which authority witness or enforcement layer failed;
5. fix the layer rather than adding only a path-specific exception;
6. update rule revision, tests, this design, and user-facing recovery guidance;
   and
7. assess whether signing, updates, or prior versions require a coordinated
   advisory.

Before public cleanup release, the repository MUST add a `SECURITY.md` defining
a private reporting channel, supported-version policy, expected response, and
safe disclosure process. Until that exists, this document does not claim a
project-operated private reporting channel. DUX does not add telemetry after an
incident as a substitute for deterministic local evidence.

The native Explorer Overview and Candidates view do not widen this boundary.
Overview renders cached
startup-volume capacity and the current path-free Home-scan aggregate from the
single shared app model, never starts work merely because its window opened,
and Candidates renders bounded deterministic summary facts, scan-bound review
status transitions, and user-selected exact historical path/evidence pages
from the same retained review lease. Detail responses are capped at 64 rows,
65,536 encoded path bytes, 262,144 display bytes, and a 24 MiB aggregate page
payload at both the Rust FFI projection and Swift adapter. Pages replace rather
than append, repeat scan/candidate/immutable-body/cursor validation in Swift,
and are discarded after any snapshot, mode, selection, or page generation
change. Current task cancellation clears loading state without accepting a
reply; review expiry releases and invalidates the complete snapshot. The
inspector labels these values as historical and read-only and contains no
live-path conversion. Neither view creates plans, approvals, AI requests, or
cleanup effects. Its capacity composition uses ordinary
filesystem availability;
important-use availability remains a separately labelled observation. Scan
coverage is labelled as Home-scoped and unknown coverage stays unknown.

## 19. Implementation checkpoint matrix

| Control | Current state | Gate before app cleanup |
|---|---|---|
| Strict lexical/live path evidence | Implemented, crate-private and non-authoritative, including retained-descriptor bounded regular-file prefix and full-file SHA-256 reads on Unix. A separate Unix/macOS Trash witness preserves a final symlink as the link object without canonicalization or target inspection, while rejecting symlinked roots/intermediate ancestors and special entries. A repeated filesystem-boundary witness now retains bounded no-follow root-to-scan ancestry plus descriptor-bound platform mount identity | Bind trusted account/home, volume/location, and rule witnesses and executor revalidation; APFS firmlink semantics and Windows handle-relative/reparse evidence remain open |
| Protected-root registry | Implemented text-only policy; Unix/macOS current-account home discovery is code-owned and fail-closed (OS account database, UID ambiguity, no-follow identity, owner check); Windows construction remains unavailable | Trusted OS profile/mount/firmlink evidence and stable rule grants |
| Dangerous-path corpus and fuzzing | Implemented | Keep cross-platform and promote every crash regression |
| Rule schema/loader | Strict schema plus build-time digest/policy gating and strict load-time catalog validation. Nine rules remain selected-root RevealOnly observations. Independently researched Rust-target revision 3 and Python `__pycache__` revision 2 rules propose SafeRegenerable/RemoveKnownRegenerableContents but remain unschedulable and retain `ProtectedPath`; only Rust has the sealed live/Cargo promotion chain described above. Independently researched Homebrew and pip revision-1 rules exist only in the exact user-cache scope, require candidate-local complete seven-day observations, remain unschedulable, and always retain `MissingOrIncompleteEvidence` plus `ProtectedPath`; descendant symlinks add `SymlinkBoundary`. Evaluator revision 4 and the exact 13-rule digest bind both scopes and policies | Developer ID signing must cover catalog bytes; Python and cache-provider live-writer/location/ownership authority remains open, and no proposed safe rule may lose a blocker without its separately reviewed live evidence and executor chain |
| Candidate and cleanup-plan records | Completed fresh scans create deterministic, snapshot-bound durable candidate batches; exact-scan summaries plus bounded lossless path/evidence pages and semantic review commands remain non-authoritative history. A crate-private planner module seals an exact current-evaluator/current-catalog source to a retained snapshot and live Rust-target/Cargo witnesses. One private Rust-target authority chain constructs and approves an exact permanent-safe plan and atomically claims its durable candidate while preserving the sole `ProtectedPath` history fact; an insertion-only typed coupling, schema-v12 exact active-claim seal, complete immutable-body/blocker comparison, current revision-3 marker/age facts, and rule-specific policy checks prevent generic or stale blocked candidates from borrowing that path. Contract v33 retains the real reviewed plan behind an opaque child and carries its newest observed mtime plus exact seven-day requirement; only that exact engine-bound child can be consumed into the serialized task. The native controller stores and compares the entire displayed immutable record before consuming its child. Displayed fields still cannot mint approval, journal metadata, paths, callbacks, scheduling, CLI, AI, or Trash authority | Complete §17.3 before enabling the internal confirmation path in Release; keep the blocker exception rule-specific and non-forgeable |
| Rust-target dry run | Core consumes the exact opaque reviewed-plan child into a separate non-effect `RustTargetDryRun` task. A shared inert validator gives dry run parity with the permanent pre-effect plan/Cargo/process/home/mount/protection/identity/recency/subtree/marker checks, but only the permanent branch can mint an effect witness. Persistence writes an uncoupled ownerless terminal graph under the cleanup/exclusion lock, with no candidate or trusted-rule claim, effect receipt, capacity delta, removed bytes, or permanent-policy dependency. Exclusion overrides success only; cancellation is linearized before recording, unavailable evidence remains distinct from drift, and unresolved metadata never quarantines filesystem cleanup. UniFFI v35 exposes only a distinct consume-once opaque-review start and path-free task; Swift independently validates its envelope and Explorer presents a Release-visible point-in-time dry check with zero-effect accounting and a fresh-preview requirement. It still has no CLI, AI, scheduler, callback, driver, or upgrade-to-cleanup edge | Add parity evidence before exposing another rule or cleanup mode; keep unsupported modes unavailable |
| macOS app cleanup | Confirmation-gated Explorer Trash is implemented. Permanent-safe confirmation, generation-fenced task observation, explicit cancellation, and shutdown quiescence exist only behind `DUX_INTERNAL_PERMANENT_SAFE_CLEANUP` in Debug; Release shows no action and the release pipeline rejects the condition | Entire permanent-safe cleanup release gate in §17.3 |
| CLI cleanup authority | Retired. The CLI remains a read-only scan/navigation/history/reveal client; its former raw permanent-delete adapter, shortcuts, workers, and lint exceptions are absent | Any future CLI cleanup must consume the same current reviewed-plan executor without accepting caller paths or restoring client-owned effects |
| Centralized executor | A private production-core Rust-target driver and typed admission/journal/revalidation chain exist behind an engine-owned `PermanentSafeCleanup` task. The task consumes only the exact opaque review, mints all approval/session inputs inside Rust, serializes with Trash, returns path-free results, and quarantines unresolved claim/effect capabilities. A shared bounded descriptor-relative validator enforces the revision-3 seven-day cutoff before preview, before effect admission, and again inside the driver before any unlink; the driver retains exact per-entry identity/type/link/size/mtime checks. Its capacity sampler derives only from the approved plan's unanimous trusted kernel mount scope, rechecks macOS `statfs` identity/location/type, and brackets real effect time; missing telemetry remains unknown. UniFFI v33 can start this task only by irreversibly consuming the exact engine-bound opaque review; the separately confirmed Explorer Trash route cannot nominate its driver inputs. Native confirmation and observation are integrated behind the Debug-only feature condition; the shipped Release UI has no permanent-safe start action | Remaining §17.3 release gates |
| Engine/FFI task and plan API | Core handle, pre-worker compatibility handshake, bounded task registry, durable scans/evaluations, maintenance, history/detail/settings, and engine-owned Rust-target review tasks are implemented. UniFFI v40's targeted boundary exposes only the Rust-owned cache-first root catalog, path-free digest, ordinal admission, task observations, read-only results, and an exact Critical recovery finalization. Evaluator revision 4 publishes exact Homebrew/pip findings for the user-cache root; pending user-cache evaluation is not replayed without a fresh OS-account root witness, and stale evaluator/catalog results cannot satisfy current targeted reuse. The revision-1 guide adds only fixed-order path-free counts and exact review navigation, with no byte forecast or cleanup authority. Contract v43 separately lets one exact retained Explorer file cross only as a consume-once path to the iCloud metadata callback; Rust owns its provider, kind, allocation, clock, and path-free classification. Contract v44 adds only a Rust-owned, allocation-ranked, 32-row path-free observation source for one retained snapshot directory subtree, with a 200,000-descendant hard failure budget. Contract v47 attaches one exact predecessor lease to an active Explorer review, exposes lossless matched historical union pages, separately accounted change treemaps, and both optional historical kinds, and drives the generation-fenced native Changes mode; the opaque child and presentation contain no live-target resolver or cleanup edge. Contract v48 separately reports a 64-row-plus-lookahead path-free census of unclaimed running-scan bookkeeping with no row selector, liveness probe, recovery, or effect edge. Contract v49 adds an independent 64-row-plus-lookahead claimed-row provenance census whose five aggregate categories distinguish stored-unproven claims from unavailable current comparison context without exposing a claim, selector, process fact, or recovery edge. Contract v53 adds the managed cache as the third additive path-free footprint component plus one consume-once exact two-minute cache-clear lease; no caller-supplied path, key, selector, AI value, or general delete capability crosses that boundary. The callback cannot nominate or return a target, and neither source, assessment, comparison, nor census can enter a candidate or effect graph. Swift validates the full targeted, emergency, snapshot-comparison, and both census records; Release exposes exact observations and the non-effect Rust-target dry check while permanent execution remains internal Debug UI | Provider-aware cache live witnesses, cloud candidate/effect authority, the remaining unsupported emergency evidence sources, and release-gated product cleanup remain later |
| Global permanent-cleanup FFI opt-in | UniFFI contract v22 introduced only the revisioned enabled/default-or-stored observation and typed get/set/reset failures; contract v34 makes rowless/reset state disabled and rejects `Default(true)`. Value-schema-v2 migration preserves schema-v1 explicit Stored consent but strengthens legacy enabled Default epochs. The gate remains deny-only and cannot carry a path, plan, approval, callback, or executor input. Swift EngineService/AppModel/Settings maps every typed error, rejects malformed shapes, generation-fences work, requires an authoritative loaded policy plus exact confirmation before enable, and lets disable/reset strengthen protection immediately. The Release execution action remains absent | The separate path-bearing exclusion boundary and remaining §17.3 Release gates |
| User cleanup exclusions | UniFFI contract v23 exposes a bounded lossless path-byte observation and replacement/reset operations with explicit source, revision, timestamp, and changed state. Rust validates absolute lexical prefixes, encoding, count, size, canonical order, storage races, and the shared cleanup exclusion lock; Swift treats returned bytes as display-only observations, allows adding a local prefix, and requires explicit confirmation before weakening protection by removing one or resetting all. No path is accepted as a plan, approval, callback, or executor input | Future planner/executor lifecycle and richer review presentation |
| Targeted reclaim roots | UniFFI v37 exposes the bounded lossless configured-project setting; v38 adds the path-free ordinal Warning/Critical runner; v39 prepends the fixed OS-account-derived current-user cache root, exact-excludes DUX's cache, removes overlap, and binds root kind/order/source/identity/availability/budget/exclusions to one echoed SHA-256 catalog and unchanged pressure checkpoint. Reuse binds the same facts, pressure episode, snapshot, and current evaluator/catalog. The combined pass is capped at 200,000 nodes, with at most 100,000 for caches and 10,000–50,000 per configured root. The separate user-cache scope emits exact direct-child Homebrew and pip observations under evaluator revision 4 with candidate-local age/coverage/symlink evidence, while mandatory blockers prevent planning or execution. Contract v40 finalizes a Critical-only fixed recovery order from exact current evaluations under a one-hour freshness window; unavailable roots remain typed permission/coverage counts and no supported group carries byte totals or effect authority. Settings starts no scan, and targeted evidence creates no plan, approval, AI request, access grant, provider command, or cleanup effect | Add authoritative evidence for the currently omitted cloud, Trash, installer/archive, and large-file lanes without widening scan or cleanup authority |
| Candidate review intent and detail | UniFFI v26, Swift, and Explorer expose four fixed scan-bound review commands plus bounded 64-item summary, path, and evidence pages. Rust revalidates complete source binding; FFI/Swift enforce path/payload/cursor/count/identity/generation bounds. Recommendations' exact-scan action now opens Candidates directly. Revision-bound friendly rule names retain raw rule/revision detail; page-local category/safety/action cards intentionally omit byte totals, and previous/next controls expose the full bounded result without turning a group into authority. Contract v33's exact Rust-target opaque review remains separately validated and generation-fenced; internal Debug alone exposes permanent confirmation while Release retains a locked unavailable action | Close §17.3 before enabling any permanent-safe cleanup effect in Release |
| Cleanup-history observation and clearing | UniFFI contracts v25/v29 expose bounded path-free newest-first summary pages and exact-session detail with lifecycle, policy, estimates, optional verified capacity delta, ordered item outcomes, and warnings. Contract v41 adds the independently loaded exact-session rule-outcome derivation; contract v42 adds the newest-32, first-12 deterministic recurring-rule ranking with explicit source/group truncation, exact observed bytes/duration ordering, and a same-revision manual-history threshold that grants no scheduling authority. Rust, FFI, and Swift independently validate each bounded graph and presentation shape; native outcome and ranking loads have separate cancellable generations and keep immutable history or an earlier valid observation visible on failure. History selectors and aggregates cannot enter a planner, scheduler, or executor. Contract v30 separately provides one Settings-only, engine-bound, consume-once preview over the exact terminal five-table graph. The two-minute authority is monotonic, accepts no selector/path/plan/AI/effect input, preserves active/recovering/uncertain evidence, recomputes its SHA-256 witness under cleanup exclusion and an immediate transaction, authorizes deletes only from the five history tables, and reconciles every unproven commit to outcome-unknown without retry. Swift requires exact count/range confirmation, fences concurrent history publication, and performs one read-only terminal refresh; the UI promises no file cleanup or freed space | Live partial-progress controls and planner/executor wiring remain separate |
| SQLite compatibility store | Checksummed v1–v17 migrations have exact per-version fingerprints. Schema v10 adds nonnegative policy revisions on raw/daily capacity history with v9 rows preserved at implicit-default revision 0; schema v13 adds a nullable domain-separated digest of each newly admitted scan root's code-owned filesystem identity plus a bounded exact-root/start index; schema v14 adds nullable fixed-size stable-host and boot-scope digests plus the bounded `resumable` cleanup recovery-policy value; schema v15 adds a partial running-scan/start index for the bounded, path-free unclaimed-record census; schema v16 adds an all-null-or-complete immutable stable-host/boot provenance tuple, the sole `interrupt_only` scan policy, and a global claimed-time keyset index; schema v17 adds a bounded immutable exact-token scan-scope registry with owner/provenance indexes and transitional blocking by every legacy running root. Migrated cleanup rows and v9–v15 scan claims retain all-`NULL`, explicitly unproven provenance tuples, and v16→v17 fabricates no scope lease. General, subtree, targeted, and progressive CLI scans bind one canonical root; exact/ancestor/descendant scopes are excluded across processes while siblings remain independent. Migrated scans remain explicitly identity-unknown and cannot satisfy later outcome comparability. Lossless bounded path codec, bounded full/lightweight inspection, private atomic provisioning with durable initialization evidence, cross-platform process writer/version-race coverage, durable writer-locked cleanup-lock layout upgrade, private tri-state process-instance liveness evidence plus immutable indexed running-scan process claims, bounded same-scope exact-CAS recovery, same-host/prior-boot history-only interruption, exact-token cross-process scan exclusion, newer-schema read-only transition, rollback/WAL recovery, engine-integrated scan lifecycle/coverage, atomic exact-snapshot candidate-evaluation batches, VM/time/decoded-memory-bounded exact-scan evaluation and exact-candidate observations, typed review state with semantic scan-bound commands, sealed evaluator invalidation state, claim-preserving atomic planner/journal candidate projection, planned-cleanup history, bounded path-free cleanup-history scalar/exact observations, exact expiry settlement, a bounded cleanup-lock-coupled owner-generation journal state machine, authorizer-constrained bounded capacity/AI-cache retention, exact-identity append-only snapshot tombstones, bounded explicit snapshot-review leases, the partial lossless snapshot-path retention lookup, one exact-key typed snapshot-cap setting with UniFFI v51/native get/set/reset presentation, the UniFFI v53/native path-free read-only database/snapshot/managed-cache footprint with embedded non-additive AI logical-content accounting, bounded immutable snapshot-temp leases with row-before-file creation and atomic success consumption, one-victim locked cap enforcement with exact tombstone reconciliation, bounded exact-path orphan classification plus exact guarded parent/root/status validation, one-row terminal temp-lease reconciliation, complete-lease-population physical-only unleased-temp reconciliation, current-schema-fenced root-local provisioning-stage reconciliation, bounded read-only rule-outcome/regrowth derivation, its newest-32/first-12 recurring-rule aggregation, and separate bounded read-only censuses of unclaimed durable `running` records and claimed-row provenance are implemented. Stored paths, root/provenance digests, status, policy, derived outcomes, recurring ranks, footprint classes, and census classifications remain non-authoritative observations. Native Cleanup History preserves exact item binding plus independent dynamic outcome/ranking lifetimes and does not use or populate legacy `rule_outcomes`. The v48 and v49 censuses and v53 footprint expose only bounded scalar aggregates and never perform recovery, liveness probing, user-file traversal, or deletion. Schema v17 changes only observation-work admission; its public CLI lease cannot create scan history or any effect authority. App/FFI owns Explorer review-lease lifetime and requests the sealed maintenance batches through a native idle scheduler | Planner engine lifecycle, Windows host-scope proof, native Windows temp/final/stage-removal verification, reset-app-data transport, executor integration, and claimed legacy-v8 running-row policy |
| Capacity sample persistence | Typed raw/daily history, policy revisions, and transaction-coupled Warning/Critical episodes are implemented with stable volume IDs, bounded retention, exact retry reconciliation, and anchored readers. FFI v36 exposes bounded pressure episodes; v39 binds the complete cache-first targeted catalog to the exact latest open Warning/Critical anchor and unchanged final checkpoint; v40 adds exact Critical-only recovery finalization without changing capacity authority. Native presentation shows signed changes, a 30-day graph, pressure ribbons, current-period context, timestamped targeted observations, and fixed-order review guidance; the scheduler samples at launch, five-minute cadence, wake, volume changes, and changed settings. Important-only or incomplete observations remain display-only. Current targeted cache findings are ordinary blocked scan evidence and do not change pressure authority. FFI v41 presents bounded exact-session post-cleanup outcomes, and v42 ranks only their observed estimated zero-to-nonzero growth without attributing a capacity delta to any rule | CLI capacity sampling wiring |
| Binary full-tree snapshot store | Independent v1 wire has bounded pre-allocation, exact graph/path/aggregate/flag semantics, frozen golden digests, SHA-256 references, private marker-owned storage, unique temps, atomic no-replace publication, read-only final handles, database→snapshot lock ordering, version-skew fencing, exact file-first scan-summary-plus-coverage reconciliation, schema-v5 tombstone-before-file load gating, schema-v6 sealed retained-handle review leases, schema-v7's bounded sequential-handle inventory with exact-root latest-two ranking, strict pin reconciliation, final/control revalidation, settings-backed cap input, and logical/allocated/charged accounting, plus schema-v8 row-before-file temp leases, retained kernel writer locks, active/quiescent/unleased reporting, exact same-scan residual retry, and atomic success consumption. A separate terminal-temp batch consumes at most one exact failed/cancelled/interrupted row-only or quiescent residual after guarded parent validation, physical-first identity/usage revalidation, checked accounting, handle-close-before-sync durability, and exact row reconciliation. Schema v16 may interrupt only the exact prior-boot scan history and deliberately preserves that lease and every physical byte for this terminal-temp boundary. A separate physical-only unleased-temp batch proves exact-name absence from the complete bounded lease population, skips active entries, and removes at most one lexicographic quiescent exact-grammar temp after fresh identity/usage/private/name/one-link/kernel-lock proof without SQLite mutation or adoption. A separate root-local provisioning-stage batch completely inventories the retained database root, proves exact marker-owned bounded children, and non-recursively removes at most one lexical stage with checked control-byte accounting and typed post-effect uncertainty. A sealed one-final cap batch repeats policy under the final locks, fully validates the observed body, commits the tombstone first, and performs identity-safe retained unlink plus directory durability or exact residual retry. A separate sealed one-final orphan batch proves zero references, fully validates the body-derived filename and exact guarded parent/root/status, performs checked accounting, and removes only the identity/usage-revalidated final with typed post-unlink uncertainty. The core engine can request exactly one scan-recovery, pending candidate-evaluation recovery, cap, orphan, terminal-temp, unleased-temp, or provisioning-stage batch without supplying authority-bearing inputs. App/FFI owns exact Explorer review leases and fair periodic requests for every sealed batch; durable engine tasks publish only a completed-only fresh-scan converter's lossless canonical DFS nodes and fail-closed hard-link accounting, while standalone inventory observations remain non-authoritative. Generated balanced/wide 1M and balanced 5M Release fixtures now measure real publication/review memory and latency; measured 999,999-child paging/treemap is admitted with a compact sort cache and post-work lease revalidation, while 5M review is refused before decode by the fixed 1 GiB budget after a measured roughly 2.01 GB publication peak | Legacy-v8 running-row policy, a future streaming/indexed representation for interactive 5M review, and native Windows temp/final/stage-removal plus sparse/compressed-allocation verification |
| Explicit older-snapshot clearing | UniFFI v54 exposes one path-free, engine-bound, consume-once two-minute preview over the exact retention-eligible available finals and already-tombstoned physical residuals. The private witness includes every final, latest-two rank, active pin count, orphan, temporary, residual lease, usage fact, and control aggregate; no identity or selector crosses FFI. Consumption repeats current-schema database-before-snapshot locking, complete body validation, one atomic tombstone batch, and exact retained-handle removal. Latest-two snapshots per exact root and active reviews stay protected; maintenance objects, history, cache, AI, settings, legacy data, and user files are unreachable. Pre-effect drift is rejected, possible post-effect ambiguity is outcome unknown, and native remeasures once without retry | Native Windows final-removal evidence remains required before claiming Windows support; reset-app-data remains a separate lifecycle boundary |
| App-data reset lifecycle | A dormant Unix/macOS reset coordinator provides the external durable-journal foundation. Its fixed sibling is independently marker-owned, atomically no-replace-published from a complete random private stage, component-wise no-follow opened, exact-owner/mode/link/ACL/identity validated, permanently writer-locked, and bounded-inventory checked. Its canonical 4 KiB maximum v1 record domain-separates SHA-256 over one exact random transaction, data/cache identities, derived detached-stage names, and six strict forward-only phases. Begin and advance are exact locked comparisons; atomic replace, directory sync, and read-back preserve typed outcome uncertainty. Marker-complete abandoned provisioning stages are removed only under their retained stage lock; partial or hostile stages remain untouched. Core terminal arbitration now atomically records ordinary-close versus reset intent under the task-registry mutex, cancels queued/running work for the sole reset winner, and yields one move-only quiescence proof only after lifecycle `Closed` and joined workers; timeout consumes authority and leaves the old engine terminal. Neither checkpoint owns a reset target or has an FFI, CLI, or Swift caller, so neither can detach or delete application or user data. The accepted contract is `docs/APP_DATA_RESET.md` | Retained coordinator session, cleanup/scan-journal blockers, exact namespace witnesses and detach, pre-open roll-forward recovery, bounded physical stage drain, path-free FFI, native confirmation/relaunch/preference allowlist/accessibility, macOS release evidence, and all Windows handle/DACL/reparse evidence |
| Typed scan coverage/issues | Implemented as bounded semantic domain values, authoritative scanner terminal outcomes, engine task results/events, atomic SQLite-v2 summary children, truthful fresh/legacy-cache CLI labels, changed-hard-link observations, and a path-free aggregate FFI/Swift summary; observations grant no plan or cleanup authority | Paged Explorer issue details and permission onboarding |
| Cache semantic/input validation | Legacy v7 remains CRC/version-only and has no production caller. Managed v1 is a separate SHA-256-bound format with fixed-header preflight, 64 MiB file, 200,000-node, depth/name/path/metadata and 192 MiB modeled-residency caps, exact root/config binding, complete graph/path/aggregate validation, and no legacy fallback | Keep the legacy decoder isolated for compatibility tests; add equivalent native Windows storage evidence before enabling managed persistence there |
| Managed scan-cache ownership and clearing | The fixed `Dux/scan-cache-v1` child has independent marker/lock controls, descriptor-relative no-follow access, exact current-user 0700/0600/one-link/ACL validation, bounded inventory, retained writer lock, atomic publication, and typed uncertain outcomes. UniFFI v53 reports it as the third additive DUX-owned physical component and exposes only a two-minute, engine-bound, consume-once exact clear preview. The final unchanged inventory can remove entries/recognized temporaries only; controls, outer/legacy siblings, AI, database/history, snapshots, settings, and user files are unreachable. Swift remeasures changed/unknown outcomes and never retries deletion | Native Windows handle/DACL/reparse/allocation evidence; the reset-data boundary remains distinct |
| Hard-link accounting and policy | Fresh completed scans deterministically count allocation once per stable identity and fail conflicts/unknown identity closed; no cleanup policy or authority derives from it | Native Windows sparse/compressed verification plus explicit planner/executor per-mode admission and live revalidation rules |
| Forbidden destructive-call lint | Implemented with compiler-resolved Rust denial, cross-language repository scan, scoped annotations, self-tests, and CI; the legacy CLI exception set is removed and reintroduction is rejected | Keep the remaining exception set exact |
| Durable operation journal/history | Schema, typed immutable `planned` insert/load, permanent cleanup OS lock, tri-state process evidence, a private cleanup-lock-coupled owner/generation state machine, bounded path-free recent-session plus exact-session/item observations, and explicit terminal-metadata clearing are implemented. Clearing preserves active/recovering authority and distinguishes proven applied/not-applied from outcome-unknown. The journal covers validation, durable effect intent, outcomes, cancellation, terminal derivation, same-scope death recovery, and explicit unknown reconciliation. Schema v14 stores complete stable-host/boot provenance only on new claims; migrated, partial, malformed, prior-boot, foreign-host, and Windows-unproven observations remain typed non-executable no-ops. No prior-boot reconciliation handle exists. Generation-one claim ambiguity now retains the exact approved capability/lease and retries only the same claim; post-claim comparison ambiguity retains the live session. Panics inside permanent-safe or Trash one-shot callbacks record `outcome_unknown`, and ambiguous settlement retains its exact receipt/session so persistence-only retry cannot repeat the effect. Continued ambiguity holds the physical store's cleanup lock for the process lifetime; same-process reopen remains denied and process restart hands authority to durable recovery. The rule-specific schema-v12 active-claim seal still preserves the sole trusted `ProtectedPath` fact and ordinary blocked candidates fail closed. History cannot become a planner witness | Non-resumable prior-boot diagnostics/reconciliation, legacy-v8 running-row policy, native Windows proof, native permanent-safe observation/confirmation, and app release gates |
| Private 0700/0600 stores | SQLite and application snapshot roots/controls/data enforce ownership, no-follow identity, links, and exact Unix modes; macOS rejects final-object ACLs but accepts deny-only publication-parent ACLs; Windows uses exact protected DACLs, handle-bound publication, retained identity, rename guards, and handle-derived logical/allocation reporting. Unix managed scan-cache controls/data now enforce the equivalent private boundary inside the fixed marker-owned child. The bounded v53 footprint counts only proven database, snapshot, and managed-cache objects, excludes directory metadata, outer cache siblings, and unattributable stages, and leaves embedded AI logical bytes non-additive; legacy cache files remain non-private and excluded | Extend equivalent guarantees to Windows managed cache, logs, provider temp data, and bounded abandoned-stage/temp maintenance |
| Trash executor | Explorer explicit-selection executor is implemented through the core journal fence, UniFFI v21 callback, and macOS adapter; contract v24 preserves `ChangedSincePlan` as a distinct path-free rejection. Known platform outcomes are timestamped only after the callback returns and terminalized with explicitly unknown capacity delta, so Trash never claims immediate free space. Generation-one claim ambiguity retains and retries only the exact lease before the callback moves; known post-claim refusal terminalizes before releasing its owner, while ambiguous admission retains the exact claim/receipt. Callback panic becomes durable `outcome_unknown`; ambiguous cancellation/effect/terminal settlement retains the same capability in process-lifetime quarantine, and persistence-only retry cannot invoke Trash again. Permanent delete, AI, CLI, and scheduling remain excluded | Add trustworthy post-effect capacity reconciliation and Linux/Windows adapters before expanding authority |
| Cloud eviction | Contract v43 implements a read-only selected-file iCloud Drive metadata probe. V44 adds a Rust-owned, path-free, allocation-ranked source of at most 32 complete files from one retained snapshot directory subtree and an explicit single-flight serial **iCloud Status** review with stop-after-current semantics. V45 brackets two complete Foundation samples with account and file-version observations, reports account/item-generation/file-version stability independently from sync eligibility, and includes shared/sync-paused facts. Stable container identity for an arbitrary user-selected iCloud Drive item is unsupported on the current public Foundation surface, so production identity readiness remains false. Rust revalidates each retained regular single-link target and owns provider/kind/allocation/time; Swift consumes one exact path per manual check and returns bounded facts; Rust emits a path-free fail-closed assessment. Results are non-atomic, memory-only discovery and are not summed or persisted. No rule, candidate, emergency group, plan, approval, journal/history row, provider command, cleanup button, or effect exists | Prove a supported stable container witness under the isolated real-device protocol; only then design separately versioned durable evidence, purpose-built candidate admission, final live proof, a journal-fenced no-retry supported API executor, and destructive disposable-account race verification |
| Scheduled cleanup | Absent. Contract v42 exposes only a same-revision repeated-manual-history threshold; it cannot create, enable, or execute a schedule and does not satisfy current-candidate eligibility | Schedule model, explicit user controls, fresh re-planning/revalidation, and every §15/Milestone 8 automation gate |
| Notification authorization | Settings reads authoritative macOS status and can explicitly request Alert/Sound permission from Not Determined. Native delivery is gated by a newly stored Warning/Critical transition, keeps independent 24-hour per-volume/per-urgency cooldowns only after accepted delivery, carries a bounded path-free Recommendations payload, and validates that payload again before deep-linking to the review-only Explorer surface. No notification can nominate or execute cleanup | Add targeted pressure-triggered scan results and emergency recovery ordering without widening notification authority |
| AI providers | Disabled/absent | Adversarial authority spike; remains explanation-only |
| Signed/notarized macOS release | Fail-closed local app/DMG workflow, reviewed empty entitlements, explicit signing order, notarization-log/staple/Gatekeeper checks, immutable output, and checksums are implemented; no public artifact or frozen production identity exists | Freeze identity and perform real Developer ID/notary validation, then add protected release CI in Milestone 9 |

This matrix is intentionally conservative. “Implemented” means the named layer
exists, not that cleanup is safe to expose.

## 20. Design provenance

This design is grounded in DUX's own typed Rust architecture, adversarial
corpus, platform decisions, and independently researched rule requirements.
Mole's published security design informed the value of an explicit threat
model, centralized deletion wrappers, forbidden-call CI, protected-category
maintenance, native Trash routing, dry run, and fuzz regressions. DUX does not
copy Mole source, shell policy, protection lists, corpora, wording, or its
assumption that arbitrary user paths are outside protection. DUX is MIT and
uses clean-room behavioral inspiration only.

Related documents:

- [Product and implementation roadmap](ROADMAP.md)
- [ADR 0002: Direct Developer ID distribution](docs/adr/0002-direct-developer-id-distribution.md)
- [ADR 0003: Primary build without App Sandbox](docs/adr/0003-primary-build-without-app-sandbox.md)
- [ADR 0004: Shared Rust engine](docs/adr/0004-shared-rust-engine.md)
- [ADR 0005: UniFFI for the Swift/Rust transport](docs/adr/0005-uniffi-swift-rust-transport.md)
- [ADR 0007: Prior-boot running-scan history interruption](docs/adr/0007-prior-boot-running-scan-interruption.md)
- [Mole security design (behavioral research only)](https://github.com/tw93/Mole/blob/main/docs/SECURITY_DESIGN.md)
