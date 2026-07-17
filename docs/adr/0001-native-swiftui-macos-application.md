# ADR 0001: Native SwiftUI macOS application

- Status: Accepted
- Date: 2026-07-15
- Scope: macOS application shell and presentation layer

## Context

DUX is becoming a menu bar-first macOS application while retaining the current
cross-platform CLI. The app needs a persistent menu bar surface, a normal
Explorer window for deeper navigation, a Settings window, charts, system
notifications, login-item control, Finder integration, accessibility, and
correct macOS activation/window behavior.

The product is macOS-specific at the presentation layer and initially targets
macOS 14 or later. The filesystem engine remains cross-platform Rust; choosing
a native app does not imply rewriting the engine in Swift.

The UI must remain responsive while scans, classification, and later cleanup
validation run. A menu bar popover is useful for status and small actions, but
it is not the only application window and must not become a miniature version
of the full Explorer.

## Decision

Build the primary macOS application with the SwiftUI app lifecycle and native
Apple frameworks.

The initial application scenes are:

- one `MenuBarExtra` using the window style for status, disk pressure, current
  work, and a small number of high-confidence recommendations;
- one singleton normal Explorer `Window(id:)` opened from the menu bar for drill-down,
  comparison, treemap/list navigation, and plan review;
- one native `Settings` scene for launch-at-login, scanning, privacy, AI
  provider configuration, CLI installation, and later automation controls.

The menu bar process is the main application process. Do not add a second
menu-extra helper, launch agent, daemon, or privileged process in the initial
architecture. User-controlled launch at login uses `SMAppService.mainApp`.

Swift owns presentation and macOS integration:

- scenes, commands, focus, navigation, accessibility, and localization;
- SwiftUI and Swift Charts rendering;
- notifications and links into System Settings;
- Finder and other platform integration;
- Foundation volume-capacity sampling;
- settings UI and command-provider configuration.

Rust ownership is defined by [ADR 0004](0004-shared-rust-engine.md). SwiftUI
views must not reproduce scanner, classification, candidate, safety, or cleanup
policy.

The native presentation layer owns non-authoritative display preferences. The
menu-bar label mode is therefore a validated, versioned UserDefaults value in
the shared `AppModel`, rather than a Rust/SQLite/FFI setting. Its three modes
render only the cached Rust-classified startup-volume observation and cannot
start sampling, scanning, planning, or cleanup. Missing or unknown preference
values fall back in memory without rewriting future data. Capacity and pressure
remain engine/monitor observations; Swift owns only compact conservative
formatting, appearance-independent symbols, localization, and accessibility.

## Implementation constraints

### Scene and navigation model

- Treat the menu bar extra, Explorer, and Settings as separate scenes sharing
  one application model, not as nested states of one popover.
- Model Explorer with `Window(id:)`, not `WindowGroup`, so repeated open actions
  address one logical window instead of creating duplicate scan surfaces.
- Opening Explorer from the menu bar must activate and focus an existing
  Explorer window when appropriate instead of creating unlimited duplicates.
- Closing Explorer must leave the menu bar process running.
- Ship the initial app as an accessory/menu-bar application with `LSUIElement`
  enabled and no Dock icon by default. Use a focused AppKit activation bridge
  when opening Explorer. Runtime Dock-icon switching is out of scope for v1.
- Keep the menu bar extra permanently available in v1; do not expose a setting
  that can remove the user's only way to reopen the app UI.
- Use small, explicit AppKit bridges when SwiftUI lacks reliable activation,
  window placement, or system integration. Keep those bridges behind focused
  adapters and do not move general UI ownership to AppKit.

### State and concurrency

- `@MainActor AppModel` contains immutable or value-like render state only.
- `EngineService` owns the FFI handle and converts typed engine events into
  `AsyncStream` or another structured-concurrency boundary.
- Never perform scans, blocking FFI calls, database work, package generation,
  filesystem enumeration, or AI command execution on the main actor.
- Coalesce progress updates before publishing them to SwiftUI. Rendering must
  not scale with raw filesystem event frequency.
