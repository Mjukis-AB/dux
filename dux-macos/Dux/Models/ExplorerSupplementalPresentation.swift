import SwiftUI

/// Immutable, read-only Explorer facts available to an optional presentation
/// layer. It carries no path, candidate, plan, approval, cleanup, or effect
/// capability.
struct ExplorerSupplementalPresentationContext: Equatable, Sendable {
    let revision: UInt64
    let sourceScanID: String?
    let selectedDirectoryNodeID: UInt64?
    let currentDirectoryNodeID: UInt64?
    let canPrepare: Bool
    let visibleObservedNodeIDs: Set<UInt64>
}

/// The only information an optional presentation may use when Explorer moves
/// into a new context. The Browser never receives a preservation decision or
/// presentation value in return.
struct ExplorerSupplementalPresentationScope: Equatable, Sendable {
    let sourceScanID: String
    let rootNodeID: UInt64
    let nextContextRevision: UInt64
}

@MainActor
protocol ExplorerSupplementalPresentationInvalidating: AnyObject {
    func invalidateBeforeExplorerContextChange(
        preserving scope: ExplorerSupplementalPresentationScope?
    ) async
}

/// Type-erased rendering slots are deliberately one-way. The action-owning
/// Explorer views can render or append accessibility text, but cannot inspect
/// optional presentation state or extract an observed node identifier from it.
@MainActor
protocol ExplorerSnapshotSupplementalPresenting: AnyObject {
    func modalPresenter() -> AnyView
    func status(openSettings: @escaping @MainActor () -> Void) -> AnyView
    func inspectorControl() -> AnyView
    func tableDecoration(forObservedNodeID nodeID: UInt64) -> AnyView
    func treemapDecoration(forObservedNodeID nodeID: UInt64) -> AnyView
    func accessibilitySuffix(forObservedNodeID nodeID: UInt64) -> String
}

@MainActor
final class EmptyExplorerSnapshotSupplementalPresentation:
    ExplorerSnapshotSupplementalPresenting
{
    static let shared = EmptyExplorerSnapshotSupplementalPresentation()

    private init() {}

    func modalPresenter() -> AnyView { AnyView(EmptyView()) }

    func status(openSettings _: @escaping @MainActor () -> Void) -> AnyView {
        AnyView(EmptyView())
    }

    func inspectorControl() -> AnyView { AnyView(EmptyView()) }

    func tableDecoration(forObservedNodeID _: UInt64) -> AnyView {
        AnyView(EmptyView())
    }

    func treemapDecoration(forObservedNodeID _: UInt64) -> AnyView {
        AnyView(EmptyView())
    }

    func accessibilitySuffix(forObservedNodeID _: UInt64) -> String { "" }
}
