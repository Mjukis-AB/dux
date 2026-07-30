# DUX macOS application

This directory contains the native macOS menu-bar application and its generated
Swift bindings to the shared Rust engine.

## Build the Rust XCFramework

Requirements:

- Xcode command-line tools with `xcodebuild` and `lipo`;
- the repository's Rust toolchain;
- the `aarch64-apple-darwin` and `x86_64-apple-darwin` Rust targets.

Run:

```bash
./dux-macos/scripts/build-rust-xcframework.sh
```

The default Release artifact is written to
`dux-macos/Generated/DuxFFI.xcframework`. Generated output is ignored by Git.
Set `CONFIGURATION=Debug` for a Debug build, or pass a different output path as
the first argument.

## Build the bundled universal CLI

The app embeds a fixed Release-mode `dux` companion plus a strict signed-bundle
manifest. Build both macOS architectures and atomically publish the ignored
generated pair with:

```bash
./dux-macos/scripts/build-bundled-cli.sh
```

The fixed outputs are:

```text
dux-macos/Generated/dux-cli-bundled
dux-macos/Generated/dux-cli-bundled-metadata.json
```

The manifest is derived only from the built CLI's hidden
`dux __bundle-metadata` response and adds the complete executable SHA-256 plus
the exact `arm64`/`x86_64` architecture set. The builder rejects malformed,
duplicate, missing, unknown, or version-skewed metadata and verifies the final
Mach-O deployment target is macOS 14. It applies a deterministic ad-hoc
Hardened Runtime signature with the development-only
`se.mjukis.dux.spike.cli.debug` identifier before hashing. The notarized release
lane replaces that signature with the frozen Developer ID
`<bundle-id>.cli` identity and rebinds the outer signed manifest to those final
bytes.

## Generate bindings and build the spike app

Generate the matching C header, module map, reviewable Swift source, and final
XCFramework. The same command also regenerates the universal bundled CLI and
its manifest before Xcode copies them into the application resources.
`CONFIGURATION` must match the Xcode configuration that will consume the FFI
library:

```bash
CONFIGURATION=Debug ./dux-macos/scripts/generate-bindings.sh
```

Then build the unsigned spike application:

```bash
xcodebuild \
  -project dux-macos/Dux.xcodeproj \
  -scheme Dux \
  -configuration Debug \
  -derivedDataPath target/dux-derived-data \
  CODE_SIGNING_ALLOWED=NO \
  build
```

For a Release build, regenerate the library and bindings before invoking
Xcode so a stale Debug XCFramework cannot be packaged:

```bash
CONFIGURATION=Release ./dux-macos/scripts/generate-bindings.sh
xcodebuild \
  -project dux-macos/Dux.xcodeproj \
  -scheme Dux \
  -configuration Release \
  -destination 'generic/platform=macOS' \
  -derivedDataPath target/dux-derived-data-release \
  ARCHS="arm64 x86_64" \
  ONLY_ACTIVE_ARCH=NO \
  CODE_SIGNING_ALLOWED=NO \
  build
```

The generated high-level Swift source is committed at
`Dux/Generated/DuxFFI.swift` so interface changes are reviewable. The generated
C header/module map, XCFramework, bundled CLI, and CLI manifest remain under
ignored `Generated/` and must be recreated before building from a clean
checkout. Never combine bindings, a library, CLI bytes, or metadata produced
from different source revisions or build configurations.

## Review storage changes

Explorer’s **Changes** mode compares the selected retained snapshot with its
immediately preceding comparable snapshot. Open **Latest Snapshot**, choose a
retained scan, then select **Changes**. DUX shows the older-to-newer timestamps,
both scans’ coverage, current and previous logical sizes, separate observed
growth and shrinkage, a sortable six-column table, and a magnitude treemap.
Return opens a selected matched or one-sided historical directory; Backspace
returns to its parent. Selecting a row or treemap cell updates the same
read-only inspector.

