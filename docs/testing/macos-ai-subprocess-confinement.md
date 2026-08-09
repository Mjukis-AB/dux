# macOS AI subprocess confinement spike

## Purpose

This protocol answers one narrow shipping question from ADR 0003 and Milestone
7: may unsandboxed DUX directly launch a local AI command while guaranteeing
that the command cannot use DUX's ambient filesystem or TCC authority?

The answer is **no** for a direct local subprocess. The adversarial probe fails
the gate at the ordinary same-user filesystem layer, before a TCC-specific
claim is necessary. ADR 0009 records the binding product decision.

This is a negative security spike, not provider code. It does not invoke
Claude, Codex, a network, the Rust engine, FFI, a shell, or personal storage.

## Safety boundary

The probe:

- creates a unique owned temporary root with a 0600 sentinel and a separate
  empty working directory;
- closes the sentinel before launch and closes every nonstandard child file
  descriptor;
- gives the hostile child a fixed minimal environment;
- deliberately gives the child the sentinel's absolute path, because a
  confinement boundary must resist a process that already knows common target
  paths;
- asks only whether a bounded canary can be read and never emits its contents;
- routes successful and setup-failure runs through descriptor- and identity-
  checked removal of only the exact files and directories it created. If an
  identity becomes unprovable, it refuses that removal and may leave a private
  0700 temporary root containing only the fixed non-secret canary;
- emits a bounded, path-free observation; the accepted frozen v1 record adds
  only source, tested-binary, compiler, and repository-base provenance before
  schema validation.

It MUST NOT be pointed at Mail, Messages, browser data, credentials, a TCC
database, another application, or any real user file. Do not add a path option.

## Gate logic

DUX's gate is a union: retaining **either** ordinary ambient filesystem access
**or** broader FDA/TCC access rejects the direct adapter. Therefore:

1. a successful out-of-working-directory sentinel read is a complete negative
   result;
2. a failed read would not be a positive result without stable Developer ID,
   FDA, attribution-chain, macOS-version, architecture, and escape-topology
   evidence;
3. no positive TCC conclusion is drawn from Debug, ad-hoc, Terminal, CI, or
   Codex-hosted execution;
4. once the ordinary read succeeds, probing real privacy-protected data cannot
   make the direct architecture safe and is prohibited by this protocol.

## Lanes

### Direct child

The host starts the hostile child by exact executable path without a shell. It
uses a minimal environment, an empty working directory, and only standard file
descriptors. The child attempts one known absolute read outside that directory.

Required result: the positive control must read the canary. That proves the
test is capable of detecting ambient authority. Under the DUX gate this same
result means `confined=false` and `direct_local_adapter=no_go`.

### Deprecated Seatbelt comparison

On macOS, the host repeats the identical child under a deliberately narrow
`sandbox-exec` denial. A denial demonstrates that the test distinguishes a
kernel-enforced restriction from environment/cwd hygiene.

This lane is comparison evidence only. `sandbox-exec(1)` and
`sandbox_init(3)` are deprecated, the custom profile language is not a stable
public product contract, and a passing result MUST report
`production_eligible=false`.

### TCC attribution

The committed harness intentionally does not touch real TCC-protected data.
Apple documents TCC in terms of responsible-code attribution and provides no
general public “has FDA” query. If a future, separately confined architecture
needs approval, run its signed stable identity with disposable canaries on
fresh VM snapshots and capture only the redacted `AttributionChain` evidence
described by Apple's PPPC guidance.

That future matrix must include no FDA, FDA granted, and FDA revoked; direct,
grandchild, interpreter, `setsid`, daemonized, parent-exited, and launchd-
handoff topologies; macOS 14 and the newest supported macOS; and Apple Silicon
plus Intel while both ship. It must probe actual protected APIs rather than a
TCC database. None of that can overturn the direct-child no-go recorded here;
it applies only to a new outer boundary.

## Reproduction

Build and run from a clean checkout using the commands in
[`spikes/ai-subprocess-confinement/README.md`](../../spikes/ai-subprocess-confinement/README.md).
Run the direct lane in an ordinary environment. The deprecated Seatbelt lane
may require execution outside an enclosing sandbox; failure to apply that
profile is `unavailable`, never a passing confinement result.

Before accepting evidence:

1. record the exact repository base commit and commit the probe, schema, and
   accepted evidence together as one immutable checkpoint;
2. compile with the system compiler and warnings as errors;
3. run both lanes without modifying the source or environment contract;
4. compute SHA-256 for the schema, both source files, and both tested binaries,
   record the exact Apple Clang version/build and repository base commit, and
   add only that closed `source` object to the host-produced observation;
5. validate the resulting record against
   [`evidence-v1.schema.json`](../../spikes/ai-subprocess-confinement/evidence-v1.schema.json);
6. confirm the output contains no path, username, PID, temporary identifier,
   environment value, or canary content;
7. store only the validated redacted record under
   `docs/testing/evidence/macos-ai-subprocess-confinement/`.

## Recorded result

The [2026-08-09 record](evidence/macos-ai-subprocess-confinement/2026-08-09-macos-26.5-arm64.json)
was produced on macOS 26.5 (build 25F71), arm64. The direct lane read the out-
of-scope 0600 sentinel. The deprecated Seatbelt comparison denied the same read
when its profile was applied outside the enclosing development sandbox. The
result is a no-go for direct local Claude, Codex, and custom-command adapters.

This run is not claimed as stable-identity FDA evidence. It does not need to
be: the ordinary ambient-read failure is already disqualifying, and avoiding
personal TCC data is the safer decisive test.

## Primary references

- [Apple DTS: On File System Permissions](https://developer.apple.com/forums/thread/678819)
- [Apple: Discovering and diagnosing App Sandbox violations](https://developer.apple.com/documentation/security/discovering-and-diagnosing-app-sandbox-violations)
- [Apple: Enabling App Sandbox inheritance](https://developer.apple.com/library/archive/documentation/Miscellaneous/Reference/EntitlementKeyReference/Chapters/EnablingAppSandbox.html)
- [Apple WWDC22: What's new in Endpoint Security](https://developer.apple.com/videos/play/wwdc2022/110345/)
- [Apple Platform Deployment: PPPC payload settings](https://support.apple.com/guide/deployment/privacy-preferences-policy-control-payload-settings-dep38df53c2a/web)
- Local macOS manual pages: `sandbox(7)`, `sandbox-exec(1)`, and
  `sandbox_init(3)`
