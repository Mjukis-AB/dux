# M7 AI authority-isolation security review

- Review date: 2026-08-09
- Reviewed commit: the commit containing this document
- Reviewed base: `60c4401` (`feat: add consent-gated AI explanations`)
- Scope: native explanation presentation ownership and every reachable
  candidate/plan/cleanup/dry-run/Trash admission edge
- Result: approved for the uncached M7 explanation flow

## Conclusion

The compiled native graph has no callable or type-conversion edge from AI
output to candidate, plan, approval, cleanup, dry-run, Trash, scheduler,
persistence, CLI, or executor authority. AI remains optional, read-only, and
memory-only. Provider success, hostile fake membership, failure, cancellation,
dismissal, and the disabled provider leave deterministic Browser state
unchanged and produce zero calls to the reviewed authority services.

This review does not approve AI persistence. The reserved `ai_insights` schema,
cache migration, sealed cache admission, and explicit clear-cache controls
remain the final open M7 implementation slice.

## Compiled dependency and capability graph

```text
retained Explorer review
    -> narrow DuxAIMetadataPreviewLease
    -> fixed native Anthropic orchestrator
    -> opaque Rust validation attempt
    -> DuxAITransport SPI (one production importer)
    -> private raw membership in DuxAIExplanationPresentation
    -> inert decoration / text / accessibility presentation

immutable Browser supplemental context
    -> weak ExplorerAIExplanationContextAdapter
    -> DuxAIExplanationPresentation lifecycle model

DuxAIExplanationPresentation invalidation request
    -> write-only ExplorerSupplementalPresentationInvalidating
    <- Browser calls before deterministic context changes

deterministic Browser selection/candidate
    -> core/review-minted opaque plan review
    -> explicit cleanup confirmation
    -> cleanup or dry-run task

deterministic Browser selection
    -> Browser-minted opaque ExplorerTrashConfirmation
    -> exact-generation, one-shot Trash execution
```

The first three paths can render observations only. They do not receive a
reference, closure, protocol, identifier conversion, or target dependency that
can invoke either action path.

## Compiler boundary

`DuxAIExplanationPresentation` is an XcodeGen `library.static` target. It has no
target or package dependency and imports only Foundation, Observation, and
SwiftUI. The DUX application depends on it in one direction. `DuxTests` has a
non-linking build dependency so the hosted test process uses the app's one
linked copy rather than duplicating the static-library types.

The isolated target owns:

- the exact local disclosure and path-free preview DTOs;
- the narrow explanation service/session protocols;
- the consent, authority-drift, cancellation, exact-once release, and joined
  terminal-fence lifecycle model;
- private validated group membership; and
- consent, status, result, inspector, badge, and modal presentation views.

It does not import or name DUX, DuxFFI, Sparkle, AppModel, snapshot review,
candidate/rule, planner, plan review, approval, scheduler, persistence, cleanup,
Trash, live file actions, or executor code. It has no file or network API.

## Public type and SPI audit

The ordinary public result surface exposes provider/model/revision binding,
source review identity, explained root identity, digest, inert prose, group
count, and a one-item decoration lookup. Raw group membership is stored in the
private `ExplorerAIExplanationGroup` type. A decoration contains only ordinal,
title, and reason; it contains no item identity, path, candidate, safety claim,
action, callback, plan, approval, schedule, cleanup, or executor input.

Raw Rust-validated membership crosses only through
`ExplorerAIExplanationTransportGroup`, which is protected by the
`DuxAITransport` SPI. `ExplorerAIExplanationService.swift` is the sole
production SPI importer and immediately hands the validated values to the
isolated module. No Browser or action view imports that SPI.

The native service stores only `DuxAIMetadataPreviewLease`. It does not receive
the broad Browser/review interface, AppModel, candidate services, action
presenters, schedulers, persistence, plan handles, or cleanup tasks. Generated
UniFFI attempt types remain behind `EngineService` as established by FFI v60.

## Host projection audit

Browser publishes an immutable `ExplorerSupplementalPresentationContext` with
only revision, source scan ID, selected/current directory IDs, eligibility, and
already visible observed IDs. The AI context adapter holds Browser weakly and
maps only those facts. Browser holds only a write-only invalidation existential;
it receives no AI state, result, group, or preservation decision in return.

Action-owning Browser, table, treemap, and inspector sources contain no AI
model/session/disclosure/result type or raw AI membership. They render only
type-erased views and append inert accessibility prose. Decorations have hit
testing disabled and are accessibility-hidden as independent controls.

