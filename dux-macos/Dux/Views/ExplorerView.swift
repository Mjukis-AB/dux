import Foundation
import SwiftUI

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
                ExplorerRecommendationsView(
                    model: model,
                    reviewScan: { scanID in
                        snapshotBrowser.prepareExactCandidateReview(scanID: scanID)
                        selection = .snapshot
                    }
                )
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
    @Environment(\.openSettings) private var openSettings

    let model: AppModel
    let reviewScan: (String) -> Void

    var body: some View {
        let focused = TargetedReclaimScanPresentation.make(
            model.targetedReclaimScanState
        )
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

                GroupBox {
                    VStack(alignment: .leading, spacing: 12) {
                        HStack(alignment: .firstTextBaseline, spacing: 10) {
                            Image(systemName: focused.symbol)
                                .foregroundStyle(color(focused.tone))
                                .accessibilityHidden(true)
                            VStack(alignment: .leading, spacing: 3) {
                                Text(focused.title)
                                    .font(.headline)
                                Text(focused.detail)
                                    .font(.callout)
                                    .foregroundStyle(.secondary)
                            }
                        }
                        .accessibilityElement(children: .combine)
                        .accessibilityIdentifier(
                            ExplorerAccessibility.targetedReclaimScanStatus
                        )

                        if let progress = focused.progressValue,
                           let label = focused.progressLabel
                        {
                            ProgressView(value: progress) {
                                Text(label)
                                    .font(.caption)
                            }
                            .accessibilityValue(label)
                            .accessibilityIdentifier(
                                ExplorerAccessibility.targetedReclaimScanProgress
                            )
                        }

                        if !focused.rows.isEmpty {
                            Divider()
                            VStack(alignment: .leading, spacing: 10) {
                                ForEach(focused.rows) { row in
                                    HStack(alignment: .top, spacing: 10) {
                                        Image(
                                            systemName: row.tone == .success
                                                ? "checkmark.circle.fill"
                                                : "exclamationmark.circle.fill"
                                        )
                                        .foregroundStyle(color(row.tone))
                                        .accessibilityHidden(true)
                                        VStack(alignment: .leading, spacing: 2) {
                                            Text(row.path)
                                                .font(.callout.monospaced())
                                                .lineLimit(2)
                                                .textSelection(.enabled)
                                            Text(row.title)
                                                .font(.caption.weight(.semibold))
                                            Text(row.detail)
                                                .font(.caption)
                                                .foregroundStyle(.secondary)
                                            if let scanID = row.scanID {
                                                Button("Review findings") {
                                                    reviewScan(scanID)
                                                }
                                                .buttonStyle(.link)
                                                .help(
                                                    "Open this exact read-only scan in Explorer"
                                                )
                                                .accessibilityIdentifier(
                                                    ExplorerAccessibility
                                                        .targetedReclaimScanReview(
                                                            ordinal: row.ordinal
                                                        )
                                                )
                                            }
                                        }
                                    }
                                    .accessibilityElement(children: .contain)
                                    .accessibilityIdentifier(
                                        ExplorerAccessibility.targetedReclaimScanRoot(
                                            ordinal: row.ordinal
                                        )
                                    )
                                }
                            }
                        }

                        if focused.canCancel || focused.canRetry
                            || focused.showsConfigureRoots
                        {
                            HStack(spacing: 10) {
                                if focused.canCancel {
                                    Button("Stop focused scan", role: .cancel) {
                                        Task {
                                            await model.cancelTargetedReclaimScan()
                                        }
                                    }
                                    .accessibilityIdentifier(
                                        ExplorerAccessibility.targetedReclaimScanCancel
                                    )
                                }
                                if focused.canRetry,
                                   let snapshot = model.volumeState.snapshot
                                {
                                    Button("Retry focused scan") {
                                        Task {
                                            await model.reconcileTargetedReclaimScan(
                                                for: snapshot
                                            )
                                        }
                                    }
                                    .accessibilityIdentifier(
                                        ExplorerAccessibility.targetedReclaimScanRetry
                                    )
                                }
                                if focused.showsConfigureRoots {
                                    Button("Configure folders…") {
                                        AppActivation.openSettings(using: openSettings)
                                    }
                                }
                            }
                        }
                    }
                } label: {
                    Label("Low-space focused discovery", systemImage: "scope")
                }
                .accessibilityIdentifier(ExplorerAccessibility.targetedReclaimScan)
            }
            .padding(28)
            .frame(maxWidth: 860, alignment: .leading)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    private func color(_ tone: TargetedReclaimScanTone) -> Color {
        switch tone {
        case .neutral: .secondary
        case .active: .blue
        case .success: .green
        case .warning: .orange
        case .failure: .red
        }
    }
}