- Scene state must tolerate cached, loading, partial, stale, failed, and
  cancelled engine states.

### UI quality

- Use native semantic colors, typography, materials, keyboard focus, VoiceOver
  labels, reduced-motion behavior, and system appearance.
- Centralize user-facing strings in a String Catalog from the first app commit.
- Charts must have textual equivalents and must not be the only way to compare
  values or understand risk.
- The popover should open from cached state within the roadmap budget and must
  not trigger a full scan merely because it became visible.

### Repository shape

The app lives under `dux-macos/`. Keep Xcode-specific build scripts there and
keep generated bindings reproducible. The app consumes the universal Rust
artifact; it does not build Rust opportunistically during normal UI rendering
or at application startup.

## Consequences

Benefits:

- First-class menu bar, window, Settings, accessibility, and macOS behavior.
- Direct access to Foundation, Service Management, User Notifications, Finder,
  and other platform APIs without a webview or cross-platform UI shim.
- A clear separation between platform presentation and portable engine policy.
- Visual and interaction behavior can follow macOS conventions rather than the
  constraints of the existing terminal UI.

Costs and risks:

- The repository gains Xcode/Swift build and testing infrastructure in addition
  to Cargo.
- Some window and activation behavior may still require small AppKit bridges.
- Swift concurrency annotations around generated FFI code may require wrappers.
- UI code is macOS-only even though the engine and CLI remain cross-platform.
- Two UI implementations must stay behaviorally consistent through shared
  engine contracts rather than shared presentation code.

## Alternatives considered

### Keep only the terminal UI

Rejected because it cannot provide the always-available disk-pressure surface,
native notifications, settings, visual exploration, and guided permissions
experience required by the product.

### Electron or another webview shell

Rejected because it adds a large runtime and indirection around platform APIs
without providing meaningful cross-platform value for a deliberately
macOS-specific app surface.

### Tauri or a Rust-native GUI toolkit

Rejected for the initial app because the product depends heavily on native
menu bar, scene, accessibility, Settings, and system-integration behavior.
Rust remains the right engine language without also owning presentation.

### AppKit-only application

Rejected as the default because SwiftUI directly models the required scenes and
state-driven UI. Focused AppKit adapters remain allowed where they are more
reliable.

### Mac Catalyst

Rejected because there is no iPad application to share and the app needs native
macOS filesystem and window behavior.

## Validation criteria

The Phase 0 shell must prove all of the following before substantial UI work:

- Debug and Release builds succeed from a clean checkout.
- One universal app supports arm64 and x86_64.
- The menu bar extra opens without creating an Explorer window.
- A menu bar action opens or focuses a normal Explorer window.
- Repeating that action still yields one Explorer window and one scan session.
- Settings opens through the native Settings scene.
- Closing all normal windows leaves the menu bar process alive.
- One typed Rust value is fetched off the main actor and rendered safely.
- VoiceOver can identify the status item and the primary actions.
- The app remains responsive while a simulated long engine operation runs.

## Reconsider when

Create a superseding ADR if the shell spike demonstrates a blocking SwiftUI
scene/focus defect that cannot be contained behind a small AppKit adapter, or if
Apple removes support required by the chosen deployment target. Ordinary
SwiftUI workarounds are not sufficient reason to replace the application stack.

## References

- [Roadmap §3.1 and §3.2](../../ROADMAP.md#31-platform-and-distribution)
- [Roadmap target repository layout](../../ROADMAP.md#6-target-repository-layout)
- [Apple: MenuBarExtra](https://developer.apple.com/documentation/swiftui/menubarextra)
- [Apple: MenuBarExtra window style](https://developer.apple.com/documentation/swiftui/menubarextrastyle/window)
- [Apple: OpenSettingsAction](https://developer.apple.com/documentation/swiftui/opensettingsaction)
- [Apple: OpenWindowAction](https://developer.apple.com/documentation/swiftui/openwindowaction)
- [Apple: SMAppService.mainApp](https://developer.apple.com/documentation/servicemanagement/smappservice/mainapp)
