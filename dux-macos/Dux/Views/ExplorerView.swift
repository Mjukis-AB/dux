import SwiftUI
import Foundation

struct ExplorerView: View {
    @Environment(\.openSettings) private var openSettings
    @State private var selection = ExplorerDestination.overview

    let model: AppModel
    let snapshotBrowser: ExplorerSnapshotBrowserModel

    var body: some View {
        let presentation = ExplorerPresentation.make(
            volumeState: model.volumeState,
            scanState: model.scanState
        )
        let storageAccess = StorageAccessOnboardingPresentation.make(
            scanState: model.scanState,
            broaderAnalysisRequested: model.broaderStorageAnalysisRequested,
            probeState: model.storageAccessProbeState
        )

        NavigationSplitView {
            List(selection: $selection) {
                NavigationLink(value: ExplorerDestination.overview) {
                    Label("Overview", systemImage: "chart.pie")
                }
                .accessibilityIdentifier(ExplorerAccessibility.overviewDestination)

                NavigationLink(value: ExplorerDestination.snapshot) {
                    Label("Explore Snapshot", systemImage: "internaldrive")
                }
                .accessibilityIdentifier(ExplorerAccessibility.snapshotDestination)

                NavigationLink(value: ExplorerDestination.recommendations) {
                    Label("Recommendations", systemImage: "sparkles.rectangle.stack")
                }
                .accessibilityIdentifier(ExplorerAccessibility.recommendationsDestination)

                NavigationLink(value: ExplorerDestination.cleanupHistory) {
                    Label("Cleanup history", systemImage: "clock.arrow.circlepath")
                }
                .accessibilityIdentifier(ExplorerAccessibility.cleanupHistoryDestination)

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
            switch selection {
            case .overview:
                ExplorerOverviewView(
                    presentation: presentation,
                    storageAccess: storageAccess,
                    model: model
                )
                .navigationTitle("Overview")
            case .snapshot:
                ExplorerSnapshotBrowserView(
                    browser: snapshotBrowser,
                    model: model
                )
                .navigationTitle("Explore Snapshot")
            case .recommendations:
                ExplorerRecommendationsView(model: model)
                    .navigationTitle("Recommendations")
            case .cleanupHistory:
                ExplorerCleanupHistoryView(model: model)
                    .navigationTitle("Cleanup history")
            }
        }
        .navigationSplitViewStyle(.balanced)
        .toolbar {
            ToolbarItem(placement: .navigation) {
                if selection == .snapshot {
                    Label("Snapshot", systemImage: "internaldrive")
                        .help("The selected read-only storage snapshot")
                } else {
                    Label("Home", systemImage: "house")
                        .help("The default read-only scan root")
                }
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

                if presentation.actions.showScanNow, selection == .overview {
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
                    .help("Request cancellation of the current storage scan")
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
            if let pending = model.pendingExplorerDestination {
                selection = pending
            }
        }
        .onChange(of: model.pendingExplorerDestination) { _, destination in
            if let destination {
                selection = destination
            }
        }
    }
}

private struct ExplorerRecommendationsView: View {
    let model: AppModel

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 18) {
                Label("Safe ways to reclaim space", systemImage: "checkmark.shield")
                    .font(.largeTitle.bold())
                Text("DUX can explain storage and group reviewable cleanup ideas. It never lets an AI model delete files, and this view does not perform cleanup by itself.")
                    .foregroundStyle(.secondary)

                GroupBox {
                    VStack(alignment: .leading, spacing: 10) {
                        Label("Review before changing anything", systemImage: "hand.raised")
                            .font(.headline)
                        Text("Recommendations will appear after a completed, read-only scan. Verify each item in Finder or the Explorer before taking action.")
                            .foregroundStyle(.secondary)
                        if model.volumeState.snapshot == nil {
                            Text("Capacity is not available yet. Refresh the overview to continue.")
                                .font(.callout)
                                .foregroundStyle(.orange)
                        }
                    }
                    .accessibilityElement(children: .combine)
                }
                .accessibilityIdentifier(ExplorerAccessibility.recommendations)
            }
            .padding(28)
            .frame(maxWidth: 860, alignment: .leading)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }
}

private struct ExplorerCleanupHistoryView: View {
    let model: AppModel

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 18) {
                VStack(alignment: .leading, spacing: 6) {
                    Text("Cleanup history")
                        .font(.largeTitle.bold())
                    Text(
                        "A read-only record of reviewed cleanup outcomes. DUX never treats history as permission to repeat an action."
                    )
                    .foregroundStyle(.secondary)
                }

                historyContent
            }
            .padding(28)
            .frame(maxWidth: 860, alignment: .leading)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .accessibilityIdentifier(ExplorerAccessibility.cleanupHistory)
        .task {
            await model.loadCleanupHistory()
        }
    }

    @ViewBuilder
    private var historyContent: some View {
        switch model.cleanupHistoryState {
        case .idle:
            ProgressView("Loading cleanup history…")
                .accessibilityIdentifier(ExplorerAccessibility.cleanupHistoryStatus)
        case .loading where model.cleanupHistoryRecords.isEmpty:
            ProgressView("Loading cleanup history…")
                .accessibilityIdentifier(ExplorerAccessibility.cleanupHistoryStatus)
        case let .failed(error):
            GroupBox {
                VStack(alignment: .leading, spacing: 10) {
                    Label("History unavailable", systemImage: "exclamationmark.triangle")
                        .font(.headline)
                    Text(cleanupHistoryErrorMessage(error))
                        .foregroundStyle(.secondary)
                    Button("Try again") {
                        Task { await model.loadCleanupHistory() }
                    }
                }
                .accessibilityElement(children: .combine)
            }
            .accessibilityIdentifier(ExplorerAccessibility.cleanupHistoryStatus)
        case .loaded, .loading:
            if model.cleanupHistoryRecords.isEmpty {
                ContentUnavailableView(
                    "No cleanup sessions yet",
                    systemImage: "clock.arrow.circlepath",
                    description: Text(
                        "Reviewed cleanup outcomes will appear here. Scanning and explanations never change files."
                    )
                )
                .accessibilityIdentifier(ExplorerAccessibility.cleanupHistoryStatus)
            } else {
                VStack(alignment: .leading, spacing: 12) {
                    ForEach(model.cleanupHistoryRecords) { record in
                        CleanupHistoryRow(record: record)
                    }
                    if model.cleanupHistoryNextCursor != nil {
                        Button {
                            Task { await model.loadMoreCleanupHistory() }
                        } label: {
                            Label("Load more history", systemImage: "chevron.down")
                                .frame(maxWidth: .infinity)
                        }
                        .disabled(model.cleanupHistoryState == .loading)
                        .accessibilityIdentifier(ExplorerAccessibility.cleanupHistoryLoadMore)
                    }
                }
            }
        }
    }

    private func cleanupHistoryErrorMessage(
        _ error: CleanupHistoryServiceError
    ) -> String {
        switch error {
        case .closed: "The storage engine is closed. Reopen Explorer to read history."
        case .retryable: "The storage engine is busy. Try again shortly."
        case .incompatibleSchema: "This history was written by a newer DUX version."
        case .unsafeStorage, .corruptData, .internalState:
            "History failed its safety checks and was not shown."
        case .budgetExceeded: "History is temporarily too large to read safely."
        case .invalidLimit, .invalidCursor, .sessionNotFound, .unavailable, .invalidResponse:
            "History returned an invalid or unavailable response."
        }
    }
}

