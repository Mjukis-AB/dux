# ADR 0012: Non-shipping signed cleanup qualification

**Status:** Accepted
**Decision date:** 2026-08-09

## Context

DUX has two ordinary macOS configurations. Debug carries the internal
permanent-safe action but uses the spike identity; Release uses the frozen
production identity but deliberately compiles that action out. The existing
real-effect FFI harness is unsigned and does not exercise the generated Swift/C
boundary, native confirmation, installation, Gatekeeper, TCC identity, or real
macOS Trash.

Enabling the action in ordinary Release merely to test it would make test bytes
indistinguishable from releasable bytes. Signing Debug would not prove the
production identity or stable installed behavior. A hidden launch argument or
test-only effect API would bypass the authority path that must be qualified.

## Decision

Add one configuration named `CleanupQualification`. It inherits release
optimization, the reviewed empty entitlements, Hardened Runtime, the frozen
`se.mjukis.dux` bundle identity, Team `SMQ3E8Y57T`, the universal bundled CLI,
and the same application source as Release. It differs in exactly the
qualification surfaces required to test cleanup:

- `DUX_INTERNAL_PERMANENT_SAFE_CLEANUP` compiles the existing native
  confirmation and observation path;
- `DUX_CLEANUP_QUALIFICATION` compiles an unavoidable visible warning in both
  the menu popover and Explorer;
- the display name is **DUX Cleanup Qualification**; and
- Info.plist carries protocol version `1` plus the exact 40-character source
  commit supplied by the clean build.

Ordinary Debug and Release carry protocol version `0` and source `none`.
Release packaging verifies those values and rejects both qualification
compilation conditions. Qualification bytes use the production identity only
because stable signing, Gatekeeper, installation, and TCC behavior are part of
the evidence. They are never public release candidates, appcast enclosures,
GitHub artifacts, Homebrew artifacts, or installed over a user's normal DUX.

The qualification is supervised on disposable macOS accounts/devices. It uses
only disposable fixtures and the normal product UI:

1. real Explorer Trash with explicit confirmation and Finder Put Back;
2. the Release-visible Rust-target dry check with zero mutation;
3. durable opt-in, current review, exact destructive confirmation, and the
   existing central Rust permanent-safe task; and
4. path-free terminal/history correlation with no retry.

There is no qualification-only engine, FFI, path, approval, callback, cleanup
command, or automatic UI driver. The build and installed-app verifiers are
read-only. The protocol may prepare known test data, but DUX must discover and
authorize it through the same scan, candidate, plan, consent, confirmation,
revalidation, journal, and executor graph intended for Release.

## Release boundary

Creating the configuration does not enable public cleanup and does not satisfy
the signed-app gate. Completion requires independently reviewed runtime
evidence from the same signed/notarized universal bytes on:

- an Intel Mac running macOS 14 or a later supported version; and
- an Apple Silicon Mac running the newest supported macOS.

The evidence must include exact commit/version/build, Developer ID/Team and
designated requirement, notarization/staple/Gatekeeper results, installed
`/Applications` path, one exact final private-archive SHA-256 shared across
both hosts, signed CLI metadata bound to its post-signing bytes, AI-disabled
and limited-access behavior, disposable
before/after facts, preserved files, terminal history correlation, and the
absence of retry. Real Trash and permanent-safe results are distinct gates.

The external actions require explicit authorization: Developer ID/notarization
credential use, installing production-identity qualification bytes, and real
filesystem effects on the disposable fixtures. Public Release keeps the
action compiled out until this evidence, active private vulnerability
reporting, every applicable §17.2/§17.3 item, and a separate final enablement
review all pass.

## Consequences

Positive:

- signed behavior can be tested without making qualification bytes releasable;
- production identity and native UI/FFI behavior are covered together;
- the visible warning reduces confusion on a disposable device;
- the source commit is carried in the signed Info.plist; and
- the protocol exercises both real macOS Trash and permanent-safe cleanup.

Costs and limitations:

- qualification uses the production identity, so it must never share a normal
  user account or overwrite an installed public DUX;
- real evidence needs two architectures, Apple credentials, reviewers, and
  disposable devices/accounts;
- repository tests can prove the lane and fail-closed packaging boundary, but
  cannot manufacture signing, notarization, Gatekeeper, TCC, or effect facts;
  and
- a signed run may still expose a product failure and therefore not satisfy
  the gate.

## Alternatives rejected

- **Temporarily enable the action in Release.** Qualification and releasable
  bytes would be indistinguishable.
- **Sign Debug.** The spike identity and debug compilation do not qualify the
  production install.
- **Use a different bundle identifier.** It avoids identity collision but also
  fails to prove the stable production identity/TCC boundary.
- **Add a hidden automatic cleanup launch mode.** It would bypass explicit
  confirmation and create a second authority path.
- **Treat the unsigned FFI fixture as sufficient.** It omits the signed app,
  generated ABI, Swift adapter, UI, Trash, and installed environment.
- **Upload the signed qualification app from public-repository CI.** A
  destructive-enabled production-identity artifact must not become a
  repository-readable artifact.

## Validation and reconsideration

Repository validation must prove all three configurations, visible markers,
exact public-Release rejection, deterministic project generation, universal
qualification output, read-only verifiers, and the complete documented
evidence schema. Runtime gate closure requires the external two-architecture
matrix.

Reconsider this ADR if Apple changes Developer ID/notarization identity
semantics, the app becomes sandboxed/App Store distributed, the qualification
artifact can be confined without losing production-identity evidence, or DUX
adds another cleanup mode/platform.

## References

- [Signed-app destructive qualification protocol](../testing/signed-app-destructive-qualification.md)
- [Permanent-safe cleanup qualification](../testing/permanent-safe-cleanup-qualification.md)
- [ADR 0002: Direct Developer ID distribution](0002-direct-developer-id-distribution.md)
- [ADR 0011: Diagnostic-only cleanup crash debt in v1](0011-diagnostic-only-cleanup-crash-debt-v1.md)
- [Security design §17](../../SECURITY_DESIGN.md#17-verification-and-enforcement)
- [Roadmap Milestone 5](../../ROADMAP.md#milestone-5-deterministic-recommendations-and-reviewed-cleanup)