Orange `↑ Grew`, blue `↓ Shrunk`, purple `+ First observed`, gray
`− No longer observed`, and pink `⇄ Replaced` use distinct symbols and border
patterns as well as color. Omitted cells are split into **Other Growth** and
**Other Shrinkage**, while zero-magnitude observations stay in the table.

The comparison is deliberately not a cleanup estimate. Its persistent
disclosure is: “Logical-size observations, not verified capacity change or
reclaimable space.” It contains no live paths, Trash or cleanup actions,
candidate admission, AI authority, or effect capability. If no comparable
snapshot exists or the child lease expires, ordinary Browse remains available.

The repository Cargo configuration pins `MACOSX_DEPLOYMENT_TARGET=14.0` for
Cargo invocations started inside the DUX checkout, including native
dependencies compiled by build scripts. The macOS helper scripts also export
the baseline when invoked from elsewhere. Release and app CI verify the final
Mach-O load command with:

```bash
bash scripts/check_macos_deployment_target.sh path/to/Mach-O
```

This is a DUX workspace and release-artifact guarantee. A downstream crate
consumer builds under its own workspace Cargo configuration and deployment
policy.

`project.yml` is the source for `Dux.xcodeproj`. Regenerate the project with
XcodeGen 2.44.1 after changing project structure:

```bash
xcodegen generate --spec dux-macos/project.yml
```

The `se.mjukis.dux.spike` bundle identifier is intentionally temporary and must
not be used for TCC, launch-at-login, or release identity testing. The production
identifier and signing identity remain an explicit later decision.

## Build a signed and notarized local release

The fail-closed release script is present before the production identity is
frozen, but it deliberately cannot invent or use the temporary spike identity.
After Milestone 9 selects the final bundle ID, Apple team, and signing identity,
store notarization credentials interactively in Keychain:

```bash
xcrun notarytool store-credentials dux-notary
```

From a clean commit exactly tagged `vX.Y.Z`, with all DUX Cargo package versions
equal to `X.Y.Z`, run:

```bash
DUX_VERSION=X.Y.Z \
DUX_BUILD_NUMBER=1 \
DUX_BUNDLE_IDENTIFIER=the.frozen.bundle.id \
DUX_TEAM_ID=ABCDEFGHIJ \
DUX_SIGNING_IDENTITY='Developer ID Application: Exact Name (ABCDEFGHIJ)' \
DUX_NOTARYTOOL_PROFILE=dux-notary \
./dux-macos/scripts/release-notarized-dmg.sh
```

The script accepts no password, Apple ID, or API private-key path. It runs all
local release gates, requires matching universal Debug/Release layouts and
byte-identical pre-signing CLI resources, verifies the reviewed empty
`Config/Release.entitlements`, signs the bundled CLI with the explicit
`<bundle-id>.cli` code identifier, signs all remaining code inside-out,
rebinds the outer signed resource manifest to the signed CLI bytes, notarizes
and staples the app, creates the DMG with an Applications link, then
independently signs, notarizes, staples, mounts, and Gatekeeper-assesses the DMG
and contained app. The mounted artifact must retain the exact universal,
macOS-14 CLI, signed manifest hash/version/schema tuple, Team ID, Developer ID
authority, Hardened Runtime, and secure timestamp. It publishes only after every
check succeeds, under
`target/dux-macos-release/vX.Y.Z`, with the DMG, SHA-256 sidecar, manifest,
sanitized submission records, and full Apple notarization logs. Output is
same-filesystem atomically published and immutable: an existing version
directory is never overwritten. Failed private staging is retained at the path
printed by the script for diagnosis.

The app owns one opaque real `DuxEngine` through `EngineService`. Construction,
migration, every synchronous UniFFI call, and explicit close run lazily on its
dedicated utility queue; app launch never opens the database on the main actor.
The service maps generated errors to app-owned errors and converts generated
records before they reach render state. ARC release is not a substitute for
explicit cancellation, review release, or close. Run the linked lifecycle and
concurrency tests with the same `xcodebuild` arguments above, replacing `build`
with `test`.

