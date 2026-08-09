# Changelog

- Accepted diagnostic-only durable non-executability as DUX v1's cleanup
  crash-debt policy. Prior-boot, foreign-host, migrated, malformed, and
  otherwise unproven active cleanup rows remain unchanged and cannot be
  selected, reconciled, resumed, terminalized, cleared, or consumed by AI,
  pressure handling, schedules, notifications, the CLI, or later cleanup.
  ADR 0011 records that retained debt does not block a new independently
  reviewed cleanup session. This adds no production mutation and does not
  enable permanent cleanup in public Release builds.
- Added a strictly read-only, path-free diagnostic for unfinished cleanup
  bookkeeping. One bounded observation independently partitions active journal
  rows by phase and stored host/boot provenance without probing processes,
  acquiring a recovery claim, traversing files, estimating reclaimable space,
  or changing durable state. Settings presents separate accessible charts,
  lazy generation-fenced refresh, retained earlier results after refresh
  failure, and explicit non-authority copy. ADR 0010 defines the observation
  boundary and ADR 0011 selects diagnostic-only durable non-executability for
  v1; public Release cleanup stays disabled.
- Hardened the internal native permanent-safe cleanup experience without
  enabling it in public Release builds. Startup now loads the durable global
  opt-in before presenting an effect action; unknown, failed, changing, or
  impossible default-enabled state fails closed, and confirmation rechecks the
  loaded stored consent before consuming the opaque review. A path-free task
  card now remains visible above every Explorer destination, preserves
  cancelled terminal aggregates and exact history correlation, waits visibly
  for the runtime-owned history refresh before allowing dismissal, and opens
  either the exact freshly verified Cleanup History session or the read-only
  history list. Concurrent exact-session routes are last-writer-wins and never
  select from a stale row after refresh failure. Release still omits the
  permanent removal action behind `DUX_INTERNAL_PERMANENT_SAFE_CLEANUP`.
- Promoted the real-process macOS Rust-target cleanup harness into a mandatory,
  bounded CI qualification lane for the production Rust API layer of
  `dux-ffi`. The harness obtains the exact test executable from colorless Cargo
  output and then runs only the two reviewed ignored fixtures directly, after
  the Cargo process has exited. Disposable fixtures prove the
  opaque review is engine-bound and consume-once, the path-free task result
  correlates with exact-session history, only stale target contents are
  removed, Cargo markers/manifests/locks/source survive, a busy refusal cannot
  restore authority, and engine close drains admitted work. Public Release
  cleanup remains disabled pending the remaining security release gates.
- Rejected direct local AI command adapters after an adversarial macOS
  confinement spike. A non-shipping hostile child given a minimal environment,
  empty working directory, standard descriptors, and no canary contents could
  still read a known out-of-scope 0600 same-user sentinel. That ordinary
  ambient authority fails the shipping gate before any favorable Full Disk
  Access/TCC assumption can matter. A deprecated `sandbox-exec` comparison
  denied the same read but is explicitly ineligible for production. ADR 0009,
  a reproducible no-personal-data protocol, frozen path-free evidence, and
  regression tests close the conditional Claude/Codex direct adapters without
  adding any provider, process, FFI, engine, Swift, CLI, network, or cleanup
  edge. Disabled/no-provider remains the only state until a separately accepted
  remote, sandboxed, or virtualized transport exists.
- Added the dormant core-owned AI privacy shaper without enabling a provider.
  It accepts only a validated immutable snapshot observation with complete scan
  coverage, inspects at most 200,000 nodes for one selected directory, rejects
  incomplete or unrepresentable observations, and applies an independent
  revision-1 sensitive-category deny policy before constructing any wire
  metadata. Source paths and names are replaced with code-owned generic labels
  and request-local IDs; credentials, keychains, browser profiles, private
  communications, password managers, security/management state, VM/container
  disks, all recognized cloud-document state, and protected system roots are
  excluded; exact per-user Library/AppData roots also fail closed. A sensitive
  descendant removes its whole direct-child aggregate,
  so filtered bytes never survive in root or omission totals. Eligible totals
  and leaf-based age buckets are recomputed exactly before the frozen typed
  digest and JSON encoding. Only the shaper can construct the non-cloneable
  path-free proof, which remains private and has no engine, FFI, Swift, CLI,
  provider, process, network, cache, plan, or cleanup consumer.
- Added DUX's provider-neutral AI explanation v1 contract without enabling AI.
  Draft 2020-12 input/output schemas and a crate-private Rust validator enforce
  256-KiB/64-KiB byte ceilings, one bounded direct-child level, request-local
  node IDs, exact 53-bit integer accounting, exact logical/age omissions,
  false-only protected/content flags, a frozen formatting-independent typed
  metadata digest, deny-unknown fields, conservative path-shaped input
  rejection, action-filtered inert output text, and atomic known/disjoint
  output references.
  The intentionally narrow v1 omits allocated observations until they can be
  reconciled without contradictory subordinate facts. The module
  imports no DUX authority layer and has no engine, persistence, FFI, Swift,
  CLI, provider, subprocess, network, or cleanup surface. Shape validation is
  explicitly not privacy authorization; redaction and sensitive-category
  exclusion remain the next gate.
- Sealed the complete security-sensitive macOS release script behind a
  reviewed SHA-256 policy checkpoint, while retaining semantic release tests
  as readable invariants. Git attributes force the sealed script to LF on
  every platform so Windows checkout conversion cannot invalidate or weaken
  the byte-exact review boundary.
- Hardened the dormant Sparkle and macOS distribution boundary. Sparkle is
  upgraded and exactly pinned to 2.9.5 security fixes; the updater now requires
  the exact production bundle ID, dedicated DUX public key, HTTPS feed, signed
  feed, pre-extraction verification, and non-expiring signature failures. The
  feed remains absent. A separate manual `macos-release-signing` workflow
  validates an exact stable tag on the default-branch line, uses pinned
  Rust/Xcode/XcodeGen, completes every build/test on an unprivileged runner,
  repeats the clean tag gate immediately before sealing the unsigned app, and
  transfers only that three-file envelope with one-day retention to a fresh
  protected runner. Full-SHA Node 24 artifact actions must pass their service
  digest, while a deterministic whole-envelope digest crosses the separate job
  output and is independently recomputed. That runner verifies the envelope
  before credentials exist and restricts its ephemeral
  Keychain interval to hash verification, signing, and notarization. It
  attempts Keychain deletion before exact seven-file verification and uploads
  no signed DMG or notarization log. It neither publishes nor receives the
  Sparkle private key and the standalone CLI lane remains independent. The
  custody/recovery runbook pins the exact
  Sparkle tool archive, hardens dual-media export, adds a public-key-only canary
  verifier, and records the unresolved stock-Sparkle rotation trust-model gate;
  no key export, backup, CI import, retained notarized release, appcast, or
  public signed-release artifact is claimed by this checkpoint.
- Added a bounded, read-only diagnostic for possible pre-correction snapshot
  provisioning remnants. UniFFI contract v55 reports only the number of raw
  direct-child names inspected, the number matching the exact legacy stage
  grammar, and whether the bounded inspection completed. DUX does not open
  those children, attribute them to a database, measure their bytes, include
  them in owned-storage totals, or expose a selector or removal action.
  **Storage & Privacy** presents the observation in a separate dashed,
  question-mark callout with explicit partial-result and VoiceOver wording;
  the database/snapshot/cache chart remains unchanged.
- Finished the private Unix/macOS app-data-reset recovery lifecycle through
  exact fresh-origin retirement, durable V2 `Complete`, and physical-state-
  gated ordinary engine admission. Incomplete recovery still never opens
  SQLite. A completed open retains its shared coordinator lease, opens only the
  existing journal-bound fresh root, refuses missing/replaced roots, exact
  transaction stages, and any sibling random provisioning stage, and repeats
  its state-appropriate narrow or evolved inventory under the writer lock
  immediately before the first sidecar or SQLite effect for both new and reused
  same-process coordinators. Cache and journal validation repeat at that same
  gate, and the final cache publication fence remains retained through the
  first store effect. Fence release never waits for the in-process registry;
  contention leaves its identity claimed. Reuse retains the original registry/
  connection/writer deadline. Exact raw canonical names reject macOS case-
  folded root/cache/stage aliases, the preliminary initialized state is bound
  into fenced store preparation, and that preparation cannot repair permissions
  or provision controls before the final gate. Pre-initialization roots admit
  only the database, controls, and recognized crash sidecars and require
  canonical cache absence; initialized roots may evolve through
  ordinary reserved/snapshot state and a distinct later cache. The cache
  publication fence must still prove exact reset-stage absence; only the exact
  canonical-cache object-error allowlist is optional afterward. Ordinary
  database errors remain ordinary database errors, valid same-version/newer-
  schema crash prefixes resume, exact V1 `Complete` keeps legacy direct
  admission, and random-stage ownership is never inferred or removed. Public
  reset transport, native confirmation, preferences, relaunch, accessibility,
  production reset/release evidence, and Windows support remain separate work.
- Completed the detached old-store structural tail for private app-data-reset
  recovery. Once cache, snapshot, SQLite sidecars, and the old main database
  are exactly absent, later pre-open passes now admit only five monotonic
  states: full controls, lock controls, cleanup-plus-writer controls, writer
  only, and an empty detached root. Each pass removes exactly one protocol-
  ordered control or the empty root while retaining the data-parent fence and
  every still-present advisory exclusion. Unknown children, non-monotonic
  control shapes, aliases, replacements, mount boundaries, unsafe metadata,
  contention, and pre-effect drift fail closed. A successful unlink or
  `rmdir` alone starts the shared 250 ms synchronization/read-back deadline;
  restart observes the exact next typestate after every tested uncertainty
  seam. Exact old-root absence is a typed no-effect witness. The journal
  deliberately remains `Draining`: completed-state validation, the `Complete`
  transition, ordinary engine admission, random provisioning-debt handling,
  public transport/native reset UX, capacity claims, and Windows support
  remain separate checkpoints.
- Continued private app-data-reset recovery past exact snapshot absence into
  the detached old SQLite payload. A reset-only opener now accepts only the
  bounded database-present or exact controls-only tail, selects private
  sidecars in raw-byte lexical order, and makes the main database reachable
  only after every sidecar is absent. The coordinator joins the exact durable
  `Draining` journal, sealed transaction, old/fresh roots, snapshot absence,
  cache-absence witness, retained cleanup/writer exclusions, and original
  deadline before minting one consume-once descriptor-relative unlink
  capability. One pass removes at most one file and returns only path- and
  byte-free progress; exact controls-only absence is validation-only. Strict
  opener/admission errors cannot fall back to an earlier effect, unreconciled
  journal-stage debt cannot be consumed beside an unlink, and a successful
  effect shares one fresh 250 ms budget across directory, old/fresh, cache, and
  journal read-back. Sidecar/main ordering, final-main certainty, all local and
  coordinator uncertainty seams, both cross-transaction joins, strict
  no-fallback behavior, and final journal/fresh/cache/old/snapshot drift are
  covered. Old controls/root retirement, `Complete`, public transport, native
  reset UX, capacity claims, and Windows support remain outside this checkpoint.
- Completed the detached old snapshot-store structural tail for private app-
  data-reset recovery. After exact cache absence and an empty snapshot payload
  inventory, later passes now recognize only `FullControlsEmpty` →
  `WriterOnly` → `EmptyDirectory` → `Absent`, removing exactly one retained
  marker, exclusively locked writer control, or empty snapshot directory per
  open. The coordinator joins the exact durable `Draining` journal, old/fresh
  roots, typed snapshot state, and cache-absence witness before minting the
  separate consume-once structural authority; exact absence is a no-effect
  witness and cannot fabricate removal authority. Marker-only and every other
  partial or disputed shape fail closed. Each unlink/rmdir is descriptor-
  relative and identity-bound, repeats the entire state at the final effect
  gate, synchronizes its retained parent, and shares one newly minted 250 ms
  deadline across snapshot, old/fresh-root, cache, and journal read-back.
  Restart, deadline, writer-contention, unsafe-shape and case-only-alias,
  cross-transaction/cross-layer drift, and every pre/post-effect uncertainty
  seam converge without repeating an effect. Final-payload removal now hands
  its updated retained state to the shared read-back, and the post-effect
  window is deliberately independent of an expired admission deadline.
  The old database/data-root tail, `Complete`, public reset transport, native
  confirmation/relaunch/preferences, and reclaimed-capacity claims remain
  outside this checkpoint.
- Continued private app-data-reset recovery into the detached old snapshot
  store after exact managed-cache absence. A pass already observing durable
  `Draining` now joins the exact journal, fresh canonical root, old detached
  root, complete retained snapshot inventory, and typed cache-absence witness
  before a coordinator-only capability can remove at most the
  lexicographically first snapshot final or quiescent recognized temporary.
  Admission requires one filesystem from the old data root through the
  snapshot directory, both controls, and every payload. The original recovery
  deadline remains the sole pre-effect deadline through the final inventory
  gate; after unlink succeeds, one reset-specific 250 ms deadline is shared by
  directory synchronization and every snapshot/root/cache/journal read-back.
  The descriptor-relative effect returns only path- and byte-free progress,
  refuses active writers and every unsafe or changed inventory, and resumes all
  before/after-effect uncertainty gaps without repeating an already absent
  object. Snapshot controls and directory, the old database and data-root tail,
  random provisioning debt, `Complete`, public reset transport, native
  confirmation/relaunch, preferences, and reclaimed-capacity claims remain
  outside this checkpoint.
- Completed the detached managed-cache tail of private app-data-reset
  recovery. A pass already observing exact durable `Draining` now routes a
  full store with recognized payloads through the existing one-file drainer;
  only an empty store or the exact monotonic `writer-only`/empty/absent tail
  can enter a separate coordinator-issued structural capability. Later passes
  remove at most the ownership marker, the retained-and-locked writer control,
  or the empty transaction-bound stage shell, synchronizing and reading back
  every transition. Marker-only, partial tails with payloads, aliases,
  replacements, unknown children, wrong identities, and canonical/stage
  conflicts fail closed without falling back to payload removal. The journal
  remains `Draining`; old data and snapshots, `Complete`, public reset
  transport, preferences, relaunch, and reclaimed-capacity claims remain
  outside this checkpoint.
- Extended private pre-open app-data-reset recovery from
  `FreshNamespaceReady` into durable `Draining` and the first bounded physical
  debt batch. The exact journal, fresh canonical root, old detached root, and
  detached-or-proven-absent managed cache are revalidated together before a
  coordinator-only opaque capability can reach unlink. Each recovery pass
  removes at most the lexicographically first validated managed-cache payload
  or temporary object, synchronizes and reads back the detached cache, and
  reports only path- and byte-free object progress. Journal-write uncertainty
  cannot mint deletion authority; unlink uncertainty is resumed from durable
  `Draining` without repeating an already absent object. Cache controls, the
  cache stage shell, all old data/snapshot objects, the fresh namespace,
  unknown siblings, and user files remain untouched. This checkpoint still
  cannot advance to `Complete`, admit an ordinary engine, or claim reclaimed
  capacity.
- Extended private pre-open app-data-reset recovery through transaction-bound
  fresh canonical publication and durable `FreshNamespaceReady`. Recovery now
  builds an exact five-entry pre-SQLite bootstrap in a random private stage,
  validates and synchronizes it before no-replace publication to a typed
  `.dux-reset-fresh-<id>` stage and then the canonical root, and seals the
  distinct fresh device/inode in journal V2. V2 also persists the canonical
  root component as lossless lowercase hex minted only by the retained root
  witness, so a restart under another sibling cannot redirect publication.
  The fixed origin record binds the transaction plus old and fresh identities;
  a published typestate also binds the current durable journal, coordinator
  parent, canonical root, and typed stage so it cannot advance another
  transaction. V1 `Prepared`/`CacheDetached` records upgrade to that binding
  only while the exact canonical old-root identity still proves the name;
  identity-less V1 `DataDetached`/fresh/draining records are incompatible,
  while legacy V1 `Complete` remains admissible. All six
  rename/sync/read-back gaps converge, collisions and wrong provenance remain
  untouched, and every open still returns reset-recovery-required because this
  checkpoint neither opens SQLite nor drains or deletes detached stages.
- Moved macOS app Settings into the Storage Explorer window as a fifth
  sidebar destination alongside Overview, Explore Snapshot, Recommendations,
  and History; the sidebar, menu bar popover, and in-app "Configure folders…"
  actions now navigate there instead of opening the separate Settings window.
  The `Settings` scene remains registered (scene-order invariant) and reuses
  the same view. Refreshed the shell UI with a shared DUX theme: sectioned
  sidebar with colored icon tiles, card-styled group boxes, a pressure-tinted
  capacity hero on Overview and in the menu bar popover, capsule disk-pressure
  badges, and an area-filled capacity sparkline. Presentation only: no scan,
  cleanup, or persistence behavior changed.
- Added private pre-open roll-forward recovery for the implemented app-data
  reset detach phases. An incomplete shared-lease observation now becomes a
  move-only intent retaining the original coordinator descriptors and exact
  journal; recovery acquires the exclusive lock on that same storage and
  rejects a missing or changed handoff before any namespace effect. One
  five-second admission deadline covers descriptor-only data/cache admission
  and pre-effect validation; an atomic data-root move retains only its bounded
  250 ms post-effect durability/read-back proof. `Prepared` can reconcile cache
  then data, `CacheDetached` can reconcile data, and `DataDetached` is
  revalidated; every
  path remains recovery-required because no fresh canonical namespace is
  provisioned. The runner never opens SQLite, repairs or creates ordinary
  storage, drains stages, exposes a public API, or reaches user files.
- Extended the private app-data-reset transaction through exact
  `CacheDetached` → `DataDetached`. The consume-once continuation repeats every
  journal, cache, data-root, retained-store, snapshot, reserved-child, and
  deadline proof before a descriptor-relative no-replace rename; it then
  synchronizes the data parent, proves canonical absence plus the exact staged
  identity/layout, and advances the journal only after read-back. Every drift,
  collision, rename/sync/read-back, panic-gap, or journal uncertainty remains
  payload-free recovery-required and non-retryable. Ordinary engine open now
  takes a shared reset-coordinator lease before storage and worker publication,
  keeps it until quiescent `Closed`, refuses incomplete journals before
  canonical-store creation, and distinguishes corrupt/unsafe coordinator state.
  The shared reader validates but never reconciles journal-stage debt. This is
  still an internal safety checkpoint: it adds no roll-forward runner, fresh
  namespace, drain, public reset action, preference mutation, relaunch,
  user-file effect, or Windows support.
- Added the first exact app-data-reset namespace effect. A committed private
  `Prepared` continuation can now be consumed exactly once to detach the
  marker-owned managed cache to its transaction-derived stage and durably
  advance the journal to `CacheDetached`; prepared cache absence advances
  without provisioning. The descriptor-relative no-replace rename retains the
  cache writer and publication fences, synchronizes `Caches/Dux`, and proves
  canonical absence plus the exact staged identity, controls, and inventory.
  Every failure after `Prepared` is payload-free recovery-required, every
  ambiguous witness is consumed rather than retryable, stale canonical handles
  fail closed, and unknown outer siblings plus the data namespace remain
  untouched. This remains private and adds no data detach, recovery runner,
  fresh namespace, stage drain, public FFI/Swift/CLI action, or cleanup of user
  files.
- Added the first durable app-data-reset intent under the complete retained
  core admission proof. The internal consume-once transition derives exact
  data/cache device and inode identities from descriptor-backed witnesses,
  durably publishes only the singleton `Prepared` journal, and retains every
  lock and witness in a higher-ranked committed continuation. Pre-publication
  drift or expiry is a proven no-intent refusal; any rename-attempt or later
  durability/read-back failure is outcome-unknown and requires recovery rather
  than retry. The continuation can only revalidate the exact journal and
  provisioning-debt-free namespace state. It exposes no detach, fresh
  namespace, draining, public FFI/API, Swift/UI/CLI caller, preference mutation,
  relaunch, or user-file cleanup authority.
- Added the effect-dormant native terminal gate required before a future
  **Reset DUX** operation can consume the private FFI reset handoff.
  `AppRuntime` now has one immutable ordinary-quit/app-data-reset winner and a
  retained, caller-cancellation-safe drain. Before the first suspension it
  fences AppModel, CLI, and Explorer admission, then joins startup, Settings
  operations and lease releases, scans, Explorer work, reviews, capacity
  sampling, and maintenance. Ordinary quit closes the engine exactly once
  after the aggregate native proof; the dormant reset winner never calls
  ordinary close. Cleared UI task/preview slots cannot erase accepted work,
  concurrent terminal callers share the same completion, and a source-layering
  policy keeps terminal callbacks out of drained child owners. This checkpoint
  adds no Reset UI, public FFI call, durable intent, namespace effect,
  preference clearing, relaunch, or cleanup authority.
- Added the private, effect-dormant FFI terminal gate required before a future
  **Reset DUX** operation can enter core validation. One session-wide
  `Open`/typed-`Closing`/`Closed` lifecycle now rejects later engine, child,
  task-poll, and task-cancel admission; ordinary close and reset contenders
  rendezvous on one terminal owner. Reset validation uses the caller's original
  absolute deadline, checks release of every live child in all seven FFI
  registries, joins admitted callbacks and plan operations, and only then calls
  a callback-free, path-free core validation seam. Release failure, poison, or
  deadline exhaustion remains terminal and is published as unquiesced to every
  observer. This checkpoint adds no UniFFI reset method, generated binding,
  durable `Prepared` intent, namespace effect, Swift/CLI caller, or cleanup
  authority.
- Integrated Sparkle 2 as DUX's sole in-app update framework. XcodeGen and the
  committed Swift Package resolution pin the reviewed 2.9.2 release, and
  Settings owns a standard **Check for Updates…** surface backed by
  `SPUStandardUpdaterController`. The current spike build remains deliberately
  dormant: it cannot instantiate or contact Sparkle until a non-placeholder
  bundle identity, HTTPS `SUFeedURL`, and a valid base64-encoded 32-byte
  `SUPublicEDKey` are all present. No feed, private key, release credential,
  custom updater, CLI
  mutation, or update authority is introduced by this checkpoint. The
  production release workflow now admits only Sparkle's exact reviewed 2.9.2
  framework, updater app, installer/downloader XPC services, and autoupdater,
  verifies all five executables are universal, and signs them inside-out with
  Hardened Runtime while preserving the downloader's upstream entitlements.
- Added the first native terminal-quiescence prerequisite for future
  **Reset DUX** admission. `CLIInstallationModel` now fences later work before
  its first suspension, joins any already-confirmed install, upgrade,
  reinstall, or uninstall without cancellation or retry, discards an unused
  confirmation exactly once, closes the installer service, and only then
  returns a narrowly typed `ConfirmedCLIMutationQuiescence` marker. One
  retained terminal task coalesces ordinary-shutdown callers through service
  close even when an initiating caller is cancelled. `AppRuntime` must consume
  that unforgeable marker before engine close. This does not claim general
  native or FFI-child quiescence and adds no reset transport, journal intent,
  storage effect, UI, preference clearing, relaunch, or CLI command.
- Added effect-dormant namespace publication admission to the private
  **Reset DUX** composition. One sealed 128-bit transaction derives
  role-specific data/cache stage components for the journal and witnesses;
  callers cannot supply paths or swap destinations. The data witness retains
  the same parent-directory fence used by normal root publication, binds the
  exact canonical private root and same-filesystem boundary, proves the stage
  absent, and requires the matching database guard. Unimplemented `ai` and
  `logs` children fail closed.
- Managed-cache reset admission now retains the same parent and optional
  `Caches/Dux` publication fences used by normal provisioning. It can prove an
  absent outer container or fixed child without creating either one, while a
  present `scan-cache-v1` additionally retains its writer, complete inventory,
  canonical identity, same-filesystem boundary, and absent typed stage.
  Unknown outer siblings remain untouched. The full private order is
  coordinator → data publication → cleanup → database → snapshot → cache
  publication → optional cache writer under one deadline, with same-process
  and subprocess race coverage. This checkpoint still writes no reset intent,
  performs no detach or deletion, and exposes no FFI, CLI, Swift, or UI action.
