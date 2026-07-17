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

`VolumeMonitor` samples the startup volume (`/`) through Foundation on a utility
queue. It prefers `volumeAvailableCapacityForImportantUsage`, records whether it
had to fall back to ordinary filesystem availability, and retains both values
when available. The shared `AppModel` deduplicates initial loads across the menu
bar, Explorer, and Settings scenes. Capacity is never derived from directory
scan totals, and the Rust engine pressure evaluator remains the only owner of
Healthy/Warning/Critical thresholds and hysteresis.

Settings uses FFI contract v6's typed pressure-policy get/set/reset calls. Rust
persists canonical exact integer configuration and remains the sole semantic
validator/evaluator. Swift holds GiB and percentage edits as text and converts
them with checked integer arithmetic, so arbitrary stored byte values round-trip
without `Double`, `Decimal`, or silent rounding. A changed save or reset routes
one manual signal through the capacity scheduler; unchanged, failed, cancelled,
or superseded operations signal nothing. Policy state is classification-only
and carries no scan, notification, schedule, plan, or cleanup authority.