`AppRuntime` owns the engine service, five-minute capacity scheduler,
private-store maintenance scheduler, pressure-policy resample router, and
Explorer review controller. Shutdown first generation-invalidates policy work
and the resample router, then stops capacity sampling, cancels maintenance,
releases all reviews, and closes Rust. Review leases renew every five minutes and on
wake/significant time change; generation tokens discard acquisitions or renewal
failures that complete after the selected review changed. FFI close also drains
still-live registered review pins and rejects later renewal.

Settings exposes FFI v27's explicit **Rust project discovery** enrollment. The
picker accepts one canonical direct file named `cargo`; the usual
`~/.cargo/bin/cargo` rustup proxy is a symlink and is intentionally rejected.
For a rustup-managed toolchain, choose its direct binary below
`~/.rustup/toolchains/<toolchain>/bin/cargo`. DUX does not search `PATH`, invoke
rustup, install or update a toolchain, or accept command text.

The first phase is static inspection: it runs no selected bytes and presents the
exact lossless path, executable SHA-256, and bounded macOS signing evidence.
Only the opaque engine-owned preview can reach the second phase. The final
confirmation is bound to the exact displayed preview and permits one
core-owned, bounded verbose-version invocation of those bytes to verify Cargo
1.96.0. Ad-hoc signing is local integrity evidence, not publisher identity; CMS
evidence does not create a publisher allowlist. A confirmed invocation is shown
as finishing and is not falsely cancellable. Dismissal releases unconfirmed
previews, and one memoized ordered shutdown waits for any confirmed mutation
before closing the engine exactly once even when multiple callers request
shutdown.

Enrollment grants deterministic Rust workspace discovery provenance only. It
cannot select a candidate, clear `ProtectedPath`, create or approve a cleanup
plan, schedule work, grant AI authority, or delete data. Revocation is separately
confirmed and deletes nothing. If a native mutation returns an unprovable or
malformed response, DUX never retries it: Settings performs one read-only status
reload, visibly blocks further mutations until authoritative state is available,
and offers an explicit **Reload Cargo status** action if that read fails.

Cleanup safety is deny-by-default. A fresh store and **Restore DUX default**
both block permanent-cleanup effects. Settings can enable the path-free global
gate only after loading its authoritative state and receiving the exact typed
sentence `ENABLE PERMANENT CLEANUP`; disable and reset act immediately. This
choice cannot select a target or approve a plan, and AI, CLI, schedules, low
disk pressure, and display records cannot override it. Existing explicit stored
consent migrates forward; an enabled legacy default does not.

Explorer exposes FFI v33's **Permanent-safe plan preview** for one exact
revision-3 Rust-target candidate. Preparing it passes only the candidate ID
through the already-retained snapshot review. Rust derives and revalidates the
current target, source scan, permanent-safe mode, plan identity, estimate,
warnings, newest observed modification time, exact seven-day minimum age, and
short effective expiry. All three timestamps cross FFI losslessly as
seconds/nanoseconds. It inventories the live target descriptor-relatively
before publishing the preview and rejects recent, future, missing, linked,
special, symlinked, or changed entries. The returned opaque child supports
information reads, release, and the gated consume-once transition into the
core-owned permanent-safe task. The native service and review controller
implement that consuming edge without accepting a path or reconstructed plan:
the controller stores the complete immutable information shown to the user,
requires exact equality at execution, and removes the child before the
asynchronous start. Its start accepts no path, identifier, timestamp, approval
Boolean, callback, AI result, command, or retry token; its task observation is
path-free. The existing confirmation-gated Explorer Trash route is separate
and cannot consume the preview child or its display DTO.

The controller owns the child separately from its renewable parent review,
refreshes the immutable observation every 15 seconds, and releases the child
before the parent on candidate, mode, snapshot, expiry, cancellation, window
close, or app shutdown transitions. Parent renewal never extends the frozen
child expiry. The UI distinguishes the exact current target from historical
candidate evidence and shows its estimate, ordered warnings, and expiry with
only **Prepare plan preview**, **Check again**, and **Close preview** actions.
It explicitly states that the preview is not approved and changed no files.
Current paths are lossless byte observations: unsafe/hidden Unicode and
non-UTF-8 bytes use a deterministic escaped display that Swift validates
byte-for-byte before presentation.

