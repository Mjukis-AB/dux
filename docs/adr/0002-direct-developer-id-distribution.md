# ADR 0002: Direct Developer ID distribution

- Status: Accepted
- Date: 2026-07-15
- Amended: 2026-07-31 (dormant Sparkle 2 integration scaffold)
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

Direct distribution also requires an update mechanism that preserves the same
release identity and verification boundary after initial installation. DUX
shares release infrastructure conventions with other projects that use
Sparkle 2, and Sparkle provides the standard appcast, EdDSA, update-consent,
atomic replacement, and menu-bar-app behavior needed here.

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

The standalone tag lane also remains operationally independent: Apple signing,
notarization, Xcode, and Sparkle credentials are not prerequisites for its
cross-platform verification, crates.io publication, GitHub CLI archives, or
Homebrew formula update. Repository policy tests enforce that separation and
the ordered checksum-backed release chain. App artifacts may share a reviewed
version, but their unavailable credentials or failed publication cannot grant,
replace, or silently absorb standalone CLI installation authority.

Use Sparkle 2 for in-app updates after the production bundle identifier,
Developer ID identity, designated requirement, and signing pipeline are stable.
Integrate the reviewed Sparkle 2 release through Swift Package Manager and use
`SPUStandardUpdaterController` with Sparkle's standard user interface for the
initial release. Do not build a custom downloader, verifier, installer, or
update UI for the first implementation.

The dependency and native adapter may land before those release prerequisites,
but they must remain fail-closed. The accepted scaffold pins Sparkle 2.9.2 and
creates no updater unless the host bundle has a non-placeholder identity, an
HTTPS `SUFeedURL`, and a valid base64-encoded 32-byte `SUPublicEDKey`. This lets
ordinary builds compile and test the integration without inventing release
identity, keys, or network authority.

Because the repository owns a custom fail-closed release workflow, Sparkle's
nested code has an explicit signing policy. Only the pinned framework's
version-B updater app, installer/downloader XPC services, autoupdater, and
framework are admitted. Each executable must be universal. The helpers and
framework are signed inside-out with Hardened Runtime before the outer DUX app;
the downloader's reviewed upstream entitlements are preserved. Any additional
nested bundle remains a release failure.

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
- Sparkle 2 is the sole in-app updater. Pin its exact reviewed package version
  in the resolved dependency graph; upgrades require dependency review and the
  complete release/update verification lane.
- Publish the appcast and release archives over HTTPS. Embed only the Sparkle
  EdDSA public key in the app. Keep the private key outside the repository,
  application bundle, artifact host, and public pull-request environment.
- Require both a valid Sparkle EdDSA signature and the expected Apple Developer
  ID code-signing identity. A notarized archive must be immutable before its
  appcast entry is signed and published.
- Require a signed appcast/feed. CI must verify the feed signature, enclosure
  signature, enclosure length, version monotonicity, download URL, minimum
  system version, Apple signature, notarization, and staple before publication.
- Expose **Check for Updates…** in Settings. Automatic checking follows
  Sparkle's explicit user-consent flow; automatic download/install remains a
  user-controlled setting. DUX must not silently override either preference.
- Ship one stable channel first. Beta channels and phased rollout may be added
  only after stable updating is proven, and channel selection must be explicit
  and reversible in Settings.
- Never publish a lower build number as rollback. Respond to a bad release with
  a higher, monotonically versioned corrective release. A release may be
  withdrawn from the appcast, but already installed state is not fabricated or
  silently downgraded.
- Before Sparkle is enabled, the app may direct the user to an authenticated
  project download page, but it must not download and execute an unsigned
  replacement.
- The primary app is not sandboxed, so do not add Sparkle sandbox XPC services
  or sandbox entitlements. Revisit Sparkle's sandbox integration if that
  architecture changes.
- Homebrew Cask may be added as a secondary installation channel. It is not the
  source of truth for signing or application updates.

### Compatibility and data safety

- App, bundled CLI, and standalone CLI must advertise schema compatibility.
- Older binaries fail gracefully on newer database/snapshot formats and never
  downgrade or corrupt shared state.
- A release must include migration/compatibility checks before it can update a
  previous installation.
- Sparkle updates only the application bundle. It must not overwrite a
  separately installed CLI. After an app update, Settings reports bundled and
  installed CLI versions and offers the same explicit, atomic CLI
  installer/upgrader when reconciliation is needed.
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
- Sparkle EdDSA key loss or compromise becomes a separate release incident.
  Rotation and emergency appcast withdrawal procedures must be documented and
  tested without storing the private key on the artifact host.
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

### Manual in-app updater

Rejected because DUX should not own bespoke network download, signature
verification, privileged replacement, quarantine, relaunch, and recovery code
when Sparkle 2 already provides a reviewed macOS-specific implementation.

### Website-only update notifications

Rejected as the long-term mechanism because they make security fixes slower to
adopt and provide no atomic application replacement. This remains the temporary
behavior until the signed Sparkle release lane is proven.

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
- a signed previously released build updates to the candidate on Intel and
  Apple Silicon without losing settings, history, TCC identity, login-item
  state, or menu-bar relaunch behavior;
- tampered feeds, archives, signatures, lengths, versions, code identities, and
  incompatible schema transitions fail closed without replacing the app;
- offline, interrupted, read-only-volume, App Translocation, withdrawn-release,
  and already-current behavior is tested;
- downgrade refusal and the higher-version corrective-release path are tested;
- the optional installed CLI remains unchanged by Sparkle and its explicit
  Settings reconciliation path is tested separately;
- install, update, and uninstall paths are tested from a clean user account.

## Reconsider when

Revisit if an App Store channel becomes strategically necessary, Apple changes
Developer ID/notarization requirements, or Sparkle 2 can no longer satisfy the
signed direct-update boundary. A future App Store variant must not silently
weaken scan coverage or safety semantics.

## References

- [ADR 0003: Primary build without App Sandbox](0003-primary-build-without-app-sandbox.md)
- [Roadmap §3.1](../../ROADMAP.md#31-platform-and-distribution)
- [Roadmap Milestone 9](../../ROADMAP.md#milestone-9-cli-companion-and-production-distribution)
- [Apple: Preparing your app for distribution](https://developer.apple.com/documentation/xcode/preparing-your-app-for-distribution)
- [Apple: Notarizing macOS software before distribution](https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution)
- [Apple: Customizing the notarization workflow](https://developer.apple.com/documentation/security/customizing-the-notarization-workflow)
- [Apple: Hardened Runtime](https://developer.apple.com/documentation/security/hardened-runtime)
- [Sparkle 2 documentation](https://sparkle-project.org/documentation/)
- [Sparkle security and reliability](https://sparkle-project.org/documentation/security-and-reliability/)
