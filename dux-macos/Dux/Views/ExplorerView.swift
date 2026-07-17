import SwiftUI

struct ExplorerView: View {
    @Environment(\.openSettings) private var openSettings
    @State private var selection = ExplorerDestination.overview

    let model: AppModel

    var body: some View {
        let presentation = ExplorerPresentation.make(
            volumeState: model.volumeState,
            scanState: model.scanState
        )

        NavigationSplitView {
            List(selection: $selection) {
                NavigationLink(value: ExplorerDestination.overview) {
                    Label("Overview", systemImage: "chart.pie")
                }
                .accessibilityIdentifier(ExplorerAccessibility.overviewDestination)

                Section {
                    Button {
                        AppActivation.openSettings(using: openSettings)
                    } label: {
                        Label("Settings…", systemImage: "gearshape")
                    }
                    .buttonStyle(.plain)
                    .keyboardShortcut(
                        KeyEquivalent(ExplorerKeyboardShortcut.settings),
                        modifiers: [.command]
                    )
                    .accessibilityIdentifier(ExplorerAccessibility.settingsShortcut)
                }
            }
            .navigationTitle("Storage Explorer")
            .navigationSplitViewColumnWidth(min: 180, ideal: 210, max: 250)
            .accessibilityIdentifier(ExplorerAccessibility.sidebar)
        } detail: {
            ExplorerOverviewView(presentation: presentation)
                .navigationTitle("Overview")
        }
        .navigationSplitViewStyle(.balanced)
        .toolbar {
            ToolbarItem(placement: .navigation) {
                Label("Home", systemImage: "house")
                    .help("The read-only scan root for this version of DUX")
            }

            ToolbarItemGroup(placement: .primaryAction) {
                Button {
                    Task { await model.refreshVolumeCapacity() }
                } label: {
                    Label("Refresh capacity", systemImage: "arrow.clockwise")
                }
                .disabled(!presentation.actions.refreshCapacityEnabled)
                .help("Check startup-disk capacity without starting a scan")
                .accessibilityIdentifier(ExplorerAccessibility.refreshCapacity)

                if presentation.actions.showScanNow {
                    Button {
                        Task { await model.startHomeScan() }
                    } label: {
                        Label("Scan Home", systemImage: "magnifyingglass")
                    }
                    .disabled(!presentation.actions.scanNowEnabled)
                    .help("Scan Home without changing files")
                    .keyboardShortcut(
                        KeyEquivalent(ExplorerKeyboardShortcut.scanNow),
                        modifiers: [.command]
                    )
                    .accessibilityIdentifier(ExplorerAccessibility.scanNow)
                }

                if presentation.actions.showCancelScan {
                    Button {
                        Task { await model.cancelHomeScan() }
                    } label: {
                        Label("Cancel scan", systemImage: "stop.circle")
                    }
                    .disabled(!presentation.actions.cancelScanEnabled)
                    .help("Request cancellation of the current Home scan")
                    .keyboardShortcut(
                        KeyEquivalent(ExplorerKeyboardShortcut.cancelScan),
                        modifiers: [.command]
                    )
                    .accessibilityIdentifier(ExplorerAccessibility.cancelScan)
                }
            }
        }
        .frame(minWidth: 700, minHeight: 480)
        .accessibilityIdentifier(ExplorerAccessibility.root)
        .task {
            await model.loadInitialState()
        }
    }
}

