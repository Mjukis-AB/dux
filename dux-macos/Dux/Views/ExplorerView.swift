import SwiftUI

struct ExplorerView: View {
    let model: AppModel

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 24) {
                VStack(alignment: .leading, spacing: 6) {
                    Text("Storage Explorer")
                        .font(.largeTitle.bold())
                    Text("A normal window for understanding and navigating disk usage.")
                        .foregroundStyle(.secondary)
                }

                VolumeOverviewView(state: model.volumeState)

                GroupBox("Shared engine connection") {
                    EngineConnectionView(state: model.engineState)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(.vertical, 6)
                }

                Text("Drill-down navigation will build on the shared engine snapshot. Capacity remains independent from directory scan totals.")
                    .foregroundStyle(.secondary)
                    .frame(maxWidth: 560, alignment: .leading)
            }
            .padding(32)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .frame(minWidth: 620, minHeight: 420)
        .task {
            await model.loadInitialState()
        }
    }
}

private struct VolumeOverviewView: View {
    let state: VolumeCapacityState

    var body: some View {
        GroupBox("Startup volume") {
            switch state {
            case .idle, .loading:
                HStack(spacing: 10) {
                    ProgressView()
                        .controlSize(.small)
                    Text("Checking startup disk…")
                        .foregroundStyle(.secondary)
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.vertical, 6)
            case let .loaded(snapshot):
                snapshotContent(snapshot)
            case let .refreshing(snapshot):
                VStack(alignment: .leading, spacing: 10) {
                    snapshotContent(snapshot)
                    Label("Refreshing capacity…", systemImage: "arrow.triangle.2.circlepath")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            case let .stale(snapshot, _):
                VStack(alignment: .leading, spacing: 10) {
                    snapshotContent(snapshot)
                    Label("Showing the last successful capacity sample", systemImage: "exclamationmark.triangle")
                        .font(.caption)
                        .foregroundStyle(.orange)
                }
            case .failed:
                Label("Storage capacity unavailable", systemImage: "exclamationmark.triangle.fill")
                    .foregroundStyle(.red)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.vertical, 6)
            }
        }
    }

    private func snapshotContent(_ snapshot: VolumeCapacitySnapshot) -> some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack(alignment: .firstTextBaseline) {
                if let displayName = snapshot.displayName {
                    Text(verbatim: displayName)
                        .font(.headline)
                } else {
                    Text("Startup Disk")
                        .font(.headline)
                }
                Spacer()
                Text("\(snapshot.availablePercentage)% available")
                    .font(.title3.monospacedDigit())
            }

            CapacityBar(snapshot: snapshot)

            DiskPressureBadge(pressure: snapshot.pressure)

            Grid(alignment: .leading, horizontalSpacing: 28, verticalSpacing: 8) {
                row(availableLabel(for: snapshot), snapshot.effectiveAvailableBytes)
                optionalRow("Used", snapshot.usedBytes)
                row("Total", snapshot.totalBytes)
            }

            if snapshot.availabilityBasis == .importantUsage {
                Text("Available for important use can include purgeable space, so it may not equal total minus used.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            } else {
                Text("Important-usage capacity was unavailable; showing the filesystem fallback.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
        .padding(.vertical, 6)
    }

    private func availableLabel(for snapshot: VolumeCapacitySnapshot) -> LocalizedStringKey {
        snapshot.availabilityBasis == .importantUsage ? "Available for important use" : "Available"
    }

    private func row(_ label: LocalizedStringKey, _ bytes: UInt64) -> some View {
        GridRow {
            Text(label)
                .foregroundStyle(.secondary)
            Text(verbatim: StorageByteFormatter.string(from: bytes))
                .monospacedDigit()
                .textSelection(.enabled)
        }
    }

    private func optionalRow(_ label: LocalizedStringKey, _ bytes: UInt64?) -> some View {
        GridRow {
            Text(label)
                .foregroundStyle(.secondary)
            if let bytes {
                Text(verbatim: StorageByteFormatter.string(from: bytes))
                    .monospacedDigit()
                    .textSelection(.enabled)
            } else {
                Text("Unavailable")
                    .foregroundStyle(.secondary)
            }
        }
    }
}

private struct EngineConnectionView: View {
    let state: EngineConnectionState

    var body: some View {
        switch state {
        case .idle, .loading:
            HStack(spacing: 10) {
                ProgressView()
                    .controlSize(.small)
                Text("Connecting to the storage engine…")
                    .foregroundStyle(.secondary)
            }
        case let .loaded(result):
            VStack(alignment: .leading, spacing: 14) {
                Grid(alignment: .leading, horizontalSpacing: 24, verticalSpacing: 8) {
                    row("Library", result.libraryVersion)
                    row("FFI contract", String(result.ffiContractVersion))
                }

                if result.executedOffMainThread {
                    Label(
                        "Rust call completed off the main thread",
                        systemImage: "checkmark.circle.fill"
                    )
                    .foregroundStyle(.green)
                } else {
                    Label(
                        "Rust call reached the main thread",
                        systemImage: "exclamationmark.triangle.fill"
                    )
                    .foregroundStyle(.red)
                }
            }
        case let .failed(error):
            switch error {
            case .closed:
                Label("The Rust engine session is closed.", systemImage: "xmark.circle.fill")
                    .foregroundStyle(.red)
            case .invalidCapacityObservation, .conflictingCapacityObservation,
                 .supersededCapacityObservation, .retryable, .unavailable, .unexpected:
                Label("The Rust engine could not be loaded.", systemImage: "exclamationmark.triangle.fill")
                    .foregroundStyle(.red)
            }
        }
    }

    private func row(_ label: LocalizedStringKey, _ value: String) -> some View {
        GridRow {
            Text(label)
                .foregroundStyle(.secondary)
            Text(verbatim: value)
                .textSelection(.enabled)
        }
    }
}
