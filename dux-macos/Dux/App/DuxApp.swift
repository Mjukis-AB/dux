import SwiftUI

@main
@MainActor
struct DuxApp: App {
    @NSApplicationDelegateAdaptor(DuxAppDelegate.self) private var appDelegate
    @State private var model = AppRuntime.shared.model
    @State private var explorerSnapshotBrowser = AppRuntime.shared.explorerSnapshotBrowser
    @State private var aiProviderSettings = AppRuntime.shared.aiProviderSettings
    @Environment(\.openWindow) private var openWindow

    var body: some Scene {
        let _ = AppRuntime.shared.installExplorerOpener { destination in
            model.requestExplorerDestination(destination)
            AppActivation.openExplorer(using: openWindow)
        }
        MenuBarExtra(
            isInserted: Binding(
                get: { model.isMenuBarItemInserted },
                set: { _ in
                    // Settings policy and the explicit reopen escape hatch own insertion.
                }
            )
        ) {
            MenuBarContentView(model: model)
        } label: {
            MenuBarStatusLabel(
                mode: model.menuBarLabelMode,
                volumeState: model.volumeState
            )
        }
        .menuBarExtraStyle(.window)

        explorerScene

        Settings {
            DuxSettingsView(
                model: model,
                aiProviderSettings: aiProviderSettings
            )
                .frame(width: 620, height: 800)
        }
    }

    private var explorerScene: some Scene {
        Window("DUX Explorer", id: DuxSceneID.explorer) {
            ExplorerView(
                model: model,
                snapshotBrowser: explorerSnapshotBrowser,
                aiProviderSettings: aiProviderSettings
            )
        }
        .defaultSize(width: 960, height: 680)
        .windowResizability(.contentMinSize)
    }
}