- Extended the dormant **Reset DUX** core admission through the snapshot and
  present managed-cache writer layers. Database-before-snapshot ordering is
  encoded in the callback API; the snapshot lease retains and repeats its
  complete canonical inventory and refuses any active staged writer. A
  read-only cache re-probe never provisions missing storage, while a present
  cache retains its cross-process writer lock and exact bounded inventory.
  Canonical-directory replacement, ignored-lock child insertion, contention,
  unwind, and real subprocess exclusion all fail closed.
- Reset lock acquisition now consumes one monotonic deadline across
  coordinator, core-worker quiescence, cleanup/database, snapshot, and cache
  admission instead of restarting the timeout at every layer. Callback-visible
  admission types borrow every owned lock; even panic or `mem::forget` cannot
  leak a guard or reverse cache → snapshot → database → cleanup → coordinator
  release. Terminal and runtime-observation mutexes are deadline-aware, queued
  closures drain through tracked worker quiescence, and a final expiry gate
  prevents slow revalidation from handing off late admission. A contended
  coordinator-provisioning stage remains marker-complete and is reconciled on
  the next successful open. An absent cache remains an explicit unfenced
  refusal. This checkpoint still grants no journal transition, namespace
  witness, detach, deletion, FFI, CLI, or native reset authority.
- Added a private coordinator-first terminal/store preflight for the future
  **Reset DUX** lifecycle. The engine derives the coordinator only from the
  store's revalidated canonical database identity, then rejects incomplete
  journal state or unproven coordinator provisioning debt before attempting
  terminal reset. A pre-terminal refusal leaves the prior lifecycle unchanged
  by that attempt; every refusal after a winning reset claim explicitly leaves
  the old engine terminal.
- After terminal arbitration, the composition cancels and joins every worker,
  verifies the complete registry quiescence inventory, checks active and
  process-quarantined cleanup on both sides of retained store admission, and
  revalidates coordinator plus cleanup/database exclusion before invoking one
  higher-ranked callback. Active snapshot-review pins now join the bounded
  path-free durable blocker set; expired pins remain inactive but are still
  fully validated. The callback receives no path, target, journal transition,
  namespace witness, or reset effect. Independent coordinator provisioning or
  reconciliation is the only possible filesystem mutation in this checkpoint;
  no FFI, CLI, or native reset action exists.
- Added a retained coordinator-to-database admission seam for the future
  **Reset DUX** lifecycle. One callback-scoped coordinator session now holds
  the same permanent writer lock across typed journal recovery and transitions;
  a same-instance atomic fence rejects nested and concurrent attempts without
  re-entering or weakening the outer OS lock. Store admission is reachable
  only inside that retained session and uses a higher-ranked callback so its
  move-only cleanup/database guard cannot escape or invert the required lock
  order.
- Checksummed SQLite schema v18 adds two partial indexes for bounded,
  path-free probes of unresolved cleanup item/path effects. Reset admission
  retains cleanup exclusion plus the database writer/connection and reports
  only scalar blockers for lock contention, active cleanup, uncertain effects,
  running scan/process-claim evidence, and scan-scope leases. Cross-process
  tests prove cleanup ownership blocks admission and that an admitted guard
  prevents a new scan-scope commit until release. This checkpoint still owns
  no reset target, composes no durable reset intent, performs no detach or
  deletion, and exposes no engine, FFI, CLI, or native action.
- Added core terminal arbitration for the future **Reset DUX** lifecycle.
  Ordinary close and reset now race under the task-registry mutex, and only
  the reset winner receives a non-cloneable shutdown capability. Winning
  reset admission closes every later task admission, cancels queued and
  running work, and cannot be upgraded, duplicated, or reversed by a later
  close. A separate non-cloneable quiescence proof is returned only after the
  engine reaches `Closed` and every worker handle is joined; timeout consumes
  authority and leaves the old engine terminal. This checkpoint creates no
  durable reset intent, opens no reset coordinator, performs no filesystem
  effect, and exposes no FFI, CLI, or native reset action.
- Added a dormant Unix/macOS foundation for a future **Reset DUX** lifecycle.
  A fixed, independently marker-owned sibling coordinator is atomically
  provisioned from a complete private stage and stores one bounded,
  checksummed, canonical, forward-only reset journal under a permanent
  cross-process lock. Component-wise no-follow traversal, exact owner/mode/
  link/ACL and retained-identity checks, bounded inventories, durable
  publication, exact compare-and-advance, and marker-complete abandoned-stage
  reconciliation fail closed around every transition. This checkpoint owns no
  reset target, performs no namespace detach or deletion, and exposes no
  engine, FFI, CLI, or native action. The accepted lifecycle contract keeps
  scanned files, unknown cache content, the installed CLI, OS permissions and
  integrations, and Sparkle state outside Reset DUX; terminal engine recovery,
  bounded physical draining, relaunch, and user confirmation remain required
  before a button can exist.
- Added separately confirmed **Clear older snapshots** storage through UniFFI
  v54 and native **Storage & Privacy**. One engine-bound, consume-once
  two-minute preview freezes the exact path-free population of
  retention-eligible older snapshots and already-retired physical residuals,
  plus protected and excluded accounting. Final admission repeats the complete
  database-before-snapshot proof, rejects any drift before effect, atomically
  appends every required immutable tombstone, and removes only exact
  retained-handle finals. The latest two snapshots for every exact root and
  all active reviews remain protected; orphans, temporary and provisioning
  maintenance, controls, history, scan cache, AI content, settings, legacy
  cache data, and user files remain unreachable. Native confirmation
  serializes against cache clearing, invalidates stale measurements before
  effect, remeasures exactly once after every terminal attempt, never retries
  deletion, and makes no free-space promise.
- Added the engine-owned managed TUI scan cache and its exact native
  **Storage & Privacy** lifecycle through UniFFI v53. DUX now owns only the
  fixed, marker-validated `Dux/scan-cache-v1` child beneath the conventional
  cache container; legacy caller-selected cache files and every unknown outer
  sibling remain excluded from accounting and clearing. The independent
  SHA-256-bound wire format validates headers before allocation, never falls
  back to the legacy decoder, and caps files at 64 MiB, trees at 200,000
  nodes, depth at 512, and modeled decode residency at 192 MiB. The private
  store uses descriptor-relative no-follow validation, exact ownership,
  permissions and link checks, bounded inventories, retained writer locks,
  atomic publication, and typed pre-effect versus outcome-unknown failures.
  Completed CLI scans publish only while consuming their exact scan-scope
  lease; cache read/write failure remains non-fatal and triggers or preserves
  a fresh scan.
- Extended DUX-owned storage accounting with a third additive managed-cache
  share and an exact, path-free two-minute clear preview. Native Settings
  shows database/history, snapshots, and scan cache with distinct text,
  symbols, colors, and patterns; exposes entry, temporary-remnant, control,
  and charged-byte accounting; and requires a separate destructive
  confirmation that names the exact object counts and exclusions. Clearing
  can remove only the unchanged marker-owned entries and recognized
  temporaries. It cannot remove controls, the outer/legacy cache, embedded AI,
  database/history, snapshots, settings, or user files, and it makes no
  free-space promise. Changed or uncertain outcomes are remeasured and never
  retried automatically.
- Added a bounded, read-only DUX private-storage measurement through UniFFI
  v52 and native **Storage & Privacy**. The core now reports conservative
  logical, allocated, and per-file charged usage for the exact marker-owned
  SQLite main/sidecar/control files and the bounded snapshot store, with
  independently reconciled snapshot policy/debt classes and a checked additive
  physical total. Embedded AI insight IDs, digests, provider/adapter/model
  labels, and payload bytes are shown separately as logical content already
  inside SQLite, never double-counted as physical or reclaimable space. The
  path-free API accepts no selector and exposes no mutation capability; it
  excludes directory metadata, unattributable provisioning stages, and the
  unmarked caller-selected legacy CLI cache. Native Settings performs an
  explicit off-main measurement, preserves the last complete observation on
  failure, shows a non-color-only database/snapshot chart with full-range
  `UInt64` formatting, and states that the result is neither free space nor a
  cleanup promise.
- Added snapshot-retention cap controls to native **Storage & Privacy** through
  UniFFI v51. Versioned, path-free get/set/reset records preserve exact
  `u64` values, default/stored provenance, idempotence, and typed
  compatibility/uncertainty failures without starting retention or granting
  snapshot-removal authority. The isolated native model serializes work off
  the main thread, generation-fences stale completions, preserves the last
  confirmed value on failure, and requires an explicit reload after an unknown
  result. Settings accepts exact GiB values including zero, clearly explains
  that the cap is neither an immediate clear command nor a hard physical
  ceiling, and restores the versioned core default separately.
- Added checksummed SQLite schema v17 and one bounded, durable
  cross-process scan-scope lease shared by the native app, engine-backed
  scans, and the progressive CLI. A random move-only lease is acquired before
  work is queued or terminal takeover begins, so exact, ancestor, and
  descendant canonical roots conflict while disjoint siblings remain
  concurrently observable. Cache-only CLI browsing requires no lease.
  Cancellation, close, panic, graceful completion, and rescan paths retain the
  lease until the scanner has quiesced and exact-reconcile release without
  creating cleanup, snapshot, candidate, AI, or effect authority. The bounded
  migration fabricates no leases, preserves v16 rows byte-for-byte, and treats
  every legacy `running` scan—including an unclaimed row—as a transitional
  blocker. Newer-schema stores make older clients read-only; stale leases are
  reclaimed only with same-host prior-boot evidence or reliable exact
  process-instance death, never from age or PID alone. The CLI reports
  conflicts before emitting terminal control bytes and preserves an existing
  tree when a rescan is refused.
- Added the final versioned inspection CLI surfaces: bounded path-free
  `scan-detail` coverage issues, deterministic `candidates` pages, semantic
  `review-state` transitions, and keyset-paged `cleanup-history list/show`.
  Every nested JSON object carries schema version 1, stable typed errors stay
  on stderr, and the frozen contract omits scan-root components, candidate
  paths/evidence payloads, cleanup targets, owners, claims, receipts, and
  effect fences. Review commands retain the exact Explorer snapshot lease,
  report `cleanup_performed: false`, and cannot plan, approve, schedule, retry,
  or execute cleanup; an uncertain mutation is explicitly non-retryable until
  candidate history is reloaded. Human-readable forms and command grammar
  preserve the existing TUI as the default.
- Added the optional macOS Settings CLI companion lifecycle. DUX now packages
  one reproducible, universal arm64/x86_64, macOS-14 CLI plus a strict
  version/schema/snapshot/hash manifest and exposes its exact compatibility
  tuple through UniFFI v50. Settings can inspect, explicitly confirm, install,
  upgrade, reinstall, or uninstall only the fixed `~/.local/bin/dux`
  destination. The app never executes that destination, edits shell startup
  files, escalates privileges, overwrites an unmanaged file, silently
  downgrades a newer CLI, or removes a binary it cannot prove it installed.
  Descriptor-relative no-follow traversal, owner/mode/link checks, a
  DUX-specific xattr marker, full-byte hashing, universal Mach-O and static
  code-signature validation, directory locking, atomic exchange, displaced
  inode revalidation, rollback, `fsync`, one-shot confirmations, and
  authoritative reload after an uncertain outcome protect every mutation.
  Sparkle and app removal remain deliberately separate from this installed
  companion.
- Added a separate bounded claimed-running-scan provenance census through
  UniFFI v49 and native **Storage & Privacy** diagnostics. One synchronous,
  read-only query inspects at most 64 claimed `running` rows plus one
  lookahead and reports only the inspected total, same-host/current-boot,
  same-host/prior-boot, foreign-host, stored-unproven, and
  current-context-unavailable counts with explicit truncation. Migrated
  all-`NULL` claims remain distinct from complete stored provenance that cannot
  be compared to unavailable current host/boot context; malformed provenance
  in the inspected page or lookahead fails the census. No scan or claim
  identity, process fact, digest, policy, timestamp, path, byte estimate, or
  row selector crosses the boundary, and the query cannot probe liveness,
  start recovery, inspect user or temporary files, mutate storage, or
  authorize cleanup.
- Added a Rust-only prior-boot running-scan recovery checkpoint in checksummed
  SQLite schema v16. New immutable scan claims can bind separate fixed-size,
  domain-separated stable-host and boot-scope observations with the sole
  `interrupt_only` policy; every v9–v15 claim remains all-`NULL` and explicitly
  unproven. Same-host prior-boot claims may now change one exact pristine scan
  to `interrupted` as history-only reconciliation without probing an old PID.
  Same-boot recovery still requires an exact `DefinitelyGone` process result;
  foreign-host, out-of-scope legacy, unavailable, partial, and malformed
  provenance cannot produce a write. Global 64-row keyset pages plus one
  lookahead preserve bounded convergence, and recovery retains snapshot-temp
  leases and adds no filesystem, cleanup, AI, FFI, or Swift authority.
- Added a bounded, read-only running-scan recovery-debt census through
  checksummed SQLite schema v15 and UniFFI v48. Settings can now show up to 64
  unclaimed durable records plus explicit truncation, separating exact
  pristine and unexplained shapes without exposing paths, IDs, process facts,
  timestamps, or byte estimates. The diagnostic performs no liveness probe,
  recovery, temporary-folder search, or mutation and explicitly makes no
  reclaimable-space claim.
- Hardened the schema-v14 cleanup-owner migration proof with a populated v13
  active journal graph. The migrated graph must decode through the production
  journal loader as explicitly `Unproven`, and attempting recovery must leave
  its complete mutable journal state byte-identical. This adds no recovery,
  reconciliation, validation, resume, or effect authority.
- Added the bounded cleanup-owner provenance checkpoint in checksummed SQLite
  schema v14. New cleanup claims can persist separate fixed-size,
  domain-separated stable-host and boot-scope digests with the sole current
  `resumable` policy; every v1–v13 row migrates with all three fields `NULL`
  and therefore remains explicitly unproven. Runtime classification rejects
  partial or malformed tuples and keeps prior-boot, foreign-host, migrated,
  and otherwise unproven observations typed and non-executable. This adds no
  reconciliation handle, generation claim, validation, resume, or cleanup
  effect authority.
- Added the native Explorer **Changes** experience and completed its
  parent-scoped lifecycle through UniFFI contract v47. Entering Changes lazily
  prepares the exact predecessor comparison; leaving the mode, replacing a
  snapshot or subtree, closing Explorer, parent expiry, and app shutdown
  generation-fence and release the child before its parent. The app strictly
  validates both historical node kinds, presence, arithmetic, pagination,
  ordering, coverage, and treemap accounting before publishing app-owned
  state. The view shows the older-to-newer timeline, both coverage states,
  current/previous logical size, separate observed growth and shrinkage,
  a six-column navigable table, and a magnitude treemap whose color, symbol,
  dash pattern, text, and VoiceOver descriptions distinguish every state.
  Other Growth and Other Shrinkage remain separate; zero-magnitude rows remain
  in the table. The dedicated inspector is historical and read-only, and the
  persistent disclosure says these are logical-size observations—not verified
  capacity change or reclaimable space. Contract v47 adds optional
  current/baseline node kinds so Swift can independently validate replacements
  and one-sided directory navigation; it adds no live path, candidate, plan,
  AI, cleanup, or effect authority.
- Added the Rust-owned foundation for Explorer snapshot changes through UniFFI
  contract v46. An exact active Explorer review can now prepare one subordinate
  comparison against its immediately preceding succeeded, non-targeted,
  non-tombstoned snapshot only when the lossless root bytes and non-null
  code-owned root-identity digest match. Selection uses the same completion,
  start, and scan-ID order as latest-two retention. The child reuses the parent
  review and owns exactly one baseline lease, preserving the existing
  two-document/1 GiB decoded-review ceiling; partial acquisition, close,
  release, expiry, and wrong-parent cases fail closed. Rust matches
  root-relative component bytes rather than snapshot-local IDs or lossy
  display names and exposes bounded union pages plus magnitude-ranked treemaps,
  so additions, removals, growth, shrinkage, unchanged entries, type
  replacements, and removed-only directory drill-down remain explicit.
  Logical change is represented as direction plus `u64` magnitude, and
  treemaps account for omitted growth and shrinkage separately so opposing
  changes never cancel in “Other.” The transport contains no live path,
  candidate, reclaimability, plan, approval, AI input, provider command, or
  cleanup capability.
- Added a read-only iCloud identity-capability probe through UniFFI contract
  v45. A manual item check now brackets two complete Foundation samples with
  current-account and current-file-version observations, then reports bounded
  account, item-generation, and file-version stability independently from
  upload/download eligibility. Shared-item and sync-paused facts are also
  explicit. Stable container identity for an arbitrary user-selected iCloud
  Drive item is unavailable on the current public Foundation surface, so the
  production container fact is `unsupported` and identity readiness remains
  false even when current sync metadata supports review. Comparison material
  is not decoded, displayed, or persisted. Explorer distinguishes sync
  eligibility from incomplete identity proof; no persistence schema, rule,
  candidate, plan, approval, journal/history row, provider command, cleanup
  button, retry, AI/CLI/schedule edge, or eviction effect was added. A
  dedicated disposable-account real-device protocol now gates any stronger
  identity claim and separately gates future destructive eviction tests.
  Final verification passes formatting, workspace and fuzz Clippy with
  warnings denied, Rust 1.88 workspace/fuzz compatibility, all 31 policy
  tests, and the 265-source destructive-call boundary. The serialized direct
  core lane covers all 1,168 runnable tests across its complete run and one
  exact retry after a transient FSEvents probe failure; two
  helper/performance tests remain intentionally ignored. All 37 CLI tests,
  all 13 projection tests, all 91 ordinary FFI tests plus both isolated effect
  regressions, and all 535 linked native tests pass. Debug and Release Swift
  bindings are byte-identical; their Rust archive and unsigned apps are
  universal arm64/x86_64, target macOS 14, and retain `LSUIElement=true`.
  Release has the expected three-file app layout and excludes the internal
  permanent-cleanup condition; Xcode 26.5 adds its two expected dynamic-
  replacement dylibs only to Debug.
- Added a bounded manual **iCloud Status** review through UniFFI contract v44.
  Rust now selects at most 32 complete regular files from the exact retained
  snapshot directory subtree, ranking known nonzero historical allocation
  deterministically within a 200,000-descendant all-or-nothing traversal
  budget. The path-free source carries exact coverage and omission accounting;
  Swift rejects malformed versions, echoes, counts, ranks, nodes, contexts,
  and ordering. Explorer loads no live metadata automatically. An explicit
  check processes the source serially through the existing before/after
  filesystem witness and Foundation fact policy, with code-owned single-flight
  state and honest **Stop after current check** behavior for the synchronous
  system metadata read. The accessible table and non-destructive inspector
  distinguish favorable, blocked, changed, failed, stopped, and unattempted
  observations without summing historical allocation or claiming reclaimable
  capacity. Results remain memory-only discovery and create no candidate,
  durable evidence, plan, approval, journal/history row, AI input, schedule,
  provider command, cleanup button, retry, or eviction effect.
  Final verification passes formatting, workspace and fuzz Clippy with
  warnings denied, Rust 1.88 workspace/fuzz compatibility, all 31 policy tests,
  and the 265-source destructive-call boundary. Serialized Rust lanes pass
  1,163 ordinary core tests plus three timing-sensitive exact regressions, all
  37 CLI tests, all 13 projection tests, and all 90 ordinary FFI tests plus
  both isolated effect regressions; two helper/performance tests remain
  intentionally ignored. All 524 linked native tests pass. Debug and Release
  Swift bindings are byte-identical; the universal Rust archive and both
  unsigned apps are arm64/x86_64, target macOS 14, retain identical three-file
  layouts and `LSUIElement=true`, and keep the internal permanent-cleanup
  condition out of Release.
- Added the read-only foundation for iCloud Drive local-copy recovery through
  UniFFI contract v43. An accepted provider-specific ADR now requires a
  retained Explorer review to select and revalidate one exact non-root,
  regular, single-link file with known nonzero local allocation. The macOS
  adapter receives only a consume-once exact path, clears cached Foundation
  values, and reports bounded tri-state ubiquitous, upload, download,
  conflict, exclusion, and current-local-copy metadata. Rust—not Swift—owns
  provider, kind, allocation, timestamp, and deterministic classification;
  every missing, stale, active, errored, conflicted, excluded, remote-only, or
  otherwise unsupported state fails closed. The result is path-free discovery
  metadata and deliberately creates no rule, candidate, plan, approval,
  journal entry, emergency recommendation, AI input, schedule, cleanup button,
  provider command, or eviction effect. Actual
  `FileManager.evictUbiquitousItem(at:)` integration remains gated on durable
  provider/account/version binding, fresh pre-effect proof, a journal-fenced
  no-retry executor, truthful UI/accessibility, and disposable-account race
  tests. Final verification passes formatting, workspace Clippy with warnings
  denied, all 31 policy-checker tests, and the 265-source destructive-call
  boundary, which confirms that no production eviction primitive exists. The
  serialized Rust lanes pass 1,160 core tests plus three timing-sensitive
  regressions in their dedicated isolated lane (with two performance helpers
  ignored), all 37 CLI tests, and all 90 ordinary FFI tests plus both isolated
  effect regressions. All 495 linked native tests pass. Generated Debug and
  Release Swift bindings are byte-identical; their universal Rust archive and
  both unsigned apps are arm64/x86_64, target macOS 14, retain identical
  three-file bundle layouts and `LSUIElement=true`, and keep the internal
  permanent-cleanup condition out of Release.
- Added deterministic recurring “storage thief” ranking through UniFFI
  contract v42 and native Cleanup History. Core analyzes at most the newest 32
  completed or partially completed schema-v2 permanent-safe sessions, groups
  complete successful rule effects by stable rule ID, and publishes at most
  12 ranked groups plus the total group count and explicit older-history
  disclosure. Only the existing compatible explicit-zero-to-nonzero
  `Regrown` outcome contributes to the checked aggregate bytes/time rate;
  absence, a first later size, zero without later growth, supersession, and
  ineligible evidence never become growth. Exact fraction comparison avoids
  overflow before cleanup-count, cycle-count, recency, and stable-ID
  tie-breakers, while the displayed bytes/day value is floored and reports
  saturation explicitly. Manual successes and manual cycles remain distinct;
  the same latest rule revision needs two successful manual sessions and one
  manual confirmed cycle to expose only a historical “repeated manual
  pattern.” It is not automation eligibility, creates no schedule, and cannot
  reach a candidate, plan, approval, driver, or effect. The query reuses fixed
  scan, journal, materialization, SQLite, time, and Rust-work budgets, returns
  no partial result, never reads the legacy `rule_outcomes` table, and leaves
  storage unchanged. Core revalidates its derived rate/cap pair; FFI
  independently validates versions, bounds, counts, unique IDs, exact order,
  durations, timestamps, and threshold prerequisites, while Swift repeats the
  bounded response-shape, uniqueness, duration, timestamp, exact
  overflow-free fraction order, derived bytes/day cap state, and threshold
  checks. Cleanup History lazily presents accessible ranked
  bars, observation freshness, window/group truncation, empty/loading/failure
  states, and a read-only refresh; scan/history changes and lifecycle
  generations fence stale replies while earlier valid results remain visible.
- Completed rule-outcome/regrowth delivery through UniFFI contract v41 and the
  native Cleanup History experience. The versioned exact-session endpoint
  projects one bounded path-free state per immutable cleanup item and preserves
  all six eligibility failures and all six observation states. Rust rejects
  invalid chronology, zero/nonzero contradictions, overflow, pre-epoch times,
  and sub-millisecond chronology that would collapse at the Swift boundary;
  Swift independently requires exact session, ordinal, rule, revision, count,
  time, and byte agreement. Later observations have their own cancellable,
  generation-fenced load state, so a failure never hides the immutable cleanup
  record and a stale reply cannot cross selection, refresh, close, or shutdown.
  Successful home, subtree, and low-disk targeted scan observations re-read an
  open outcome view; users can also explicitly read again and see when the
  result was fetched. Cleanup History adds a compact six-state distribution,
  per-item explanations and timestamps, honest loading/failure/legacy
  accessibility copy, and explicit language that these comparisons neither
  verify bytes freed nor authorize cleanup. No paths, candidate IDs, plans,
  approvals, AI input, schedule input, callbacks, drivers, or effect authority
  cross the new boundary.
