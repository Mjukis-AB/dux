# DUX macOS application

This directory will contain the native macOS application. The current Phase 0
spike packages the shared Rust library before the Xcode project is introduced.

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

The spike app owns one opaque `DuxEngine` through `EngineService`. The service
serializes synchronous calls and explicit close on its dedicated queue, maps
generated typed errors to app-owned errors, converts generated records to a
Sendable app value, and publishes that value through a `@MainActor` model. ARC
release frees the Rust object but is not a substitute for explicit close or
later task cancellation. Run the linked lifecycle and concurrency tests with
the same `xcodebuild` arguments above, replacing `build` with `test`.

The application shell is menu bar-first. `MenuBarExtra` must remain the first
scene so the macOS 14 automatic scene-launch behavior does not open Explorer at
startup. `Window(id: "explorer")` supplies one reusable normal window, and the
native `Settings` scene shares the same `AppModel`. The focused `AppActivation`
bridge requests foreground activation with `NSApplication.activate()` after a
menu action opens Explorer or Settings. The generated Info.plist sets
`LSUIElement=true`, so closing Explorer leaves DUX running without a default
Dock icon. User-facing shell keys live in `Dux/Resources/Localizable.xcstrings`.

`VolumeMonitor` samples the startup volume (`/`) through Foundation on a utility
queue. It prefers `volumeAvailableCapacityForImportantUsage`, records whether it
had to fall back to ordinary filesystem availability, and retains both values
when available. The shared `AppModel` deduplicates initial loads across the menu
bar, Explorer, and Settings scenes. Capacity is never derived from directory
scan totals, and the later Rust pressure evaluator remains the only owner of
Healthy/Warning/Critical thresholds and hysteresis.
