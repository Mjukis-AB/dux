# DUX Security Design

Status: normative design and implementation gate

Last reviewed: 2026-07-15

Applies to: `dux-core`, `dux-cli`, `dux-ffi`, and the direct-download macOS app

## 1. Purpose and reading rules

DUX scans storage and will eventually move, remove, or evict filesystem data.
That combination makes filesystem identity, user intent, privacy, and truthful
reporting product correctness requirements rather than optional hardening.

This document defines the security boundary for those capabilities. It is the
normative companion to the product roadmap and the architecture decisions in
`docs/adr/`. When implementation and this document disagree, new/public macOS
cleanup remains disabled until the code and document are reconciled in the same
reviewed change. The active legacy CLI exception is documented in §5 and is
migration debt, not compliance with the target authority graph.

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
- the hardened legacy CLI deletion path from the future shared executor.

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

For future shared cleanup, user approval of an unintended item does not bypass
protected roots, sensitive categories, action compatibility, freshness, or
explicit mode. The current legacy CLI does not enforce those policy layers. An
advanced confirmation is never a universal policy bypass.

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

As of the review date, the macOS application is read-only. It samples startup
volume capacity and proves the Swift/Rust boundary; it has no scan, candidate,
plan, cleanup, AI-provider, history/settings, or scheduling API. The shared
core engine opens its private SQLite compatibility store internally before
workers start, but that store has no app-facing CRUD surface.

`dux-core` has typed rule, candidate, cleanup-plan, lexical validation,
filesystem evidence, protected-root policy, and dangerous-path test models.
Their constructors and exports are intentionally restricted. They do not form a
complete authority chain because trusted home/volume discovery, stable rule
scope grants, plan-time source witnesses, full platform ancestry guarantees,
and executor-time mutation revalidation do not yet exist.

The existing CLI still offers permanent deletion, but its filesystem effect is
now centralized in the temporary core-owned
`dux-core/src/cleanup/legacy_cli.rs` adapter. The CLI can only prepare an opaque,
target-bound plan before confirmation and consume that exact plan through the
adapter; it no longer owns or directly invokes recursive deletion. The adapter
captures target, marker, and ancestor identity before confirmation and rechecks
those facts immediately before calling `remove_file` or `remove_dir_all`. The
CLI still waits for tracked deletion workers on graceful quit. This is useful
containment, but it remains legacy behavior because:

- the temporary adapter is not the production centralized executor or an
  implementation of the reviewed authority graph;
- it is not constructed from the new immutable cleanup-plan model;
- it has no durable operation journal or pre/post capacity verification;
- its byte accounting is derived from scan estimates rather than measured
  post-operation capacity (the CLI now labels that distinction explicitly);
- its final removal is path-based and retains a TOCTOU window;
- unchanged directory identity does not freeze descendants; and
- cache-rebuilt paths can be lossy and are not valid cleanup identity.

The legacy identity snapshots do not record target size, mtime, content digest,
or planned kind, and marker snapshots record identity rather than contents.
In-place edits to a file or marker can therefore retain identity and still pass
the legacy recheck.

It also does not apply the new protected-root registry, hard-link policy, plan
expiry, or authoritative mount-location grant. A broad scan of `/`, `/Users`,
or a home directory can expose critical descendants to the permanent `d`, then
`y` flow. A final symlink is removed as the link on Unix. These are unresolved
legacy risks, not accepted exceptions.

This checkpoint closes one immediate forged-cache escape in that legacy path:
admission now requires an absolute strict descendant whose complete relative
path contains only normal, valid-text, control-free components, and it rejects
a target, ancestor, or marker on a different filesystem from the scan root.
Previously a forged terminal `.` or `..` cache name could alias the scan root or
its parent because only the target parent was checked. The cache format itself
still lacks semantic tree validation and authenticity.

The CLI MUST keep clearly labeling this action as permanent until migration.
The macOS app MUST NOT call, wrap, or expose the legacy path.

Current scans also lack typed coverage records. They skip access failures and
some slow or unsuitable filesystems, and CLI flags can opt into symlink or
cross-filesystem traversal. Unknown filesystem classification preserves scan
availability rather than proving locality. Current scan totals therefore MUST
NOT be described as complete, and relaxed scan flags MUST NOT imply cleanup
authority.