private struct ExplorerCleanupHistoryView: View {
    let model: AppModel

    var body: some View {
        Group {
            if model.selectedCleanupHistorySessionID == nil {
                historyList
            } else {
                historyDetail
            }
        }
        .accessibilityIdentifier(ExplorerAccessibility.cleanupHistory)
        .task {
            await model.loadCleanupHistory()
        }
    }

    private var historyList: some View {
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
    }

    private var historyDetail: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 18) {
                Button {
                    model.closeCleanupHistorySession()
                } label: {
                    Label("Back to cleanup history", systemImage: "chevron.left")
                }
                .accessibilityIdentifier(ExplorerAccessibility.cleanupHistoryDetailBack)
                .accessibilityHint("Returns to the read-only cleanup session list")

                VStack(alignment: .leading, spacing: 6) {
                    Text("Cleanup session")
                        .font(.largeTitle.bold())
                    Text(
                        "A path-free outcome record. It cannot approve, retry, or repeat cleanup."
                    )
                    .foregroundStyle(.secondary)
                }

                cleanupHistoryDetailContent
            }
            .padding(28)
            .frame(maxWidth: 860, alignment: .leading)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .accessibilityIdentifier(ExplorerAccessibility.cleanupHistoryDetail)
        .onDisappear {
            model.closeCleanupHistorySession()
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
            CleanupHistoryLoadedList(model: model)
        }
    }

    @ViewBuilder
    private var cleanupHistoryDetailContent: some View {
        switch model.cleanupHistoryDetailState {
        case .idle, .loading:
            ProgressView("Loading session details…")
                .accessibilityIdentifier(
                    ExplorerAccessibility.cleanupHistoryDetailStatus
                )
        case let .failed(error):
            GroupBox {
                VStack(alignment: .leading, spacing: 10) {
                    Label("Session details unavailable", systemImage: "exclamationmark.triangle")
                        .font(.headline)
                    Text(cleanupHistoryErrorMessage(error))
                        .foregroundStyle(.secondary)
                    Button("Try reading again") {
                        Task { await model.retryCleanupHistorySession() }
                    }
                    .accessibilityIdentifier(
                        ExplorerAccessibility.cleanupHistoryDetailRetry
                    )
                    .accessibilityHint(
                        "Reads this history record again without retrying cleanup"
                    )
                }
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            .accessibilityIdentifier(
                ExplorerAccessibility.cleanupHistoryDetailStatus
            )
        case let .loaded(detail):
            CleanupHistoryDetailView(detail: detail)
        }
    }

    private func cleanupHistoryErrorMessage(
        _ error: CleanupHistoryServiceError
    ) -> String {
        switch error {
        case .closed: "The storage engine is closed. Reopen Explorer to read history."
        case .invalidSessionID: "The selected session identifier is invalid."
        case .retryable: "The storage engine is busy. Try again shortly."
        case .incompatibleSchema: "This history was written by a newer DUX version."
        case .unsafeStorage, .corruptData, .internalState:
            "History failed its safety checks and was not shown."
        case .budgetExceeded: "History is temporarily too large to read safely."
        case .sessionNotFound: "This cleanup session is no longer available."
        case .invalidLimit, .invalidCursor, .unavailable, .invalidResponse:
            "History returned an invalid or unavailable response."
        }
    }
}

private struct CleanupHistoryLoadedList: View {
    let model: AppModel

    var body: some View {
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
                    CleanupHistorySelectableRow(record: record, model: model)
                }
                if model.cleanupHistoryNextCursor != nil {
                    Button {
                        Task { await model.loadMoreCleanupHistory() }
                    } label: {
                        Label("Load more history", systemImage: "chevron.down")
                            .frame(maxWidth: .infinity)
                    }
                    .disabled(model.cleanupHistoryState == .loading)
                    .accessibilityIdentifier(
                        ExplorerAccessibility.cleanupHistoryLoadMore
                    )
                }
            }
        }
    }
}

private struct CleanupHistorySelectableRow: View {
    let record: CleanupHistorySessionSummaryModel
    let model: AppModel