- Added the Rust-core rule-outcome/regrowth measurement boundary without
  trusting the legacy `rule_outcomes` table or adding cleanup authority.
  Candidate evaluator revision 5 now excludes the preserved direct
  `CACHEDIR.TAG` allocation from Rust-target estimates, so a marker-only target
  can produce an explicit zero observation; unknown allocation for any
  reclaimable descendant fails closed instead of collapsing to zero. The
  read-only exact-session query derives one path-free state per cleanup item
  from a terminal permanent-safe v2 journal plus current-policy, complete,
  same-root-identity v13 scan/evaluation/candidate observations. Candidate
  absence is never zero, a first nonzero observation is only “later size,” and
  “regrown” requires an earlier explicit zero followed by a compatible nonzero
  observation. Completion chronology, overlapping scan intervals, later
  overlapping cleanups, source-plan chronology, evaluator/catalog/scope
  identity, native path components, fixed row/materialization/work budgets,
  and typed ineligibility reasons all fail closed. Full candidate and journal
  payloads are compacted and dropped per observation; the public result carries
  no path, candidate ID, snapshot identity, plan, approval, AI input, schedule,
  filesystem handle, driver, or effect capability. FFI and native Cleanup
  History presentation remain a later slice.
- Added the fail-closed scan-identity prerequisite for rule outcome and
  regrowth measurement. Checksummed SQLite schema v13 stores a
  domain-separated SHA-256 digest of the code-owned filesystem identity for
  every newly admitted scan root and adds a bounded exact-root/start lookup.
  General, subtree, and targeted scans now capture and revalidate the same
  root identity before durable start and after traversal. Migrated scans keep
  an explicit `NULL` identity and therefore cannot become comparable outcome
  evidence. The digest is historical observation data only: it exposes no
  path, filesystem handle, cleanup plan, approval, executor, or AI authority.
  Verification covers v1/v12 migration upgrades, exact digest persistence,
  byte-exact root lookup, general/targeted/subtree identity drift, CLI and FFI
  reopen paths, both isolated cleanup-effect regressions, workspace Clippy
  with warnings denied, and the projection suite.
- Added deterministic Critical-pressure recovery guidance through UniFFI
  contract v40. Rust owns the fixed seven-lane §13.3 order and emits only
  currently supported evidence-backed stale-regenerable, guided-exploration,
  and permission/coverage groups. Finalization is bound to the exact Critical
  pressure episode and targeted-root catalog, reselects exact immutable scans
  plus current terminal evaluations, enforces a one-hour evidence window, and
  repeats the pressure/catalog checkpoint after projection. Root-level
  unavailable evidence remains a typed count with no fake scan ID or hidden
  byte estimate. The path-free contract contains no candidate ID, byte
  forecast, cleanup mode, plan, approval, AI input, schedule, callback,
  provider command, driver, or effect capability. FFI and Swift independently
  reject malformed versions, counts, ranks, lane order, semantic duplicates,
  unsupported lanes, contradictory lane fields, invalid roots, and timestamps;
  a malformed response cannot consume the retained exact proof. Explorer adds
  an accessible Critical recovery hero and ordered review-only cards that open
  exact Candidates, Browse, or Coverage views. The menu bar mirrors the first
  three core-ranked cards, preserves the real roadmap step numbers, and
  discloses Current/Updating/Earlier freshness. Complete-zero, incomplete,
  stale, and unavailable states stay distinct, overlapping estimates are never
  totaled, and neither surface claims cleanup approval or bytes freed. The
  checkpoint passes the serialized Rust/FFI suites (including both isolated
  effect regressions), 460 native tests, policy and destructive-call gates,
  byte-identical Debug/Release bindings, and universal arm64/x86_64 Debug and
  Release apps targeting macOS 14.
- Completed the Warning/Critical targeted-reclaim scan with deterministic,
  read-only Homebrew and pip findings. Candidate evaluator revision 4 and the
  reviewed 13-rule catalog add exact direct-child
  `developer.homebrew.cache` and `developer.python.pip_cache` revision-1
  matches only inside the Rust-derived current-account `Library/Caches`
  scope. Both require candidate-local coverage and complete inclusive
  seven-day newest-mtime evidence, retain descendant-symlink observations,
  and remain unschedulable and non-executable behind
  `MissingOrIncompleteEvidence` plus `ProtectedPath`; recency is never treated
  as provider inactivity. Immutable-snapshot replay reproduces the same
  result, scope/scan-ID mismatches fail closed, pending known-cache evaluation
  is not recovered without live OS-account root proof, and targeted durable
  reuse rejects stale evaluator/catalog observations. Recommendations'
  **Review findings** now opens the exact scan directly in Candidates. The
  native view adds revision-bound provider names, raw rule/revision detail,
  page-local category/safety/action cards that do not sum overlapping
  estimates, honest “Observed estimate” labels, and bounded 64-item
  previous/next paging. Independent provider reviews document the discovery
  and future-promotion boundaries; no plan, approval, AI, scheduler, command,
  or filesystem-effect authority was added. The final checkpoint passes the
  serialized Rust/FFI suites (including both isolated effect regressions), all
  444 native tests, policy and destructive-call gates, byte-identical
  Debug/Release bindings, and universal arm64/x86_64 Debug and Release builds
  targeting macOS 14.
- Added the Rust-owned known-user-cache foundation to the Warning/Critical
  focused scanner through UniFFI contract v39. The engine derives the single
  current account `Library/Caches` root from OS account evidence, never from
  `HOME`, Swift, or FFI input, and revalidates retained no-follow home, mount,
  and root identity evidence through publication. DUX's own cache subtree is
  excluded at the scanner boundary with exact component matching. Known
  caches run before non-overlapping configured project roots under one
  deterministic 200,000-node pass: at most 100,000 nodes for caches and
  10,000–50,000 for each project root. A versioned catalog digest binds the
  exact root kinds, order, identities, availability, budgets, and exclusions;
  every ordinal after zero and the final checkpoint must echo it. Active-task
  reuse additionally requires the exact catalog, root identity, scan budget,
  exclusions, and pressure episode; a same-path replacement or changed
  context returns busy instead of joining stale work. Durable reuse also binds
  the root source and retained snapshot identity. Swift repeats the
  full catalog, root-kind, budget, and digest validation and labels the cache
  row distinctly. Automatic cache observations use a separate evaluator scope
  and intentionally yield zero candidates until deterministic cache-specific
  rules exist, so they cannot inherit user-selected Rust-target eligibility.
  The transport remains observation-only and accepts no caller path, candidate,
  plan, approval, AI input, cleanup mode, or executor authority.
- Added the configured-project-root portion of the Warning/Critical focused
  reclaim scanner through UniFFI contract v38. At each exact latest durable
  capacity anchor, Rust rereads the revisioned root registry and current open
  pressure episode, admits one stored ordinal at a time, and scans roots
  sequentially with a 50,000-node per-root and 200,000-node aggregate bound.
  The request carries no path, candidate, plan, approval, AI input, cleanup
  mode, or executor field. Roots are no-follow identity fenced, must be on the
  pressured startup volume, and handle macOS's sealed System/Data APFS pair
  without allowing unrelated mounted devices. Warning and Critical use their
  own current episode identities; successful exact-root evidence is reused
  durably only when its snapshot and terminal candidate evaluation remain
  valid. Targeted tasks have lower priority than interactive scans, only exact
  targeted work may be observed, and owned cancellation is polled to a terminal
  state before foreground scope is considered released. Targeted snapshots no
  longer replace generic Home history. Swift independently validates every
  record, echoed volume/anchor/revision/ordinal, lossless root, node budget,
  disposition shape, task result, and final path-free checkpoint. Explorer and
  the menu bar show bounded read-only progress, honest scan time/allocated and
  logical size/coverage/issues/candidate counts, partial failures, retry and
  cancellation states; **Review findings** opens the exact immutable targeted
  snapshot. No focused result can approve or perform cleanup. Known user-cache
  roots and emergency recovery ordering remain later Milestone 6 slices.
- Added the durable configured-project-root registry needed by the future
  targeted low-disk scan. Core now stores at most 16 lossless absolute,
  normalized, non-root host paths in canonical byte order, rejects duplicates,
  nesting/overlap, control bytes, oversized input, corrupt rows, and newer
  value schemas, and distinguishes a rowless empty Default from an explicitly
  stored empty set. Revisioned replace/reset writes are exact-no-op aware and
  reconcile uncertain commits. UniFFI contract v37 and the native adapter
  independently validate bounds, encoding, ordering, overlap, source,
  revision, and timestamp shape. Ambiguous writes and post-write projection
  failures block more edits until an authoritative reload succeeds. Settings
  adds accessible folder-only add/remove/reset controls and keeps the registry
  explicitly read-only:
  selecting a root starts no scan, grants no filesystem access, and creates no
  candidate, plan, approval, AI request, or cleanup effect. The pressure
  runner remains a later Milestone 6 slice.
- Completed the first reviewable disk-pressure history surface through UniFFI
  contract v36. Core now returns at most 64 newest-first Warning/Critical
  episodes for one validated startup-volume identity at one exact capacity
  anchor. Persistence validates volume-lifetime bounds, strict ordering,
  non-overlap, the single-newest-open invariant, and one lookahead row before
  truncation, so malformed history cannot hide across a page boundary. Swift
  repeats record, identity, anchor, timestamp, interval, ordering, open-state,
  and `has_more` validation off the main actor; `AppModel` coalesces identical
  requests and fences late results across volume or anchor changes. An accepted
  history anchor must be either an exact durable raw observation or the current
  exact `last_seen` observation when hourly cadence suppressed its raw row;
  arbitrary instants inside a volume lifetime fail closed. Capacity-trend
  caching separately remembers the requesting snapshot anchor, so a legitimate
  older at-or-before raw trend point does not disappear or trigger reloads.
  Explorer now combines signed 24-hour/7-day changes with a full 30-day
  capacity chart, pressure-colored points, Warning/Critical ribbons, and a
  recent low-space period list with honest loading, stale, truncated, and
  warming states. The
  menu popover shows a compact current pressure-period line only when it
  matches the accepted volume, anchor, and pressure. The transport remains
  path-free observation-only telemetry and cannot name, approve, or execute
  cleanup. The broad Rust gate also now models the preceding permanent-cleanup
  opt-in correctly: effect-path fixtures opt in explicitly, multi-path
  Rust-target sessions prove zero mutation under the current one-item/one-path
  seal, and only the pinned SHA-256 dependency is optimized in Debug/test
  profiles so complete Cargo-executable attestation no longer takes minutes.
  A macOS FSEvents teardown fix now retains callback state through the stream
  lifetime and drains the private dispatch queue after invalidation, closing a
  concurrent callback use-after-free found by the broad gate without weakening
  dropped-event fail-closed behavior.
- Exposed the effect-free Rust-target validation path end to end in the native
  app through UniFFI contract v35. The new endpoint consumes only the exact
  opaque reviewed-plan child and returns a separate path-free dry-run task;
  it accepts no path, identifier, mode flag, approval, callback, driver,
  capacity input, or permanent-cleanup policy. Rust and Swift independently
  validate task kind, correlation prefix, terminal status, poll shape,
  monotonic revision, and cancellation state. Explorer now offers a
  Release-visible **Run dry check** action with point-in-time wording,
  **No files changed · 0 B freed** accounting, cancellation, durable history
  correlation, and an explicit fresh-preview requirement. Permanent cleanup
  remains a distinct Debug-only, confirmation-gated action and cannot be
  reached or chained from the dry-run result.
- Added the first production-core Rust-target dry run. It consumes the same
  exact opaque reviewed-plan child as permanent cleanup, projects that frozen
  plan internally to `DryRun`, and repeats the shared Cargo/read-set,
  process-quiescence, home/mount, protected-root, identity, seven-day recency,
  complete-subtree, manifest, and cache-tag validation. The dry-run authority
  cannot approve, create an effect witness, select a capacity sampler, or
  receive a platform driver. A separate serialized engine task records one
  uncoupled, ownerless terminal history graph with no candidate claim, effect
  receipt, verified capacity delta, or removed-byte result. User exclusions
  convert only an otherwise successful observation to durable rejection under
  the same cleanup lock; cancellation and more specific validation refusals
  retain precedence. A terminalization boundary prevents late cancellation
  from being reported as accepted, unavailable evidence remains distinct from
  changed evidence, and unresolved metadata releases its lease without
  quarantining filesystem cleanup. The permanent-cleanup opt-in is deliberately
  irrelevant. A real stale Cargo fixture proves the complete project tree is
  byte- and identity-stable, and focused parity, refusal, cancellation-state,
  exclusion, and ambiguous-write tests cover the non-mutating path. This core
  At that core-only checkpoint the API was not exposed through UniFFI or the
  native app and did not open the public cleanup Release gate.
- Made permanent-safe cleanup explicitly opt-in before its Release gate can be
  considered. Fresh and reset state is now core-owned disabled Default state;
  only durable Stored consent can admit an effect. Typed setting value schema
  v2 preserves schema-v1 explicit Stored choices, strengthens legacy
  Default-enabled epochs to disabled, and keeps corrupt/newer state fail-closed.
  The claimed executor records disabled work as Rejected before any effect
  receipt and the final journal gate still rechecks under the shared cleanup
  exclusion, so the platform driver receives no call and removes zero bytes.
  UniFFI contract v34 rejects impossible Default-enabled projections. Native
  Settings requires an authoritative loaded policy and the exact
  `ENABLE PERMANENT CLEANUP` phrase before its sole enable path; disabling and
  reset-to-safe-default are immediate. Copy and accessibility now present the
  disabled state as normal protection. Public Release execution remains
  compiled out and all §17.3 gates remain open.
- Raised `developer.rust.target` to revision 3 and made inclusive seven-day
  inactivity a deterministic authority requirement. Fresh evaluation and
  retained-snapshot replay now bind the exact scheduled instant, require
  complete file/directory modification-time coverage, emit one exact
  `MinimumAge` fact, and fail closed for recent, future, or missing required
  timestamps. Planning requires the exact four marker/age facts. One shared,
  bounded descriptor-relative no-follow validator inventories the complete
  target tree before publishing a preview and again immediately before
  `effect_started`; the concrete driver repeats that inventory and compares
  per-file identity, type, link count, logical size, and mtime before unlink.
  Pre-preview and pre-effect drift therefore remove nothing. Historical sealed
  revision-2 sessions remain decodable only for safe recovery
  terminalization—new minting and every effect-authority boundary require the
  current revision. UniFFI contract v33 carries every plan timestamp losslessly
  as seconds/nanoseconds; Swift independently validates the newest observation,
  creation time, expiry, and exact seven-day requirement. Explorer
  displays both in the plan card and destructive confirmation while the Release
  cleanup action remains disabled.
- Added bounded startup recovery for durable pending candidate evaluations.
  Core now exposes an eighth idle-only maintenance task that selects at most
  one oldest pending row, replays only its exact retained immutable snapshot,
  and returns only `None`, recovered candidate count, or `Incompatible` plus a
  bounded `has_more` hint. The task accepts no path, scan ID, timestamp,
  evaluator input, catalog, AI output, plan, approval, or cleanup command;
  Rust owns the clock and all replay inputs. Applying linearizes cancellation,
  late cancellation preserves the exact terminal result, invalid clocks and
  malformed/missing state fail closed, and core never self-enqueues. UniFFI
  contract v32 carries the task as a path-free opaque observer with strict
  kind, phase, failure, outcome, count, and unused-field validation. The native
  eight-kind scheduler requests candidate recovery immediately after abandoned
  scan recovery, retains its startup grace, energy gates, fair cadence,
  bounded backoffs, and ordered cancellation on quit. Swift independently
  validates every maintenance result family before scheduling from it.
- Retired the legacy CLI arbitrary-descendant permanent-delete path. The CLI
  remains a supported read-only companion for scanning, navigation, selection,
  computed views, history, and reveal; its former `d` action is inert.
  Confirmation/progress UI, background delete workers, tree/cache mutation,
  the temporary `dux-core::cleanup::legacy_cli` adapter, its raw recursive
  filesystem effects, and all three `legacy-adapter-delete-*` policy
  exceptions are removed. Repository policy and a fixture-backed input
  regression prevent that authority route from returning. Any future CLI
  cleanup must consume the same current, unexpired, reviewed-plan executor as
  the native product. This removes a noncompliant authority edge without
  enabling permanent-safe cleanup in Release.
- Exposed the reviewed Rust-target permanent-safe task through UniFFI contract
  v31. The only start input is the exact engine-bound opaque plan review:
  paths, identifiers, timestamps, approval flags, callbacks, AI output,
  commands, and retry tokens cannot nominate cleanup. Foreign-engine refusal
  leaves the review untouched; every correct-engine attempt is irreversible,
  including an information-read race or a later core admission refusal. The
  opaque task supports explicit cancellation and path-free polling with strict
  phase/failure/result validation and durable history correlation. Dropping it
  neither cancels nor retries work. Generated Swift bindings include the new
  transport. The native service now maps the consuming edge into an app-owned,
  consume-once task without accepting a caller path or reconstructed plan. The
  review controller binds execution to the complete immutable
  information shown in confirmation, removes the exact child before
  suspension, and rejects a same-UUID handle if any displayed plan field
  differs. Explorer generation-fences confirmation and task observation,
  presents queued, running, cancellation, changed-since-plan, partial, failed,
  unknown, and terminal path-free states in a persistent accessible banner,
  never retries, keeps observation alive when the Explorer window closes, and
  requests cancellation plus waits during app shutdown. This execution action
  is compiled only with `DUX_INTERNAL_PERMANENT_SAFE_CLEANUP` in Debug builds;
  Release shows the review preview as unavailable for execution, and the
  notarized-release pipeline rejects resolved Release settings containing that
  condition. The public §17.3 cleanup release gate therefore remains closed.
  Process-quiescence proof revision 2 continues to read a
  bounded complete libproc PID/name table but obtains executable identity only
  for exact guarded `cargo`/`rustc` names; inaccessible guarded identities
  still fail closed, while unrelated applications no longer block planning
  solely because macOS withholds their image path. Zero-length and full PID
  buffers reject as incomplete or possibly truncated. Failed PID inspection is
  skipped only for `ESRCH` or
  when a second complete PID table proves that exact PID disappeared.
  A dedicated production runner compiles the FFI suite, exits Cargo, requires
  exactly one Cargo-reported test binary, and verifies that each exact named
  regression ran and passed once, so the real no-active-Cargo guard can be
  exercised truthfully without a zero-test false green.
- Added the first engine-owned permanent-safe cleanup task for an exact opaque
  Rust-target plan review. Rejected start admission returns the unconsumed
  review; acceptance synchronously consumes and approves the exact child before
  returning. Consume completion rechecks parent liveness and both parent/child
  deadlines after the final authorization revalidation, so release or expiry
  cannot race authority transfer into queued work. The worker alone mints
  session identity/trigger metadata and enters the existing
  journal-fenced, descriptor-relative executor. A shared reservation serializes
  permanent cleanup with Explorer Trash, queued cancellation creates no
  journal, known terminal outcomes return a path-free result, and unknown tasks
  retain their exact session correlation without retry. Ambiguous claims,
  post-claim comparisons, and effect settlements retain their exact live
  capabilities. Quarantine is keyed to the physical store and lasts for the
  process lifetime, so closing and reopening an engine in the same process
  cannot release an owner that durable recovery still considers alive.
  Explorer Trash now has equivalent claim/receipt retention: every post-claim
  admission refusal is durably terminalized before releasing its owner, or the
  exact claim and any minted receipt enter quarantine. Its callback is caught
  after `effect_started`, panic becomes durable `outcome_unknown`, and
  settlement retry cannot invoke Trash twice. Focused regressions cover
  completion-time parent release and expiry, reusable foreign/full/busy
  refusal, queued and running contention, claim/admission/settlement ambiguity,
  Trash panic, same-store reopen, session correlation, drift, and preservation
  of Cargo metadata/source. The permanent-safe capability is core-only in this
  checkpoint: UniFFI and the native app still cannot start it.
- Defined cross-reboot and Windows-unproven cleanup-journal recovery as an
  explicit non-executable refusal. A changed macOS/Linux combined boot scope
  remains indistinguishable from a foreign copied database, while Windows
  process death remains unproven without a qualified host/boot scope. Both
  return no journal claim and change no cleanup or candidate-claim row; PID,
  heartbeat age, lock availability, and durable plan history cannot substitute
  for liveness proof. Deterministic regressions compare the complete mutable
  graph byte-for-byte; the same subprocess test requires `Unknown` when run on
  native Windows.
- Added separately confirmed cleanup-history clearing through UniFFI contract
  v30 and native macOS Settings. Rust prepares one short-lived, engine-bound,
  consume-once preview over the exact validated terminal five-table journal
  graph; commit recomputes its SHA-256 witness under the cleanup exclusion and
  an immediate transaction, then an authorizer permits deletion from only
  those five history tables. Validation, hashing, and deletion are keyset-
  paged with fixed per-page/per-session budgets so retained history does not
  acquire an accidental global deadline. Active and recovering sessions,
  uncertain effects, live claims, scans, candidates, snapshots, settings,
  exclusions, capacity samples, AI insights, and filesystem content remain
  outside the deletion scope. Post-commit failures return success only after
  proving the exact terminal graph is gone, preserve a definite failure only
  after proving it remains unchanged, and otherwise return outcome-unknown
  without retrying. Settings shows the exact terminal-session count and date
  range, requires destructive confirmation, releases an unconfirmed preview
  when dismissed, waits for a confirmed operation during shutdown, and
  performs exactly one read-only history refresh after every terminal result.
  The UI explicitly says this privacy action runs no cleanup, compaction, or
  capacity resample and promises no freed space. Explorer Trash now closes
  every known terminal outcome only after its platform callback returns and
  records an unknown capacity delta; outcome-unknown effects remain recovery
  evidence.
- Bound permanent-safe capacity verification to the exact trusted target
  volume in the private production Rust-target bridge. The approved plan's
  revalidated rule-scope grant now yields a crate-private kernel mount scope;
  the core-owned macOS sampler accepts only the same filesystem ID, mount
  location, and filesystem type returned by `statfs`. Callers cannot nominate
  a path, volume label, identifier, or capacity value. Verification now
  brackets the real effect duration independently from journal timestamps and
  reads the authority clock again after pre-sampling, immediately before each
  live effect witness is rebuilt, so telemetry cannot extend an expired
  approval. Verification still requires matching capacity shape, total bytes,
  source, bounded skew, and a representable signed delta. Sampling failure,
  volume drift, or timing mismatch keeps the history outcome explicitly
  unknown without changing cleanup authority. The existing app-facing plan
  review remains observation-only: no approval, execution, FFI, Swift, CLI,
  scheduler, or AI route was added.
- Added exact, path-free cleanup-session drill-down through UniFFI contract
  v29 and the native Cleanup History view. Selecting one bounded session ID
  from the recent summary feed reloads and independently validates the complete
  stored graph, then shows lifecycle timing, mode/trigger, ordered warnings,
  signed verified capacity change beside the separate plan estimate, accessible
  item/path outcome charts, and ordered rule/item cards. Rust and Swift both
  reject malformed versions, IDs, lifecycle/legacy shapes, ordinals, totals,
  checked estimate sums, status counts, warning order, duplicate session IDs,
  and bounded error categories. AppModel
  generation-fences selection, refresh, retry-read, close, and shutdown races.
  Missing verified capacity remains explicitly unknown rather than zero. No
  paths, evidence payloads, candidate IDs, claims, receipts, history clearing,
  cleanup retry, approval, scheduler, AI, or executor authority crosses this
  observation-only route.