private struct ExplorerOverviewView: View {
    let presentation: ExplorerPresentation

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 24) {
                VStack(alignment: .leading, spacing: 6) {
                    Text("Storage overview")
                        .font(.largeTitle.bold())
                    Text("Startup-volume capacity and this window’s Home allocation scan.")
                        .foregroundStyle(.secondary)
                }

                capacityCard(presentation.capacity)
                scanCard(
                    coverage: presentation.coverage,
                    scan: presentation.scan
                )
            }
            .padding(28)
            .frame(maxWidth: 860, alignment: .leading)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    @ViewBuilder
    private func capacityCard(_ capacity: ExplorerCapacityPresentation) -> some View {
        GroupBox {
            switch capacity {
            case let .loading(message):
                HStack(spacing: 10) {
                    ProgressView()
                        .controlSize(.small)
                    Text(verbatim: message)
                        .foregroundStyle(.secondary)
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.vertical, 10)

            case let .snapshot(snapshot, status):
                capacitySnapshot(snapshot, status: status)

            case let .failed(title, detail):
                Label {
                    VStack(alignment: .leading, spacing: 4) {
                        Text(verbatim: title)
                            .font(.headline)
                        Text(verbatim: detail)
                            .foregroundStyle(.secondary)
                    }
                } icon: {
                    Image(systemName: "exclamationmark.triangle.fill")
                        .foregroundStyle(.red)
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.vertical, 10)
            }
        } label: {
            Label("Startup disk", systemImage: "internaldrive")
        }
        .accessibilityIdentifier(ExplorerAccessibility.capacityCard)
    }

    private func capacitySnapshot(
        _ snapshot: ExplorerCapacitySnapshotPresentation,
        status: ExplorerCapacityStatus?
    ) -> some View {
        VStack(alignment: .leading, spacing: 18) {
            HStack(alignment: .firstTextBaseline, spacing: 12) {
                Text(verbatim: snapshot.volumeName)
                    .font(.title2.bold())
                    .lineLimit(1)
                    .truncationMode(.middle)
                Spacer()
                DiskPressureBadge(pressure: snapshot.pressure)
                    .accessibilityIdentifier(ExplorerAccessibility.pressure)
            }

            ExplorerSegmentedCapacityBar(
                breakdown: snapshot.breakdown,
                accessibilitySummary: snapshot.accessibilitySummary
            )

            HStack(alignment: .top, spacing: 12) {
                metric(
                    title: "Available",
                    value: snapshot.availableValue,
                    identifier: ExplorerAccessibility.available
                )
                metric(
                    title: "Used",
                    value: snapshot.usedValue,
                    identifier: ExplorerAccessibility.used
                )
                metric(
                    title: "Total",
                    value: snapshot.totalValue,
                    identifier: ExplorerAccessibility.total
                )
            }

            switch snapshot.breakdown {
            case .known:
                HStack(spacing: 18) {
                    ExplorerCapacityLegend(title: "Used", color: .accentColor)
                    ExplorerCapacityLegend(title: "Filesystem available", color: .secondary)
                }
            case let .unavailable(message):
                Label {
                    Text(verbatim: message)
                } icon: {
                    Image(systemName: "questionmark.circle")
                }
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }

            HStack(spacing: 8) {
                Text(verbatim: snapshot.availabilityBasis)
                Text(verbatim: "·")
                Text(verbatim: snapshot.freshness)
                    .accessibilityIdentifier(ExplorerAccessibility.freshness)
            }
            .font(.caption)
            .foregroundStyle(.secondary)

            if let status {
                capacityStatus(status)
            }
        }
        .padding(.vertical, 8)
    }

    private func metric(
        title: LocalizedStringKey,
        value: String,
        identifier: String
    ) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(title)
                .font(.caption)
                .foregroundStyle(.secondary)
            Text(verbatim: value)
                .font(.title3.monospacedDigit().weight(.semibold))
                .textSelection(.enabled)
        }
        .padding(12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(.quaternary.opacity(0.5), in: RoundedRectangle(cornerRadius: 8))
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier(identifier)
    }

    @ViewBuilder
    private func capacityStatus(_ status: ExplorerCapacityStatus) -> some View {
        switch status {
        case let .refreshing(message):
            Label {
                Text(verbatim: message)
            } icon: {
                ProgressView()
                    .controlSize(.small)
            }
            .font(.caption)
            .foregroundStyle(.secondary)
        case let .stale(message):
            Label {
                Text(verbatim: message)
            } icon: {
                Image(systemName: "exclamationmark.triangle")
            }
                .font(.caption)
                .foregroundStyle(.orange)
        }
    }

    private func scanCard(
        coverage: ExplorerCoveragePresentation,
        scan: ExplorerScanPresentation?
    ) -> some View {
        GroupBox {
            VStack(alignment: .leading, spacing: 16) {
                Label {
                    VStack(alignment: .leading, spacing: 4) {
                        Text(verbatim: coverage.title)
                            .font(.headline)
                        Text(verbatim: coverage.detail)
                            .foregroundStyle(.secondary)
                    }
                } icon: {
                    Image(systemName: coverageSymbol(for: coverage.coverage))
                        .foregroundStyle(coverageColor(for: coverage.coverage))
                }
                .accessibilityElement(children: .combine)
                .accessibilityIdentifier(ExplorerAccessibility.coverage)

                if let scan {
                    Divider()
                    HStack(alignment: .top, spacing: 10) {
                        if scan.showsIndeterminateProgress {
                            ProgressView()
                                .controlSize(.small)
                        } else {
                            Image(systemName: scanSymbol(for: scan.style))
                                .foregroundStyle(scanColor(for: scan.style))
                        }
                        VStack(alignment: .leading, spacing: 3) {
                            Text(verbatim: scan.title)
                                .font(.subheadline.weight(.semibold))
                            if let detail = scan.detail {
                                Text(verbatim: detail)
                                    .font(.caption)
                                    .foregroundStyle(.secondary)
                            }
                        }
                    }
                    .accessibilityElement(children: .combine)
                    .accessibilityIdentifier(ExplorerAccessibility.scanStatus)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.vertical, 8)
        } label: {
            Label("Home scan", systemImage: "house.and.flag")
        }
        .accessibilityIdentifier(ExplorerAccessibility.scanCard)
    }

    private func coverageSymbol(for coverage: AppScanCoverage) -> String {
        switch coverage {
        case .complete: "checkmark.shield.fill"
        case .limitedAccess: "lock.trianglebadge.exclamationmark"
        case .partial: "exclamationmark.shield.fill"
        case .unknown: "questionmark.diamond"
        }
    }

    private func coverageColor(for coverage: AppScanCoverage) -> Color {
        switch coverage {
        case .complete: .green
        case .limitedAccess, .partial: .orange
        case .unknown: .secondary
        }
    }

    private func scanSymbol(for style: ExplorerScanStyle) -> String {
        switch style {
        case .progress: "arrow.triangle.2.circlepath"
        case .success: "checkmark.circle.fill"
        case .cancelled: "stop.circle"
        case .failure: "exclamationmark.triangle.fill"
        }
    }

    private func scanColor(for style: ExplorerScanStyle) -> Color {
        switch style {
        case .progress: .secondary
        case .success: .green
        case .cancelled: .secondary
        case .failure: .red
        }
    }
}

private struct ExplorerSegmentedCapacityBar: View {
    let breakdown: ExplorerCapacityBreakdown
    let accessibilitySummary: String

    var body: some View {
        GeometryReader { geometry in
            ZStack(alignment: .leading) {
                RoundedRectangle(cornerRadius: 5)
                    .fill(.quaternary)

                if case let .known(usedFraction) = breakdown {
                    RoundedRectangle(cornerRadius: 5)
                        .fill(.tint)
                        .frame(width: geometry.size.width * usedFraction)
                }
            }
        }
        .frame(height: 12)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("Storage capacity")
        .accessibilityValue(Text(verbatim: accessibilitySummary))
        .accessibilityIdentifier(ExplorerAccessibility.capacityBar)
    }
}

private struct ExplorerCapacityLegend: View {
    let title: LocalizedStringKey
    let color: Color

    var body: some View {
        Label {
            Text(title)
        } icon: {
            Circle()
                .fill(color)
                .frame(width: 8, height: 8)
        }
        .font(.caption)
        .foregroundStyle(.secondary)
    }
}
