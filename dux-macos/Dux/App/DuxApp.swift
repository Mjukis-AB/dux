import SwiftUI

@main
@MainActor
struct DuxApp: App {
    @NSApplicationDelegateAdaptor(DuxAppDelegate.self) private var appDelegate
    @State private var model = AppRuntime.shared.model

    var body: some Scene {
        MenuBarExtra {
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
            DuxSettingsView(model: model)
        }
    }

    private var explorerScene: some Scene {
        Window("DUX Explorer", id: DuxSceneID.explorer) {
            ExplorerView(model: model)
        }
        .defaultSize(width: 960, height: 680)
        .windowResizability(.contentMinSize)
    }
}