- Added an observation-only Rust-target plan preview through UniFFI contract
  v28 and the native Explorer. Rust derives the exact current target, source
  scan, permanent-safe mode, plan identity, estimate, ordered warnings, and
  effective expiry from one retained Explorer review plus one candidate ID.
  The non-cloneable child exposes only bounded `info` and idempotent `release`;
  it cannot approve, persist, claim, schedule, invoke AI, or execute cleanup.
  Exact-parent session identity and liveness reject same-scan substitution,
  release, drop, or expiry. One engine admits at most one preparation or live
  child, reaps stale children, performs expensive live checks outside
  parent/registry locks, post-validates before publication, and tracks
  preparation, inspection, and lease teardown through bounded close. Current
  paths cross only as lossless bytes plus a byte-derived display. Controls,
  non-UTF-8 bytes, backslashes, and the pinned Unicode-16
  format/default-ignorable set are escaped; Swift compares UTF-8 display bytes
  so canonical-equivalent text cannot substitute for Rust's exact projection.
  The app adapter independently validates the exact rule/candidate/one-path
  shape and short lifetime. Controller and browser generation fences release
  children before parents across candidate, mode, snapshot, refresh, expiry,
  cancellation, and shutdown races. The accessible
  **Permanent-safe plan preview** shows the exact current target, estimate,
  warnings, and expiry with only prepare/check-again/close controls and
  explicit “not approved; no files changed” disclosure.
- Added explicit Direct Cargo discovery enrollment to UniFFI contract v27 and
  native macOS Settings. Users choose one exact direct `cargo` executable;
  static inspection runs no selected bytes and returns a bounded,
  engine-bound, consume-once preview showing the lossless path, executable
  digest, and macOS signature evidence. At most one preview may be live,
  foreign engines cannot consume it, commit consumes it before the core's
  fixed Cargo 1.96.0 version validation, and explicit release or engine close
  destroys it. Settings clearly separates inspection from execution consent,
  distinguishes ad-hoc integrity from CMS evidence, generation-fences late
  presentation, binds confirmation to the exact evidence shown, releases a
  preview that arrives after Settings closes, and separately confirms
  revocation. Confirmed enrollment is a bounded non-cancellable settings
  operation on the utility queue; it is shown as finishing and never
  automatically retried after failure or uncertainty. Any untrusted
  post-mutation response triggers a read-only authoritative status reload and
  visibly blocks another mutation until that reload succeeds. FFI projection
  failures after core reports a successful mutation are explicitly
  outcome-unknown, while pre-mutation core errors retain their typed result.
  Runtime shutdown installs one shared pipeline before its first suspension,
  so concurrent callers wait for the same confirmed mutation and close the
  engine exactly once.
  Enrollment enables deterministic Rust discovery only and adds no candidate,
  plan, approval, journal, cleanup, scheduler, CLI, or AI authority.
- Joined the private Rust-target production acquisition chain to its reviewed
  plan, schema-v12 journal, and descriptor-relative executor in one end-to-end
  macOS regression. The fixture now enrolls the exact direct Cargo executable,
  scans a real one-package workspace, and obtains plan facts only through the
  production engine pipeline; the synthetic facts constructor and Cargo
  revalidation bypass were removed. The journal's legitimate `Discovered` →
  `Planned` candidate transition is accepted only after rebinding the retained
  source to the exact sealed planned session/item graph, complete frozen plan,
  immutable candidate body, sole `ProtectedPath` blocker, scan, evaluation,
  and snapshot before owner claim. Binding failure cannot abandon an active
  session. Success preserves Cargo metadata and source while removing only
  target contents and settling all claims; post-planning manifest drift still
  fails before journal insertion or mutation. The bridge remains crate-private
  and has no FFI, Swift, CLI, scheduler, AI, or app cleanup caller.
- Reconciled the destructive-call checker with every existing reviewed
  filesystem-effect suppression. The production Rust-target `unlinkat` remains
  bound to the descriptor-relative driver and exact primitive; test mutations
  now use unique one-use path/rule/context bindings instead of broad or reused
  annotations.
- Added schema v12's revisioned active-claim seal for the private trusted
  Rust-target cleanup handoff. The seal binds the exact candidate, session, and
  item ordinal, and only the private retained-blocker insertion path can create
  it. Planned, running, and recovering loads reconstruct the special coupling
  only from that seal and re-check the complete candidate body plus its sole
  `ProtectedPath` blocker; ordinary claims, forged seals, moved claims, missing
  seals, extra blockers, and pre-v12 unsealed trusted work fail closed. Schema
  upgrades create no trust. Terminal candidate-claim settlement removes the
  seal by foreign-key cascade, preserving path-free history without retaining
  active authority.
- Added exact, read-only candidate drill-down to the native Snapshot Explorer.
  Selecting a candidate now loads bounded historical path and deterministic
  evidence pages through the retained scan review, presents blockers, safety,
  proposed action, review status, and source facts in an accessible inspector,
  and provides independent previous/next paging without retaining unbounded
  results. Snapshot, candidate, immutable-body, cursor, count, and generation
  checks suppress stale or inconsistent responses; review expiry releases and
  invalidates the whole snapshot, and direct task cancellation clears its
  loading latch. FFI and Swift independently cap encoded/display path values
  and the aggregate page payload; sub-second age evidence remains precise.
  Visible/VoiceOver page status and a named row action make detail navigation
  explicit. The screen states that it uses no AI and creates no plan or cleanup
  authority. The macOS service now expects the actual UniFFI v26 contract,
  handles its complete error enum, and preserves typed cleanup-history
  closed-state errors. Universal Debug/Release builds and the full macOS test
  suite pass.
- Closed the durable Rust-target candidate-claim gap without weakening the
  general blocker rule. The private reviewed-plan handoff now mints an
  insertion-only typed coupling for exactly one revision-2
  `developer.rust.target` permanent-safe item. Candidate history keeps its sole
  `ProtectedPath` blocker, while immutable rule, source-scan, path, evidence,
  byte, timestamp, policy, and review-state facts must still match before the
  atomic planned claim. Schema v12's active seal now preserves that private
  coupling across planned and recovery loads without granting it to ordinary
  plan claims. Every other blocked candidate continues to fail closed; focused
  tests prove both the retained-blocker end-to-end lifecycle and the generic
  rejection with no partial session or claim.
- Added macOS regression coverage for the private Rust-target facts-to-executor
  lifecycle. A test-owned trusted-rule fixture proves the complete candidate →
  planner facts → approval → owner-fenced journal claim → descriptor-relative
  execution path, including marker preservation and completed history. The
  production Cargo/home/protected-root/process/descendant grant chain remains
  covered by dedicated planner tests. Target read-set drift is rejected before
  journal persistence or filesystem mutation, while Cargo manifest revalidation
  remains covered by the production planner tests; the bridge remains
  private and cannot be reached by FFI, Swift, CLI, scheduling, AI, or
  production UI.
- Added a read-only Explorer Candidates view backed by the exact snapshot
  review lease. It presents bounded deterministic candidates with category,
  observed size, safety, and status, and exposes only Select/Clear/Dismiss/
  Restore review intent. Every decision is generation-fenced and explicitly
  states that no cleanup action was performed.
- Added UniFFI contract v26's bounded candidate review-intent endpoint. Swift can
  select, clear selection, dismiss, or restore one candidate inside an exact
  retained scan review; Rust validates the scan/candidate binding and returns
  only the resulting status. This remains observation state: it cannot create
  a plan, approve cleanup, disclose paths, or authorize an effect.
- Added the private promotion-to-`RustTargetPlanFacts` handoff. It repeats
  grant validation while retaining canonical scan-root/target witnesses and
  the unresolved `ProtectedPath` blocker; plan IDs, review, approval, journal,
  FFI, UI, scheduling, AI, and effects remain outside the boundary.
- Added the private live-input-to-promotion join for macOS. It consumes
  enrolled-Cargo metadata plus current-account home/mount, protected-root,
  process-quiescence, and empty-descendant-policy evidence, then returns only
  a non-cloneable promotion token that retains `ProtectedPath`; plans,
  approvals, journals, FFI, UI, scheduling, AI, and effects remain closed.
- Added the first private production-core evaluator-to-planner pipeline for
  the staged Rust-target rule. It acquires only an exact succeeded
  scan/evaluation source, rehydrates the domain candidate from the current
  bundled catalog, compares every immutable body field with durable history,
  and returns a lease-backed live witness plus candidate. It stops before
  Cargo/protected-root grants, plans, approval, journal, FFI, UI, scheduling,
  AI, or effects.
- Added the private Rust-target facts-to-executor bridge. It performs the
  final open-engine check, uses the canonical approved-plan/journal handoff,
  and passes only the non-cloneable claimed session to the existing
  descriptor-relative permanent-safe executor. No paths, callbacks, AI/CLI
  input, FFI values, or UI route can reach it; production orchestration remains
  gated.
- Added a private Rust-target promotion checkpoint that binds the blocked
  candidate to the full retained durable discovery body and a revalidated
  Cargo/home/protected-rule grant. The non-Clone token remains non-actionable:
  it cannot clear `ProtectedPath`, construct or approve a plan, persist,
  schedule, cross FFI, or mutate; generic exact review still fails closed.
- Added the next private Rust-target `RustTargetPlanFacts` capability. It
  consumes promotion only after immediate grant revalidation and retains the
  original blocked candidate plus canonical scan-root/target witnesses; no
  plan, approval, journal, FFI, schedule, or effect authority is exposed.
- Added a private facts-to-plan join that constructs a permanent-safe domain
  plan only from `RustTargetPlanFacts`, repeats mode/source/overlap/byte
  validation, and returns the retained authorization alongside the plan. It is
  not yet connected to review, approval, journal, FFI, scheduling, or effects.
- Added a private Rust-target plan handoff into `TrustedReviewedCleanupPlan`.
  It requires one exact item/path, revalidates the retained grant again, and
  remains unreachable from UI, FFI, scheduling, journal, and filesystem effects.
- Added a private expiry-checked approval handoff for Rust-target facts. It
  reuses the reviewed-plan approval capability and still stops before journal
  claims, FFI, scheduling, and filesystem effects.
- Added a private approved Rust-target → journal handoff using the existing
  canonical-time planned-session persistence and owner-fenced claim path. It
  still stops before executor effect admission.
- Promoted current-account home/mount and protected-root observations into
  private revisioned planner grants with stable rule-boundary keys. Grants
  revalidate exact policy, volume, and target evidence while `ProtectedPath`
  and all plan/effect authority remain closed.
- Made exact inactive `cargo` and `rustc` process guards mandatory for the
  private Rust-target boundary. The bounded libproc witness fails closed on
  active, incomplete, malformed, replaced, or wrong-guard observations and is
  revalidated at each reuse; no cleanup authority is exposed.
- Added explicit descendant-policy coverage to the Rust-target boundary. The
  current no-selector catalog carries a revalidated empty witness, while
  non-empty selectors are rejected until they have rule-specific policy and
  executor proof.
- Added a private Cargo-bound Rust-target planning grant. It joins the exact
  Cargo/read-set witness, source candidate identity, target snapshot,
  current-account home/mount boundary, and protected-root no-textual-match
  assessment with repeated fail-closed revalidation. The grant remains
  non-actionable: it cannot clear `ProtectedPath`, create a plan, approve,
  schedule, cross FFI, or perform cleanup.
- Added the first bounded restart-recovery seam for pending deterministic
  candidate evaluation. Core selects at most one oldest pending row, replays
  only its checksummed immutable snapshot, verifies evaluator/catalog/context
  identity and materialization limits, and atomically records a terminal
  result. Incompatible state fails closed; no path, plan, approval, or cleanup
  effect crosses the seam. Startup scheduling and FFI/Swift wiring remain
  intentionally separate.
- Added the native Explorer Cleanup History destination. AppModel now
  generation-fences bounded history loading and pagination, while the UI shows
  honest lifecycle/mode/trigger/estimate summaries and accessible item-status
  distribution bars without exposing paths or offering repeat-cleanup actions.
- Added UniFFI contract v25's bounded, newest-first cleanup-history summary
  feed. EngineService now exposes path-free lifecycle, mode/trigger, estimate,
  optional verified capacity delta, cancellation, and status-count observations
  with strict cursor/record validation; paths, evidence, plans, approvals, and
  executor authority remain sealed.
- Preserved reviewed Explorer Trash changes as a distinct `ChangedSincePlan`
  outcome through core, UniFFI contract v24, and the native UI. DUX now tells
  the user to rescan before retrying instead of reporting a generic review
  failure; no stale plan or effect is retried.
- Added UniFFI contract v23 cleanup exclusions. The bounded deny-only path
  prefix set crosses the boundary as lossless encoded observations with typed
  validation and storage errors; Settings can add a local prefix and requires
  explicit confirmation before removing one or resetting the set. Exclusions
  never grant cleanup authority.
- Added the native Swift presentation for the contract-v22 permanent-cleanup
  kill switch. EngineService maps typed Rust errors and rejects malformed
  versioned status, AppModel generation-fences load/mutation/reset work and
  invalidates it at shutdown, and Settings exposes an accessible deny-only
  control. Disabling is immediate; re-enable and reset require typing
  `ENABLE PERMANENT CLEANUP`. Path-bearing exclusions remain separate.
- Fixed menu-bar popover dismissal terminating DUX. The app bundle now opts out
  of automatic termination, while the one balanced launch lease reasserts that
  opt-out if AppKit resets the ProcessInfo flag while tearing down and restoring
  the transient popover scene.
- Added UniFFI contract v22's path-free permanent-cleanup kill-switch
  get/set/reset records and typed errors. Rust remains the only semantic
  validator; the switch can deny effects but cannot create plans or carry
  cleanup authority. Regenerated the universal Debug bindings.
- Integrated the private bounded pre/post capacity verifier with approved
  permanent-safe session orchestration. Verified signed available-space deltas
  are persisted only for matching stable-volume evidence; missing or conflicting
  telemetry stays unknown, and outcome-unknown sessions remain recoverable.
- Added a crate-private multi-path permanent-safe session orchestrator. It
  keeps one journal owner/generation fence across ordered paths, continues
  after settled failures, stops conservatively for unknown outcomes, and
  records cancellation/partial terminal states only after remaining paths are
  settled. Prefix validation prevents switching to a different target between
  durable validation and live witness capture; no FFI, Swift, CLI, AI, or
  production cleanup caller can reach this boundary.
- Added the first crate-private engine bridge for approved permanent-safe
  sessions. It accepts only the non-cloneable journal capability, selects the
  descriptor-relative Rust driver, preserves Cargo cache markers, and records
  terminal cleanup history in a project-local end-to-end fixture; no FFI, Swift,
  CLI, AI, or production UI caller can reach it. The handoff now canonicalizes
  millisecond start times and verifies the exact `validating` path before live
  witness capture, closing a sub-millisecond journal-claim gap.
- Added a revisioned global permanent-cleanup kill switch. Missing state is
  enabled by default, explicit disable/enable/reset operations are serialized
  with the store-wide cleanup exclusion, and the journal rechecks the switch
  immediately before a permanent effect can enter `effect_started`. Corrupt or
  newer settings fail closed; no arbitrary path exclusion or UI/FFI control is
  exposed yet.
- Completed the first user-facing Explorer Trash path. A retained snapshot
  review now feeds a fixed review-required one-item plan, a journal-fenced
  one-shot core callback, and the macOS Foundation adapter only after an
  explicit confirmation. UniFFI contract v21 carries bounded target metadata
  and outcomes; foreign reviews, stale identities, storage failures, and
  unknown Foundation results fail closed. Trash is never emptied automatically,
  and AI/CLI/scheduled paths cannot invoke this action.
- Added a bounded lossless user cleanup-exclusion set. Exact absolute lexical
  prefixes are sorted, deduplicated, revisioned, and persisted without
  canonicalization; the journal reloads them under the cleanup exclusion and
  refuses matching targets before `effect_started`. Exclusions are deny-only
  and do not bypass protected-path or live identity checks.
- Added a private bounded pre/post cleanup-capacity verification boundary.
  Stable volume identity, sample timing, total capacity, headline source, and
  availability shape must agree before a signed available-space delta can be
  journaled; unknown evidence stays unknown and never becomes an estimate.
  The boundary is staged for later executor/UI/FFI integration and has no
  filesystem authority.
- Fixed menu-bar scene teardown eligibility. DUX no longer opts its agent process
  into AppKit automatic termination, and its repeated scene reassertions keep
  every counter-based opt-out balanced, so opening and closing the popover cannot
  make DUX disappear as if it crashed.
- Hardened the permanent-safe executor admission lifecycle. Journal validation
  is durable before live Rust-target evidence is rebuilt, and changed targets,
  stale approvals, or journal failures now receive bounded terminal validation
  outcomes instead of remaining in `Planned`; no filesystem effect starts on
  those rejection paths.
- Bounded interactive Home scans to 200,000 retained nodes and a single
  traversal worker. The scanner now limits children before they enter jwalk's
  queue and returns a truthful partial `IssueLimitReached` result at the bound,
  preventing large Home scans from retaining unbounded tree/path state. CLI and
  explicitly configured library scans keep their existing defaults.
- Added a private, descriptor-relative permanent-safe contents driver for the
  reviewed Rust-target rule. It inventories before mutation, preserves the
  direct `CACHEDIR.TAG`, rejects unsafe descendants and hard links, rechecks
  ancestor identities before each unlink, and maps cancellation/partial
  failure to conservative journal outcomes. It remains unexported and has no
  production UI, FFI, or real cleanup caller.
- Added notification-response routing to the native Explorer. Only the exact
  bounded versioned Recommendations payload is accepted; valid responses open
  and focus the review-only Recommendations destination, while malformed or
  unknown payloads fail closed. The surface makes clear that scans are
  read-only and no AI or notification callback can delete files.
- Added transition-gated low-disk notifications. Newly stored Warning/Critical
  pressure changes preserve the prior durable level, use a bounded versioned
  Recommendations payload, and respect independent 24-hour per-volume and
  per-urgency cooldowns. Cooldowns are recorded only after delivery succeeds;
  repeated samples and delivery failures do not alter authoritative capacity
  state. The notification suggests safe review and carries no cleanup authority.
- Added a private Rust-target permanent-safe evidence boundary. Approved
  plan authorizations now retain deterministic item/path order, and the
  executor-facing witness revalidates the exact target, Cargo manifest,
  cache-tag signature, hard-link policy, and no-follow ancestry without
  clearing the production `ProtectedPath` blocker or performing deletion.
- Added the first native capacity trend presentation. The long-lived app
  model refreshes the bounded trend after accepted capacity samples, fences
  late or volume-mismatched responses, and clears completed/cancelled loads.
  The menu-bar popover now shows signed 24-hour/7-day available-space changes
  plus an accessible 30-day sparkline when history is ready; missing history
  remains explicitly warming up. This remains telemetry-only and carries no
  scan, plan, approval, notification, or cleanup authority.
- Added schema v11 durable pressure episodes. Warning/Critical entries,
  Healthy recovery, escalation, and policy-revision boundaries are written
  atomically with raw capacity samples. Unknown pressure never opens or
  claims recovery; exact retries remain idempotent; bounded readers reject
  malformed or overlapping history. Episodes are telemetry only and remain
  outside FFI, notifications, and cleanup authority.
- Added a bounded core capacity trend contract with signed 24-hour and 7-day
  changes plus one validated point per UTC day over a 30-day window. Daily
  rollups are preferred for completed days, the current raw anchor wins for
  today, missing baselines remain unknown, and important-usage deltas require
  both endpoints. The core contract is path-free telemetry.
- Exposed capacity trends through UniFFI contract v20 and the off-main-actor
  Swift service adapter. Requests remain stable-ID/time-only; responses retain
  optional baselines, signed deltas, source-labeled points, and bounded
  ordering validation. Generated bindings and the real Rust FFI round trip are
  covered; no cleanup authority is carried.
- Added the first private approved-plan to cleanup-journal handoff. It
  revalidates the expiring approval, persists one planned session, compares
  the frozen plan before and after an owner/generation-fenced journal claim,
  and retains the approved capability with the non-cloneable claim. No path,
  callback, FFI, scheduling, or filesystem-effect authority is exposed.
- Added a private pre-effect revalidation witness for the approved journal
  session. It rechecks expiry, every retained trusted grant, and the exact
  frozen plan under the owner claim, but returns no target or effect
  capability; permanent execution remains unavailable.
- Staged a private, bounded macOS process-activity witness for future rule
  guards. It reads libproc directly, retains PID/start-time/executable
  identity, rejects active/malformed/incomplete observations, refuses bundle
  guards without signed bundle identity, and revalidates before use. The
  current catalog has no activity guards, so no blocker, plan, schedule, or
  cleanup authority changed.
- Staged a private, bounded exact descendant-policy witness. Protected and
  excluded selectors are component-aware, no-follow, identity-bound, and
  fail closed on missing, overlapping, multiply-linked, symlinked, replaced,
  or removed entries. The optional rule-boundary join revalidates it, while
  recursive ownership, plans, approvals, FFI, schedules, and cleanup effects
  remain unavailable.
- Added the first trusted deterministic-rule scope authorization. It is
  macOS-only, allowlists only the revision-2 Rust-target and Python cache
  rules, binds current-account home/mount and protected-root evidence, and
  revalidates one exact target identity. It remains path-private and cannot
  create plans, clear `ProtectedPath`, persist, cross FFI, schedule, or mutate.
- Added the sealed permanent-safe plan-construction checkpoint. Exact review
  now retains selected candidate facts and consumes one matching trusted scope
  authorization per live target into a private domain `CleanupPlan`; mode,
  overlap, and candidate validation remain fail-closed. The plan retains its
  authorization tokens for future executor revalidation and has no approval,
  journal, FFI, scheduling, or effect path.
- Staged UniFFI contract v19's core-issued, one-shot Trash callback request.
  The request has no public constructor, exposes only the exact ephemeral path
  bytes and target kind to a future synchronous platform callback, and can be
  consumed once. No plan, approval, journal, callback registration, or
  filesystem mutation is exposed yet.
- Added the inert Swift callback-side request adapter. It strictly validates
  core-issued Unix path bytes before URL construction, maps malformed requests
  to `Failed`, maps Foundation throws to `OutcomeUnknown`, and remains
  unregistered until reviewed-plan approval exists.
- Exact-path review now retains the repeated filesystem-boundary witness for
  its scan root and revalidates ancestry and mount identity before publishing
  review evidence. Boundary drift fails closed; this remains observational and
  does not grant volume/location or cleanup authority.
- Added the crate-private, non-cloneable `TrustedVolumeLocationWitness`
  boundary. It is minted only from a canonical scan-root witness, retains
  repeated ancestry and kernel mount identity, rejects macOS alias/firmlink-like
  spelling ambiguity and Linux device-only mount evidence, and fails closed on
  unsupported platforms. It remains observation-only and cannot clear
  `ProtectedPath` or authorize planning or effects.
- Closed the current-account home/mount observation gap without adding
  authority: Unix boundary capture now retains owner identity from the same
  no-follow descriptor as ancestry and mount data, and macOS has a
  non-cloneable `TrustedHomeMountWitness` requiring exact home ancestry and
  mount continuity. Account/home evidence and both boundaries are reread on
  revalidation; Linux and Windows fail closed for this profile. ProtectedPath,
  planning, FFI, scheduling, and effects are unchanged.
- Added a consumed, path-private `RustTargetRuleBoundaryEvidence` join for
  the Rust/Cargo rule. It binds retained Cargo provenance to the exact macOS
  current-account home-mount boundary, revalidates both evidence chains, and
  preserves the unresolved `ProtectedPath` marker. It cannot create a plan,
  clear blockers, cross FFI, schedule, or invoke an effect.
- Cargo metadata witnesses now retain every read-set guard, the descriptor-
  retained project directory, exact executable/version observation, optional
  enrollment guard, and filesystem boundary after publication. Private
  revalidation rejects post-publication manifest, configuration, Cargo, live
  target, or boundary changes. This closes stale-evidence risk without adding
  planning, approval, FFI, or cleanup authority.
- Added a consumed, path-private Cargo planning-provenance token. It binds
  source-scan/candidate identity, witness and resolution revisions, and the
  unresolved protected-path marker, while retaining only revalidation and
  explicit lease release operations. It cannot create plans, approvals, FFI,
  schedules, or cleanup effects.

All notable changes to DUX will be documented in this file.

## [Unreleased]