Current binary scan caches are atomic and checksummed, but checksums detect
corruption rather than malicious tampering and the current cache writer does
not enforce the future 0700-directory/0600-file permissions. Cached paths are
therefore sensitive, non-authoritative data. The SQLite store now has private
permissions and migration tests; binary snapshot/cache storage does not, so
public app cleanup remains blocked.

The current loader reads the complete cache before structural validation, has
no retention cap, and accepts same-user replacement as ordinary input. A forged
cache cannot itself mint target identity, but it can affect displayed names,
estimates, and selection and can consume memory. Future stores MUST bound input
before allocation and treat all persisted bytes as untrusted.

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
  existing Rust artifacts.

The source tree configures Hardened Runtime for the app spike, but no public
Developer ID-signed/notarized artifact exists yet. The explicit DUX FFI version
is displayed rather than enforced by the Swift service today; this must become
a startup rejection before real engine APIs ship. SQLite schema migration and
store coordination are implemented, but settings/history CRUD is not. AI,
Trash, eviction, scheduling, notifications, launch-at-login, and typed TCC
coverage remain unimplemented.

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
recovery witness, plan capability, or effect capability—and is not yet coupled
to cleanup journals or an executor.

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

On Windows, the current validator reopens cumulative full paths and is
non-actionable until handle-relative or equivalent executor revalidation closes
ancestor reparse races. On Linux, equal device identity does not distinguish a
bind mount; authoritative mount-location evidence remains required. On macOS,
firmlink and APFS volume/location behavior requires explicit integration tests.
No current rule or plan may authorize a volume crossing. Selecting an external
volume as its own validated scan root is a distinct operation, not an exception
to this rule.

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

Future scan accounting MUST deduplicate hard-linked allocation within its
declared scope, and planning MUST record link counts and overlap decisions. A
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

### 9.2 Trash

Arbitrary Explorer cleanup defaults to Trash. On macOS, the platform executor
uses `FileManager.trashItem(at:)`; it MUST NOT manually move data into
`~/.Trash`. A symlink selection trashes the link itself, never its target, and
the review UI says so.

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

Arbitrary-path advanced permanent removal is excluded from the first production
authority graph. Adding it later requires a separate threat model and revision
of this design; it remains subject to protected roots, sensitive categories,
path identity, exact review, and global disable and cannot become safe,
schedule-eligible, or a generic force flag.

### 9.4 Cloud-local-copy eviction

Eviction is neither deletion nor Trash. It is allowed only through a supported
provider/platform API after current evidence proves the item is fully uploaded
with no local-only changes. UI states that the item remains in cloud storage
and requires network access to download again. Provider-managed files are never
freed by direct filesystem deletion.

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
and one coordinated writer model. Binary snapshots use unique private temporary
files, checksum/version validation, durable flush where required, and atomic
replacement. CRC checks detect accidental corruption, not authenticity or
confidentiality.

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
accepts only its database, ownership marker, initialization sentinel, cleanup
lock and ready checkpoint, known SQLite sidecars, and the exact reserved
`snapshots`, `ai`, and `logs` siblings in the final DUX directory; the later
owners of those sibling stores must perform their own no-follow identity and
permission validation.

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
session successful, failed, or interrupted. Future recovery updates must claim
a new generation transactionally and compare-and-set every journal write
against owner plus generation. All persisted facts remain historical
observations: they are not canonical path witnesses, current evidence,
approval, or executor capabilities, and they do not add an AI/history-to-plan
authority edge.

The implemented typed candidate-history API prepares all bounded data before
locking, atomically inserts a format-2 parent and contiguous ordered children,
and requires the referenced scan to exist. Its exact-ID reader shares the
bounded SQLite progress budget across parent and child queries, checks storage
types and byte lengths before materialization, and returns either a dedicated
complete observation or an explicit legacy summary. It rejects legacy child
pollution, missing/gapped/over-limit children, dangling scans, malformed
evidence, incompatible policy, and incomplete cloud-upload facts. It cannot
construct a domain `Candidate`, change status, create a plan, or execute an
effect.

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

The storage layer also implements the permanent store-wide cleanup exclusion
primitive required before execution-state journaling. Its immutable lock and
ready control are exact root entries, provisioned for legacy owned stores only
while the writer lock is held and flushed before the root-ownership marker's
durable layout-v2 transition. Both are retained with private no-follow identity
evidence and never recreated after that transition.
The cleanup and writer locks remain independent so a future holder can perform
short fenced journal transactions in the required cleanup-before-writer order.
Same-process and subprocess tests prove bounded contention and release; native
Windows handles deny delete sharing to prevent path replacement. No history,
FFI, CLI, Swift, AI, plan, or effect API can obtain this guard yet.

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

