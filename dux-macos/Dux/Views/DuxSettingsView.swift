import SwiftUI

struct DuxSettingsView: View {
    let model: AppModel

    var body: some View {
        Form {
            Section("Application") {
                LabeledContent("App mode") {
                    Text("Menu bar helper")
                }
                Text("Closing Explorer keeps DUX available from the menu bar.")
                    .foregroundStyle(.secondary)
            }

            Section("Engine") {
                switch model.engineState {
                case .idle, .loading:
                    LabeledContent("Status") {
                        Text("Connecting…")
                    }
                case let .loaded(result):
                    LabeledContent("Status") {
                        Text("Ready")
                    }
                    LabeledContent("Library version") {
                        Text(verbatim: result.libraryVersion)
                    }
                    LabeledContent("FFI contract") {
                        Text(verbatim: String(result.ffiContractVersion))
                    }
                case .failed:
                    LabeledContent("Status") {
                        Text("Unavailable")
                    }
                }
            }
        }
        .formStyle(.grouped)
        .frame(width: 480, height: 280)
        .task {
            await model.loadInitialState()
        }
    }
}