### Fixed
- Restored the documented automatic-termination contract for the menu-bar
  agent. The lifetime lease now enables support before taking one balanced
  opt-out, while repeated popover-close notifications only restore that flag;
  closing the transient popover cannot retire the process or accumulate
  unbalanced termination leases.
- Made menu-bar lifetime reassertion idempotent. Repeated popover close and
  scene-resignation notifications now restore AppKit support without leaking
  automatic-termination disable leases.
- Moved the initial engine and volume load out of the transient menu-bar
  popover task and into the long-lived app runtime. Dismissing the popover no
  longer cancels startup work or causes repeated opens to duplicate it.
- Hardened popover teardown lifetime handling again: transient window close,
  key resignation, application resignation, last-window callbacks, and
  incidental termination requests now reassert the AppKit lifetime lease both
  immediately and after the scene teardown run-loop turns. This prevents the
  menu-bar process from disappearing when the popover is opened and closed.
- Hardened the menu-bar app lifetime lease to restore
  `automaticTerminationSupportEnabled` before reasserting the disable lease.
  This prevents AppKit's transient `MenuBarExtra` teardown from terminating
  DUX when the popover is closed.

### Added
- Completed the deterministic candidate-grouping boundary used by exact-path
  review. Stable candidate IDs select equivalent duplicates, component-aware
  parent ownership coalesces only same-rule findings, and every ambiguous or
  malformed overlap remains unresolved with zero actionable bytes. The review
  retains the grouping witness instead of recomputing selections downstream.
- Completed the exact-path plan-review boundary. Bounded no-follow evidence,
  protected-policy observations, deterministic grouping, matching rule-scope
  grants, frozen expiry, and explicit approval are now joined in private
  planner capabilities. They remain non-Clone and cannot persist, cross FFI,
  schedule, or mutate; those authorities stay in the executor milestones.
- Exact-path reviews now retain the immutable candidate-grouping and overlap
  witness that selected their items, preventing downstream reconstruction from
  a reordered candidate slice while keeping approval and execution sealed.
- Added a crate-private, expiry-bound approval capability for trusted
  permanent-safe plans. Approval revalidates retained rule-scope grants and
  still cannot reach persistence, FFI, scheduling, or filesystem effects.
- Approved plans can now persist a bounded planned cleanup session/item history
  record after a final expiry and grant revalidation. The stored observation
  carries no approval or execution authority.
- Continued Milestone 5 with journal-fenced, one-shot core Trash admission.
  The opaque Explorer witness must match the exact frozen journal path, then
  repeats no-follow identity checks, records and immediately revalidates an
  owner/generation `effect_started` receipt, and can be cancelled before any
  platform call. Errors are typed and path-free. This slice performs no
  filesystem mutation and does not add approval, FFI, Swift, or
  `FileManager.trashItem` authority.
- Continued Milestone 5 with a private synchronous Trash platform-driver seam.
  The consuming admission repeats its no-follow target and durable receipt
  checks immediately before the driver, records `Trashed`, `Failed`, or
  conservative `OutcomeUnknown` while the journal claim is held, and returns
  bounded path-free errors. Recording adapters verify exact-path delivery,
  no mutation, single-call behavior, and no retry after an unknown outcome;
  no Swift/FFI caller or real platform effect is connected.
- Continued Milestone 5 with an internal macOS Trash adapter contract. The
  Foundation dependency is injected behind a narrow `TrashFileManaging`
  protocol; a successful synchronous `FileManager.trashItem` maps to success,
  while every thrown Foundation result maps conservatively to path-free
  `OutcomeUnknown`. Fake-only tests verify exact URL delivery, no mutation, one
  call, and no retry. UI/FFI wiring and real cleanup remain gated on the
  core-owned callback.
- Continued Milestone 5 with a separate core-owned Trash review witness for
  Explorer selections. A retained snapshot node is resolved to fresh no-follow
  ancestry and target identity; final symlinks remain link objects while
  symlinked roots/intermediate ancestors, special entries, missing targets, and
  replacements fail closed. The witness never crosses the read-only live-target
  FFI record and cannot approve, plan, journal, or invoke a platform effect;
  the centralized executor and macOS Trash adapter remain open.
- Continued Milestone 5 by binding code-owned protected-root policy into
  exact-path review. Requested and canonical target forms are both assessed;
  hard denies fail closed, specific-rule requirements remain explicit
  non-actionable metadata, and no-textual-match retains its policy revision
  without becoming an allow. Registry construction/assessment failure also
  fails closed; no volume grant, approval, plan, FFI, or executor capability was
  added.
- Continued Milestone 5 with code-owned current-account home discovery for the
  protected-root text policy on Unix/macOS. The resolver uses the OS account
  database rather than mutable environment variables, rejects real/effective
  UID ambiguity and lossy/relative homes, captures no-follow home identity
  twice, and checks final-directory ownership. It remains non-authoritative:
  no protected-root grant, plan, FFI, approval, or cleanup effect was added;
  Windows known-folder/reparse evidence remains unsupported.
- Continued Milestone 5 with a private repeated filesystem-boundary witness
  for future protected-volume planning. Unix captures the complete no-follow
  root-to-scan ancestry; macOS records descriptor-bound `fstatfs` identity and
  mount location; Linux requires descriptor-relative `statx` mount identity
  plus filesystem statistics; unsupported Windows remains fail-closed. The
  bounded evidence can be revalidated but does not issue a trusted grant,
  clear `ProtectedPath`, cross FFI, construct a plan, or authorize cleanup.
- Continued Milestone 5 with a crate-private no-follow final-link witness for
  future Trash review. Unix/macOS captures preserve the selected symlink's own
  identity and lexical object path, including dangling and loop links, without
  resolving or reading the target; root/intermediate symlinks and special
  entries fail closed. Windows remains unsupported at this evidence boundary,
  and no plan, approval, FFI, journal, or mutation authority was added.
- Continued Milestone 5 with a sealed planner-owned exact-path review
  evidence boundary. A code-owned canonical scan-root observation is required;
  selected candidates are grouped and coalesced deterministically, then each
  strict descendant is checked with lossless lexical validation and no-follow
  live identity snapshots. The review retains requested/canonical/relative
  paths, target kind, volume/object identity, ancestor identities, hard-link
  count, rule facts, estimates, and warnings. Blockers, incompatible modes,
  unresolved overlap, invalid scope, missing/symlink/special targets,
  multiply-linked permanent files, and arithmetic overflow fail closed. The
  result is deliberately non-Clone, non-serializable, non-actionable evidence:
  trusted protected-root/volume grants, approval, plan construction,
  persistence, FFI, and mutation remain unavailable.
- Continued Milestone 5 with a sealed core candidate-grouping and overlap
  result. Deterministic groups use category, safety tier, and proposed action;
  equivalent same-rule observations coalesce by stable candidate ID, and a
  same-rule parent owns a child only when it covers every child path. Blocked,
  mixed-rule/policy, conflicting-fact, and internally overlapping observations
  remain unresolved with zero actionable bytes. The result is non-persistent,
  non-FFI, non-approving, and non-executable; exact-path plan review remains
  the next boundary.
- Continued Milestone 5 with the independently researched Python
  `developer.python.pycache` rule at revision 2. A direct regular, non-symlink
  `.py` sibling and symlink-free ancestry are required before the immutable
  classifier proposes SafeRegenerable/RemoveKnownRegenerableContents. The
  catalog/build digest now allowlists two safe rules, but both remain
  unschedulable and every candidate remains blocked by ProtectedPath. Python's
  import reference, FAQ, and PEP 3147 are recorded in
  `docs/rules/developer-python-pycache.md`; no live writer witness, plan,
  executor, AI, or FFI cleanup authority was added.
- Continued Milestone 4 with FFI contract v18 progressive scan events. The
  existing bounded task-event ring is now transported with each scan poll as
  typed, sequenced, path-free observations plus an explicit last-delivered
  cursor, oldest-retained sequence, and sticky truncation signal. The Swift
  adapter rejects malformed versions, sequence/cursor regressions, impossible
  terminal transitions, and candidate-event shapes before publishing an
  app-owned event model. AppModel retains at most the latest 64 events per scan
  generation and preserves them through terminal results. Internal maintenance
  events are deliberately coalesced and remain non-authoritative; no event
  grants path, AI, planning, or cleanup authority.
- Continued Milestone 5 with policy-2 exact replay for Cargo's potential
  ancestor-manifest namespace. On macOS DUX captures an event cursor before
  discovery, arms one bounded ephemeral FSEvents history replay for the
  farthest absent-candidate directory, and requires history completion,
  monotonic event IDs, and volume-UUID continuity before advancing the cursor.
  Exact `Cargo.toml` candidates are matched with the mounted volume's
  case-sensitivity semantics; candidate-file writes and directory
  delete/rename/revoke events remain terminal, while unrelated sibling
  activity is ignored. Dropped/coalesced, wrapped, unknown, or incomplete
  event coverage fails closed, and kqueue `NOTE_WRITE` is treated as a
  namespace replay request rather than trusted as a complete observation.
  Absent ancestor create/remove during discovery is now terminal under
  resolution policy 12. This remains path-based stability evidence rather
  than kernel-level proof of Cargo's actual reads, and adds no cleanup or
  `ProtectedPath` authority.
- Continued Milestone 5 with independent Cargo 1.96 package README and
  `license-file` provenance. Package-metadata policy 1 derives direct,
  suppressed, `true`, and workspace-inherited declarations from the exact
  retained TOML 1.1.2 manifest bytes and requires Cargo's reported relative
  strings to match exactly. For an absent `readme`, DUX binds Cargo's ordered
  `README.md`, `README.txt`, and `README` lookup, including directory-versus-
  regular-file state; symlinks, hard links, special entries, escapes, and
  out-of-workspace paths fail closed. Local-APFS package-root generation
  watches make even create/remove restoration terminal across the accepted
  pass. Explicit README/license targets are declaration evidence only because
  `cargo metadata` does not require or read those files. Resolution policy 11
  binds package, manifest, declaration, probe, selection, identity, and
  closure evidence. No planning, scheduling, execution, or `ProtectedPath`
  authority was added.
- Continued Milestone 5 with independent local path-dependency provenance from
  the exact fenced workspace manifests. Dependency-manifest policy 1 parses
  Cargo TOML with the pinned 1.1.2 generation, derives direct and
  workspace-inherited `path` entries across normal, development, build, and
  target-specific tables, preserves duplicate owner-to-target edges, and
  requires an exact match with Cargo's reported local dependency multiset.
  Every derived target must be one of the canonical single-link manifests
  already retained by the workspace guard. The profile is capped at 4,096
  local declarations and 256 KiB of path text; malformed, escaping,
  unreported, invented, omitted, or unsupported inheritance fails closed.
  Resolution policy 10 binds the independent counts and domain-separated
  closure beside the existing reported-edge evidence. Remote dependency
  semantics, README/license probes, kernel read identity, and all cleanup
  authority remain outside this slice; `ProtectedPath` is unchanged.
- Continued Milestone 5 with pre-discovery Cargo 1.96 workspace-glob
  provenance. DUX now parses the exact root manifest, reproduces bounded glob
  0.3.3 member/default-member expansion while retaining Cargo's literal
  exclude behavior and raw-file fallback distinction, and fences every
  conservatively consulted local-APFS directory through both metadata passes.
  Literal components use native targeted lookup and parent-generation watches,
  duplicate default-member rows and recursive derivations retain Cargo's
  order/multiplicity, and workspace roots with glob metacharacters fail closed
  when a declaration actually invokes glob expansion.
  Streaming directory, pattern, path, recursion, match, and comparison bounds
  fail closed; symlinks, special entries, escapes, and malformed patterns are
  unsupported. A reported-graph consistency check requires expanded/root
  seeds to reach every package through Cargo's serialized local path graph and
  reproduces default-member IDs exactly. It rejects disconnected output but
  does not independently authenticate dependency declarations from manifest
  bytes. Resolution policy 9 binds policy-1 namespace and consistency
  evidence. No cleanup authority or `ProtectedPath` change was added.
- Continued Milestone 5 with bounded Cargo 1.96 reported-package
  target/source/build discovery provenance. Every package must expose its
  complete target array; DUX binds reported sources and the conventional lib,
  bin, example, test, bench, build-script, nested `main.rs`, and edition-2015
  fallback namespaces under explicit package, target, kind, text, path, record,
  and descriptor limits. Local-APFS directory writes and source identity
  changes are terminal across the accepted pass. Resolution policy 8 records
  target-namespace policy 1 evidence without granting cleanup authority.
- Continued Milestone 5 with a closed, bounded Cargo 1.96 path-dependency
  graph for accepted metadata. Every package must expose its dependency list;
  local-source declarations require an absolute, normalized, control-free
  path whose exact `Cargo.toml` is already one of the reported workspace
  package manifests guarded during the second pass. DUX rejects malformed
  source/path pairs, more than 4,096 dependency declarations, more than
  256 KiB of aggregate local-path text, and every unreported local target.
  Path-dependency policy 1 records total/local/unique counts and a
  domain-separated digest of the sorted, duplicate-preserving owner-to-
  manifest edges; resolution policy 7 binds that evidence across identical
  metadata passes. Real pinned-Cargo tests accept an internal implicit member
  and reject an unreported false-target optional build dependency. This is a
  conservative accepted-profile rule, not attestation of an external
  manifest: discovery may read an external manifest before DUX rejects its
  output, while standalone or excluded declarations may be rejected even when
  Cargo did not read them. No cleanup authority or `ProtectedPath` change was
  added.
- Continued Milestone 5 with a bounded Cargo 1.96 ancestor-manifest probe
  namespace captured before metadata discovery. DUX reproduces Cargo's exact
  nearest-to-farthest candidate ordering plus `target/package` and Cargo-home
  stop rules, binds every candidate directory and present `Cargo.toml`, and
  captures present UTF-8 single-link regular manifests under 64-probe,
  4-MiB/file, 64-MiB aggregate, and path bounds. On macOS, local-APFS file
  fences make present-manifest write/restore and candidate-directory identity
  events terminal; directory entry-write events cause exact namespace
  revalidation so persistent create/remove is rejected without failing on
  unrelated restored high-ancestor activity.
  Resolution policy 6 records probe policy 1, counts, bytes, and a
  domain-separated closure digest across both metadata passes. A transient
  absent-entry create/remove remains unproven, as do attestation of unreported
  path-dependency manifests and kernel-level Cargo read identity. No cleanup
  authority or `ProtectedPath` change was added.
- Continued Milestone 5 with positive, bounded Cargo 1.96 configuration and
  include provenance. DUX discovers accepted project/ancestor config roots,
  parses only their top-level include declarations with Cargo's exact TOML
  1.1.2 parser generation, and captures every admitted single-link regular
  file from one retained descriptor with strict file, graph, depth, aggregate,
  and path bounds. Configuration policy 3 binds identities, full bytes,
  include edges, root/read order, and exact lookup/watch semantics. Both fixed
  metadata passes require the exact reviewed `30a34c6821b57de0aaec83a901aca39f88f6778c`
  Cargo commit, enable its pinned `cargo::util::context` trace, and require
  its complete ordered load intent to equal the independent closure; missing,
  reordered, extra, malformed, and spoofed records reject. Exact local-APFS
  file and ancestry vnode fences remain live throughout both passes, and
  resolution policy 5 records the closure and intent evidence. Cargo-home
  configs, ambiguous dual filenames, missing optional includes, aliases,
  cycles, and unsupported paths fail closed. This is path-intent and
  reviewed-filesystem stability evidence, not kernel proof of Cargo's exact
  open file descriptors or its complete manifest/namespace read set. The
  pinned `metadata --no-deps` path deliberately does not read or create
  `Cargo.lock`; real-Cargo tests lock that version-specific assumption. No
  cleanup authority or `ProtectedPath` change was added.
- Continued Milestone 5 with guarded two-pass Cargo workspace-manifest
  provenance. DUX strictly validates a bounded non-empty one-to-one local
  package/member declaration, includes virtual roots, captures every exact
  single-link descendant `Cargo.toml`, and binds role, opaque member ID, native
  path, identity, length, and full SHA-256 in manifest policy 1. An identical
  second metadata command runs behind local-APFS file/complete-ancestry vnode
  fences integrated with launch/config polling and a reserved descriptor-budget
  preflight. Resolution policy 4 records counts,
  closure and accepted-output digests. Real virtual-workspace and adversarial
  coverage exercises malformed relations, aliases, bounds, second-pass drift,
  and write/restore. This attests reported manifests rather than Cargo's full
  read set or glob namespace and adds no cleanup authority.
- Continued Milestone 5 with macOS selected-running-code continuity for the
  enrolled Cargo executable. Production arms local-APFS vnode fences on the
  exact file and every canonical ancestor, uses direct `posix_spawn` with the
  child stopped before user-space, installs the retained cwd by file action,
  and verifies the stopped process identity, generation, group, credentials,
  cwd, dynamic Security.framework validity, and selected enrolled Code
  Directory hash before `SIGCONT`. Full executable digest/identity and all
  launch/config fences bracket resume, bounded output, exact process-group
  termination, and reaping. Resolution policy 3 records launch-policy revision
  1 and a digest of the running hash. Tests cover wrong images, descriptor
  leakage, closed standard descriptors, in-place writes, ancestor renames, and
  no execution before DUX's own resume absent external signaling.
  This remains path-based event-backed continuity rather than fd-based exec or
  confinement; same-UID signaling is an explicit limitation and no cleanup
  authority or `ProtectedPath` change was added.
- Continued Milestone 5 with a bounded negative Cargo 1.96 configuration
  closure and descriptor-retained metadata cwd. Production rejects any
  `config` or `config.toml` in Cargo's cwd-ancestor and Cargo-home lookup set,
  bounding and digesting the lookup state observed empty at each checkpoint.
  Configuration policy 2 binds the corrected watch semantics. Identity-bound
  close-on-exec macOS vnode watches make project-root and existing
  non-Cargo-home `.cargo` entry changes terminal while Cargo runs; exact
  before/after absence covers higher missing lookups and Cargo home without
  false failures from unrelated ancestor/cache writes. The child enters a
  retained no-follow project directory through `fchdir`, and path/descriptor
  continuity is rechecked around launch. Configured projects fail before
  metadata execution. Non-local, non-APFS, and unprobeable lookup directories
  reject. This remains reviewed-filesystem inference and discovery evidence
  only. Positive config reads remain open, and `ProtectedPath` is unchanged.
- Continued Milestone 5 with explicit revisioned trust enrollment for one
  direct macOS Cargo 1.96.0 executable. A read-only, non-cloneable preview
  executes no selected bytes and binds the canonical single-link file, full
  digest, scrubbed environment identities, and strict bounded
  Security.framework static-code evidence. Commit consumes that same-engine,
  same-settings-revision preview, revalidates it, and explicitly authorizes a
  bounded verbose-version execution before the exact version digest and
  enrollment are conditionally written. Exact retries are no-ops,
  replacements advance the revision, and
  revocation retains a tombstone that rejects stale previews. Ad-hoc signing
  is identified as byte-integrity evidence only and gains local trust solely
  through explicit enrollment. The sealed metadata entry derives Cargo from
  the durable source's own store, proves the current digest/signature and
  rereads that exact enrollment before any automatic execution, then brackets
  metadata with the same guard. No FFI, blocker removal, plan, schedule, or
  effect was added; positive config/include provenance, path-independent Cargo
  launch, and remaining swap/restore exclusion remain required.
- Continued Milestone 5 with an exact full-batch evaluator replay inside the
  retained Rust-target source boundary. A snapshot-native single pass shares
  the catalog's marker declarations and final candidate policy without
  reconstructing another full-path tree; bounded directory summaries and
  bottom-up frames reproduce marker preference, cross-rule ancestor
  suppression, known allocation estimates, newest directory/file times, and
  the exact 4,096/4,097 failure boundary. Result paths are charged
  incrementally against the existing 32 MiB durable-batch budget. Every
  immutable field of every
  durable candidate must match the ID-keyed replay, so forged size/time,
  reordered evidence, omitted rows, and injected rows fail closed while the
  charged cleanup-review lease is held. Replay returns no candidate or
  capability, releases memory/pin ownership on failure, and still cannot clear
  `ProtectedPath`, create a plan, cross FFI, schedule, or execute.
- Continued Milestone 5 by sealing Rust-target validation to one exact durable
  discovery source. Production acquisition now requires a succeeded complete-
  coverage scan, its exact checksummed snapshot, the current evaluator/catalog/
  context identity, and the exact still-discovered deterministic candidate. A
  non-cloneable source owns a bounded cleanup-review lease, uses the charged
  decoded-review budget and index to verify the snapshot graph, carries
  scan-time Unix observations for the root, complete ancestor chain, target,
  manifest, and tag, and is retained through both live and Cargo witnesses.
  Fresh lease and complete-source checks bracket acquisition and revalidation;
  different observed device/inode objects, stale evaluator state, forged candidate IDs,
  review-status races, repository mismatch, pin expiry, and source drift fail
  closed. Failure, explicit release, and drop make a best-effort exact pin
  release so ordinary retries cannot exhaust the review-pin cap. Inode reuse
  remains an explicit limitation. The source stays non-authoritative and cannot clear
  `ProtectedPath`, create a plan, cross FFI, schedule, or execute.
- Continued Milestone 5 with a second sealed Unix Rust-target checkpoint that
  consumes the live layout witness and resolves its exact workspace root and
  target directory through an observed canonical executable named `cargo`. The
  runner rejects symlink launchers, including the usual rustup proxy, binds the
  file's full SHA-256 and the reviewed Cargo 1.96.0 verbose version, preserves
  that exact executable/environment evidence in the witness, uses a canonical
  manifest-parent working directory, clears the ambient environment, and invokes only fixed
  `metadata --format-version 1 --no-deps --locked --offline` arguments. Stdout,
  stderr, time, and JSON shape are bounded; timeout or output overflow kills
  the original process group and reaps the direct child. Full-file manifest hashing now
  detects same-inode content changes around both live and Cargo checks. The new
  witness remains crate-private, non-cloneable, non-serializable, blocked by
  `ProtectedPath`, and unable to plan, cross FFI, schedule, or execute.
- Continued Milestone 5 with a sealed, crate-private Rust-target live witness.
  It accepts only the exact unschedulable `developer.rust.target` revision-2
  candidate and unresolved `ProtectedPath` blocker, then no-follow validates
  the selected root, direct `target` directory, single-link `Cargo.toml`
  sibling, and single-link `CACHEDIR.TAG` child on one volume. A new generic
  nonblocking retained-descriptor prefix reader binds the tag read to
  before/open/after identities and requires Cargo's exact 43-byte standard
  signature while permitting standards-compliant trailing comments. The
  witness cannot be cloned, serialized, converted to a plan, sent across FFI,
  or used to reach an effect; it cannot clear a blocker. Cargo metadata/config
  resolution, authoritative volume and protected-root grants,
  process/descendant guards, approval, and executor-time revalidation remain
  required.
- Began Milestone 5 with an independently researched, fail-closed Rust Cargo
  `target` rule. Revision 2 requires snapshot evidence of a direct regular
  `Cargo.toml` sibling and `CACHEDIR.TAG` child before proposing a
  safe-regenerable known-contents cleanup. The exact policy is build/load-time
  allowlisted and adversarially tested, but remains unschedulable and every
  candidate retains `ProtectedPath`; selection and plan construction therefore
  still reject it until live Cargo, volume, protected-root, change, and executor
  witnesses exist.
- Completed Milestone 4's million-node performance-fixture slice. Generated
  balanced and worst-case wide snapshots now exercise real durable immutable
  publication and Explorer review without creating millions of filesystem
  entries; scaled versions run in normal CI, and isolated Release 1M/5M lanes
  emit versioned timing/size observations with peak-RSS logs through a scheduled
  or manual workflow. Checked Apple-silicon baselines justify admitting exactly
  999,999 sortable direct children with post-work lease revalidation and
  retain the typed pre-decode refusal for a 5M review rather than raising the
  fixed 1 GiB UI-process budget after a roughly 2.01 GB publication peak. A
  repeated maximum-cell native treemap layout also gates the 60 Hz interaction
  budget. The fixtures and projections remain read-only and non-authoritative.
