import SwiftUI

struct MenuBarContentView: View {
    @Environment(\.openSettings) private var openSettings
    @Environment(\.openWindow) private var openWindow

    let model: AppModel

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Label("DUX Storage", systemImage: "externaldrive.fill")
                .font(.headline)
                .accessibilityAddTraits(.isHeader)

            volumeSummary

            engineSummary

            Divider()

            Button {
                AppActivation.openExplorer(using: openWindow)
            } label: {
                Label("Open Explorer", systemImage: "rectangle.on.rectangle")
                    .frame(maxWidth: .infinity)
            }
            .buttonStyle(.borderedProminent)
            .controlSize(.large)
            .keyboardShortcut("o", modifiers: [.command])

            HStack {
                Button {
                    AppActivation.openSettings(using: openSettings)
                } label: {
                    Label("Settings…", systemImage: "gearshape")
                }

                Spacer()

                Button("Quit DUX") {
                    AppActivation.quit()
                }
                .keyboardShortcut("q", modifiers: [.command])
            }
        }
        .padding(18)
        .frame(width: 360)
        .task {
            await model.loadInitialState()
        }
    }

    @ViewBuilder
    private var volumeSummary: some View {
        switch model.volumeState {
        case .idle, .loading:
            HStack(spacing: 10) {
                ProgressView()
                    .controlSize(.small)
                Text("Checking startup disk…")
                    .foregroundStyle(.secondary)
            }
        case let .loaded(snapshot):
            volumeSnapshot(snapshot)
        case let .refreshing(snapshot):
            VStack(alignment: .leading, spacing: 8) {
                volumeSnapshot(snapshot)
                Label("Refreshing capacity…", systemImage: "arrow.triangle.2.circlepath")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        case let .stale(snapshot, _):
            VStack(alignment: .leading, spacing: 8) {
                volumeSnapshot(snapshot)
                Label("Last known capacity", systemImage: "exclamationmark.triangle")
                    .font(.caption)
                    .foregroundStyle(.orange)
            }
        case .failed:
            Label("Storage capacity unavailable", systemImage: "exclamationmark.triangle.fill")
                .foregroundStyle(.red)
        }
    }

    private func volumeSnapshot(_ snapshot: VolumeCapacitySnapshot) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            if let displayName = snapshot.displayName {
                Text(verbatim: displayName)
                    .font(.subheadline.weight(.semibold))
            } else {
                Text("Startup Disk")
                    .font(.subheadline.weight(.semibold))
            }

            HStack(alignment: .firstTextBaseline) {
                Text(verbatim: StorageByteFormatter.string(from: snapshot.effectiveAvailableBytes))
                    .font(.title2.bold())
                    .contentTransition(.numericText())
                Spacer()
                Text("\(snapshot.availablePercentage)% available")
                    .font(.caption.monospacedDigit())
                    .foregroundStyle(.secondary)
            }

            if snapshot.availabilityBasis == .importantUsage {
                Text("Available for important use")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            } else {
                Text("Available")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }

            DiskPressureBadge(pressure: snapshot.pressure)

            CapacityBar(snapshot: snapshot)

            HStack {
                Text("Used")
                Spacer()
                if let usedBytes = snapshot.usedBytes {
                    Text(verbatim: StorageByteFormatter.string(from: usedBytes))
                } else {
                    Text("Unavailable")
                }
            }
            .font(.caption)
            .foregroundStyle(.secondary)
        }
    }

    @ViewBuilder
    private var engineSummary: some View {
        switch model.engineState {
        case .idle, .loading:
            HStack(spacing: 10) {
                ProgressView()
                    .controlSize(.small)
                Text("Connecting to the storage engine…")
                    .foregroundStyle(.secondary)
            }
            .accessibilityElement(children: .combine)
        case let .loaded(result):
            HStack(alignment: .top, spacing: 10) {
                Image(systemName: "checkmark.circle.fill")
                    .foregroundStyle(.green)
                    .accessibilityHidden(true)

                VStack(alignment: .leading, spacing: 3) {
                    Text("Storage engine ready")
                        .fontWeight(.medium)
                    Text(verbatim: "v\(result.libraryVersion) · FFI \(result.ffiContractVersion)")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }
            .accessibilityElement(children: .combine)
        case .failed:
            Label("Storage engine unavailable", systemImage: "exclamationmark.triangle.fill")
                .foregroundStyle(.red)
        }
    }
}