    var body: some View {
        Button {
            Task {
                await model.selectCleanupHistorySession(record.sessionID)
            }
        } label: {
            CleanupHistoryRow(record: record)
        }
        .buttonStyle(.plain)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(Text(verbatim: accessibilityTitle))
        .accessibilityValue(Text(verbatim: accessibilityValue))
        .accessibilityHint("Opens the path-free session outcome details")
        .accessibilityIdentifier(
            ExplorerAccessibility.cleanupHistoryRow(sessionID: record.sessionID)
        )
    }

    private var accessibilityTitle: String {
        "\(CleanupHistoryPresentation.sessionStatusTitle(record.status)) cleanup session"
    }

    private var accessibilityValue: String {
        "\(CleanupHistoryPresentation.modeTitle(record.mode)), "
            + "\(CleanupHistoryPresentation.triggerTitle(record.trigger)), "
            + "\(record.itemTotal) items, "
            + "\(StorageByteFormatter.string(from: record.estimatedBytes)) estimated"
    }
}

private struct CleanupHistoryRow: View {
    let record: CleanupHistorySessionSummaryModel

    var body: some View {
        GroupBox {
            VStack(alignment: .leading, spacing: 10) {
                HStack(alignment: .firstTextBaseline) {
                    Label(
                        CleanupHistoryPresentation.sessionStatusTitle(record.status),
                        systemImage: CleanupHistoryPresentation.sessionStatusSymbol(
                            record.status
                        )
                    )
                    .font(.headline)
                    Spacer()
                    Text(StorageByteFormatter.string(from: record.estimatedBytes))
                        .monospacedDigit()
                        .foregroundStyle(.secondary)
                }
                HStack(spacing: 8) {
                    Text(CleanupHistoryPresentation.modeTitle(record.mode))
                    Text("·")
                    Text(CleanupHistoryPresentation.triggerTitle(record.trigger))
                    Text("·")
                    Text(record.startedAt.formatted(date: .abbreviated, time: .shortened))
                }
                .font(.caption)
                .foregroundStyle(.secondary)

                CleanupHistoryStatusBar(counts: record.itemStatusCounts)
                    .accessibilityIdentifier(
                        ExplorerAccessibility.cleanupHistoryRowChart(
                            sessionID: record.sessionID
                        )
                    )
                    .accessibilityLabel(Text("Item outcome distribution"))
                    .accessibilityValue(Text(statusSummary))

                HStack {
                    Text("\(record.itemTotal) items · \(record.pathTotal) path records")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                    Spacer()
                    Label("Open details", systemImage: "chevron.right")
                        .font(.caption.weight(.semibold))
                }
            }
            .accessibilityElement(children: .contain)
        }
    }

    private var statusSummary: String {
        CleanupHistoryPresentation.outcomeGroups(
            record.itemStatusCounts
        ).accessibilitySummary
    }
}

private struct CleanupHistoryStatusBar: View {
    let counts: CleanupHistoryStatusCounts

