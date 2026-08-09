# ADR 0009: Reject direct local AI subprocess adapters

- Status: Accepted
- Decision date: 2026-08-09

Follow-up: [ADR 0013](0013-metadata-only-remote-ai-transport.md) accepts a
future fixed metadata-only remote HTTPS architecture. It does not supersede
this ADR's prohibition on direct local Claude, Codex, or custom commands.

## Context

DUX is an unsandboxed, directly distributed disk-usage application. Users may
grant its stable production identity Full Disk Access (FDA) so it can explain
otherwise hidden storage. Milestone 7 proposed optional Claude and Codex CLI
adapters that would receive only bounded, redacted metadata.

The provider input boundary does not confine provider code. An executable that
DUX starts directly runs as the same user and can issue its own filesystem and
IPC operations. A clean environment, empty working directory, fixed arguments,
closed descriptors, tool-disable flags, and JSON-only standard input reduce
accidental exposure but do not restrict `open(2)` against another known path.

TCC makes the consequence less predictable, not safer. Apple describes file
privacy decisions in terms of the process considered responsible for an
operation. A command-line descendant can be attributed to its launcher; Apple
uses a responsible Terminal or SSH process as the FDA principal in its own
Endpoint Security examples. The attribution link can also change when a child
daemonizes or transfers work. DUX therefore cannot use child identity, provider
prompts, or an unobserved TCC assumption as a confidentiality boundary.

ADR 0003 made an adversarial macOS spike a shipping gate. The reproducible
protocol and path-free evidence are in
[`docs/testing/macos-ai-subprocess-confinement.md`](../testing/macos-ai-subprocess-confinement.md).
The hostile child received a minimal environment, an empty working directory,
only standard descriptors, and no sentinel contents. It nevertheless read a
known absolute 0600 same-user sentinel outside its working directory. This
fails the gate before any FDA-specific assumption is needed.

For comparison, a custom Seatbelt profile denied the same read on the tested
host. That lane is not a viable shipping result: local `sandbox-exec(1)` and
the public `sandbox_init(3)` surface are deprecated, and Apple directs
developers to App Sandbox. A custom, undocumented profile language is not a
stable production contract.

## Decision

DUX MUST NOT directly launch Claude CLI, Codex CLI, or a generic user-selected
AI command from its unsandboxed app process. The disabled/no-provider adapter
remains the only permitted local-provider state.

Specifically:

- do not add a `Process`, `NSTask`, `posix_spawn`, shell, CLI, FFI, or Rust
  engine edge that invokes a local AI provider;
- do not use `sandbox-exec`, a custom Seatbelt profile, Endpoint Security
  observation, provider permission prompts, or provider-internal tool
  restrictions as the sole outer boundary;
- do not treat an empty working directory, environment scrubbing, closed file
  descriptors, fixed arguments, output limits, or process-tree termination as
  filesystem confinement;
- do not probe real personal data merely to reconfirm this negative result;
- keep the private redacted-input proof without a provider consumer until a
  separately accepted transport architecture exists.

The conditional Claude and Codex direct-adapter roadmap tasks are closed as
intentionally not implemented. This decision does not approve a remote API,
sandboxed helper, XPC service, virtual machine, or bundled provider. Each is a
new architecture that must prove its own data, credential, network, lifecycle,
update, and confinement boundaries before it can consume the privacy proof.

## Implementation boundaries

The checked-in spike is non-shipping test material. It creates only disposable
temporary sentinels, never reads a real TCC-protected location, never uses the
network, never imports DUX engine state, and reports no path or sentinel data.
Its deprecated-sandbox comparison is a positive control, not product code.

A future supported outer boundary must, at minimum:

1. confine the provider host itself and all descendants, not only provider-
   initiated tools;
2. deny reads outside the one disclosed metadata document even when the target
   path is known and DUX has FDA;
3. start fail-closed and expose no inherited descriptor, powerful IPC broker,
   automatic approval channel, or general network escape;
4. prove the intended model-service egress without allowing unrelated
   exfiltration;
5. survive child daemonization, parent exit, cancellation, timeout, update,
   and provider failure;