Internal Debug builds add an explicit destructive confirmation bound to that
immutable preview and a generation-fenced global status banner. The banner
keeps observing after the Explorer window closes, never retries, and exposes
explicit cancellation; ordered app shutdown requests cancellation and waits
for the observer. Dropping a v31 task observer still has no cancellation or
retry semantics, and every owning-engine start attempt consumes the opaque
review once even if later core admission refuses it.

This action is deliberately unavailable in public Release builds until every
cleanup release condition in `SECURITY_DESIGN.md` §17.3 is evidenced. XcodeGen
defines `DUX_INTERNAL_PERMANENT_SAFE_CLEANUP` only for Debug, Release shows a
locked unavailable label instead of the destructive action, and the notarized
release script fails if resolved Release build settings contain that condition.

Cleanup History exposes FFI v29's exact-session, observation-only drill-down.
The list supplies one bounded stable session ID; Rust reloads and validates the
complete stored graph, and Swift independently checks the version, exact ID,
lifecycle and legacy/complete shape, contiguous item ordinals, aggregate and
status counts, checked complete-session estimate sum, exact derived warning
order, unique summary IDs, and bounded stable categories. Selection, summary
refresh, read retry, Back, destination close, and shutdown all
generation-fence late replies.

The detail view keeps the plan estimate separate from the signed verified
available-capacity change and labels missing verification as unknown rather
than zero. Accessible item and path-record charts have textual legends, and
ordered cards show only rule, policy, status, error-category, and aggregate
metadata. No path, evidence payload, candidate ID, claim, receipt, history
clear, cleanup retry, approval, scheduler, AI, or executor input crosses this
route. **Try reading again** only reloads immutable history.

Settings exposes FFI v30's separate **Clear cleanup history…** privacy action.
It first prepares one two-minute, engine-bound, consume-once preview of the
exact validated terminal history graph, then shows the exact session count and
date range in a destructive confirmation. No session selector or path crosses
the boundary. Active, recovering, and uncertain cleanup evidence is preserved;
the confirmed transaction can delete only DUX's five cleanup-history tables.
Closing Settings releases an unconfirmed preview, while app shutdown waits for
an already confirmed operation. Success, failure, changed-history, and
outcome-unknown responses are never retried and each triggers exactly one
read-only history refresh. The screen explicitly distinguishes this metadata
privacy action from cleanup: it removes no files, snapshots, scans, candidates,
settings, exclusions, capacity samples, or AI insights, performs no database
compaction or capacity resample, and promises no free space.

Private-store maintenance is deliberately separate from user cleanup and from
Milestone 8 automations. After a 60-second startup grace, the scheduler runs at
most one sealed Rust batch at a time across scan recovery, terminal temps,
unleased temps, provisioning stages, physical orphans, snapshot retention, and
history. Recovery is ordered before terminal-temp reconciliation so a proven
dead scan can become `interrupted` without touching its temp lease; the next
sealed owner may then reconcile that residual. The scheduler uses one-minute
inter-batch spacing, a six-hour normal cycle, distinct bounded
backoffs, Low Power Mode/thermal gates, and wake handling that cannot erase
startup grace or a failure/resource backoff. No Swift or FFI input selects a
path, process owner, inventory item, or victim.

The application shell is menu bar-first. `MenuBarExtra` must remain the first
scene so the macOS 14 automatic scene-launch behavior does not open Explorer at
startup. `Window(id: "explorer")` supplies one reusable normal window, and the
native `Settings` scene shares the same `AppModel`. The focused `AppActivation`
bridge requests foreground activation with `NSApplication.activate()` after a
menu action opens Explorer or Settings. The generated Info.plist sets
`LSUIElement=true`, so closing Explorer leaves DUX running without a default
Dock icon. User-facing shell keys live in `Dux/Resources/Localizable.xcstrings`.