    var body: some View {
        GeometryReader { geometry in
            let groups = CleanupHistoryPresentation.outcomeGroups(counts)
            let total = max(CGFloat(groups.total), 1)
            HStack(spacing: 1) {
                segment(
                    groups.changedOnDisk,
                    color: .green,
                    total: total,
                    width: geometry.size.width
                )
                segment(
                    groups.notChanged,
                    color: .orange,
                    total: total,
                    width: geometry.size.width
                )
                segment(
                    groups.needsAttention,
                    color: .red,
                    total: total,
                    width: geometry.size.width
                )
                segment(
                    groups.unresolved,
                    color: .gray,
                    total: total,
                    width: geometry.size.width
                )
            }
        }
        .frame(height: 8)
        .background(Color.gray.opacity(0.15))
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

private struct CleanupHistoryDetailView: View {
    let detail: CleanupHistorySessionDetailModel

    private var summary: CleanupHistorySessionSummaryModel {
        detail.summary
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            sessionSummary
            spaceAccounting

            LazyVGrid(
                columns: [GridItem(.adaptive(minimum: 300), spacing: 12)],
                alignment: .leading,
                spacing: 12
            ) {
                CleanupHistoryOutcomeChart(
                    title: "Item outcomes",
                    countLabel: "\(summary.itemTotal) items",
                    counts: summary.itemStatusCounts,
                    accessibilityIdentifier: ExplorerAccessibility.cleanupHistoryItemChart
                )
                CleanupHistoryOutcomeChart(
                    title: "Path-record outcomes",
                    countLabel: "\(summary.pathTotal) records",
                    counts: summary.pathStatusCounts,
                    accessibilityIdentifier: ExplorerAccessibility.cleanupHistoryPathChart
                )
            }

            if !detail.warnings.isEmpty {
                warningSection
            }

            VStack(alignment: .leading, spacing: 10) {
                Text("Ordered items")
                    .font(.title2.bold())
                if detail.items.isEmpty {
                    ContentUnavailableView(
                        "No item details recorded",
                        systemImage: "tray",
                        description: Text(
                            "This session contains no path-free item records. Missing legacy data is not reconstructed."
                        )
                    )
                } else {
                    ForEach(detail.items) { item in
                        CleanupHistoryItemCard(item: item)
                    }
                }
            }
        }
        .accessibilityIdentifier(ExplorerAccessibility.cleanupHistoryDetailStatus)
    }

    private var sessionSummary: some View {
        GroupBox {
            VStack(alignment: .leading, spacing: 12) {
                HStack(alignment: .firstTextBaseline) {
                    Label(
                        CleanupHistoryPresentation.sessionStatusTitle(summary.status),
                        systemImage: CleanupHistoryPresentation.sessionStatusSymbol(
                            summary.status
                        )
                    )
                    .font(.title2.bold())
                    Spacer()
                    Text(CleanupHistoryPresentation.modeTitle(summary.mode))
                        .font(.callout.weight(.semibold))
                        .padding(.horizontal, 9)
                        .padding(.vertical, 4)
                        .background(.quaternary, in: Capsule())
                }

                Text(verbatim: summary.sessionID)
                    .font(.caption.monospaced())
                    .foregroundStyle(.secondary)
                    .textSelection(.enabled)

                Grid(alignment: .leading, horizontalSpacing: 18, verticalSpacing: 8) {
                    detailRow("Started", timestamp(summary.startedAt))
                    detailRow(
                        "Completed",
                        summary.completedAt.map(timestamp) ?? "Not completed"
                    )
                    detailRow(
                        "Plan created",
                        summary.planCreatedAt.map(timestamp) ?? "Not recorded"
                    )
                    detailRow(
                        "Plan expired",
                        summary.planExpiresAt.map(timestamp) ?? "Not recorded"
                    )
                    detailRow(
                        "Trigger",
                        CleanupHistoryPresentation.triggerTitle(summary.trigger)
                    )
                    detailRow("Record format", formatTitle)
                    detailRow("Cancellation requested", cancellationTitle)
                }

                if summary.format == .legacyIncomplete {
                    Label(
                        "This migrated record is incomplete. Missing fields remain explicitly unrecorded.",
                        systemImage: "archivebox"
                    )
                    .font(.callout)
                    .foregroundStyle(.secondary)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .accessibilityIdentifier(ExplorerAccessibility.cleanupHistoryDetailSummary)
    }

    private var spaceAccounting: some View {
        let outcome = CleanupHistoryPresentation.capacityOutcome(
            deltaBytes: summary.verifiedCapacityDeltaBytes
        )
        return GroupBox("Space accounting") {
            LazyVGrid(
                columns: [GridItem(.adaptive(minimum: 260), spacing: 12)],
                alignment: .leading,
                spacing: 12
            ) {
                metricCard(
                    title: "Plan estimate",
                    value: StorageByteFormatter.string(from: summary.estimatedBytes),
                    detail: "Estimated before cleanup; not a measured capacity result.",
                    symbol: "ruler",
                    color: .secondary
                )
                metricCard(
                    title: "Verified capacity change",
                    value: outcome.value,
                    detail: outcome.detail,
                    symbol: capacitySymbol(outcome.kind),
                    color: capacityColor(outcome.kind)
                )
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    private var warningSection: some View {
        GroupBox("Important context") {
            VStack(alignment: .leading, spacing: 10) {
                ForEach(Array(detail.warnings.enumerated()), id: \.offset) { _, warning in
                    let presentation = CleanupHistoryPresentation.warning(warning)
                    Label {
                        VStack(alignment: .leading, spacing: 2) {
                            Text(presentation.title)
                                .font(.callout.weight(.semibold))
                            Text(presentation.detail)
                                .font(.caption)
                                .foregroundStyle(.secondary)
                        }
                    } icon: {
                        Image(systemName: presentation.symbol)
                            .foregroundStyle(.orange)
                    }
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .accessibilityIdentifier(ExplorerAccessibility.cleanupHistoryWarnings)
    }

    private func metricCard(
        title: String,
        value: String,
        detail: String,
        symbol: String,
        color: Color
    ) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            Label(title, systemImage: symbol)
                .font(.headline)
                .foregroundStyle(color)
            Text(verbatim: value)
                .font(.title3.bold())
                .monospacedDigit()
            Text(detail)
                .font(.caption)
                .foregroundStyle(.secondary)
        }
        .padding(12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(.quaternary.opacity(0.6), in: RoundedRectangle(cornerRadius: 10))
        .accessibilityElement(children: .combine)
    }

    private func detailRow(_ title: String, _ value: String) -> some View {
        GridRow {
            Text(title)
                .foregroundStyle(.secondary)
            Text(verbatim: value)
                .textSelection(.enabled)
        }
    }

    private func timestamp(_ date: Date) -> String {
        date.formatted(date: .abbreviated, time: .standard)
    }

    private var formatTitle: String {
        switch summary.format {
        case .complete: "Complete"
        case .legacyIncomplete: "Legacy · incomplete"
        }
    }

    private var cancellationTitle: String {
        switch summary.cancellationRequested {
        case true: "Yes"
        case false: "No"
        case nil: "Not recorded"
        }
    }

    private func capacitySymbol(
        _ kind: CleanupHistoryCapacityOutcomeKind
    ) -> String {
        switch kind {
        case .unknown: "questionmark.circle"
        case .increased: "arrow.up.circle.fill"
        case .unchanged: "equal.circle"
        case .decreased: "arrow.down.circle.fill"
        }
    }

    private func capacityColor(
        _ kind: CleanupHistoryCapacityOutcomeKind
    ) -> Color {
        switch kind {
        case .unknown, .unchanged: .secondary
        case .increased: .green
        case .decreased: .orange
        }
    }
}

private struct CleanupHistoryOutcomeChart: View {
    let title: String
    let countLabel: String
    let counts: CleanupHistoryStatusCounts
    let accessibilityIdentifier: String

    private var groups: CleanupHistoryOutcomeGroups {
        CleanupHistoryPresentation.outcomeGroups(counts)
    }

    var body: some View {
        GroupBox {
            VStack(alignment: .leading, spacing: 10) {
                HStack {
                    Text(title)
                        .font(.headline)
                    Spacer()
                    Text(countLabel)
                        .font(.caption.monospacedDigit())
                        .foregroundStyle(.secondary)
                }
                CleanupHistoryStatusBar(counts: counts)
                LazyVGrid(
                    columns: [GridItem(.adaptive(minimum: 120), alignment: .leading)],
                    alignment: .leading,
                    spacing: 6
                ) {
                    legend("Changed on disk", count: groups.changedOnDisk, color: .green)
                    legend("Not changed", count: groups.notChanged, color: .orange)
                    legend("Needs attention", count: groups.needsAttention, color: .red)
                    legend("Unresolved", count: groups.unresolved, color: .gray)
                }
            }
        }
        .accessibilityElement(children: .combine)
        .accessibilityLabel(Text(verbatim: title))
        .accessibilityValue(Text(verbatim: groups.accessibilitySummary))
        .accessibilityIdentifier(accessibilityIdentifier)
    }

    private func legend(
        _ title: String,
        count: UInt16,
        color: Color
    ) -> some View {
        HStack(spacing: 6) {
            Circle()
                .fill(color)
                .frame(width: 8, height: 8)
                .accessibilityHidden(true)
            Text("\(title) \(count)")
                .font(.caption)
                .monospacedDigit()
        }
    }
}

private struct CleanupHistoryItemCard: View {
    let item: CleanupHistoryItemModel

    var body: some View {
        GroupBox {
            VStack(alignment: .leading, spacing: 10) {
                HStack(alignment: .firstTextBaseline) {
                    VStack(alignment: .leading, spacing: 2) {
                        Text("Item \(Int(item.ordinal) + 1)")
                            .font(.caption.weight(.semibold))
                            .foregroundStyle(.secondary)
                        Text(verbatim: item.ruleID)
                            .font(.headline)
                            .textSelection(.enabled)
                    }
                    Spacer()
                    Label(
                        CleanupHistoryPresentation.itemStatusTitle(item.status),
                        systemImage: CleanupHistoryPresentation.itemStatusSymbol(
                            item.status
                        )
                    )
                    .font(.callout.weight(.semibold))
                }

                Grid(alignment: .leading, horizontalSpacing: 18, verticalSpacing: 7) {
                    detailRow("Rule revision", "\(item.ruleRevision)")
                    detailRow("Category", item.category?.displayName ?? "Not recorded")
                    detailRow("Safety", item.safety?.displayName ?? "Not recorded")
                    detailRow("Action", item.action?.displayName ?? "Not recorded")
                    detailRow(
                        "Schedule eligible",
                        item.ruleScheduleEligible.map { $0 ? "Yes" : "No" }
                            ?? "Not recorded"
                    )
                    detailRow(
                        "Newest observation",
                        item.newestModificationAt.map(timestamp) ?? "Not recorded"
                    )
                    detailRow(
                        "Estimated size",
                        StorageByteFormatter.string(from: item.estimatedBytes)
                    )
                    detailRow("Path records", "\(item.pathCount)")
                    detailRow("Evidence records", "\(item.evidenceCount)")
                    if item.errorRecorded {
                        detailRow(
                            "Error category",
                            item.errorCategory ?? "Recorded without a category"
                        )
                    }
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .accessibilityElement(children: .contain)
        .accessibilityLabel(
            Text("Item \(Int(item.ordinal) + 1), rule \(item.ruleID)")
        )
        .accessibilityValue(
            Text(
                "\(CleanupHistoryPresentation.itemStatusTitle(item.status)), "
                    + "\(StorageByteFormatter.string(from: item.estimatedBytes)) estimated, "
                    + "\(item.pathCount) path records, \(item.evidenceCount) evidence records"
            )
        )
        .accessibilityIdentifier(
            ExplorerAccessibility.cleanupHistoryItem(ordinal: item.ordinal)
        )
    }

    private func detailRow(_ title: String, _ value: String) -> some View {
        GridRow {
            Text(title)
                .foregroundStyle(.secondary)
            Text(verbatim: value)
                .textSelection(.enabled)
        }
    }

    private func timestamp(_ date: Date) -> String {
        date.formatted(date: .abbreviated, time: .standard)
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
                capacityHistoryCard(
                    trend: model.capacityTrend,
                    pressureState: model.pressureHistoryState
                )
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

    private func capacityHistoryCard(
        trend: VolumeCapacityTrend?,
        pressureState: VolumePressureHistoryState
    ) -> some View {
        GroupBox {
            VStack(alignment: .leading, spacing: 16) {
                if let trend {
                    HStack(spacing: 12) {
                        historyMetric(title: "24-hour change", change: trend.change24h)
                        historyMetric(title: "7-day change", change: trend.change7d)
                    }

                    if trend.points.isEmpty {
                        Label(
                            "Recorded samples are still warming up.",
                            systemImage: "chart.line.uptrend.xyaxis"
                        )
                        .foregroundStyle(.secondary)
                    } else {
                        ExplorerCapacityHistoryChart(
                            points: trend.points,
                            pressureHistory: pressureState.history,
                            anchorAt: trend.sampledAt
                        )
                        .frame(height: 156)
                        .accessibilityIdentifier(ExplorerAccessibility.capacityHistoryChart)
                        .accessibilityLabel("30-day available-space history")
                        .accessibilityValue(
                            capacityHistoryAccessibilitySummary(
                                trend: trend,
                                pressureState: pressureState
                            )
                        )
                    }
                } else {
                    Label(
                        "Not enough recorded samples for 24-hour and 7-day changes yet.",
                        systemImage: "clock.arrow.circlepath"
                    )
                    .foregroundStyle(.secondary)
                    .accessibilityIdentifier(ExplorerAccessibility.capacityHistoryStatus)
                }

                Divider()
                pressureHistorySection(pressureState)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.vertical, 8)
        } label: {
            Label("Capacity history", systemImage: "chart.xyaxis.line")
        }
        .accessibilityIdentifier(ExplorerAccessibility.capacityHistoryCard)
    }

    private func historyMetric(
        title: LocalizedStringKey,
        change: VolumeCapacityTrendChange?
    ) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(title)
                .font(.caption)
                .foregroundStyle(.secondary)
            Text(verbatim: change.map { signedCapacity($0.availableBytes) } ?? "Not enough history")
                .font(.headline.monospacedDigit())
        }
        .padding(10)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(.quaternary.opacity(0.45), in: RoundedRectangle(cornerRadius: 8))
        .accessibilityElement(children: .combine)
    }

    @ViewBuilder
    private func pressureHistorySection(_ state: VolumePressureHistoryState) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("Low-space periods")
                .font(.headline)
            Text("Stored Warning and Critical classifications from recorded disk samples.")
                .font(.caption)
                .foregroundStyle(.secondary)

            switch state {
            case .idle:
                Label(
                    "Low-space history becomes available after the startup disk is identified.",
                    systemImage: "externaldrive"
                )
                .foregroundStyle(.secondary)
                .accessibilityIdentifier(ExplorerAccessibility.capacityHistoryStatus)

            case .loading:
                HStack(spacing: 8) {
                    ProgressView()
                        .controlSize(.small)
                    Text("Loading recorded low-space periods…")
                        .foregroundStyle(.secondary)
                }
                .accessibilityIdentifier(ExplorerAccessibility.capacityHistoryStatus)

            case let .failed(failure):
                pressureHistoryFailure(failure, stale: false)

            case let .stale(history, failure):
                pressureHistoryFailure(failure, stale: true)
                pressureEpisodeList(history)

            case let .loaded(history):
                pressureEpisodeList(history)
            }
        }
    }

    private func pressureHistoryFailure(
        _: VolumePressureHistoryFailure,
        stale: Bool
    ) -> some View {
        Label(
            stale
                ? "Showing the last confirmed periods; the latest history could not be loaded."
                : "Recorded low-space periods are temporarily unavailable.",
            systemImage: "exclamationmark.triangle"
        )
        .font(.caption)
        .foregroundStyle(.orange)
        .accessibilityIdentifier(ExplorerAccessibility.capacityHistoryStatus)
    }

    @ViewBuilder
    private func pressureEpisodeList(_ history: VolumePressureHistory) -> some View {
        if history.episodes.isEmpty {
            Text("No low-space periods have been recorded for this startup disk yet.")
                .foregroundStyle(.secondary)
                .accessibilityIdentifier(ExplorerAccessibility.pressureEpisodeList)
        } else {
            VStack(alignment: .leading, spacing: 8) {
                ForEach(history.episodes.prefix(5)) { episode in
                    HStack(alignment: .top, spacing: 10) {
                        Image(
                            systemName: episode.level == .critical
                                ? "exclamationmark.octagon.fill"
                                : "exclamationmark.triangle.fill"
                        )
                        .foregroundStyle(
                            episode.level == .critical ? Color.red : Color.orange
                        )
                        VStack(alignment: .leading, spacing: 2) {
                            Text(episode.level == .critical ? "Critical" : "Warning")
                                .font(.subheadline.weight(.semibold))
                            Text(
                                episodeDescription(
                                    episode,
                                    anchorAt: history.anchorAt
                                )
                            )
                            .font(.caption)
                            .foregroundStyle(.secondary)
                        }
                    }
                    .accessibilityElement(children: .combine)
                    .accessibilityIdentifier(
                        "\(ExplorerAccessibility.pressureEpisodeList)-\(episode.id)"
                    )
                }
                if history.episodes.count > 5 {
                    Text("\(history.episodes.count - 5) more recorded periods")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
                if history.hasMore {
                    Text("Showing the 64 most recent recorded periods.")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }
            .accessibilityIdentifier(ExplorerAccessibility.pressureEpisodeList)
        }
    }

    private func episodeDescription(
        _ episode: VolumePressureEpisode,
        anchorAt: Date
    ) -> String {
        let end = episode.exitedAt ?? anchorAt
        let range = if let exitedAt = episode.exitedAt {
            "\(episode.enteredAt.formatted(date: .abbreviated, time: .shortened)) – \(exitedAt.formatted(date: .abbreviated, time: .shortened))"
        } else {
            "\(episode.enteredAt.formatted(date: .abbreviated, time: .shortened)) · ongoing at latest sample"
        }
        return "\(range) · \(durationText(end.timeIntervalSince(episode.enteredAt)))"
    }

    private func durationText(_ interval: TimeInterval) -> String {
        let seconds = max(0, Int(interval.rounded(.down)))
        let days = seconds / 86_400
        let hours = (seconds % 86_400) / 3_600
        let minutes = (seconds % 3_600) / 60
        if days > 0 { return "\(days)d \(hours)h" }
        if hours > 0 { return "\(hours)h \(minutes)m" }
        return "\(minutes)m"
    }

    private func signedCapacity(_ value: Int64) -> String {
        let magnitude = MenuBarCapacityFormatter.gib(value.magnitude, locale: .current)
        if value > 0 { return "+\(magnitude)" }
        if value < 0 { return "-\(magnitude)" }
        return magnitude
    }

    private func capacityHistoryAccessibilitySummary(
        trend: VolumeCapacityTrend,
        pressureState: VolumePressureHistoryState
    ) -> String {
        let first = trend.points.first.map { MenuBarCapacityFormatter.gib($0.availableBytes) }
        let last = trend.points.last.map { MenuBarCapacityFormatter.gib($0.availableBytes) }
        let pointSummary = if let first, let last {
            "\(trend.points.count) recorded samples, from \(first) to \(last) available"
        } else {
            "No recorded chart samples"
        }
        let episodeSummary = pressureState.history.map {
            "\($0.episodes.count) recorded low-space periods"
                + ($0.hasMore ? ", additional older periods exist" : "")
        } ?? "Low-space period history unavailable"
        return "\(pointSummary). \(episodeSummary)."
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

private struct ExplorerCapacityHistoryChart: View {
    let points: [VolumeCapacityTrendPoint]
    let pressureHistory: VolumePressureHistory?
    let anchorAt: Date

    private var windowStart: Date {
        anchorAt.addingTimeInterval(-30 * 24 * 60 * 60)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 5) {
            Canvas { context, size in
                drawGrid(context: &context, size: size)
                drawAvailableSpace(context: &context, size: size)
                drawPressurePeriods(context: &context, size: size)
            }
            .accessibilityIdentifier(ExplorerAccessibility.pressureEpisodeTimeline)

            HStack {
                Text(windowStart, format: .dateTime.month(.abbreviated).day())
                Spacer()
                Label("Recorded available space", systemImage: "circle.fill")
                Spacer()
                Text(anchorAt, format: .dateTime.month(.abbreviated).day())
            }
            .font(.caption2)
            .foregroundStyle(.secondary)
        }
    }

    private func drawGrid(context: inout GraphicsContext, size: CGSize) {
        for index in 0 ... 3 {
            let y = CGFloat(index) * (size.height - 18) / 3
            var line = Path()
            line.move(to: CGPoint(x: 0, y: y))
            line.addLine(to: CGPoint(x: size.width, y: y))
            context.stroke(line, with: .color(.secondary.opacity(0.16)), lineWidth: 1)
        }
    }

    private func drawAvailableSpace(context: inout GraphicsContext, size: CGSize) {
        let visible = points.filter {
            $0.sampledAt >= windowStart && $0.sampledAt <= anchorAt
        }
        guard !visible.isEmpty else {
            return
        }
        let values = visible.map { Double($0.availableBytes) }
        guard let minimum = values.min(), let maximum = values.max() else {
            return
        }
        let range = max(1, maximum - minimum)
        var line = Path()
        for (index, point) in visible.enumerated() {
            let coordinate = CGPoint(
                x: x(for: point.sampledAt, width: size.width),
                y: 6 + (1 - CGFloat((Double(point.availableBytes) - minimum) / range))
                    * (size.height - 30)
            )
            if index == 0 {
                line.move(to: coordinate)
            } else {
                line.addLine(to: coordinate)
            }
            let dot = Path(
                ellipseIn: CGRect(
                    x: coordinate.x - 2.5,
                    y: coordinate.y - 2.5,
                    width: 5,
                    height: 5
                )
            )
            context.fill(dot, with: .color(color(for: point.pressure)))
        }
        context.stroke(
            line,
            with: .color(.accentColor),
            style: StrokeStyle(lineWidth: 2, lineCap: .round, lineJoin: .round)
        )
    }

    private func drawPressurePeriods(context: inout GraphicsContext, size: CGSize) {
        guard let pressureHistory else {
            return
        }
        for episode in pressureHistory.episodes {
            let start = max(episode.enteredAt, windowStart)
            let end = min(episode.exitedAt ?? pressureHistory.anchorAt, anchorAt)
            guard end >= start else {
                continue
            }
            let startX = x(for: start, width: size.width)
            let endX = x(for: end, width: size.width)
            let rectangle = Path(
                roundedRect: CGRect(
                    x: startX,
                    y: size.height - 10,
                    width: max(2, endX - startX),
                    height: 8
                ),
                cornerRadius: 2
            )
            context.fill(
                rectangle,
                with: .color(episode.level == .critical ? .red : .orange)
            )
        }
    }

    private func x(for date: Date, width: CGFloat) -> CGFloat {
        let duration = anchorAt.timeIntervalSince(windowStart)
        guard duration > 0 else {
            return width
        }
        let fraction = date.timeIntervalSince(windowStart) / duration
        return width * CGFloat(min(1, max(0, fraction)))
    }

    private func color(for pressure: DiskPressureLevel) -> Color {
        switch pressure {
        case .healthy: .green
        case .warning: .orange
        case .critical: .red
        case .unknown: .secondary
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