6. pass on macOS 14 and the newest supported macOS release, on Apple Silicon
   and Intel while both architectures are shipped, using the stable production
   signing identity and disposable canaries;
7. receive a new accepted ADR before any provider or privacy-proof consumer is
   enabled.

App Sandbox is a plausible component boundary because Apple documents that a
sandboxed parent's directly launched helpers inherit its restrictions and
recommends XPC when components need different capabilities. DUX's primary app
remains unsandboxed under ADR 0003. A separately signed sandboxed XPC service,
staged-workspace design, or VM therefore requires an independent spike; this
ADR does not assume that it can execute externally installed provider binaries,
obtain credentials safely, or constrain model-service egress.

## Consequences

Benefits:

- a compromised or malicious local provider cannot silently inherit DUX's
  broad storage visibility through an approved product path;
- the existing privacy contract remains useful for a future confined or remote
  metadata-only transport;
- DUX stays deterministic and fully functional with AI disabled;
- the negative result is reproducible without accessing personal files.

Costs:

- the initially proposed plug-and-play Claude/Codex command selection is not
  available;
- “Explain selection” cannot call a model until another transport is approved;
- a supported sandboxed component or VM is materially more engineering and
  release work than invoking an installed executable;
- a remote API would add network privacy, credential storage, provider policy,
  and availability obligations.

## Alternatives considered

### Direct process with fixed arguments and scrubbed state

Rejected. The hostile-child control demonstrates that these measures leave
ordinary ambient reads intact. FDA/TCC can only increase the potential impact.

### Provider-owned sandbox and permission flags

Rejected as the outer boundary. Provider behavior and defaults can change, and
cooperative tool policy does not constrain a compromised provider host.

### `sandbox-exec` or `sandbox_init`

Rejected for shipping. The comparison proves that a kernel denial can work on
one host, but Apple marks these interfaces deprecated and does not publish a
stable custom-profile contract.

### Endpoint Security monitor or authorizer

Rejected as the sole boundary. It adds restricted entitlement, FDA/root,
system-extension, event-coverage, deadline, and dropped-event concerns and is
not a complete read/write/IPC/network sandbox.

### Metadata-only remote API

Potentially viable, not approved here. It avoids executing provider code under
DUX's local authority but requires an accepted network/credential/privacy
design and explicit user disclosure.

### Separately sandboxed XPC component or VM

Potentially viable, not approved here. It must prove provider execution,
credential and update handling, descendant inheritance, narrow egress, and the
full supported-platform matrix without broadening the primary app.

## Validation criteria

This decision remains correctly implemented while:

- the no-provider state is the only local adapter state;
- production source has no local AI process-launch edge;
- the privacy proof has no engine, FFI, Swift, CLI, or provider consumer;
- the path-free spike evidence validates against its frozen v1 schema;
- destructive-call linting keeps the non-shipping harness process and cleanup
  primitives explicitly scoped;
- AI-disabled application and CLI tests remain green.

## Reconsider when

Supersede this ADR only after a supported outer boundary passes the complete
criteria above. A new provider version, more restrictive default flags, a
successful custom `sandbox-exec` profile, or an FDA test that happens to deny
one path is not sufficient.

## References

- [ADR 0003: Primary build without App Sandbox](0003-primary-build-without-app-sandbox.md)
- [ADR 0004: Shared Rust engine](0004-shared-rust-engine.md)
- [Apple DTS: On File System Permissions](https://developer.apple.com/forums/thread/678819)
- [Apple: Discovering and diagnosing App Sandbox violations](https://developer.apple.com/documentation/security/discovering-and-diagnosing-app-sandbox-violations)
- [Apple: Enabling App Sandbox inheritance](https://developer.apple.com/library/archive/documentation/Miscellaneous/Reference/EntitlementKeyReference/Chapters/EnablingAppSandbox.html)
- [Apple WWDC22: What's new in Endpoint Security](https://developer.apple.com/videos/play/wwdc2022/110345/)
- [Apple Platform Deployment: Privacy Preferences Policy Control payload](https://support.apple.com/guide/deployment/privacy-preferences-policy-control-payload-settings-dep38df53c2a/web)
- Local macOS manual pages: `sandbox(7)`, `sandbox-exec(1)`, and
  `sandbox_init(3)`