- Completed Milestone 4's scan-cancellation and subtree-refresh slice with FFI
  contract v16. Snapshot Explorer can rescan its current directory by sending
  only the exact retained review and node ID; Rust keeps the live path sealed,
  requires an engine-owned live review and directory, and repeats device/inode
  identity checks from admission through atomic snapshot publication. The scan
  reuses the ordinary bounded poll/cancel task and produces a new standalone
  immutable snapshot rooted at the selected folder rather than modifying the
  historical parent. The macOS app keeps the old snapshot visible until the
  exact result root, page, and treemap all validate, generation-fences every
  navigation/reload/close race, preserves Home coverage as a separate baseline,
  and exposes Rescan This Folder, Load Latest Snapshot, and accessible
  progress/cancellation UI. No live path or cleanup authority crosses FFI.
- Completed Milestone 4's candidate-detail transport slice with FFI contract
  v17. A retained snapshot review can now enumerate bounded historical
  candidate summaries, then return candidate path pages and typed evidence
  pages. Rust keeps the observations tied to the
  exact review lease; Swift rejects hostile versions, identities, cursors,
  counts, timestamps, optional-field shapes, and oversized path payloads.
  Candidate detail remains discovery-only historical display data and cannot
  become planner, executor, AI, or cleanup authority.
- Completed Milestone 4's storage-category visualization slice with FFI
  contract v15. Snapshot review nodes now carry a display-only category joined
  from the exact scan's immutable, validated candidate evaluation. Exact
  classified roots and descendants use the nearest historical assertion;
  ambiguous, absent, over-budget, or non-Unicode candidate evidence remains
  explicitly Unclassified. A 4,096-root and independent 1 MiB path-payload
  ceiling bounds the optional join, ancestor lookup is indexed, and release or
  expiry discards the index. The macOS
  treemap uses stable category colors plus symbols and a visible legend, while
  both Explorer tables, the inspector, and VoiceOver expose the same category
  in text. Category metadata contains no candidate identity, path, evidence,
  safety, action, reclaimability, AI result, plan, or cleanup authority.
- Completed Milestone 4's Finder, Copy Path, and Quick Look slice with FFI
  contract v14. Snapshot Explorer sends only the exact retained review's node
  ID and requested read-only purpose. Rust reconstructs the path from immutable
  snapshot evidence, rejects symlinks and special entries, and descriptor-walks
  the current root, ancestors, and target while matching recorded device/inode
  identities before rechecking the lease. Swift rejects hostile path records,
  fences delayed results against selection and snapshot changes, rejects
  non-Unicode paths rather than constructing a lossy URL, and keeps one lifecycle-owned Quick
  Look panel. Current paths are ephemeral UI data only and never become AI,
  candidate, plan, or cleanup input; path-based macOS APIs retain a documented
  post-validation same-user race.
- Completed Milestone 4's scan-coverage details slice with FFI contract v13.
  An exact scan-history endpoint returns bounded canonical issue pages without
  requiring a retained snapshot, so coverage evidence remains readable for
  non-succeeded and pruned scans. It exposes exact totals, all semantic issue
  kinds, and only bounded root-relative historical display context—never the
  absolute scan root or a live filesystem capability. Snapshot Explorer adds a
  lazy Coverage tab with strict cross-page validation, generation-fenced state,
  accessible measured-coverage visuals, truthful Unknown handling, and plain
  explanations. Coverage discovery cannot authorize Finder, AI, planning, or
  cleanup work.
- Continued Milestone 4 with FFI contract v12 and a bounded Large Files mode in
  Snapshot Explorer. Rust scans the retained immutable snapshot through the
  existing review lease with O(k) top-result memory, returns exact matching
  file count and logical-byte totals, and revalidates expiry after the query.
  The app requests at most the 100 largest observations (hard cap 200), defaults
  to 1 GiB, offers deterministic size and strict last-modified filters, shows
  bounded historical parent context, and discloses omitted matches and scan
  coverage limits. Swift rejects malformed ordering, totals, timestamps,
  contexts, and node shapes and generation-fences mode/filter/snapshot races.
  This is discovery only: it creates no live URL, Finder action, reclaimability
  estimate, selection for deletion, AI input, plan, or cleanup authority.
- Continued Milestone 4 with a bounded Recent Scans chooser in the macOS
  Snapshot Explorer. It requests only the newest 50 path-free history rows,
  displays all lifecycle states and truncated-history disclosure, and enables
  only succeeded rows carrying recorded-snapshot evidence. Selection still
  performs authoritative exact-ID lease acquisition and validates the root,
  first child page, and treemap before replacing the confirmed view; only then
  is the prior review released. Missing retained snapshots leave the current
  browser intact and receive a session-only unavailable mark that explicit
  history refresh clears. History loading and failure are independent from the
  active review, and late close/acquisition races release all browser-owned
  leases. No history hint grants filesystem, planning, AI, or cleanup
  authority.
- Continued Milestone 4 with FFI contract v11 and a synchronized logical-size
  treemap/inspector in the macOS snapshot browser. Every lease-bound treemap
  request returns at most 64 positive-size direct children in deterministic
  logical order plus exact path-free Other accounting, including zero-size
  children; the app requests 48. Logical ranks let a selected cell load its
  exact bounded table page without downloading the directory, while table rows
  outside the projection highlight Other without turning that aggregate into a
  node. A deterministic binary treemap, textual table fallback, keyboard and
  VoiceOver labels, and a historical-facts inspector expose sizes, counts,
  timestamps, and scan warnings. Treemap failure preserves the confirmed table;
  expiry invalidates the entire review. No category, candidate, live path,
  Finder action, reclaimability claim, AI input, plan, or cleanup authority was
  added.
- Continued Milestone 4 with the first real Latest Snapshot Browser in the
  macOS Explorer. The new destination acquires only when opened, uses the
  core-selected newest retained review, and publishes the root plus one bounded
  100-row child page only after acquisition, root, and page reads all succeed.
  Folder buttons and ID-backed breadcrumbs drill through the immutable
  snapshot; server-side name, logical, allocated, and modified sorting resets
  to the first page, while previous/next controls replace rather than accumulate
  pages. A synchronized table shows logical and allocated sizes, item counts,
  modified time, observation warnings, and proportional share bars with textual
  accessibility summaries. Leaving Explorer or shutting down generation-fences
  pending work and explicitly releases the review. Snapshot absence, expiry,
  retryable pressure, resource-budget refusal, unavailable storage, and invalid
  responses remain distinct read-only states. No display name becomes a live
  URL, filesystem identity, or cleanup authority.
- Continued Milestone 4 with FFI contract v10's bounded snapshot-node transport.
  An exact Explorer review lease now exposes the retained root plus deterministic
  direct-child pages (maximum 200) sorted by name, logical size, allocated size,
  or modification time. Rust decodes a snapshot once per active review, still
  revalidates the durable pin and retained immutable file before every cached
  read, drops invalid/expired caches immediately, and independently limits each
  engine to two retained trees within a conservative 1 GiB decoded-memory
  admission estimate. A decode-time compact child index and one sorted-child
  cache avoid rescanning an entire subtree on every page, while directories
  above 100,000 direct children remain budget-gated pending M4 latency work.
  Versioned records
  preserve lossless historical host bytes and explicit display text while
  omitting Unix identity, live handles, candidates, plans, and all cleanup
  capability. Swift maps into validated
  app-owned models off-main, and the review controller generation-fences root
  and page results by scan ID. Candidate details remain separate M4 work.
- Continued Milestone 4 with FFI contract v9's exact newest-available snapshot
  review acquisition. Rust deterministically selects the newest succeeded
  snapshot without an exact retention tombstone, then acquires the existing
  expiring review lease while repeating catalog, tombstone, retained-file
  identity, and full-format validation; retention races fail closed and a
  tombstoned newest snapshot falls back to the next usable one. Swift validates
  the authoritative scan ID returned with the lease, performs blocking calls
  off-main, owns renewal outside render state, generation-fences concurrent
  latest requests, and explicitly releases stale or malformed handles. No
  paths, nodes, candidates, plans, or cleanup authority cross this boundary.
  The complete 189-test linked Swift suite, 736-test core suite, 16 FFI tests,
  full Rust format/lint/workspace gates, all 28 script tests, and the
  183-source destructive boundary pass. Debug/Release bindings and XcodeGen
  output are deterministic, and clean universal arm64/x86_64 builds target
  macOS 14.
- Began Milestone 4 with FFI contract v8's bounded recent-scan history page for
  Explorer snapshot selection. The newest-first, path-free records expose only
  stable scan identity, lifecycle timestamps/status, succeeded counts, coverage,
  and whether SQLite recorded a snapshot reference. That hint grants no file or
  cleanup authority: opening a selected snapshot still requires the existing
  scan-bound expiring review lease and repeats repository validation. The Swift
  adapter performs the call off the main thread, converts generated values to
  app-owned immutable models, identifies the newest review candidate in the
  bounded page, and fails closed on malformed versions, IDs, ordering,
  lifecycle/count combinations, snapshot hints, timestamps, or coverage. Node,
  issue, candidate-detail, and treemap payloads remain sealed for subsequent M4
  slices. Sixteen FFI tests, four focused native tests, the complete 187-test
  linked Swift suite, full Rust format/lint/workspace tests, all 28 script tests,
  and the destructive-call boundary pass. Regenerated bindings and XcodeGen are
  deterministic, and clean Debug/Release builds produce exact universal
  arm64/x86_64 applications targeting macOS 14.
- Added a fail-closed local macOS release workflow and reviewed empty release
  entitlements. It requires a clean exact stable tag, matching workspace
  versions, a non-placeholder bundle ID, explicit Team ID and Developer ID
  Application identity, and a Keychain-only `notarytool` profile. It runs the
  repository gates, regenerates and checks bindings/project files, builds exact
  universal arm64/x86_64 code for macOS 14, rejects unknown nested bundles, and
  signs known code inside-out without using recursive signing as a shortcut.
  The app and final Applications-link DMG are each independently notarized,
  logged, stapled, signature/entitlement/identity checked, and Gatekeeper
  assessed. Successful output is atomically published to a new immutable
  version directory with sanitized submission records, full Apple logs, a
  manifest, and SHA-256 sidecar; failures retain private staging diagnostics.
  The script rejects the temporary bundle ID and accepts no Apple ID password or
  API private-key path. A real notarization run remains correctly blocked until
  Milestone 9 freezes the production identity and credentials. Seven focused
  release-script tests plus a boundary-parser regression bring the script suite
  to 28 tests; the full Rust gates, 183-test linked Swift suite, and 180-source
  authority scan pass. XcodeGen is deterministic, bindings remain unchanged,
  and unsigned Debug/Release app layouts match with exact universal arm64/x86_64
  executables targeting macOS 14. Local ad-hoc signing also verifies the empty
  entitlement extraction and actual Hardened Runtime flag format; Developer ID
  signing and Apple submission are intentionally unverified until identity
  freeze.
- Added truthful permission and coverage onboarding. A one-time menu-bar
  introduction explains local, read-only Home analysis and persists only its
  acknowledgement without starting a scan, capacity query, access probe, FFI
  call, or permission prompt. Explorer uses the latest real Home-scan coverage;
  only a measured incomplete or uncertain result offers explicit broader-access
  discovery. That bounded off-main check opens three fixed user-library
  directories without enumerating names or reading contents and publishes only
  path-free readable, unreadable, and unobserved counts. It never claims an
  authoritative Full Disk Access state. Optional generic System Settings
  guidance arms exactly one observed-access recheck on return, while updating
  coverage still requires an explicit new Home scan. Checks are single-flight,
  cancellation-safe, retain prior evidence on failure, and reject late shutdown
  results. Eleven focused onboarding tests plus activation integration bring the
  complete linked Swift suite to 183 tests. Full Rust format/lint/workspace
  tests, 20 destructive-boundary tests, and the 178-source authority scan pass.
  XcodeGen is deterministic, Debug/Release Swift bindings are byte-identical,
  and unsigned universal arm64/x86_64 Debug and Release apps target macOS 14.
- Added opt-in conditional menu-bar visibility. DUX remains always visible by
  default; Settings can instead show it when cached effective startup-disk free
  space is at or below a validated whole percentage from 1 through 100. Entry
  is immediate, hiding requires one full percentage point of recovery, and
  unknown capacity fails open. Refreshing and stale states use the last cached
  sample, while preference changes start no scan or capacity query. Missing,
  malformed, out-of-range, and future versioned UserDefaults values fall back
  without being rewritten. Reopening the running app reveals the item for the
  rest of that process session without changing the preference. Fifteen focused
  tests and the complete 170-test linked Swift suite pass, as do full Rust
  format/lint/workspace tests, 20 destructive-boundary tests, and the 175-source
  authority scan. XcodeGen is deterministic, Debug/Release bindings are
  byte-identical, and unsigned universal arm64/x86_64 Debug and Release apps
  target macOS 14. The policy is Swift-only presentation state and adds no FFI,
  pressure-policy, notification, scheduling, AI, plan, or cleanup authority.
- Added authorization-only notification Settings for the macOS app. DUX reads
  macOS's current permission, presents distinct not-requested, denied, allowed,
  provisional, and unknown states, refreshes after returning from System
  Settings, and shows the system prompt only after an explicit user action from
  a confirmed Not Determined state. Duplicate requests are single-flight,
  caller cancellation cannot abandon model state, and every attempt is followed
  by an authoritative status read. The service requests Alert and Sound only;
  its interface has no API for scheduling or delivering notifications. This
  checkpoint therefore sends no alerts and adds no cooldown, deep link, scan,
  scheduling, AI, plan, or cleanup authority; transition notifications remain
  in Milestone 6. The focused 19-test suite and complete 159-test linked Swift
  suite pass, including a production read-only status query that never requests
  permission. Full Rust format/lint/workspace tests, 20 destructive-boundary
  checker tests, and the 173-source authority scan pass. Debug/Release Swift
  bindings are byte-identical, and unsigned universal arm64/x86_64 Debug and
  Release apps target macOS 14.
- Added an opt-in Launch at Login setting backed only by
  `SMAppService.mainApp`. Settings reflects macOS's authoritative registered,
  enabled, approval-required, unavailable, and unknown states; refreshes after
  returning from System Settings; and never stores a competing UserDefaults
  preference. Registration changes are single-flight and always re-read system
  status before claiming success, including already-settled error races. The
  existing `LSUIElement` menu-bar startup remains unchanged, and no helper,
  daemon, entitlement, privilege, scan, or cleanup authority is added. All
  registration-mutation tests use an injected service and never mutate the
  developer's Login Items, while one production integration test reads the
  current status without enabling it. The focused 17-test suite and complete
  140-test linked Swift suite pass, as do the full Rust format/lint/test gates,
  the 20-test destructive-boundary checker and 170-source scan. Debug and
  Release bindings are byte-identical, and unsigned universal arm64/x86_64
  Debug and Release apps target macOS 14. Signed enable/disable and sign-in-cycle
  testing remains gated on the production bundle identity.
- Added the first native Explorer window: a single reusable, menu-bar-first
  `NavigationSplitView` whose Overview reports startup-disk capacity and the
  current in-session Home scan from the existing shared `AppModel`. Capacity
  visuals distinguish important-use availability from ordinary filesystem
  free space, preserve stale measurements without inventing values, and pair
  every segment with visible and VoiceOver-readable text. Home-scan coverage is
  explicitly scoped, retains the last confirmed aggregate while work is active
  or ends unsuccessfully, and offers mutually exclusive scan/cancel actions.
  Explorer never auto-scans and adds no path, history, candidate, plan, AI, or
  cleanup transport. Stable accessibility identifiers and Command-R,
  Command-period, and Command-comma shortcuts cover its primary interactions.
  The complete 123-test linked Swift suite, full Rust workspace format/lint/test
  gates, 20 destructive-boundary checker tests and the 167-source boundary
  scan pass. Debug and Release bindings are byte-identical, and unsigned
  universal arm64/x86_64 Debug and Release apps target macOS 14.
- Completed the Milestone 2 durable history/engine foundation and automatic
  DUX-owned retention scope, with remaining Explorer, executor, storage-control,
  cross-process, crash-debt, and platform-qualification work assigned explicitly
  to Milestones 4, 5, and 9. Durable scan startup on macOS now tolerates a
  denied boot-session sysctl by retaining an unscoped exact PID/start-token
  owner. That fallback can confirm the exact process is alive but can never
  prove death or authorize recovery. Repeated isolated-HOME scan/reopen tests
  cover the formerly intermittent failure. Full Rust/FFI/CLI and 105-test
  linked Swift suites, destructive-boundary checks, byte-identical Debug/Release
  bindings, and unsigned universal arm64/x86_64 Debug and Release app builds at
  the macOS 14 deployment target pass.
- Added the native menu-bar capacity and Home-scan popover through UniFFI
  contract v7. The popover renders the cached startup-volume name, Rust-owned
  pressure, effective available capacity, total, percentage, availability
  basis, and freshness without starting a scan or inventing missing values;
  refresh failures retain the last measurement as explicitly stale. An
  explicit Scan now action starts one shared Home scan, shows only optional
  cumulative path-free progress, exposes cancellation intent until Rust reports
  the terminal outcome, and retains the last successful scan through retries,
  cancellation, and failures. The new opaque scan task accepts one bounded
  read-only discovery root, while its polls and terminal summaries contain no
  path, candidate detail, plan, approval, or cleanup capability. All blocking
  FFI work remains on the engine utility queue, app shutdown requests scan
  cancellation before maintenance/reviews/engine close, and stable keyboard
  shortcuts plus VoiceOver identifiers cover every action and status region.
  Focused Rust and linked Swift tests cover request validation, single-flight
  lifecycle, malformed records, progress monotonicity, late cancellation,
  cached-result retention, shutdown ordering, honest capacity presentation,
  localization, and accessibility. The complete 105-test linked Swift suite,
  full Rust workspace test and lint gates, destructive-boundary checks, and
  unsigned universal arm64/x86_64 Debug and Release builds at the macOS 14
  deployment target pass.
- Added three persistent menu-bar label modes: Icon only, Icon and free space
  (GiB), and Icon and free space (%), defaulting to free GiB. A validated
  versioned Swift-owned UserDefaults preference updates the shared status item
  immediately from Settings and falls back without rewriting missing, corrupt,
  or future values. The label reads only cached startup-volume state, never
  starts a sample or scan, preserves the last value during refresh/failure,
  uses important-use availability with the disclosed filesystem fallback, and
  never fabricates zero. Exact checked integer formatting conservatively floors
  binary GiB and percent to tenths without overflow. Monochrome shape-distinct
  pressure symbols do not rely on color, and every mode speaks pressure,
  capacity, percent, basis, and freshness through one stable VoiceOver element.
  Focused tests cover all modes/states, locale and integer boundaries, valid SF
  Symbols, preference compatibility, immediate one-write propagation, and
  accessibility identifiers; the complete 82-test linked suite and universal
  arm64/x86_64 Debug and Release app builds pass.
- Added persistent user-configurable disk-pressure thresholds through SQLite
  schema v10 and UniFFI contract v6. The exact `disk_pressure_policy` setting
  stores canonical schema-v1 JSON with checked revisions and Default/Stored
  provenance; absence remains the current core defaults at revision 0 without
  a write, while explicit defaults and reset epochs remain distinguishable.
  Raw and daily capacity history retain the policy revision that classified
  them, migrated v9 rows remain revision 0, and a real policy change resets
  old-policy hysteresis and forces one same-hour baseline. Policy loading,
  classification, and history insertion share the final writer-leased
  transaction, with a cross-process regression proving observations cannot
  bypass a committed policy whose writer lease is still held. FFI v6 exposes
  typed versioned get/set/reset records and validation failures without
  changing startup-capacity record v1. Native Settings edits exact decimal GiB
  and basis-point percentages without floating point or rounding, preserves
  last-good state and attempted input on failure, and requests exactly one
  generation-safe capacity resample only after a changed save/reset. Linked
  tests cover every basis-point value, exact byte round trips through
  `UInt64.max`, locale input, hostile responses, stable Settings accessibility
  identifiers, cancellation, changed/unchanged resampling, and real Rust
  persistence. This policy changes
  classification only and grants no scan, notification, scheduling, or cleanup
  authority.
- Added native startup-volume pressure monitoring through FFI contract v5. The
  menu-bar runtime samples immediately, every five minutes, on wake, and after
  volume changes while keeping one generation-fenced request in flight. Rust is
  the sole policy owner for important-capacity preference, default
  Warning/Critical thresholds, and two-dimensional recovery hysteresis. A
  bounded engine-session baseline preserves hysteresis for newer observations
  that security policy forbids persisting. For durable observations, the engine
  selects the newest valid session/durable baseline, loads durable state, admits
  hourly or transition history, commits, and reconciles ambiguity inside one
  writer-leased SQLite transaction. Missing stable identity, incomplete
  metadata, and important-only observations remain useful path-free display
  telemetry but cannot advance history. Swift preserves cached values as
  refreshing or stale, never invents used bytes from important capacity, and
  performs all Foundation and UniFFI work off the main actor. Core/store/FFI
  boundary and race tests plus the linked Swift suite cover hysteresis, cadence,
  reopen, retry, suppressed-observation ordering across engine sessions,
  lifecycle signals, cancellation, honest partial observations,
  accessibility presentation, and the normal sampling budget. This is read-only
  status telemetry and grants no cleanup authority.
- Added conservative hard-process-death recovery for schema-v9 claimed running
  scans. Start and normal completion atomically create/consume one exact private
  process-instance claim; a bounded indexed same-scope keyset pass drops every
  SQLite lock before OS probes and exact-CASes at most one pristine row to
  `interrupted` only for `DefinitelyGone`. Live, unknown, malformed,
  cross-reboot, legacy-v8, and newer-schema cases never recover. Snapshot-temp
  leases are preserved for the independently sealed terminal-temp reconciler.
  The idle-only core task exposes path-free counts/outcomes and no caller scan or
  owner input. FFI contract v4 and the macOS scheduler add it as the first of
  seven fair maintenance kinds. Real graceful/SIGKILL child-process, indexed
  keyset-plan, paging, admission-cap, exact-scope race,
  commit-reconciliation, schema-fencing, hostile-row,
  cancellation/close, redaction, and temp-debt convergence regressions cover the
  boundary.
- Added the real macOS engine/maintenance runtime, introduced through FFI
  contract v3 and now carried by contract v4.
  `DuxEngine` now opens the shared Rust engine from input-only private data and
  cache roots, exposes all seven sealed maintenance kinds as opaque cancellable
  tasks with versioned path-free results, and exposes Explorer-only snapshot
  review sessions without paths, filenames, digests, handles, inventories, or
  cleanup authority. History row counts are distinct from every byte field.
  Review acquisition now linearizes against core close; FFI close rejects new
  renewals, attempts exact release for every still-live registered review, and
  gives concurrent callers one bounded quiescence result. A failed durable
  release expires naturally. The Swift engine opens lazily on a utility queue,
  its actor-owned review controller uses five-minute/wake renewal plus
  generation-safe explicit release, and app shutdown is ordered maintenance →
  reviews → engine with duplicate termination requests coalesced. A native
  energy-aware scheduler runs one fair seven-kind cycle after a 60-second grace,
  spaces normal batches by one minute, waits six hours between cycles, and
  preserves distinct `has_more`, deferral, busy, failure, and energy backoffs
  across activation/wake signals. Scheduler shutdown awaits any suspended
  driver before engine close. Rust and linked Swift regressions cover
  tombstone/schema/close acquisition edges, pin drainage, concurrent close,
  every maintenance kind, startup grace, cadence/backoff, pending acquisition,
  stale renewal replacement, overlapping renewal, cancellation, and duplicate
  termination.
