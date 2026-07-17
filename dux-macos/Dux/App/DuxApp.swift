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
            Image(systemName: "externaldrive.fill")
                .accessibilityLabel("DUX storage status")
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
        .defaultSize(width: 760, height: 560)
        .windowResizability(.contentMinSize)
    }
}
