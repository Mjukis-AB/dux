import DuxAIExplanationPresentation
import SwiftUI

/// The only app-side projection into the compiler-isolated explanation
/// module. It can read immutable Explorer presentation facts and cannot call
/// Browser selection, candidate, plan, cleanup, Trash, or effect methods.
@MainActor
final class ExplorerAIExplanationContextAdapter:
    ExplorerAIExplanationContextReading
{
    private weak var browser: ExplorerSnapshotBrowserModel?

    init(browser: ExplorerSnapshotBrowserModel) {
        self.browser = browser
    }

    var aiExplanationContext: ExplorerAIExplanationContext {
        guard let context = browser?.supplementalPresentationContext else {
            return ExplorerAIExplanationContext(
                revision: .max,
                sourceScanID: nil,
                selectedDirectoryNodeID: nil,
                currentDirectoryNodeID: nil,
                isEligible: false,
                visibleObservedNodeIDs: []
            )
        }
        return ExplorerAIExplanationContext(
            revision: context.revision,
            sourceScanID: context.sourceScanID,
            selectedDirectoryNodeID: context.selectedDirectoryNodeID,
            currentDirectoryNodeID: context.currentDirectoryNodeID,
            isEligible: context.canPrepare,
            visibleObservedNodeIDs: context.visibleObservedNodeIDs
        )
    }
}

@MainActor
final class ExplorerAIExplanationInvalidationAdapter:
    ExplorerSupplementalPresentationInvalidating
{
    private let model: ExplorerAIExplanationModel

    init(model: ExplorerAIExplanationModel) {
        self.model = model
    }

    func invalidateBeforeExplorerContextChange(
        preserving scope: ExplorerSupplementalPresentationScope?
    ) async {
        await model.invalidateBeforeContextChange(
            preserving: scope.map {
                ExplorerAIExplanationPreservationScope(
                    sourceScanID: $0.sourceScanID,
                    rootNodeID: $0.rootNodeID,
                    nextContextRevision: $0.nextContextRevision
                )
            }
        )
    }
}

private enum ExplorerAIExplanationCacheClearBarrierError: Error {
    case unavailable
}

/// Write-only Settings gate. It can fence, drain, and reopen AI presentation
/// but exposes no explanation state or cache identity back to Settings.
@MainActor
final class ExplorerAIExplanationCacheClearBarrierAdapter:
    DuxAIInsightCacheClearBarrier
{
    private let model: ExplorerAIExplanationModel

    init(model: ExplorerAIExplanationModel) {
        self.model = model
    }

    func beginAIInsightCacheClear() async throws
        -> any DuxAIInsightCacheClearBarrierLease
    {
        guard let fence = await model.beginCacheClearFence() else {
            throw ExplorerAIExplanationCacheClearBarrierError.unavailable
        }
        return ExplorerAIExplanationCacheClearBarrierAdapterLease(
            model: model,
            fence: fence
        )
    }
}

@MainActor
private final class ExplorerAIExplanationCacheClearBarrierAdapterLease:
    DuxAIInsightCacheClearBarrierLease
{
    private let model: ExplorerAIExplanationModel
    private let fence: ExplorerAIExplanationCacheClearFence
    private var isReleased = false

    init(
        model: ExplorerAIExplanationModel,
        fence: ExplorerAIExplanationCacheClearFence
    ) {
        self.model = model
        self.fence = fence
    }

    func releaseAndWait() async {
        guard !isReleased else { return }
        isReleased = true
        model.endCacheClearFence(fence)
    }
}

/// One-way rendering adapter. Action-owning Explorer views receive only
/// type-erased views and accessibility prose; raw model group membership never
/// leaves the isolated module.
@MainActor
final class ExplorerAIExplanationPresentationAdapter:
    ExplorerSnapshotSupplementalPresenting
{
    private let model: ExplorerAIExplanationModel

    init(model: ExplorerAIExplanationModel) {
        self.model = model
    }

    func modalPresenter() -> AnyView {
        AnyView(
            ExplorerAIExplanationModalPresenter(model: model)
                .task(id: model.context.revision) {
                    await self.model.synchronizeContext()
                }
        )
    }

    func status(
        openSettings: @escaping @MainActor () -> Void
    ) -> AnyView {
        AnyView(
            ExplorerAIExplanationStatusView(
                model: model,
                sendIntent: { intent in
                    switch intent {
                    case .openProviderSettings:
                        openSettings()
                    }
                }
            )
        )
    }

    func inspectorControl() -> AnyView {
        AnyView(ExplorerAIExplanationInspectorControl(model: model))
    }

    func tableDecoration(forObservedNodeID nodeID: UInt64) -> AnyView {
        AnyView(
            ExplorerAIExplanationDecorationView(
                model: model,
                observedNodeID: nodeID
            )
        )
    }

    func treemapDecoration(forObservedNodeID nodeID: UInt64) -> AnyView {
        tableDecoration(forObservedNodeID: nodeID)
    }

    func accessibilitySuffix(forObservedNodeID nodeID: UInt64) -> String {
        model.accessibilitySuffix(forObservedNodeID: nodeID)
    }
}