- Added bounded root-local snapshot provisioning-stage reconciliation. Under a
  current-schema database guard, one complete raw/native root inventory proves
  only exact canonical private stages containing the 16-byte store marker alone
  or with the exact writer marker. Empty and stricter-mode crash remnants defer
  without starving proven debt; malformed markers, writer-only/extra children,
  links/reparse points, broad permissions/DACLs, or a 65th stage fail before
  effect. One batch non-recursively removes at most the first lexical proven
  stage in writer/sync/marker/sync/directory/root-sync order, with checked exact
  control-byte accounting and `OutcomeUnknown` after the first namespace
  effect. The typed idle-admitted engine task accepts no authority-bearing input,
  publishes only canonical aggregate before/after observations and redacted
  outcomes, linearizes cancellation/close at Applying, and never self-enqueues.
  Unix hostile-shape/effect tests execute locally; Windows-native cases cover
  delete-on-close, DACL/reparse, malformed, and 64/65-cap behavior and
  cross-compile for CI, while native Windows execution remains pending. Legacy
  external stages, unproven empty/stricter-mode debt, and malformed
  partial-marker debt are never adopted or removed.
- Added a bounded, path-free core cleanup-history read bridge. Recent pages use
  a 1..=64 opaque keyset cursor, preserve equal-time ordering, and expose
  lifecycle, estimates versus verified capacity deltas, graph totals, and
  exhaustive item/path status counts. Scalar graph and journal-state validation
  checks size, relationships, claims, generations, times, derived outcomes, and
  mode-compatible success without reading stored target/evidence payloads;
  exact format-2 lookup additionally validates the complete journal before
  returning scrubbed item policy/status summaries and warnings. Migrated rows remain
  explicitly `LegacyIncomplete`, including arbitrary legacy error text that is
  reduced to an error-presence bit. Fixed VM/deadline and conservative 64 MiB
  per-graph read limits fail closed without claiming matching write admission.
  Public values omit paths, evidence payloads, candidate IDs, ownership fences,
  claims, and prior review state, and cannot construct a plan, approve recovery,
  or reach an effect. Empty/limit/closed/missing, reopen, cursor, status/error,
  maximum-page, secret-path, structural/lifecycle corruption, and over-budget
  regressions cover the core-only boundary; FFI/Swift/UI/CLI transport remains
  later.
- Added a bounded core candidate-review boundary. Exact path and evidence
  pagers require both scan and candidate IDs, enforce 1..=64 pages and strict
  immutable cursors, and return lossless accepted-host bytes plus display text
  or typed presentation-only evidence. Exact-candidate reads now preflight
  parent/child storage classes, counts, and the shared 32 MiB materialization
  charge before payload decoding; standalone writes apply the same budget.
  Semantic `Select`, `ClearSelection`, `Dismiss`, and `Restore` commands resolve
  their source state and scan binding inside one writer-leased transaction.
  Selection remains limited to blocker-free cleanup policy, while dismissal is
  only review visibility and creates no exclusion, plan, approval, or effect.
  Paging, all evidence variants, reopen, schema/lifecycle failures, hostile and
  over-budget rows, exact post-commit adoption, terminal-state refusal, and a
  concurrent select/dismiss race have focused coverage. Maximum-legal cleanup
  history and active-journal records retain their existing shared query budget.
  The boundary is core-only; FFI/Swift/UI/CLI transport remains later.
- Added an exact-scan durable candidate-discovery read bridge. The core engine
  distinguishes a missing scan, a real scan with no evaluation, Pending,
  Succeeded, and typed Failed evaluation history. Before any payload-bearing
  child query, the store charges row counts and encoded bytes against a
  conservative 32 MiB materialization budget also enforced during evaluation
  write preparation; the existing VM/time budget remains independent. The
  production evaluator maps a valid over-budget graph to typed discovery
  `LimitExceeded`, preserving the successful scan and snapshot instead of
  misreporting a persistence failure. The complete stored graph is then
  validated before a path-free summary exposes
  policy, estimates, counts, typed evidence kinds/blockers, and historical
  status. Exact paths and evidence payloads remain private to that summary;
  the separate explicit local pager above owns their disclosure. Reopen,
  lifecycle,
  version-skew, failure, exhaustive mapping, corruption, exact-budget, and
  over-budget preflight regressions cover the boundary. The DTO is observation
  only: it cannot reconstruct a candidate or cleanup plan, mutate review or
  planning state, or reach an effect. FFI/Swift/UI/CLI transport remains later.
- Added typed engine orchestration for one bounded snapshot-cap decision.
  `EngineHandle::start_snapshot_retention` is idle-only, deduplicated per
  session, mutually exclusive with other session maintenance, and invokes the
  sealed repository writer exactly once without accepting a cap, inventory,
  victim, scan identity, or path. Its immutable result discards the selected
  scan identity and exposes only canonical time, aggregate cap/accounting,
  `has_more`, and a path-free outcome. Applying linearizes cancellation and
  close before repository mutation; later cancellation remains intent and
  cannot rewrite the exact success or failure. Canonical clock, checked
  pre-mutation accounting, under-cap/one-victim, explicit rescheduling,
  duplicate/busy/cross-maintenance, schema-race, cancellation, stable-error,
  and shared-store multi-session regressions cover the boundary. The core does
  not self-schedule; native app/FFI review-lease ownership and periodic idle
  scheduling remain future work.
- Added bounded physical-only unleased snapshot-temp reconciliation. A separate
  repository batch holds the current-schema database guard before the snapshot
  writer lease, subtracts the complete at-most-64-row immutable lease-name
  population from one bounded marker-owned physical inventory, and removes at
  most the first lexicographic quiescent exact-generated-grammar temp. The
  effect boundary freshly repeats no-follow name/identity, exact
  logical/allocation usage, private-file, one-link, and second nonblocking
  kernel-lock proof; prefix, PID, owner, age, mtime, and prior quiescence alone
  never grant authority. Active entries are skipped without starvation,
  checked accounting precedes unlink, the delete handle closes before directory
  sync, and post-effect uncertainty is `OutcomeUnknown`. The batch performs no
  SQLite mutation or adoption and never maps the temp to a scan. The typed
  idle-only `SnapshotUnleasedTempMaintenance` task accepts no authority-bearing
  input, invokes one batch, exposes only aggregate `NoUnleasedTemp`,
  `DeferredActive`, or `Removed { bytes }` results and counts, linearizes
  cancellation at Applying, and never self-enqueues. Focused deterministic,
  accounting, active/backoff, exclusion, effect-boundary, redaction,
  admission/cancellation, failure-mapping, and multi-session tests cover the
  slice. Windows pre-v8 writable handles deny delete sharing; Unix may detach
  an incompatible old writer's open inode, but its later current-schema/name
  publication proof fails. Avoiding that same-user availability race requires
  not running old and current binaries concurrently on the owner-private store.
  Explicit clear-data and native Windows runtime verification remain future.
  Exact-marker-owned
  root-local provisioning-stage maintenance is the separate slice above;
  legacy external snapshot stages remain unattributable manual debt.
- Added bounded terminal snapshot-temp reconciliation. A separate sealed
  repository batch now inspects the complete bounded lease population and
  physical inventory under the database-before-snapshot lock order, accepts
  only exact failed/cancelled/interrupted parents with no snapshot, skips active
  row-bound files without starving later actionable debt, and reconciles at
  most one deterministic row-only or quiescent residual. Physical removal is
  identity-, usage-, name-, and kernel-lock-revalidated and precedes exact row
  consumption; checked accounting is frozen before unlink, the delete handle
  closes before directory sync, and post-effect uncertainty remains typed as
  `OutcomeUnknown`. Running rows, unleased temps, stages, scans, finals,
  tombstones, pins, and unrelated history remain untouched. The typed idle-only
  `SnapshotTerminalTempMaintenance` task accepts no authority-bearing input,
  performs one batch, publishes only aggregate counts/bytes, linearizes
  cancellation at Applying, and never self-enqueues. Focused status,
  row-only/physical, active/starvation, exclusion, ordering, accounting,
  corruption, race, effect-boundary, commit/schema, redaction, cancellation,
  panic, and multi-session tests cover the slice. Native scheduling, running-row
  recovery, provisioning-stage maintenance, clear-data, and native Windows
  runtime verification remain separate; the newer entry above records the
  independent physical-only unleased-temp boundary.
- Added bounded physical snapshot-orphan reconciliation. A separate sealed
  repository batch now classifies typed finals against the exact indexed
  catalog under the database-before-snapshot lock order, selects only one
  deterministic zero-reference final, fully decodes its checksum-valid body,
  binds its filename to the decoded scan ID, and requires an exact matching
  `running` or terminal-non-success parent with no snapshot reference. The
  retained identity and logical/allocation usage are rechecked through a
  delete-capable handle, all accounting is frozen before unlink, and directory
  durability is required for success. Post-unlink uncertainty is distinct from
  a guaranteed pre-effect failure and maps to `OutcomeUnknown`; no scan,
  temp-lease, tombstone, pin, or history row is changed. A separate idle-only
  `SnapshotOrphanMaintenance` engine task accepts no authority-bearing input,
  removes at most one final, redacts scan/name/path identity, reports bounded
  aggregate accounting and `has_more`, and never self-enqueues. Focused
  state/body/catalog/race/accounting/effect-boundary, cancellation/close,
  multi-session, and redaction regressions cover the slice. Native periodic
  scheduling, general temp/stage maintenance, clear-data actions, and native
  Windows compile/removal runtime verification were separate at that
  checkpoint; the newer terminal-temp and unleased-temp entries above record
  the row-bound and physical-only portions now implemented. The local MSVC
  cross-check stops in bundled SQLite's C build because no Windows sysroot is
  installed.
- Added the core production snapshot-cap mutation boundary. One bounded batch
  holds the current-schema database lease before the snapshot writer lease,
  rebuilds the complete cap/latest-two/active-pin inventory, refuses new
  retirement while active or unleased temps make accounting unstable, and
  selects at most the deterministic oldest eligible snapshot. Before mutation
  it reopens and fully decodes the exact observed final against the immutable
  scan digest. It then commits an append-only exact tombstone before deleting
  while keeping the digest-validated retained handle live, revalidating a
  separate deletion handle, and durably syncing the snapshot directory.
  Existing tombstoned physical residuals are retried first even below the cap;
  changed bytes or post-validation replacements remain untouched by that
  batch. Exact post-commit reconciliation, schema-race residual recovery,
  latest-two, active-pin, active/unleased-temp deferral, quiescent-temp
  continuation, one-victim, changed-content, post-validation replacement, Unix
  identity, and Windows compile regressions cover the boundary. This remains a
  sealed repository batch whose only orchestrator is the typed core engine
  task. At that checkpoint, app/FFI review-lease ownership, native periodic idle
  scheduling, orphan and unleased-temp/stage scavenging, clear-data actions, and
  native Windows runtime verification remained separate; the newer entries
  above record the subsequently implemented physical-orphan, terminal-row, and
  physical-only unleased-temp boundaries.
- Added schema v8's durable snapshot temporary-file leases. Snapshot staging
  now reserves an exact recognized name while holding the permanent
  database-before-snapshot lock order, commits a bounded immutable lease row
  before creating the file, and retains a nonblocking kernel file lock while
  bytes are being encoded. Read-only inventory distinguishes row-bound active
  and quiescent-at-observation temps from unleased legacy debt, charges every
  class, and reports row-without-file residuals separately. An exact same-scan
  retry may remove only a row-bound, identity-revalidated temp after acquiring
  its kernel lock and durably flushing the directory; active temps return busy,
  while unleased temps are never adopted or removed by that path. Normal abort
  removes the physical temp before consuming its exact row, and successful
  publication atomically consumes that row with the succeeded scan/evaluation
  transaction
  while retaining snapshot-writer exclusion. Drop remains close-only. Focused
  migration, hostile-row, row-before-file, active/quiescent, same-process retry,
  atomic completion, and cross-process kernel-lock tests cover the implemented
  boundary. This does not enable broad temp or unproven-stage scavenging, a
  tombstone writer, final-file unlink, app/FFI lease ownership, scheduling, or
  production retention enforcement; exact-marker-owned stage reconciliation is
  the separate slice above, and native Windows runtime verification of the new
  liveness/removal paths remains outstanding.
- Added the typed snapshot-retention cap prerequisite. The exact
  `snapshot_retention` setting uses canonical deny-unknown value-schema-v1 JSON,
  defaults to 2 GiB without writing a row, accepts the full `u64` policy domain,
  preserves unknown keys, rejects malformed current values, and fences newer
  per-setting schemas without overwrite. Path-free core engine get/set/reset
  APIs expose stored/default provenance and exact-reconcile ambiguous commits.
  Retention inventory now rereads the effective cap under its current-schema
  database guard before taking the snapshot lock. Focused default, boundary,
  retry/reset, reopen, corruption/version, engine lifecycle/multi-session, and
  inventory tests cover the slice. This adds no FFI/Swift setting, tombstone,
  unlink, or cap-enforcement authority.
- Added schema v7's read-only snapshot-retention inventory prerequisite. The
  checksummed migration adds a partial `scans_by_snapshot_path` index over the
  lossless snapshot-name encoding, bytes, and scan ID only for rows that carry
  a snapshot reference. A single database-before-snapshot locked pass now
  sequentially opens and strictly reconciles the bounded physical store,
  closes each observed entry handle before the next, reports exact
  logical/allocated/conservative charged bytes, assigns latest-two ranks per
  exact encoded root, validates active/expired review pins without pruning,
  and separates eligible observations from tombstoned residuals, orphans,
  controls, and unknown-liveness temps. Exact schema-object inventories,
  canonical fingerprints, a populated v6 upgrade, hostile-row/storage, cap,
  ordering, no-mutation, and cross-platform handle-accounting regressions cover
  the slice. This adds no tombstone, unlink, temp-scavenging, or cleanup
  authority.
- Added schema-v6 cross-process snapshot review leases. Exact succeeded
  snapshot identities can now be pinned explicitly for Explorer or cleanup
  review without treating durable candidate/session state as proof that a UI
  is open. Pins use a stable process-instance owner, random 128-bit IDs, fixed
  ten-minute expiry, monotonic renewal, idempotent exact release, immutable identity
  guards, and no migration backfill. Acquisition validates the complete parent
  and tombstone state, holds the database fence before the snapshot writer
  lock, and returns a retained read-only file handle. The sealed core API caps
  leases at 64 per owner and 1,024 per store, validates the complete bounded
  population once per acquisition, prunes at most 64 expired rows, treats
  expiry equality as inactive even after cross-process pruning, and
  exact-reconciles ambiguous acquire/renew/release commits without adopting a
  conflicting row. Explicit release removes the row or adopts its already-
  absent postcondition; implicit drop performs no blocking write
  and relies on expiry. App/FFI ownership, latest-two/cap selection, tombstone
  insertion, and physical unlink remain disabled.
- Added the schema-v5 prerequisite for safe snapshot retention. Immutable scan
  references now have a separate append-only tombstone bound by composite
  foreign key to their exact succeeded status, completion time, version,
  losslessly encoded relative name, and digest. Repository loads perform a
  bounded tombstone lookup under the current-schema database guard before file
  open, return a distinct unavailable result for a match, and reject malformed
  or mismatched rows as corruption. Update/delete guards activate only after
  the exact schema fingerprint passes; untrusted and newer schemas retain the
  trigger-disabled posture. V4 upgrades fabricate no retirement state,
  terminal history remains unchanged, and guarded exact-retry loads avoid
  connection-lock reacquisition. No production tombstone writer or physical
  unlink is enabled until latest-two selection, review-lease lifecycle
  integration, total-cap accounting, and retained-handle deletion are
  implemented together.
- Added a typed, idle-only engine task for bounded history maintenance. Closed,
  duplicate, and foreground-busy requests resolve before SQLite access;
  eligible requests recheck schema and admission before allocating work. Each
  worker task runs exactly one retention transaction and publishes path-free
  mutation counts plus `has_more`, leaving later batches to an explicit idle
  reschedule instead of monopolizing a worker or writer lease. Cancellation and
  the Applying point of no return are now linearized under the registry lock:
  an earlier cancellation mutates nothing, while a later request remains intent
  without falsifying a committed success. Stable typed failures, shared-store
  multi-session behavior, corruption rollback, schema races, both cancellation
  sides, duplicate admission, and panic cleanup have focused coverage. Native
  app/FFI scheduling and production snapshot selection/unlink remain separate
  work.
- Added the first bounded history-retention checkpoint. A private
  current-schema SQLite operation creates completed-day UTC capacity rollups
  from the exact last raw observation, retains raw samples for 30 exact days
  and daily rollups for 365 complete UTC days, and deletes AI cache records only
  when their stored expiration is reached. Each transaction validates every
  target, creates a required still-retainable rollup before raw pruning, rolls
  back on conflicting or malformed data, and limits work to 128 raw rows and
  their required rollups, 128 daily rows, and 16 AI rows before returning
  `has_more`. A mutation authorizer excludes scan, candidate,
  cleanup, rule-outcome, schedule, and settings history; fixed VM/deadline
  budgets and exact post-commit reconciliation make interruption and retry
  fail closed. Production snapshot selection/unlink and native app scheduling
  remain separate work.
- Added the first noninteractive shared-engine CLI surfaces: `dux status` and
  `dux history [--limit 1..=200]`, each with explicit `--json`. A bounded,
  path-free core query validates complete scan/coverage records under one
  SQLite VM/time budget, orders newest scans deterministically, reports a
  limit-plus-one `has_more` sentinel, survives process reopen, rejects hostile
  selected rows, and exposes measured counts only for succeeded scans. JSON
  schema v1 versions every object, distinguishes a newer read-only database
  from empty history, preserves unknown allocation/coverage semantics, emits
  structured path-free runtime errors to stderr, and has golden contract tests.
  The history budget covers the legal maximum page of 200 scans with 256
  coverage records each, while retaining a fixed VM/deadline ceiling. A
  completely fresh platform HOME is prepared safely before the core validates
  and publishes its private store.
  The existing `dux [PATH]` TUI and flags remain the default; explicit relative
  or `--` paths disambiguate directories named `status` or `history`. This is
  inspection-only and adds no cleanup or AI authority. A real subprocess test
  scans into an isolated temporary HOME/database, quiesces the first engine,
  and proves both commands reopen and report the same durable scan without
  terminal controls or path disclosure.
- Added deterministic, durable candidate discovery to successful engine scans.
  A build-time and engine-open-validated SHA-256-bound catalog converts only a
  fresh completed traversal's existing marker-verified developer artifacts
  into stable scan-bound observations. All initial rules are selected-root,
  Informational, RevealOnly, unschedulable, and explicitly blocked by unresolved
  protected-path authority; partial coverage remains a separate blocker. The
  evaluator has stable lossless-path IDs and ordering, binds exact catalog and
  versioned context identity, and stops at its 4,096-result bound without first
  materializing every match. Checksummed SQLite schema v4 binds one evaluation
  to the exact immutable snapshot. Normal snapshot publication atomically
  commits scan success, terminal evaluation state, and the complete candidate
  batch or typed failure, with exact ambiguity reconciliation, hostile-row
  validation, all-or-nothing inserts, and bounded set-based loading proven at
  the exact 4,096-candidate cardinality cap across process-style reopen when
  the aggregate child graph fits the decoded-materialization budget. Late
  cancellation observed before discovery's final checkpoint cancels discovery
  without falsifying an already-completed scan; later requests remain recorded
  as intent without rewriting the terminal batch;
  non-successful scans do not create evaluations. Task results and events expose
  only path-free discovery status, and no planner, cleanup, AI, or filesystem
  authority is added.
- Added checksummed SQLite schema v3 and crash-durable candidate/plan claims.
  New plans atomically freeze their graph, move only exact discovered/selected
  candidates to planned, and preserve each prior review state behind one
  delete-restricted session/item claim. Journal terminalization now projects
  each candidate in the same transaction: real success becomes completed,
  dry-run and fully effect-free cancellation restore exact review state, and
  every other terminal result becomes failed; outcome-unknown retains the
  claim. A cleanup-lock-owned exact-expiry path records rejected
  `plan_expired` history and failed candidates without creating execution or
  effect authority, including rounded time, rollback, and ambiguous-commit
  handling. Migrated v2 sessions remain explicitly uncoupled, so pristine
  legacy plans cannot be newly claimed while already-active history can finish
  without fabricated candidate state. Bounded hostile-row, race, partial-
  result, later-review-state, maximum-graph, and lifecycle migration tests
  cover the coupling. The boundary remains persistence-private and performs no
  filesystem effect.
- Added a persistence-internal, evaluator-owned candidate invalidation
  boundary. Exact source-state commands can mark `discovered`, `selected`, or
  `dismissed` observations unavailable, and can conclusively mark those states
  plus `unavailable` observations stale. Unavailable can refine to stale but
  never the reverse, and both states are terminal for the old scan-bound
  observation; neither review nor evaluator commands can revive it or enter
  planner/journal states. Full candidate facts and the succeeded source scan
  are bounded and validated
  before compare-and-set, exact retries and commit ambiguity reconcile safely,
  and review/evaluator races have one winner. The command remains sealed inside
  persistence until a deterministic evaluator supplies it; no engine, FFI, UI,
  CLI, or AI caller exists yet.
- Added a typed, crate-private candidate review-state boundary. Complete
  format-2 candidates can move between `discovered` and `selected`, from either
  exact source state to `dismissed`, and explicitly restore dismissal to
  `discovered`; there is no direct dismissed-to-selected edge, and exact target
  retries are idempotent. Selection means cleanup-review intent and requires a
  cleanup-capable, blocker-free observation, but it is not approval or cleanup
  authority. Full bounded candidate facts are validated before an exact SQLite
  compare-and-set, post-commit ambiguity is reconciled only while current
  secured storage remains valid, and legacy/corrupt/newer-schema rows fail
  closed. Complete candidates now also require a durably succeeded source scan
  on insert and load. Evaluator and planner/journal lifecycle ownership are now
  separate sealed persistence boundaries; engine and product transport remain
  later work.
- Added durable full scans to the shared engine. Scan admission canonicalizes
  and snapshot-bounds roots, fences newer read-only schemas, excludes
  overlapping root scopes within one engine session, and creates no durable ID
  for queued cancellations. Workers exact-reconcile random scan starts, bridge
  engine cancellation directly into the scanner, emit typed bounded progress,
  and publish completed-only immutable snapshots plus SQLite summaries.
  Cancelled/failed traversals receive no snapshot reference and retain
  conservative terminal summaries; a file published before an ambiguous
  database completion may remain as an unreferenced orphan. Panics best-effort
  settle Interrupted, close drives scanner cancellation before worker
  quiescence, and persistence ambiguity never rewrites success. Integration
  tests cover success/reopen, cancellation at queue/run/close, real scan
  failure, panic recovery, schema skew, canonical aliases, overlapping scopes,
  and ambiguous-start reconciliation. CLI/FFI/Swift transport, cross-process
  root leases, last-complete selection, crash recovery, and retention remain.
  The typed scan APIs are available through both `dux_core::engine` and the
  crate root; evolving task/error/result enums are explicitly non-exhaustive
  before the app boundary adopts them.
- Added completed-only, snapshot-ready scan accounting. A private provenance
  witness aligned to fresh arena node IDs carries logical bytes, optional
  physical allocation, times, object identity, link count, and flags without
  expanding the public/cache tree wire. Logical size counts every pathname;
  multiply linked file allocation is assigned once to the losslessly smallest
  path, while conflicting observations become allocation-unknown coverage
  facts. Unknown allocation is never replaced by logical size, and known CLI
  bytes remain visible when a sibling is unknown. The converter validates and
  remaps the live graph into canonical depth-first snapshot nodes, retains
  lossless host components, rejects followed symlinks unsupported by snapshot
  v1, and cannot be constructed from a cached, failed, or cancelled scan.
  Cache v7 invalidates earlier per-path hard-link totals. Focused tests cover
  sparse files, empty directories, hard links across walker thread counts,
  identity races, non-UTF-8 names on non-macOS Unix, exact codec round trips,
  invalid times, and followed-link refusal. The shared engine now consumes this
  artifact for durable publication.
