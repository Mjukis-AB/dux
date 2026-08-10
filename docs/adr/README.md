# Architecture Decision Records

This directory records durable architecture decisions for DUX. The roadmap
describes intended product behavior and implementation order; ADRs explain why
specific technical boundaries were chosen and when they may be reconsidered.

## Status vocabulary

- **Proposed**: under review and not yet binding.
- **Accepted**: the default for implementation and review.
- **Superseded**: replaced by a newer ADR, which must link back to the old one.
- **Deprecated**: retained for history but no longer applicable.

Accepted ADRs are not immutable. Replace a decision with a new ADR when its
assumptions materially change; do not silently rewrite the original decision.
Clarifications that do not change the decision may be added in place.

## Index

| ADR | Decision | Status |
| --- | --- | --- |
| [0001](0001-native-swiftui-macos-application.md) | Native SwiftUI macOS application | Accepted |
| [0002](0002-direct-developer-id-distribution.md) | Direct Developer ID distribution | Accepted |
| [0003](0003-primary-build-without-app-sandbox.md) | Primary build without App Sandbox | Accepted |
| [0004](0004-shared-rust-engine.md) | Shared Rust engine | Accepted |
| [0005](0005-uniffi-swift-rust-transport.md) | UniFFI for the Swift/Rust transport | Accepted |
| [0006](0006-icloud-local-copy-eviction.md) | iCloud local-copy eviction boundary | Accepted |
| [0007](0007-prior-boot-running-scan-interruption.md) | Prior-boot running-scan history interruption | Accepted |
| [0008](0008-legacy-unclaimed-running-scan-dismissal.md) | User-confirmed legacy unclaimed running-scan dismissal | Accepted |
| [0009](0009-reject-direct-local-ai-subprocesses.md) | Reject direct local AI subprocess adapters | Accepted |
| [0010](0010-read-only-active-cleanup-provenance-diagnostic.md) | Read-only active-cleanup provenance diagnostic | Accepted |
| [0011](0011-diagnostic-only-cleanup-crash-debt-v1.md) | Diagnostic-only cleanup crash debt in v1 | Accepted |
| [0012](0012-non-shipping-signed-cleanup-qualification.md) | Non-shipping signed cleanup qualification | Accepted |
| [0013](0013-metadata-only-remote-ai-transport.md) | Metadata-only remote AI transport | Accepted |
| [0014](0014-automation-clock-wake-and-missed-run-semantics.md) | Automation clock, wake, and missed-run semantics | Accepted |
| [0015](0015-automation-activation-and-utc-recurrence.md) | Automation activation and UTC recurrence | Accepted |
| [0016](0016-automation-category-scope-membership-consent.md) | Automation category-scope membership consent | Accepted |

## Authoring rules

Each ADR must include:

1. status and decision date;
2. the forces and constraints behind the decision;
3. the decision and its implementation boundaries;
4. positive and negative consequences;
5. alternatives considered;
6. validation criteria and explicit reconsideration triggers;
7. links to related roadmap sections, ADRs, and primary references.

Use the next four-digit number. Keep one primary decision per file. If a spike
is required before deciding, record the uncertainty rather than presenting an
untested choice as accepted.
