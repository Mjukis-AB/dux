# ADR 0003: Primary build without App Sandbox

- Status: Accepted
- Date: 2026-07-15
- Scope: primary macOS app entitlement and filesystem-access model

## Context

DUX explains storage usage across a user's Mac, including developer artifacts,
large files, caches, application containers, and areas commonly summarized as
System Data. A sandboxed application normally reaches its container,
entitlement-selected standard folders, and user-selected locations. Requiring a
folder picker for every relevant root would create incomplete and unstable scan
coverage and would not match the product's disk-pressure-assistant model.

App Sandbox and macOS privacy controls are different layers. Removing the App
Sandbox entitlement does not grant Full Disk Access, bypass TCC, override POSIX
permissions or ACLs, bypass System Integrity Protection, or authorize a
destructive operation. macOS 14 and later can also ask for access when an app
enters another app's container. The application must treat access as observable
coverage, not as an assumed global capability.

Running without App Sandbox increases the impact of an application compromise.
This decision therefore requires explicit compensating controls and does not
authorize broad entitlements, root access, arbitrary helpers, or automatic
cleanup.

## Decision

Do not enable `com.apple.security.app-sandbox` for the primary direct-download
build.

The application runs as the signed-in user with no `sudo`, privileged helper,
launch daemon, or root-owned component in the first production architecture.
It begins with access already available to that user and remains useful when
coverage is partial.

Full Disk Access is optional and user-controlled:

- do not request it during first launch;
- explain scan coverage before suggesting broader access;
- offer guided System Settings steps only when the user asks for deeper
  analysis or when a relevant area is inaccessible;
- never claim Full Disk Access based only on an entitlement or one successful
  probe;
- never claim a scan is complete when paths were denied, skipped, timed out, or
  inaccessible.

macOS provides no public authoritative API for asking whether this application
has Full Disk Access. Product state and diagnostics therefore report observed
coverage and access evidence, never an `enabled`/`disabled` Full Disk Access
boolean.

## Implementation constraints

### Access and coverage

- Every scan produces structured coverage and issue records.
- Map permission failures to user-facing categories without turning inaccessible
  bytes into zero.
- Use a small set of non-destructive probes to explain likely access limits, but
  preserve the underlying error because Full Disk Access cannot be inferred
  perfectly from a single path.
- Opening System Settings is guidance, not authorization. The user must make the
  decision in Privacy & Security.
- Recheck coverage after the app becomes active; do not poll protected paths
  continuously.
- Respect volume boundaries, network/virtual filesystem policy, TCC, POSIX
  permissions, ACLs, SIP, file flags, and symlink/reparse protections.

### Least privilege and code integrity

- Enable Hardened Runtime and library validation. Add runtime exceptions only
  through a separate security review.
- Do not load unsigned native plug-ins or arbitrary dynamic libraries.
- Sign all bundled Mach-O code, including Rust libraries and the optional CLI.
- Keep release entitlements in a reviewed file and verify the signed result in
  CI.
- Use `SMAppService.mainApp` only for opt-in launch at login. Do not install a
  daemon or auxiliary agent merely to keep the app alive.
- Never ask for administrator credentials to improve scan coverage.

### Filesystem mutation

- Unsandboxed visibility is not cleanup authorization.
- AI remains read-only interpretation and cannot create targets, approve plans,
  or invoke filesystem tools.
- All cleanup targets come from deterministic shipped rules or an explicit
  user selection, then pass the shared planner and safety validator.
- Arbitrary Explorer cleanup goes to Trash by default. Permanent deletion is
  restricted to deterministic, tested cases or an explicit advanced flow.
- Scheduled cleanup is disabled by default and limited to policy-marked rules.
- No UI or provider adapter may call raw recursive deletion outside the shared
  executor boundary.

### Sensitive local data

- Scan snapshots, path indexes, history, and databases are sensitive and remain
  local by default.
- Create application data directories as user-only and files as user-readable
  only (target permissions 0700 and 0600 respectively).
- Do not send file contents to AI by default. Redact home paths and exclude
  protected data categories before any provider call.
- The app must function fully in deterministic mode with AI disabled.

### CLI and child processes

- Bundling or installing `dux` does not give it more authority than the user who
  invokes it.
- A CLI launched separately has its own process, terminal, signing, and TCC
  context. The app must not imply that access granted to the GUI also applies
  to a separately launched CLI.
- The app must never run the CLI as a privileged backdoor to bypass its own
  safety or access checks.
- AI command-provider processes receive only the structured, redacted request
  selected by the AI boundary. They do not receive cleanup plan handles or an
  ambient instruction to inspect the filesystem.
