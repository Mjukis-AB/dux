import AppKit
import SwiftUI

enum DuxSceneID {
    static let explorer = "explorer"
}

@MainActor
enum AppActivation {
    static func openExplorer(using openWindow: OpenWindowAction) {
        openWindow(id: DuxSceneID.explorer)
        NSApplication.shared.activate()
    }

    static func openSettings(using openSettings: OpenSettingsAction) {
        openSettings()
        NSApplication.shared.activate()
    }

    static func quit() {
        NSApplication.shared.terminate(nil)
    }
}