Symlinked storage roots, ownership mismatch, unsupported schema versions, and
unsafe permissions block writes. Older clients fail read-only rather than
downgrade or corrupt shared state. The internal SQLite coordinator provides a
stable cross-process lease whenever an `EngineHandle` opens the store. App/FFI
and CLI adoption plus coordinated binary-snapshot publication remain future
integration work.

### 12.3 Retention and deletion

Retention is explicit per store. Hourly disk samples are retained for 30 days
and daily rollups for one year. Full-tree snapshots retain the latest two
complete snapshots per root plus any snapshot referenced by an active cleanup
review. The snapshot directory has a default 2 GiB total cap and evicts the
oldest unreferenced snapshots first. AI insights default to 30 days, are
user-clearable, and are invalidated when the redacted-input digest changes.
Operation history is retained until the user explicitly clears it so interrupted
and failed cleanup remains explainable.

The SQLite schema records raw and daily-rollup capacity samples as distinct,
constrained kinds so retention cannot infer a sample's lifetime from timestamp
shape. Scan coverage likewise records a constrained status separately from its
optional quantitative estimate: unknown coverage remains `NULL`, never a
misleading zero, while complete coverage is exactly 1000 permille.

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
coverage. It MUST offer System Settings guidance only after explaining a
concrete gap and MUST re-probe when the app becomes active. It MUST never ask
for administrator credentials, disable platform protections, or install a privileged helper to
gain coverage.

The public app MUST use Hardened Runtime, library validation, the smallest
reviewed entitlement set, and signed bundled code. `SMAppService.mainApp` MUST
remain opt-in launch at login, not a privilege boundary. The separately launched CLI has its own
process and TCC context; the app MUST NOT imply that GUI access transfers to it
or use the CLI as an access bypass.

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
architecture owns one engine session. Startup validates and migrates one
private SQLite store before workers are published, but exposes no raw SQL or
domain persistence. Its only production task is a read-only formatting batch:
it has no scan, AI, plan, or cleanup authority. Cancellation intent is recorded
separately from the operation-reported outcome so a late request cannot falsely
claim completed effects were rolled back, and engine `Closed` means every
worker has quiesced.
The UniFFI `DuxEngine` remains a smoke-only lifecycle object; task IDs, event
pages, results, and cancellation do not yet cross FFI.

Every expected error is typed. Rust panics are defects and MUST NOT become UI
text or unwind through Swift. Callback/event tests cover retention, completion,
cancellation, reentrancy, stale generations, and cycle avoidance before real
engine work crosses the boundary.

The future CLI JSON surface is inspection-only until shared cleanup migration.
Every object MUST have a schema version, errors MUST use stderr and nonzero
status, and golden tests prevent accidental contract drift. No JSON command accepts an executable
AI-produced plan or arbitrary permanent-cleanup path.

### 14.1 Optional CLI installation

The app may install the bundled universal CLI only through a dedicated service.
The default destination is `~/.local/bin/dux`. Installation validates the
bundled binary identity and architecture, inspects any existing destination
without following an unsafe symlink, and displays its version and source before
replacement. It writes a sibling private temporary file, verifies it, and
atomically renames it.

The installer MUST NOT overwrite a non-DUX binary, escalate privileges, write
to a system directory, modify shell startup files silently, or imply that the
CLI inherits the app's TCC access. Uninstall removes only a matching
DUX-installed binary and is separate from app uninstall and data clearing.
Standalone Homebrew/crates.io releases remain supported and schema-version
skew follows the read-only/fail-cleanly rules above.

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

Automatic updates are not yet selected. Any updater requires its own reviewed
design covering signature keys, appcast or metadata authenticity, key custody,
rollback, staged rollout, downgrade behavior, atomic replacement, and failure
recovery. DUX MUST NOT download and execute an unsigned replacement.

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
scanning. Current product mutation remains temporarily restricted to the three
identity-checked calls in `legacy_cli::execute_plan`, registered as the
`legacy-adapter-delete-*` exceptions. A repository architecture check rejects
references or re-exports outside the adapter and CLI state orchestration, so
FFI and the macOS app cannot adopt this legacy route. The public Rust surface is
temporary and unsupported rather than a sealed authority boundary; this
baseline is removed when the production centralized executor replaces the
adapter. Internal cache exceptions own only `create_new`
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
- pre/post capacity reporting that never substitutes estimated sizes;
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