- Added typed, non-authoritative scan coverage and issue reporting. Scanner
  workers now return the tree together with an authoritative terminal state and
  bounded canonical coverage facts; component policy exclusions, depth
  boundaries, permission/metadata failures, symlinks, mount boundaries,
  cancellation, probe timeouts, and pool exhaustion can no longer silently
  become zero-byte subtrees. Progress delivery is bounded and advisory, while a
  terminal cancellation claim prevents a late request from racing into a false
  completion. The probe pool distinguishes queued exhaustion from admitted
  syscall timeouts and fast-fails while every worker is known stuck.
  SQLite-v2 completion atomically stores the coverage status, optional
  permille, checked occurrence total, canonical message keys, and losslessly
  shortened issue paths with the scan summary and snapshot reference; bounded
  readers reject hostile or contradictory parent/child facts and exact retries
  revalidate retained storage and schema compatibility. Fresh CLI scans retain
  and display Complete, Limited access, or Partial coverage; legacy cache trees
  are explicitly shown as coverage unknown. No cleanup authority is derived
  from these observations. Engine scan tasks now persist them; Swift/FFI wiring
  remains a later milestone.
- Added typed raw capacity history on the existing SQLite v2 schema. Public
  opaque volume IDs and pressure labels feed a crate-private persistence layer
  that keeps required ordinary availability separate from optional
  important-usage availability, validates positive bounded capacity facts and
  lossless absolute mount observations, and never derives stable identity from
  mutable volume metadata. A single current-schema, cross-process-serialized
  transaction applies monotonic volume metadata, suppresses routine samples
  after one per UTC hour, and immediately records actual stored-pressure
  transitions. Exact collisions and ambiguous commits are reconciled only after
  retained-storage/current-schema revalidation and a complete fact plus volume-
  interval match; bounded latest/cursor reads reject hostile rows before
  returning typed observations. Important-only UI samples are intentionally
  not persisted because ordinary availability is not fabricated. Swift/engine/
  FFI/CLI wiring, pressure evaluation, daily rollups, pressure episodes, and
  retention remain separate work.
- Added a crate-private immutable application snapshot subsystem separate from
  the legacy CLI cache. Its frozen v1 depth-first wire preserves lossless host
  names, exact graph and aggregate semantics, optional times and Unix identity,
  typed scan flags, and a trailing SHA-256 checksum behind hard file/node/depth/
  path bounds; golden digests and checksum-valid hostile fixtures prevent
  silent format drift and ambiguous trees. An independently marker-owned
  private store uses unique temps, atomic no-replace publication, exact
  0700/0600 or protected-DACL validation, retained identities, and read-only
  final handles. Snapshot mutations hold the current-schema SQLite fence before
  the snapshot writer lock, retain publication exclusion through the exact
  terminal-scan CAS, and reconcile ambiguous commits without ever publishing a
  database reference first. New tests cover version-skew races, collisions,
  missing/corrupt references, restrictive umasks, macOS ACLs, and Windows
  DACL/reparse/link/publication behavior. The format remains non-authoritative;
  retention and abandoned-temp maintenance remain separate roadmap work. Typed
  coverage, the completed-only fresh-scan converter, and durable engine
  publication attach without changing the v1 snapshot wire.
- Added a crate-private cleanup-lock-coupled operation-journal state machine.
  A non-cloneable, non-shareable lease generates and owns the process identity,
  claims pristine plans as generation one, and fences every heartbeat,
  cancellation, validation, effect-intent, reconciliation, recovery, and
  terminal transition by the exact owner and generation. Full bounded loads
  validate the immutable plan and complete dynamic graph before use.
  Same-scope recovery writes only after `DefinitelyGone`, exact-CASes the stale
  claim, increments its generation, resets in-progress `validating` work, and
  preserves interrupted effects as `outcome_unknown`; live and unknown owners
  are journal-state no-ops.
  Millisecond-canonical effect-start receipts retain finer ordering time, are
  commit-issued or ambiguity-reconciled and cleanup-lock-revalidated, and
  reject late cancellation before a future OS call. Failed capability changes
  retain their lease/claim
  for exact retry reconciliation; fault injection proves an ambiguous committed
  effect intent cannot be retried with a substituted fine-grained timestamp.
  No executor, filesystem effect, FFI, CLI, Swift, or AI authority is
  introduced; focused regressions cover every outcome, cancellation,
  corruption, lock exclusion, and native death recovery.
- Added a crate-private, versioned process-instance identity and tri-state
  liveness probe as a prerequisite for cleanup-journal recovery. The bounded
  128-byte owner representation binds PID and OS start time to a hashed macOS
  boot-session or Linux boot/PID-namespace scope plus a random claim nonce.
  Only a same-scope absence or start-token change is `DefinitelyGone`; changed
  scope, malformed/partial OS evidence, permissions, and unsupported host proof
  are `Unknown`. Windows can prove an exact live match through a retained
  process handle but deliberately cannot prove death until a reliable host
  scope exists. Native subprocess tests cover live owners plus graceful and
  abrupt death. This checkpoint changes no journal state and grants no cleanup,
  recovery, plan, or effect authority.
- Added a crate-private, permanent store-wide cleanup-effect lock distinct from
  SQLite's writer lock. Existing owned stores provision the exact private
  control file only while holding the writer lock. It flushes the lock and
  ready control before durably advancing the existing ownership marker from
  layout v1 to v2; v2 makes later absence fail closed instead of recreating a
  second lock identity. Retained handles, exact markers, root inventory, ownership,
  permissions, links, and path identity are revalidated around bounded lock
  acquisition. Windows retains both cleanup controls without delete sharing so
  `LockFileEx` cannot remain on a displaced file. Same-process, independent
  writer-lock, cross-process, malformed-layout, link, special-file, and native
  replacement regressions cover the boundary. The guard proves exclusion only:
  it carries no plan, owner, recovery, journal, target, or effect authority.
- Added crate-private planned cleanup journals on SQLite schema v2. One
  immutable cleanup plan is prepared and bounded before locking, matched
  exactly to complete persisted candidate observations, and atomically stored
  with contiguous items, paths, evidence, warnings, and each proposed effect
  even in dry-run mode. The exact-ID reader returns either an explicit legacy
  summary or a complete non-executable planned observation, rejects malformed
  or polluted children and newer schemas, and fits the shared query budget at
  the 256-path/512-evidence limit. This immutable history boundary itself
  exposes no execution, owner, recovery, or transition authority; the separate
  sealed mutable-journal layer is described above.
- Added crate-private typed candidate history on SQLite schema v2. Complete
  deterministic findings are inserted atomically with ordered bounded paths,
  evidence, and blockers, then loaded only as non-executable observation
  records; migrated v1 rows return explicit incomplete summaries. Exact-ID
  reads bound SQLite work and validate storage types/lengths, ordinals, enum
  shapes, source-scan presence, policy pairs, and cloud-upload evidence before
  returning data. No status mutation, plan construction, FFI surface, or
  cleanup authority is introduced.
- Added checksummed SQLite schema v2 for complete, non-authoritative candidate
  and cleanup history. The atomic v1→v2 migration preserves legacy summaries
  under an explicit legacy format without inventing absent facts; new records
  have normalized ordered path/evidence/blocker data, nanosecond plan facts,
  frozen per-item proposed actions, multi-path operation journals, warnings,
  and owner-generation crash-recovery state. Exact per-version object
  inventories and fingerprints, populated legacy migration coverage, and
  fail-closed semantic constraints protect version skew and malformed rows.
  This schema grants no cleanup authority; typed candidate/session APIs and
  executor reconciliation remain separate checkpoints.
- Added the first typed persistence layer for non-authoritative scan history.
  Crate-private start, terminal compare-and-set, and exact-ID load operations
  preserve stable IDs, lossless accepted host-codec absolute roots, checked times and byte counts,
  reject duplicate or terminal rewrites, recheck schema compatibility while
  holding the coordinator and cross-process writer lease, and bound SQLite
  read work. Completed summaries survive coordinator teardown and reopen. Scan
  APIs remain absent from the engine; the separate snapshot subsystem above can
  now attach an exact immutable file, but historical paths stay non-actionable.
- Added a versioned private SQLite foundation owned by the shared engine. A
  checksummed v1 `STRICT` schema covers all planned aggregate/history tables
  with bounded fields, constrained semantic text, and a tested lossless
  UTF-8/UTF-16 path codec. Transactional migrations and live compatibility
  refresh use separate VM/deadline budgets, WAL, bounded lock waits, and a
  stable advisory writer lease; bounded schema materialization and runner-owned
  transaction control prevent crafted metadata or migration batches from
  escaping those limits. Valid newer schemas reopen read-only while
  foreign, unmarked, corrupt, drifted, or over-budget stores return path-free
  categories. Private staged provisioning atomically publishes an immutable
  ownership marker plus empty database without replacing a racing path, then
  durably records successful initialization so zero-length corruption cannot
  be mistaken for a fresh store, and
  permits the exact future `snapshots`, `ai`, and `logs` siblings. Unix stores
  enforce owner/mode/link/no-follow invariants, with deny-only publication-parent
  ACLs and exact final-object ACL rejection on macOS. Windows uses protected
  owner-only DACLs, handle-relative stage creation, handle-bound publication, a
  retained final-root rename guard, and exact SQLite-sidecar DACL repair. Real
  Unix crash regressions, cross-platform writer/version-race coverage, and native macOS/Windows
  storage tests cover the platform-specific boundaries. No cleanup authority
  is exposed by this checkpoint.
- Added the shared core engine handle and bounded per-session FIFO task registry with explicit disjoint storage paths, fixed workers/queue/event/result retention, opaque non-reused task IDs, typed cancellation and terminal state, panic containment, nonblocking close with quiescence, and a bounded read-only formatting operation. The application architecture owns one session; the UniFFI engine remains smoke-only, and no scan, domain-persistence operation, AI, plan, or cleanup authority is exposed by this checkpoint.
- Added a temporary core-owned adapter for the legacy CLI permanent-delete path. Single and batch requests consume opaque target-bound plans through one guarded effect boundary, and repository policy prevents the adapter from being exposed through FFI or the macOS app.
- Added a CI-enforced destructive-call boundary: compiler-resolved per-platform Rust filesystem/process denials plus a cross-language repository scanner with one-use registered exceptions, executable/shebang discovery, release enforcement, and executable policy tests. The XCFramework builder now rejects arbitrary destinations and symlinked output parents before any build tool or destructive mutation runs.
- Added the normative DUX security design: an implementation-status-aware threat model and cleanup shipping gate covering the one-way observation-to-executor authority chain, path and protected-root evidence, rule provenance, Trash/permanent/eviction semantics, automation, AI isolation, private persistence, TCC and unsandboxed authority, FFI/CLI boundaries, supply-chain integrity, incident response, and the explicit gaps in the current legacy CLI deletion path.
- Added a versioned dangerous-path corpus and deterministic path-safety properties covering host-native lexical rejection, raw encoding failures, all independently authored protected-root entries, dynamic home/profile guards, component boundaries, evidence-form precedence, and explicit non-authoritative known gaps. An isolated, dependency-audited libFuzzer target uses a bounded filesystem-free `cfg(fuzzing)` adapter, retained seeds, pinned smoke CI, and a weekly campaign; fuzz crashes must become permanent corpus regressions.
- Added a revisioned, crate-private protected-root registry with host-independent macOS, Linux, and Windows policy tables. Component-aware rules distinguish exact structural anchors, hard-denied subtrees, and descendants that require a future code-owned deterministic-rule grant; configured validated home roots and other profiles beneath conventional/configured profile containers fail closed, and requested/canonical scan and target spellings are assessed with hard denies taking precedence. Production construction remains sealed until trusted account/known-folder discovery exists. The registry returns only a non-authoritative textual disposition—not an “allowed” witness—until trusted account/volume discovery, full root ancestry, mount-location identity, and executor revalidation exist.
- Added a crate-internal, non-authoritative path-validation service with raw host-native lexical checks, strict scan-root binding, descriptor-relative no-follow inspection on Unix, repeated no-follow handle inspection on Windows, alias-preserving canonical scope verification, stable volume/object identity, ordered ancestor evidence, target kind, and hard-link counts. It rejects ambiguous syntax, invalid/lossy encoding, symlinks and reparse points, special entries, cross-volume descendants, and paths that change while being inspected; it performs no mutation and grants no cleanup authority. Windows evidence remains full-path based and cannot become actionable until later handle-bound executor revalidation exists.
- Added immutable cleanup-plan and plan-item domain types with fixed expiration, checked estimates, exact candidate/rule provenance, explicit dry-run/Trash/permanent/cloud-eviction semantics, mandatory warnings, and fail-closed structural invariants. Construction remains sealed pending validated path witnesses, and the types provide no approval or execution capability.
- Added a versioned deterministic-rule catalog schema and an internal fail-closed JSON loader with explicit fields, bounded documents and collections, stable ordering, duplicate rejection, exhaustive policy validation, an ASCII byte-bounded matcher grammar, and synthetic schema/domain fixtures. The loader is crate-private and no catalog or fixture ships in the app.
- Added product-neutral candidate and deterministic rule domain types with stable ID/revision pairs, typed evidence and blockers, validated scope/matcher/guard policy, strict safety/action compatibility, localization keys, provenance, and exact per-path cloud-upload evidence for proposed eviction. Candidate construction remains internal and non-authoritative; no cleanup rule or execution path is enabled.
- Accepted architecture decision records now define the native SwiftUI app, direct Developer ID DMG distribution, unsandboxed access model, and shared Rust engine/FFI boundary. Local AI subprocess adapters are explicitly gated on an adversarial macOS permission and confinement spike.
- Added the initial `dux-ffi` UniFFI crate with a version handshake and typed formatted-size smoke result, producing Rust library, static-library, and dynamic-library artifacts for later Swift packaging.
- Added a fail-closed macOS build script and CI job that compile `dux-ffi` for arm64 and x86_64, merge the static libraries, and package a validated universal `DuxFFI.xcframework`.
- Added reproducible UniFFI Swift/header/module-map generation and a minimal unsigned SwiftUI app that imports and links the generated bindings through the universal XCFramework.
- Added an off-main-thread Swift engine service, `@MainActor` application model, rendered typed Rust smoke result, and linked macOS test coverage for the concurrency handoff.
- Added a clean-checkout macOS build gate that regenerates configuration-matched Rust and UniFFI artifacts, tests Debug, and builds unsigned universal Debug and Release apps.
- Accepted UniFFI as the private Swift/Rust transport, with strict version, generation, concurrency, error, lifetime, and batching boundaries plus a documented C-ABI replacement path.
- Added the first opaque `DuxEngine` session with FFI contract v2, typed closed-session errors, idempotent close, off-main service error mapping, and linked Swift/Rust lifetime coverage.
- Added the menu bar-first macOS shell with a window-style `MenuBarExtra`, singleton Explorer window, native Settings scene, shared application model, accessible actions, String Catalog, and `LSUIElement` agent configuration.
- Added off-main startup-volume capacity sampling for the macOS shell. The menu bar and Explorer prefer Foundation's important-usage capacity, explicitly fall back to ordinary filesystem availability, and render an accessible capacity summary without requiring a directory scan.

### Fixed
- Kept the macOS menu-bar process ineligible for AppKit automatic termination
  for its full launched lifetime. Dismissing the `MenuBarExtra` popover can no
  longer let macOS retire DUX merely because no ordinary windows are open;
  explicit Quit still follows the existing ordered shutdown path and releases
  the lifetime lease at termination.
- Corrected snapshot-store provisioning so every new
  `.dux-snapshot-stage-<32 lowercase hex>` is created inside its retained,
  marker-owned database root and atomically published to the sibling
  `snapshots` directory without replacement. The SQLite root walk uses a
  64-stage cap, fixed total-entry and 256-KiB aggregate-name budgets, and
  sampled elapsed-time checks against 250 ms. Malformed stage names,
  non-directory/symlink/reparse entries, unsafe stage-directory
  permissions/DACLs, and a 65th stage fail closed; a current-user-owned Unix stage
  with a stricter subset of mode 0700 is tolerated as opaque interrupted
  creation debt. Contents are intentionally not inspected here, and this
  checkpoint adds no cleanup. Pre-correction stages outside the database root
  are deliberately neither adopted nor removed: their fixed marker contains no
  root identity, so two databases sharing a parent cannot safely attribute
  them. Future automatic maintenance is limited to exact-marker-owned
  root-local stages, while empty, partial, malformed, extra-entry, and every
  legacy external stage remain untouched unless stronger durable proof is
  introduced.
- Corrected the macOS CI deployment-target checks to use the generated
  `DUX.app/Contents/MacOS/DUX` product path, so the already-built universal
  Debug and Release artifacts are actually inspected instead of failing on a
  stale mixed-case path.
- Legacy CLI deletion planning now validates the complete target as an absolute, control-free, valid-text, normal-component strict descendant and rejects filesystem-boundary crossings for the target, ancestors, and marker evidence. This closes a forged-cache terminal `.`/`..` alias that could otherwise escape or collapse the selected scan root before permanent removal.
- Exact `$HOME/Library` and `$HOME/AppData` roots are now denied by protected-path policy; only their descendants and scan scopes return the non-authoritative specific-rule requirement.
- Prevented deletion of the active scan root.
- Failed filesystem deletions no longer remove items from the displayed or cached tree.
- Unicode paths are truncated safely without slicing through UTF-8 characters.
- Scanner path exclusions now use component-aware rules, and crossed filesystems are classified before traversal instead of treating every `/Volumes` path as unsafe.
- Scanner metadata and filesystem probes now use a process-wide bounded worker pool instead of spawning one detached thread per directory.
- Probe results completed after their end-to-end deadline are rejected even if the waiting scanner thread was descheduled while the result arrived.
- Permanent-delete dialogs now require an explicit `y` and show the effective item count; Enter no longer confirms deletion.
- Graceful quit now waits for active deletion workers, applies their results, and orders cache writes so an older snapshot cannot overwrite the post-delete tree.
- Permanent deletes now capture non-following filesystem identity before confirmation and re-check it immediately before removal; replaced or missing entries are skipped without changing tree state or deletion statistics.
- Build-artifact entries now require exact, regular, non-symlink marker files instead of directory names alone. Ambiguous names such as `DerivedData`, `Build`, `dist`, `vendor`, and `.cache` are omitted until stronger rules exist.
- Artifact marker identities and every directory identity from the scan root to the target parent are re-checked before permanent deletion. Changed evidence, replaced ancestors, and symlink/reparse ancestors fail closed.
- Followed symlinks retain path provenance in scan snapshots, and deletion is refused below a known followed-symlink ancestor.
- Multi-delete now uses a fixed pool of at most four workers instead of spawning one thread per selected item. Per-item panics become failures without stranding later queued deletions, and graceful quit continues waiting for the complete confirmed batch.
- Cache writers now use exclusively created, process-specific temporary files instead of one shared `.tmp` path, preventing concurrent writers from truncating each other's in-progress snapshots.
- Footer selection bytes no longer double-count descendants when their selected ancestor already includes their size; the displayed bytes now match the effective multi-delete roots.

### Changed
- CLI cleanup surfaces now consistently identify deletion as permanent and label successful-item byte totals as scan estimates rather than measured freed capacity.
- Moved artifact classification, large-file projection, and staleness evaluation from the TUI into deterministic `dux-core` APIs. The CLI now retains only view caching and presentation while continuing to pass core marker evidence through pre-delete identity validation.
- File and directory scan nodes now retain modification times from their size metadata snapshot, allowing shared projections to account for recent file activity without another filesystem query. Unsupported pre-Unix-epoch values remain explicitly unknown so they cannot break cache writes or become trusted age evidence.
- Cache version bumped to v6 so scans created with the previous scanner policy, without symlink provenance, or without file modification times auto-invalidate.
- The Build Artifacts view now labels entries as “Marker-matched”; classification identifies likely tooling ownership but does not assert that contents are automatically safe or reproducible.
- Cached headers show the original scan age. Press `r` while browsing to rescan directly from the filesystem; the previous tree remains available if the replacement scan fails.
- CI now enforces RustSec and dependency source/license policy. Release automation tests and builds every target before publishing, produces a verified `SHA256SUMS`, publishes GitHub assets through a draft, and updates Homebrew without a third-party action handling the tap token.
- The application lockfile is now committed. Crossbeam was advanced past RUSTSEC-2026-0204, Postcard's unused heapless defaults were disabled, and Ratatui/Crossterm were upgraded to remove unmaintained and yanked transitive crates. The declared minimum Rust version is now 1.88.
- The scanner diagnostics example now lives in `dux-core/examples/debug_scan.rs`, so Cargo discovers it normally and includes it in the published `dux-core` package.

### Fixed
- Hardened the menu-bar lifetime guard against SwiftUI scene restoration.
  `MenuBarExtra` can re-enable AppKit automatic termination after the launch
  delegate returns, so DUX now reasserts its lease after restoration and at
  both last-window and termination callbacks before cancelling incidental
  dismissal requests. Explicit Quit still follows the ordered shutdown path.

## [0.5.0]

### Added
- **Multi-select**: Press `v` to toggle items in/out of selection, then navigate with arrow keys or `j`/`k` to extend the range. `K`/`J` (uppercase) also extend selection. Selection count and total size shown in purple in the footer.
- **Multi-delete**: Press `d` with items selected to delete them all at once. Confirmation dialog shows up to 5 paths with sizes and a total. Deletions run concurrently with a progress overlay showing a bar, completed/total count, and freed bytes.
- **Selecting mode**: When `v` is pressed, entering selecting mode where all navigation automatically extends the selection. Press `v` on a selected item to unselect it, or `Esc` to clear all.
- **Large Files view**: Flat list of all files sorted by size, helping find big files buried deep in the tree. Press `Tab` to switch views.
- **Build Artifacts view**: Detects known build directories (`target/`, `node_modules/`, `DerivedData/`, etc.) with a staleness indicator. Press `s` to cycle the stale threshold (1d/7d/30d/90d/All).
- **View switching**: `Tab`/`Shift-Tab` cycles between Tree, Large Files, and Build Artifacts views. All views support navigation, deletion, and open-in-Finder.
- Help overlay now includes a Views section documenting the new key bindings.

### Fixed
- Scanner no longer hangs on cloud storage FUSE mounts (Google Drive, OneDrive, iCloud Drive). Added these paths to the skip list.
- Scanner now probes directories with a 5-second metadata timeout before descending. Directories that don't respond in time (slow FUSE, hung NFS, etc.) are automatically skipped.
- **Build Artifacts staleness**: Now uses the newest mtime of any descendant directory, not just the top-level directory. Recently-built `target/` directories no longer incorrectly show as stale.
- **Build Artifacts dedup**: Subdirectories inside an artifact (e.g. `target/debug/build`) no longer appear as separate entries.
- **Stale threshold cycling**: No longer triggers a full tree rebuild — updates `is_stale` flags in place.

### Changed
- `Tab` now switches views instead of toggling expand/collapse (use `Space` for toggle).
- Footer hints update dynamically based on the active view.
- Header shows the active view name for non-Tree views.
- `Esc` now clears selection first (if any), then goes back.

## [0.4.0]

### Added
- **Scan caching**: Persist scan results to disk and reload on subsequent runs if the root directory hasn't changed. Cache files are stored in the system cache directory (`~/.cache/dux/` on Linux/macOS). Use `--no-cache` to force a fresh scan.
- **Incremental tree updates**: After deleting files/directories, the tree is updated in-place without requiring a rescan. Sizes and file counts propagate correctly up to the root.
- **Delete statistics**: Track and display space freed during the session. The footer shows "Freed: X.X GB (N items)" when items have been deleted.
- **Cache indicator**: Header shows "(cached)" when the tree was loaded from cache.
- **Save cache on quit**: Deletions made in dux are now persisted to cache so deleted items no longer reappear on next launch.
- **Smarter cache invalidation**: Spot-check mtimes of the 32 largest directories on cache load to detect deep filesystem changes that root mtime alone misses.

### Changed
- Tree data structure now uses tombstones for deleted nodes, allowing efficient in-place updates.
- `DiskTree` is now serializable with serde for cache persistence.
- Cache format version bumped to v3 (old caches auto-invalidate).

## [0.1.0] - Initial Release

### Added
- Interactive TUI disk usage analyzer
- Parallel filesystem scanning with jwalk
- Tree view with expand/collapse
- Drill-down navigation
- Open in Finder (macOS)
- Delete files/directories with confirmation
- Keyboard navigation (vim-style and arrow keys)
- Size bar visualization
- Progress indicator during scan
