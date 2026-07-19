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

/// The only Foundation-facing Trash dependency. UI and FFI code must not
/// construct or pass arbitrary cleanup URLs here; the core-issued callback is
/// the sole production entry point.
protocol TrashFileManaging {
    func moveToTrash(at url: URL) throws
}

extension FileManager: TrashFileManaging {
    func moveToTrash(at url: URL) throws {
        // DUX-DESTRUCTIVE: allow=macos-trash-platform-adapter -- the sole synchronous FileManager Trash primitive in the reviewed adapter
        try trashItem(at: url, resultingItemURL: nil)
    }
}

enum MacOSTrashAdapterError: Error, Equatable {
    /// Foundation may have moved or reconciled an item before throwing. The
    /// journal must conservatively recover instead of claiming a clean
    /// failure or retrying.
    case outcomeUnknown
    case invalidRequest
}

/// The generated callback request is intentionally abstracted before it is
/// handed to the Foundation adapter. Tests can exercise byte validation without
/// constructing a Rust object, while production receives only core-issued
/// requests.
protocol TrashEffectRequestReading: Sendable {
    func recordVersion() throws -> UInt32
    func targetKind() throws -> TrashEffectTargetKind
    func pathEncoding() throws -> SnapshotNameEncoding
    func takePathBytes() throws -> Data
}

extension TrashEffectRequest: TrashEffectRequestReading {}

struct MacOSTrashPlatformAdapter {
    private let fileManager: any TrashFileManaging

    init(fileManager: any TrashFileManaging = FileManager.default) {
        self.fileManager = fileManager
    }

    /// The core-owned callback supplies the exact reviewed target while
    /// holding its one-shot journal admission; no caller may retry after this
    /// returns.
    func trash(_ url: URL) -> Result<Void, MacOSTrashAdapterError> {
        do {
            try fileManager.moveToTrash(at: url)
            return .success(())
        } catch {
            return .failure(.outcomeUnknown)
        }
    }

    /// Map one core-issued callback request to a URL and invoke the reviewed
    /// synchronous Foundation seam. No caller-supplied path is accepted.
    func trash(request: any TrashEffectRequestReading) -> TrashPlatformResult {
        let url: URL
        do {
            let targetKind = try request.targetKind()
            guard
                try request.recordVersion() == 1,
                try request.pathEncoding() == .unixBytes
            else {
                return .failed
            }
            let bytes = [UInt8](try request.takePathBytes())
            let isDirectory = targetKind == .directory
            url = try ExplorerSnapshotLivePathAdapter.url(
                fromValidatedUnixPathBytes: bytes,
                isDirectory: isDirectory
            )
        } catch {
            return .failed
        }
        switch trash(url) {
        case .success:
            return .completed
        case .failure(.outcomeUnknown):
            return .outcomeUnknown
        case .failure(.invalidRequest):
            return .failed
        }
    }
}

/// UniFFI callback implementation for the reviewed Explorer Trash path. The
/// core owns review, journal admission, and one-shot request issuance.
final class MacOSTrashPlatformDriver: TrashPlatformDriver, @unchecked Sendable {
    private let adapter: MacOSTrashPlatformAdapter

    init(adapter: MacOSTrashPlatformAdapter = MacOSTrashPlatformAdapter()) {
        self.adapter = adapter
    }

    func trash(request: TrashEffectRequest) -> TrashPlatformResult {
        adapter.trash(request: request)
    }
}
