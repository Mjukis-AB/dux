# ADR 0002: Direct Developer ID distribution

- Status: Accepted
- Date: 2026-07-15
- Scope: primary macOS application distribution and release trust

## Context

DUX needs broad, user-authorized filesystem visibility, bundles an optional CLI,
and is expected to evolve faster than an App Store review cycle during its
initial releases. The product does not require the Mac App Store for discovery,
licensing, or commerce.

Distribution outside the Mac App Store moves release integrity, update safety,
and user trust into this project. A downloaded app must pass Gatekeeper without
instructions to bypass security controls. Every executable shipped inside the
application, including Rust libraries and the optional CLI, participates in the
code-signing and notarization boundary.

The updater is not yet proven. Choosing direct distribution does not by itself
select Sparkle versus a manual updater or a website/payment system.

## Decision

Distribute the primary macOS application directly in a notarized and stapled
DMG containing the universal Developer ID-signed, Hardened Runtime-enabled app
and an Applications-folder link. A notarized ZIP may be a secondary artifact.
Do not target the Mac App Store in the initial architecture.

The release must:

1. build Rust and Swift from the tagged, reviewed source revision;
2. produce and verify arm64 and x86_64 Rust artifacts;
3. assemble a universal application and bundled universal CLI;
4. sign every nested executable and framework before signing the outer app;
5. use a Developer ID Application identity, secure timestamp, Hardened Runtime,
   and the smallest reviewed entitlement set;
6. archive the signed app without mutating it afterward;
7. submit with `notarytool`, require an accepted result, and retain the log;
8. staple the notarization ticket to the distributed artifact where supported;
9. verify signatures, Hardened Runtime, architectures, stapling, and Gatekeeper
   assessment on a clean machine or clean test environment;
10. publish a cryptographic checksum alongside the download.

The standalone CLI continues to ship through Homebrew, GitHub Releases, and
crates.io. The app bundles a compatible CLI for optional installation from
Settings, but direct app distribution must not replace or silently mutate an
existing standalone installation.

## Implementation constraints

### Release identity and reproducibility

- The app version, Rust crate versions, bundled CLI version, and release tag
  must be checked for compatibility before signing.
- Generated Swift bindings and the XCFramework must be derived from the same
  tagged Rust source used by the app build.
- Release scripts must fail closed on a missing architecture, signature,
  entitlement review, notarization ticket, or staple verification.
- Never use ad hoc signing, a development identity, `get-task-allow`, or advice
  that asks users to disable Gatekeeper for a public build.
- Signing and notarization credentials belong in protected CI environments and
  must not be exposed to pull requests or third-party actions.
- Release artifacts are immutable once announced. A corrected build receives a
  new version.
- Freeze the production bundle identifier, Apple Developer team, signing
  identity, and designated requirement before TCC and launch-at-login testing.
  Those values are release prerequisites because changing identity invalidates
  permission and login-item assumptions; this ADR does not invent them.

### Artifact and update policy

- The primary artifact is a signed, notarized, and stapled DMG with an
  Applications-folder link. A ZIP may be published for automation or secondary
  channels, but it is not the primary installation experience.
- Automatic updates are deferred. If Sparkle is adopted, add a separate ADR or
  amend this one with appcast hosting, EdDSA key custody, rollback behavior,
  staged rollout, and update-signature CI checks.
- Before an updater exists, the app may check for updates and direct the user to
  an authenticated project download page, but it must not download and execute
  unsigned replacements.
- Homebrew Cask may be added as a secondary installation channel. It is not the
  source of truth for signing or application updates.

### Compatibility and data safety

- App, bundled CLI, and standalone CLI must advertise schema compatibility.
- Older binaries fail gracefully on newer database/snapshot formats and never
  downgrade or corrupt shared state.
- A release must include migration/compatibility checks before it can update a
  previous installation.
- Uninstalling the app and uninstalling the optional CLI are separate,
  user-visible operations.

## Consequences

Benefits:

- The app can use the filesystem architecture selected by the product without
  contorting the initial release around App Store policy.
- Release timing and rollback communication remain under project control.
- The app can bundle the existing CLI and Rust engine in one verified artifact.
- Users still receive Apple code-signing, notarization, stapling, and Gatekeeper
  trust signals.

Costs and risks:

- The project owns secure artifact hosting, update communication, signing-key
  custody, notarization automation, and eventual updater security.
- No App Store discovery or automatic App Store updates are available.
- Certificate loss or compromise becomes a release incident requiring a
  documented response.
- Direct downloads can be replaced or mirrored by attackers unless the website,
  checksums, updater metadata, and release process are protected.
- Universal app releases require macOS CI and cannot be completed entirely on
  the existing Linux/Windows Rust release runners.

## Alternatives considered

### Mac App Store only

Rejected for the initial product because App Sandbox is mandatory there and the
application requires broad, explainable disk analysis plus optional CLI
installation. It can be reconsidered only with a separately scoped feature set
and a proven sandbox-compatible scan model.

### Simultaneous App Store and direct builds

Rejected initially because two entitlement, storage, helper, and update models
would multiply security and support work before the core application is proven.
Do not create a nominal sandboxed build that behaves materially differently
without documenting it as a separate product channel.

### Unsigned or ad hoc direct download

Rejected. Users must not be asked to bypass Gatekeeper, remove quarantine, or
weaken system security to run DUX.

### Homebrew Cask as the only app channel

Rejected because package-manager availability does not replace the project's
responsibility for Developer ID signing, notarization, stable downloads, and an
accessible installation path for non-technical users.

## Validation criteria

Before the first external app build:

- a clean script produces the same app layout in Debug and Release;
- `lipo`/`file` verification proves universal executables where required;
- explicit `codesign --verify --strict --verbose=2` checks succeed for every
  nested executable and the outer app; release signing never relies on
  recursive `--deep` signing as a substitute for inside-out signing;
- entitlements match the reviewed release file and Hardened Runtime is enabled;
- notarization completes through `notarytool` and the log has no ignored errors;
- `stapler validate` and Gatekeeper assessment succeed on the final artifact;
- the bundled CLI reports the expected version and architecture;
- the published checksum matches a fresh download;
- install, update, downgrade refusal, and uninstall paths are tested from a
  clean user account.

## Reconsider when

Revisit if an App Store channel becomes strategically necessary, Apple changes
Developer ID/notarization requirements, or the updater spike selects a
long-lived mechanism that warrants its own decision record. A future App Store
variant must not silently weaken scan coverage or safety semantics.

## References

- [ADR 0003: Primary build without App Sandbox](0003-primary-build-without-app-sandbox.md)
- [Roadmap §3.1](../../ROADMAP.md#31-platform-and-distribution)
- [Roadmap Milestone 9](../../ROADMAP.md#milestone-9-cli-companion-and-production-distribution)
- [Apple: Preparing your app for distribution](https://developer.apple.com/documentation/xcode/preparing-your-app-for-distribution)
- [Apple: Notarizing macOS software before distribution](https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution)
- [Apple: Customizing the notarization workflow](https://developer.apple.com/documentation/security/customizing-the-notarization-workflow)
- [Apple: Hardened Runtime](https://developer.apple.com/documentation/security/hardened-runtime)