- A sanitized environment, empty working directory, redacted request, and
  provider tool-disable flags are defense in depth, not operating-system
  confinement. A subprocess of an unsandboxed app may retain ambient filesystem
  and TCC authority, especially after the app receives Full Disk Access.
- Do not ship a Claude, Codex, or other local command-provider adapter until an
  adversarial security/TCC spike proves its authority boundary on every
  supported macOS release. If that boundary cannot be demonstrated, use a
  metadata-only remote API or another architecture with real confinement.
- [ADR 0009](0009-reject-direct-local-ai-subprocesses.md) records the completed
  negative spike: direct local commands retain disqualifying ordinary ambient
  reads, so the conditional Claude/Codex adapters are prohibited and absent.

## Consequences

Benefits:

- DUX can analyze broad user-visible storage without forcing users to select
  every directory independently.
- The app, shared Rust engine, and standalone CLI can use a coherent path model.
- Optional CLI installation and direct distribution do not require a second
  sandbox-specific product architecture.
- Coverage limitations can be explained in terms of actual macOS protections
  rather than being conflated with App Sandbox alone.

Costs and risks:

- A code-execution vulnerability in DUX can access everything available to the
  user and anything the user has granted through TCC, including Full Disk
  Access. This is materially more impact than a sandboxed compromise.
- App Store distribution is unavailable for this build.
- TCC prompts and access behavior can change across macOS releases and signing
  identities, requiring recurring platform testing.
- Users may reasonably distrust a disk tool asking for broad access; copy,
  onboarding, local-data handling, and visible safety controls must earn trust.
- The project must supply containment through small dependencies, code signing,
  hardened runtime, strict FFI, deterministic policy, and safe execution rather
  than relying on App Sandbox.

## Alternatives considered

### App Sandbox with user-selected roots and security-scoped bookmarks

Rejected for the primary build because a durable set of manually selected roots
does not provide reliable whole-disk explanation or low-disk diagnosis. It may
support a future limited/App Store variant if that variant clearly describes
its reduced coverage.

### App Sandbox plus privileged helper

Rejected. It would move broad authority into a harder-to-audit component, add
installation and update risk, and contradict the no-root first-release boundary.

### Require Full Disk Access at onboarding

Rejected. The app must first demonstrate useful read-only value and explain
what remains inaccessible. Full Disk Access is a high-trust user choice, not a
condition for showing basic volume status or scanning accessible locations.

### Run the CLI out of process to escape the sandbox

Rejected as a security boundary violation and a fragile architecture. The app
and CLI share the Rust engine; the CLI is not a privilege broker.

## Validation criteria

Before any public app build:

- the release signature has Hardened Runtime and does not have the App Sandbox
  entitlement;
- the app launches and provides volume status with no Full Disk Access;
- a limited-access scan produces `Partial` or `Limited access`, with concrete
  issues and guidance;
- granting and revoking access changes coverage without corrupting snapshots;
- no workflow requests `sudo`, installs a daemon, or writes privileged paths;
- user-only permissions are verified for local databases and snapshots;
- AI-disabled operation passes the complete read-only test suite;
- hostile-provider tests prove that every enabled AI adapter remains within the
  separately approved authority boundary, including when the app has broader
  TCC access;
- destructive-call linting and dry-run mutation tests pass before cleanup ships;
- macOS major-version testing covers TCC, app containers, SIP-protected paths,
  and Full Disk Access guidance.

## Reconsider when

Create a superseding ADR if Apple introduces a sandbox mechanism that supports
the required explainable scan coverage, if an App Store variant becomes a real
product requirement, or if security review concludes that direct unsandboxed
operation cannot meet the project's risk threshold. Do not silently enable a
privileged helper as a workaround.

## References

- [ADR 0002: Direct Developer ID distribution](0002-direct-developer-id-distribution.md)
- [Roadmap safety and privacy boundaries](../../ROADMAP.md#33-safety-boundaries)
- [Roadmap permissions onboarding](../../ROADMAP.md#1210-permissions-onboarding)
- [Apple: App Sandbox](https://developer.apple.com/documentation/security/app-sandbox)
- [Apple: Accessing files from the macOS App Sandbox](https://developer.apple.com/documentation/security/accessing-files-from-the-macos-app-sandbox)
- [ADR 0009: Reject direct local AI subprocess adapters](0009-reject-direct-local-ai-subprocesses.md)
- [Apple: Control access to files and folders](https://support.apple.com/guide/mac-help/control-access-to-files-and-folders-on-mac-mchld5a35146/mac)
- [Apple: Privacy & Security settings](https://support.apple.com/guide/mac-help/change-privacy-security-settings-on-mac-mchl211c911f/mac)