Settings owns one opt-in Launch at Login control through an injected actor that
privately wraps `SMAppService.mainApp`. macOS status is the only source of truth:
there is no mirrored UserDefaults flag. Registration and unregistration are
single-flight, always followed by a fresh status read, and approval-required is
shown as registered but currently blocked with a direct System Settings action.
The setting refreshes when Settings appears and when it returns active. It adds
no helper, launch daemon, entitlement, privilege, TCC access, or engine/FFI
capability; a login launch follows the existing `LSUIElement` menu-bar startup
and does not open Explorer or begin a Home scan. Mutation tests inject a fake
service and never touch the host's Login Items; one integration test calls only
the production status reader. Do not perform real registration testing with the
temporary `se.mjukis.dux.spike` identity; the signed stable install and sign-in
cycle remain gated on the production identity in Milestone 9.

Explorer currently contains one honest Overview destination in a
`NavigationSplitView`, plus a direct Settings shortcut. It reads only the same
cached startup-volume state and current in-session Home-scan aggregate already
owned by `AppModel`; opening or reopening the window never creates another
model, engine session, or scan. Its capacity chart uses ordinary filesystem
availability for the used/free composition and separately labels
important-use availability, so those unlike quantities are never subtracted.
Unknown values remain unknown, stale values remain visible with their
freshness, and Home coverage stays explicitly scoped. Snapshot history,
review-lease-backed paths and drill-down, recommendations, reclaimable totals,
cleanup, trends, and AI belong to later milestones rather than this Overview.

First run adds a local-analysis introduction to the menu popover. Its versioned
UserDefaults value records only that the user continued; initialization and
dismissal perform no scan, capacity sample, access check, FFI call, or permission
request. Explorer derives its coverage message from the latest actual Home-scan
summary. A complete result needs no broader-access action, while measured
incomplete or uncertain coverage can expose an explicit **Understand broader
access…** action without making the rest of the app unusable.

The resulting observed-access check runs off-main and attempts only to open
`Library/Mail`, `Library/Messages`, and `Library/Safari` as directories. It does
not enumerate entries or read contents, and only three path-free counts plus a
timestamp enter model state. These observations are evidence, never an
authoritative Full Disk Access boolean. Optional guidance opens the System
Settings app without an undocumented pane URL. That action arms one recheck on
the next activation; unrelated activations do nothing, and a new Home scan is
always a separate explicit action. Duplicate checks coalesce, the last evidence
survives refresh failure, and shutdown generation-fences late results.

The shared model also owns the menu-bar label preference. Settings can select
Icon only, Icon and free space (GiB), or Icon and free space (%); free GiB is the
first-run default. The stable `menuBar.labelMode.v1` UserDefaults value is a
Swift-only presentation preference, so it does not cross FFI or affect engine
policy. Missing, malformed, and unknown future values fall back in memory
without being overwritten. The status item reads the existing cached capacity
state and never starts a sample or scan. Visible values are conservatively
floored with checked integer arithmetic, refreshing/stale states retain their
last measurement, and loading/failure states show no invented capacity. Its
monochrome pressure symbols have distinct shapes, while VoiceOver always
receives pressure, GiB, percent, capacity basis, and freshness even in Icon-only
mode.

Settings also owns a separate Swift-only menu-bar visibility preference.
Always visible is the default; the opt-in conditional mode inserts the item at
or below a validated whole free-space percentage and hides it only after one
additional percentage point of recovery. The evaluator reads the same cached
effective startup-volume capacity as the label and starts no sample or scan.
Refreshing and stale states retain their cached decision input, while unknown
capacity keeps the item visible. Reopening the already-running app from Finder,
Spotlight, or `open` reveals it for the rest of that process session without
changing the stored preference. Only the validated preference is persisted;
insertion and reveal state are derived presentation state and never enter FFI or
Rust pressure policy.

