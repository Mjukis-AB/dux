import AppKit
import Foundation
import QuickLookUI

enum ExplorerLiveFileAction: Equatable, Sendable {
    case reveal
    case copyPath
    case quickLook
}

struct ExplorerResolvedLiveItem: Equatable, Sendable {
    let nodeID: UInt64
    let kind: ExplorerSnapshotNodeKind
    let url: URL
    /// Exact Unicode text. The strict adapter rejects any filesystem path that
    /// Foundation cannot round-trip byte-for-byte before this value is built.
    let exactTextPath: String?
}

@MainActor
protocol ExplorerLiveFileActionPresenting: AnyObject {
    func reveal(_ item: ExplorerResolvedLiveItem) -> Bool
    func copyPath(_ item: ExplorerResolvedLiveItem) -> Bool
    func quickLook(_ item: ExplorerResolvedLiveItem) -> Bool
    func dismissQuickLook()
}

/// Owns the one app-global Quick Look panel and the shortest-lived possible
/// current URL. Paths are never persisted, logged, or exposed to cleanup code.
@MainActor
final class SystemExplorerLiveFileActionPresenter: NSObject,
    ExplorerLiveFileActionPresenting, @preconcurrency QLPreviewPanelDataSource,
    QLPreviewPanelDelegate
{
    private var previewURL: URL?

    func reveal(_ item: ExplorerResolvedLiveItem) -> Bool {
        NSWorkspace.shared.activateFileViewerSelecting([item.url])
        return true
    }

    func copyPath(_ item: ExplorerResolvedLiveItem) -> Bool {
        guard let exactTextPath = item.exactTextPath else {
            return false
        }
        let pasteboard = NSPasteboard.general
        pasteboard.clearContents()
        return pasteboard.setString(exactTextPath, forType: .string)
    }

    func quickLook(_ item: ExplorerResolvedLiveItem) -> Bool {
        guard item.kind == .file, let panel = QLPreviewPanel.shared() else {
            return false
        }
        previewURL = item.url
        panel.dataSource = self
        panel.delegate = self
        panel.reloadData()
        panel.makeKeyAndOrderFront(nil)
        return true
    }

    func dismissQuickLook() {
        guard let panel = QLPreviewPanel.sharedPreviewPanelExists() ? QLPreviewPanel.shared() : nil
        else {
            previewURL = nil
            return
        }
        if panel.dataSource === self {
            panel.orderOut(nil)
            panel.dataSource = nil
        }
        if panel.delegate === self {
            panel.delegate = nil
        }
        previewURL = nil
    }

    func numberOfPreviewItems(in _: QLPreviewPanel!) -> Int {
        previewURL == nil ? 0 : 1
    }

    func previewPanel(_: QLPreviewPanel!, previewItemAt index: Int) -> (any QLPreviewItem)! {
        guard index == 0, let previewURL else {
            return nil
        }
        return previewURL as NSURL
    }

    func previewPanelWillClose(_: Notification!) {
        previewURL = nil
    }
}

@MainActor
final class UnavailableExplorerLiveFileActionPresenter: ExplorerLiveFileActionPresenting {
    func reveal(_: ExplorerResolvedLiveItem) -> Bool { false }
    func copyPath(_: ExplorerResolvedLiveItem) -> Bool { false }
    func quickLook(_: ExplorerResolvedLiveItem) -> Bool { false }
    func dismissQuickLook() {}
}