private struct CleanupHistoryRow: View {
    let record: CleanupHistorySessionSummaryModel

    var body: some View {
        GroupBox {
            VStack(alignment: .leading, spacing: 10) {
                HStack(alignment: .firstTextBaseline) {
                    Label(statusTitle, systemImage: statusSymbol)
                        .font(.headline)
                    Spacer()
                    Text(ByteCountFormatter.string(fromByteCount: Int64(clamping: record.estimatedBytes), countStyle: .file))
                        .monospacedDigit()
                        .foregroundStyle(.secondary)
                }
                HStack(spacing: 8) {
                    Text(modeTitle)
                    Text("·")
                    Text(triggerTitle)
                    Text("·")
                    Text(record.startedAt.formatted(date: .abbreviated, time: .shortened))
                }
                .font(.caption)
                .foregroundStyle(.secondary)

                CleanupHistoryStatusBar(counts: record.itemStatusCounts)
                    .accessibilityIdentifier(ExplorerAccessibility.cleanupHistoryChart)
                    .accessibilityLabel(Text("Item outcome distribution"))
                    .accessibilityValue(Text(statusSummary))
            }
            .accessibilityElement(children: .contain)
        }
        .accessibilityIdentifier("explorer-cleanup-history-row-\(record.sessionID)")
    }

    private var statusTitle: String {
        switch record.status {
        case .planned: "Planned"
        case .running: "Running"
        case .recovering: "Recovering"
        case .completed: "Completed"
        case .partiallyCompleted: "Partially completed"
        case .failed: "Failed"
        case .cancelled: "Cancelled"
        case .interrupted: "Interrupted"
        case .rejected: "Rejected"
        case .dryRun: "Dry run"
        }
    }

    private var statusSymbol: String {
        switch record.status {
        case .completed: "checkmark.circle.fill"
        case .partiallyCompleted, .failed, .cancelled, .interrupted: "exclamationmark.circle.fill"
        case .running, .recovering: "arrow.triangle.2.circlepath"
        default: "clock"
        }
    }

    private var modeTitle: String {
        switch record.mode {
        case .dryRun: "Dry run"
        case .trash: "Trash"
        case .permanentSafe: "Permanent-safe"
        case .evictLocalCopy: "Evict local copy"
        }
    }