`VolumeMonitor` samples the startup volume (`/`) through Foundation on a utility
queue. It prefers `volumeAvailableCapacityForImportantUsage`, records whether it
had to fall back to ordinary filesystem availability, and retains both values
when available. The shared `AppModel` deduplicates initial loads across the menu
bar, Explorer, and Settings scenes. Capacity is never derived from directory
scan totals, and the Rust engine pressure evaluator remains the only owner of
Healthy/Warning/Critical thresholds and hysteresis.

Settings uses FFI contract v12's carried-forward typed pressure-policy
get/set/reset calls. Rust
persists canonical exact integer configuration and remains the sole semantic
validator/evaluator. Swift holds GiB and percentage edits as text and converts
them with checked integer arithmetic, so arbitrary stored byte values round-trip
without `Double`, `Decimal`, or silent rounding. A changed save or reset routes
one manual signal through the capacity scheduler; unchanged, failed, cancelled,
or superseded operations signal nothing. Policy state is classification-only
and carries no scan, notification, schedule, plan, or cleanup authority.

The menu popover uses only cached startup-volume state until the user explicitly
chooses Scan now. It shows the volume name, Rust-classified pressure, effective
available bytes, total, percentage, capacity basis, and sample freshness; a
refresh keeps the cached observation visible and a failed refresh labels it
stale. The Scan now action is fixed to the current user's Home directory and
starts one application-owned scan driver. FFI v8's carried-forward opaque scan
task is polled on the same utility queue at 500 ms intervals and returns only
optional cumulative aggregate progress and a path-free terminal summary. No
current path, file name, node, candidate detail, plan, or cleanup authority
crosses into render state. Cancellation remains visibly pending until Rust
reports the terminal outcome, and a late success remains success. App shutdown
requests scan cancellation and quiesces the publication driver before
maintenance, review, and engine shutdown.

Explorer's FFI v12 review entry point does not trust the recent-history
hint. Rust selects the exact newest succeeded, non-tombstoned snapshot and
acquires its existing expiring review lease after repeating repository and
file-format validation. Swift validates the returned scan ID and owns lease
renewal/release outside render state; concurrent stale acquisitions are
released. The same lease exposes a historical root and bounded direct-child
pages through validated app-owned models. Lossless observation bytes support
display and drill-down only; no current path, filesystem identity, plan, or
cleanup capability crosses.

Explorer now consumes that transport in a separate Latest Snapshot destination.
It acquires no review while Overview is selected and never switches snapshots
implicitly after a scan. A successful open publishes only after the root and
first 100-row child page are both validated. Folder drill-down, ID-backed
breadcrumbs, four server-side sorts, proportional size bars, and bounded
previous/next page replacement retain only one page in Swift. Closing the
destination or the app generation-fences pending work and releases the exact
review. Empty history, expiry, engine pressure, browsing-budget refusal, and
invalid data are explicit states; observation names remain display-only and are
never converted to live URLs or filesystem actions.

The same lease now supplies a coarse logical-size treemap capped at 64
represented positive-size children; the app requests 48. Every response carries
deterministic logical ranks and exact Other child/byte accounting, including
zero-size children. Treemap selection uses those ranks to load the exact
100-row logical table page, and table selection highlights either the matching
cell or Other. A deterministic rectangular layout and a historical-only
inspector show names, kinds, sizes, counts, timestamps, and scan warnings with
textual accessibility labels. The table remains the complete fallback if the
treemap is unavailable. Other is never a node, and the inspector deliberately
offers no Finder, Quick Look, path-copy, AI, or cleanup action.

Explorer snapshot selection starts with FFI v8's bounded newest-first recent
scan page. The native adapter validates its version, stable IDs, ordering,
timestamps, lifecycle/count shape, coverage, and recorded-snapshot hints before
publishing app-owned immutable values. Snapshot Explorer requests at most the
newest 50 rows, shows non-reviewable lifecycle states instead of hiding them,
and discloses when older history is omitted. A recorded reference is only a
review candidate: opening it acquires the exact expiring scan-bound lease and
validates its root, first page, and treemap before replacing the last confirmed
view. Missing retained snapshots are marked unavailable only for the session;
refresh retries them. History failure does not close an active review or erase
the last confirmed list. No live snapshot path or cleanup authority crosses
this history-selection boundary.

