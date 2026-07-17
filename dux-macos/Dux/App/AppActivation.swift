import AppKit
import ServiceManagement
import SwiftUI

enum DuxSceneID {
    static let explorer = "explorer"
}

@MainActor
enum AppActivation {
    static func openExplorer(using openWindow: OpenWindowAction) {
        openExplorer(
            open: { openWindow(id: $0) },
            activate: { NSApplication.shared.activate() }
        )
    }

    static func openExplorer(
        open: (String) -> Void,
        activate: () -> Void
    ) {
        open(DuxSceneID.explorer)
        activate()
    }

    static func openSettings(using openSettings: OpenSettingsAction) {
        openSettings()
        NSApplication.shared.activate()
    }

    static func openLoginItemsSettings() {
        openLoginItemsSettings {
            SMAppService.openSystemSettingsLoginItems()
        }
    }

    static func openLoginItemsSettings(open: () -> Void) {
        open()
    }

    static func quit() {
        NSApplication.shared.terminate(nil)
    }
}