    private var triggerTitle: String {
        switch record.trigger {
        case .manual: "Manual"
        case .lowDisk: "Low disk"
        case .scheduled: "Scheduled"
        case .cli: "CLI"
        }
    }

    private var statusSummary: String {
        let counts = record.itemStatusCounts
        return "\(counts.total) items; \(counts.removed + counts.trashed + counts.evicted) settled; \(counts.failed + counts.outcomeUnknown) uncertain or failed"
    }
}

private struct CleanupHistoryStatusBar: View {
    let counts: CleanupHistoryStatusCounts

    var body: some View {
        GeometryReader { geometry in
            let total = max(CGFloat(counts.total), 1)
            HStack(spacing: 1) {
                segment(counts.removed + counts.trashed + counts.evicted, color: .green, total: total, width: geometry.size.width)
                segment(counts.skipped + counts.rejected, color: .orange, total: total, width: geometry.size.width)
                segment(counts.failed + counts.outcomeUnknown + counts.changedSincePlan, color: .red, total: total, width: geometry.size.width)
                segment(counts.planned + counts.validating + counts.dryRun + counts.effectStarted + counts.interrupted + counts.unavailable, color: .gray, total: total, width: geometry.size.width)
            }
        }
        .frame(height: 8)
        .clipShape(Capsule())
    }

    @ViewBuilder
    private func segment(
        _ count: UInt16,
        color: Color,
        total: CGFloat,
        width: CGFloat
    ) -> some View {
        if count > 0 {
            Rectangle()
                .fill(color)
                .frame(width: max(2, width * CGFloat(count) / total))
        }
    }
}

private struct ExplorerOverviewView: View {
    let presentation: ExplorerPresentation
    let storageAccess: StorageAccessOnboardingPresentation
    let model: AppModel

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
                storageAccessCard(storageAccess)
            }
            .padding(28)
            .frame(maxWidth: 860, alignment: .leading)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    @ViewBuilder
    private func storageAccessCard(
        _ access: StorageAccessOnboardingPresentation
    ) -> some View {
        if access.showsBroaderAnalysisAction || access.showsGuidance {
            GroupBox {
                VStack(alignment: .leading, spacing: 12) {
                    if access.showsBroaderAnalysisAction {
                        Text(
                            String(
                                localized:
                                    "Your Home scan found incomplete or uncertain coverage. DUX remains useful with the files it could read."
                            )
                        )
                        .foregroundStyle(.secondary)

                        Button("Understand broader access…") {
                            Task { await model.requestBroaderStorageAnalysis() }
                        }
                        .accessibilityIdentifier(
                            StorageAccessAccessibility.broaderAnalysis
                        )
                        .accessibilityHint(
                            "Checks bounded observed access before offering optional guidance"
                        )
                    }

                    if access.showsGuidance {
                        HStack(alignment: .top, spacing: 10) {
                            if access.showsProgress {
                                ProgressView()
                                    .controlSize(.small)
                            } else {
                                Image(systemName: "exclamationmark.shield.fill")
                                    .foregroundStyle(.orange)
                            }
                            VStack(alignment: .leading, spacing: 4) {
                                if let title = access.statusTitle {
                                    Text(verbatim: title)
                                        .font(.headline)
                                }
                                if let detail = access.statusDetail {
                                    Text(verbatim: detail)
                                        .foregroundStyle(.secondary)
                                }
                            }
                        }
                        .accessibilityElement(children: .combine)
                        .accessibilityIdentifier(StorageAccessAccessibility.probeStatus)

                        Text(
                            String(
                                localized:
                                    "Full Disk Access is optional. To try broader coverage, open System Settings, choose Privacy & Security, then Full Disk Access. Return to DUX to recheck observed access. Run a new Home scan yourself to update its coverage result."
                            )
                        )
                        .font(.caption)
                        .foregroundStyle(.secondary)

                        HStack {
                            if access.showsSystemSettingsAction {
                                Button("Open System Settings…") {
                                    model.armStorageAccessSettingsReturnProbe()
                                    AppActivation.openStorageAccessSettings()
                                }
                                .accessibilityIdentifier(
                                    StorageAccessAccessibility.openSystemSettings
                                )
                                .accessibilityHint(
                                    "Opens System Settings; choose Privacy and Security, then Full Disk Access"
                                )
                            }

                            if access.showsRefreshAction {
                                Button("Check observed access again") {
                                    Task { await model.refreshStorageAccessEvidence() }
                                }
                                .accessibilityIdentifier(StorageAccessAccessibility.refresh)
                                .accessibilityHint(
                                    "Rechecks three fixed Library locations without scanning files"
                                )
                            }
                        }
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.vertical, 8)
            } label: {
                Label("Storage access", systemImage: "lock.shield")
            }
            .accessibilityIdentifier(StorageAccessAccessibility.guidance)
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