Candidates is a fourth lazy view over the same retained review lease. UniFFI
v26 returns one bounded summary page and supports four semantic review-status
commands that never create a cleanup plan. Selecting a confirmed row loads the
first exact historical-path and deterministic-evidence pages concurrently;
previous/next controls replace one page of at most 64 rows at a time. The
Rust FFI and Swift adapter independently enforce 64-KiB encoded-path,
256-KiB display-path, and 24-MiB aggregate-page budgets. The browser repeats
scan ID, candidate ID, immutable candidate body, cursor, total, count, and
next-cursor validation above the generated adapter, and generation-fences
snapshot, mode, selection, and independent page changes. Cancellation clears
the current loading latch without accepting the response. Review expiry
releases and invalidates the whole snapshot, while late results cannot
repopulate a closed or changed inspector. The inspector presents lossless
display paths, precise evidence, blockers, safety, proposed action, and review
status as historical observations with visible and VoiceOver page status, and
explicitly disclaims AI, planning, and cleanup authority. It never converts a
displayed path into a live URL or an executor request.

Large Files is a second, lazy view over the same retained review lease. FFI v12
requires a positive size threshold, supports an optional strict modification
cutoff, and returns at most 200 deterministic file observations; the app asks
for 100 and defaults to 1 GiB. Exact matching count and logical-byte totals make
truncation explicit, while at most eight historical parent components provide
display context without constructing a live path. Size and age presets rerun
the bounded query off the main actor, and mode/filter/snapshot generations
discard late results. Unknown modification times are excluded by age filters.
The view labels all values as historical observations—not reclaimable space—and
offers no Finder, Quick Look, deletion, AI, planning, or cleanup action.

Coverage is a third lazy Explorer view backed by FFI v13's exact durable scan
history endpoint rather than the snapshot lease. The app loads canonical issue
pages of at most 64 records, validates exact cross-page totals and coverage
invariants, and displays all retained limitations with bounded root-relative
historical context. Unknown coverage is never drawn as zero; a measured bar is
shown only when the scan recorded a quantitative estimate. These locations are
display observations, not live paths, current access tests, reclaimable-space
estimates, or cleanup inputs.

FFI v14 enables three narrowly scoped live conveniences for a selected
historical file or folder: Reveal in Finder, Copy Path, and file-only Quick
Look. The app never builds a path from breadcrumbs or display names. It asks
the exact retained review to resolve a node ID for one purpose; Rust matches
the current root, every ancestor, and target to the snapshot's recorded
device/inode identities using no-follow descriptor traversal, then rechecks the
lease. Swift validates the complete response and discards delayed results after
selection, mode, snapshot, presentation, or close changes. Paths that cannot
round-trip exactly through Swift Foundation's path-based URL are rejected for
all actions rather than silently changing their bytes.

The returned path is current only at the validation instant. Finder and Quick
Look accept paths rather than retained file handles, leaving a narrow
post-validation same-user filesystem race. The feature is therefore limited to
non-destructive UI actions, says that Quick Look shows current contents, and
does not retain or route the path into AI, candidates, reclaimability, plans,
or cleanup execution.

FFI v15 adds a display-only historical storage category to the root, page,
treemap, and Large Files node records. Rust joins only the exact scan's
immutable validated candidate evaluation and returns Unclassified when no
unambiguous historical assertion exists. The optional exact-path join is
limited to 4,096 roots and 1 MiB, indexed by exact root, and cleared with the
review lifecycle. Candidate persistence currently rejects non-Unicode host
paths, so affected evaluation fails closed to Unclassified; Swift does not
infer categories from lossy names. Treemap color is always redundant with a category symbol and
visible legend; both tables, the inspector, and VoiceOver expose the same text.
The category is not reclaimability or safety evidence and cannot carry a
candidate identity, path, action, plan, AI result, or cleanup authority.
