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

## Generate bindings and build the spike app

Generate the matching C header, module map, reviewable Swift source, and final
XCFramework. `CONFIGURATION` must match the Xcode configuration that will
consume it:

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
C header/module map and XCFramework remain under ignored `Generated/` and must
be recreated before building from a clean checkout. Never combine bindings and
a library produced from different source revisions or build configurations.

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
local release gates, requires matching universal Debug/Release layouts, verifies
the reviewed empty `Config/Release.entitlements`, signs code inside-out,
notarizes and staples the app, creates the DMG with an Applications link, then
independently signs, notarizes, staples, mounts, and Gatekeeper-assesses the DMG
and contained app. It publishes only after every check succeeds, under
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

Settings uses FFI contract v9's carried-forward typed pressure-policy
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

Explorer's FFI v9 newest-snapshot entry point does not trust the recent-history
hint. Rust selects the exact newest succeeded, non-tombstoned snapshot and
acquires its existing expiring review lease after repeating repository and
file-format validation. Swift validates the returned scan ID and owns lease
renewal/release outside render state; concurrent stale acquisitions are
released. Paths and snapshot nodes remain sealed until the later paged API.

Explorer snapshot selection starts with FFI v8's bounded newest-first recent
scan page. The native adapter validates its version, stable IDs, ordering,
timestamps, lifecycle/count shape, coverage, and recorded-snapshot hints before
publishing app-owned immutable values. The newest recorded reference in that
page is only a review candidate: opening it must still acquire the existing
expiring scan-bound lease. Snapshot paths, nodes, issue paths, candidate
details, and treemap data do not cross in this slice.
