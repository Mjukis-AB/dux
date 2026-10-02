import SwiftUI

enum ExplorerExternalStorageSection: String, CaseIterable, Identifiable {
    case map = "Map"
    case browse = "Browse"

    var id: Self { self }
}

struct ExplorerExternalStorageView: View {
    @Binding var section: ExplorerExternalStorageSection
    @Bindable var browser: ExplorerSnapshotBrowserModel
    let model: AppModel
    let supplementalPresentation: any ExplorerSnapshotSupplementalPresenting
    let openSettingsDestination: @MainActor @Sendable () -> Void
    @State private var presentationID = UUID()
    @State private var diskMapShowsDirectory = false

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack(alignment: .firstTextBaseline) {
                VStack(alignment: .leading, spacing: 3) {
                    Text("External Storage")
                        .font(.title2.bold())
                    if let capacity = model.volumeState.snapshot {
                        Text(volumeSubtitle(capacity))
                            .font(.subheadline)
                            .foregroundStyle(.secondary)
                    }
                }
                Spacer()
                Picker("External Storage view", selection: $section) {
                    ForEach(ExplorerExternalStorageSection.allCases) { section in
                        Text(section.rawValue).tag(section)
                    }
                }
                .pickerStyle(.segmented)
                .frame(width: 220)
                .accessibilityIdentifier(ExplorerAccessibility.externalStorageSection)
            }
            .padding(.horizontal, 20)
            .padding(.top, 20)

            switch section {
            case .map:
                ExplorerDiskMapView(
                    browser: browser,
                    model: model,
                    showingDirectory: $diskMapShowsDirectory,
                    openBrowse: { section = .browse }
                )
                .padding(.horizontal, 20)
                .padding(.bottom, 20)
            case .browse:
                ExplorerSnapshotBrowserView(
                    browser: browser,
                    model: model,
                    supplementalPresentation: supplementalPresentation,
                    openSettingsDestination: openSettingsDestination,
                    managesPresentation: false,
                    showsHeader: false
                )
            }
        }
        .task {
            await browser.present(id: presentationID)
        }
        .onDisappear {
            let presentationID = presentationID
            Task { await browser.dismiss(id: presentationID) }
        }
        .onChange(of: section) { _, newSection in
            guard newSection == .map, browser.breadcrumbs.count > 1 else { return }
            diskMapShowsDirectory = true
        }
        .accessibilityIdentifier(ExplorerAccessibility.externalStorage)
    }

    private func volumeSubtitle(_ capacity: VolumeCapacitySnapshot) -> String {
        let name = capacity.displayName ?? "Startup volume"
        return "\(name) · \(StorageByteFormatter.string(from: capacity.totalBytes))"
    }
}