## Action admission audit

The following Browser entry points were reviewed and are guarded by the
architecture suite:

- `reviewCandidate`
- `prepareSelectedRustTargetPlanReview`
- `makeRustTargetCleanupConfirmation`
- `startConfirmedRustTargetCleanup`
- `startRustTargetDryRun`
- `executeConfirmedTrash`

None reads supplemental context or names an AI type. Existing plan and cleanup
paths still consume review/core-minted opaque values. Trash was tightened in
this checkpoint: Browser alone can construct `ExplorerTrashConfirmation`; its
scan ID, generation, and selected node ID are fileprivate, execution accepts no
raw `UInt64` or `String`, pending authority is consumed before suspension, and
reuse or selection drift fails before the service call.

## Indirect-bridge threat cases

The review considered these bypasses:

1. Rename an AI symbol and call an action helper indirectly. Target/import and
   reverse-capability guards reject the dependency; all current authority
   member bodies are also inspected.
2. Convert AI group node IDs into Trash input. Raw membership is private and
   Trash accepts only a Browser-minted opaque confirmation.
3. Change Browser selection from an AI view, then invoke an existing action.
   AI views have no Browser reference or selection/action closure. Their only
   host intent is the closed `openProviderSettings` case.
4. Smuggle an action through a generic callback. The presentation API returns
   type-erased inert views and accepts no arbitrary action callback; the sole
   host intent carries no identity or payload.
5. Preserve a stale consent session across navigation. Browser invalidates
   before context changes; the model compares immutable authority identity and
   releases sessions. Only an already validated inert result may survive an
   explicit exact-root descent.
6. Race cancellation, invalidation, terminal shutdown, or a late provider
   result. Managed sessions latch cancellation and share one exact release task;
   terminal fencing rejects new operations and joins every accepted operation.
7. Return hostile prose or unknown fake membership. The real Rust boundary
   rejects unknown request-local nodes and authority-shaped output. The runtime
   tripwire additionally bypasses that validator with a hostile fake containing
   `UInt64.max`; the value remains inert and all authority counters stay zero.
8. Disable or fail the provider. `UnavailableExplorerAIExplanationService`
   fails only the isolated presentation model. Scanning, navigation,
   deterministic findings, action availability, settings, and CLI ownership do
   not depend on an AI provider.

## Verification evidence

All verification was credential-free and performed without a provider request
or filesystem cleanup effect.

- `python3 scripts/tests/test_ai_remote_transport_architecture.py`
  - 11/11 architecture guards passed.
- `python3 -m unittest discover -s scripts/tests -p 'test_*.py'`
  - 124/124 repository policy tests passed.
- `python3 scripts/check_destructive_calls.py`
  - clean across 395 repository source files.
- Focused hosted tests for `ExplorerAIExplanationModelTests`,
  `ExplorerAIExplanationServiceTests`, `ExplorerSnapshotBrowserTests`, and the
  adversarial zero-authority-call tripwire passed.
- Full hosted macOS test suite, recorded at
  `/private/tmp/dux-m7-ai-authority.xcresult`
  - 835/835 passed; zero failures, skips, or expected failures.
- `cargo fmt --all -- --check`
  - passed.
- `cargo check --workspace --all-targets --locked`
  - passed.
- `cargo clippy --workspace --all-targets --locked -- -D warnings`
  - passed.
- `cargo test -p dux-ffi --lib --locked`
  - 135 passed, 2 intentionally ignored real-process cleanup fixtures.
- Universal Debug and Release app builds with
  `ARCHS='arm64 x86_64' ONLY_ACTIVE_ARCH=NO CODE_SIGNING_ALLOWED=NO`
  - both app executables and both
    `libDuxAIExplanationPresentation.a` products contain `x86_64 arm64`.
- `git diff --check`
  - passed.

## Explicit caveats

- No live API key was read, stored, or verified.
- No Anthropic or other provider request was made.
- No cache row was inserted, loaded, migrated, or cleared.
- No candidate, plan, cleanup, dry-run, Trash, scheduler, persistence, CLI, or
  filesystem effect was invoked as part of the AI tests.
- The two ignored FFI fixtures require an environment with no active Cargo or
  Rust compiler process and remain covered by the dedicated cleanup harness;
  they are unrelated to this presentation boundary.