## 19. Implementation checkpoint matrix

| Control | Current state | Gate before app cleanup |
|---|---|---|
| Strict lexical/live path evidence | Implemented, crate-private and non-authoritative | Bind trusted scan/volume/rule witnesses and executor revalidation |
| Protected-root registry | Implemented text-only policy; production construction sealed | Trusted OS home/profile/mount discovery and stable rule grants |
| Dangerous-path corpus and fuzzing | Implemented | Keep cross-platform and promote every crash regression |
| Rule schema/loader | Implemented for private synthetic fixtures | Signed bundled source, independently researched rules, provenance tests |
| Candidate and cleanup-plan records | Implemented as non-executable domain data | Connect only through deterministic evaluator and planner-owned witnesses |
| macOS app cleanup | Absent | Entire cleanup release gate in §17.3 |
| Legacy CLI deletion | Active arbitrary-descendant permanent path routed through a temporary core adapter; strict-target/volume/identity rechecks only; scanned-byte estimates labeled in CLI | Replace adapter with reviewed plan/approval/executor chain without weakening current checks |
| Centralized executor | Production executor absent; temporary legacy adapter is containment only | Typed admission, cross-process lease, live revalidation, and journal required |
| Engine/FFI task and plan API | Core handle, pre-worker SQLite compatibility handshake, and bounded per-session registry implemented for one read-only formatting batch; app architecture owns one session; UniFFI handle remains smoke-only, with no scan/task/plan DTOs or cleanup authority | FFI version rejection plus bounded scan/task/plan handles and cancellation |
| SQLite compatibility store | Checksummed v1/v2 migrations with exact per-version fingerprints, lossless bounded path codec, bounded full/lightweight inspection, private atomic provisioning with durable initialization evidence, cross-platform process writer/version-race coverage, durable writer-locked cleanup-lock layout upgrade, newer-schema read-only transition, rollback/WAL recovery, crate-private typed scan and candidate history, explicit legacy summaries, and atomic bounded planned-cleanup insert/exact-ID load are implemented; stored paths and policy remain non-authoritative observations | Scan/evaluator/planner lifecycle integration, candidate status lifecycle, cleanup execution-state CRUD with ordinal and owner-generation validation, retention, reconciliation, and bounded identity-safe abandoned-stage maintenance |
| Typed scan coverage/issues | Absent; current scanner counts/skips and permits relaxed flags | Required before any scan is described as complete or becomes plan input |
| Cache semantic/input validation | Atomic write plus CRC/version only; full-file read before bounds | Bounded reads, tree/path semantics, private permissions, retention, and migration |
| Hard-link accounting and policy | Absent; only non-authoritative path snapshots capture link count | Deduplicated scan accounting and explicit per-mode admission rules |
| Forbidden destructive-call lint | Implemented with compiler-resolved Rust denial, cross-language repository scan, scoped annotations, self-tests, and CI | Keep exception set exact; remove legacy baseline during executor migration |
| Durable operation journal/history | Schema plus typed immutable `planned` insert/load and a separate permanent store-wide cleanup OS lock are implemented; the lock has no history/plan/effect authority and there is no execution owner, transition, lease coupling, or reconciliation | Process-instance liveness, fenced state machine, crash reconciliation, and executor integration required before shared executor ships |
| Private 0700/0600 stores | SQLite stage/final root, database, marker, and sidecars enforce ownership, no-follow identity, links, and Unix modes; macOS rejects final-object ACLs but accepts deny-only publication-parent ACLs; Windows uses exact protected DACLs plus handle-bound publication and a retained final-root rename guard; current binary cache remains non-private | Extend equivalent ownership and atomic-publication guarantees to snapshots, caches, logs, provider temp data, and bounded abandoned-stage maintenance |
| Trash executor | Absent | Platform-native implementation and integration tests |
| Cloud eviction | Absent | Supported API plus fully-uploaded/no-local-change evidence |
| Scheduled cleanup | Absent | Manual-history maturity and all automation gates in §10 |
| AI providers | Disabled/absent | Adversarial authority spike; remains explanation-only |
| Signed/notarized macOS release | Design and build spike only | Full Developer ID/notarization/release validation |

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
- [Mole security design (behavioral research only)](https://github.com/tw93/Mole/blob/main/docs/SECURITY_DESIGN.md)
