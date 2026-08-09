import Foundation
import SwiftUI

struct ExplorerSnapshotBrowserView: View {
    @Bindable var browser: ExplorerSnapshotBrowserModel
    @State private var presentationID = UUID()
    @State private var inspectorPresented = true
    @State private var trashConfirmation: ExplorerTrashConfirmation?
    @State private var rustTargetCleanupConfirmation:
        ExplorerRustTargetCleanupConfirmation?
    let model: AppModel
    let supplementalPresentation: any ExplorerSnapshotSupplementalPresenting
    let openSettingsDestination: @MainActor @Sendable () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            header
            subtreeScanStatus
            rustTargetDryRunBanner
            content
        }
        .padding(20)
        .task {
            await browser.present(id: presentationID)
        }
        .onDisappear {
            let presentationID = presentationID
            Task { await browser.dismiss(id: presentationID) }
        }
        .onChange(of: permanentCleanupAvailability) { _, availability in
            if availability != .enabled {
                rustTargetCleanupConfirmation = nil
            }
        }
        .confirmationDialog(
            "Move item to Trash?",
            isPresented: Binding(
                get: { trashConfirmation != nil },
                set: { if !$0 { trashConfirmation = nil } }
            ),
            presenting: trashConfirmation
        ) { confirmation in
            Button("Move \(confirmation.displayName) to Trash", role: .destructive) {
                trashConfirmation = nil
                Task { await browser.executeConfirmedTrash(confirmation) }
            }
            Button("Cancel", role: .cancel) { trashConfirmation = nil }
        } message: { _ in
            Text("DUX will revalidate this reviewed item and record a one-shot operation. Empty Trash separately to reclaim disk space.")
        }
        .background {
            supplementalPresentation.modalPresenter()
                .frame(width: 0, height: 0)
        }
#if DUX_INTERNAL_PERMANENT_SAFE_CLEANUP
        .confirmationDialog(
            "Permanently remove this build output?",
            isPresented: Binding(
                get: { rustTargetCleanupConfirmation != nil },
                set: { if !$0 { rustTargetCleanupConfirmation = nil } }
            ),
            presenting: rustTargetCleanupConfirmation
        ) { confirmation in
            Button("Remove build output permanently", role: .destructive) {
                guard permanentCleanupAvailability == .enabled else {
                    rustTargetCleanupConfirmation = nil
                    return
                }
                rustTargetCleanupConfirmation = nil
                Task {
                    await browser.startConfirmedRustTargetCleanup(confirmation)
                }
            }
            .accessibilityIdentifier(
                ExplorerAccessibility.snapshotCandidateCleanupConfirm
            )
            Button("Cancel", role: .cancel) {
                rustTargetCleanupConfirmation = nil
            }
        } message: { confirmation in
            Text(
                "DUX will consume this exact short-lived review and permanently remove only the validated regenerable contents inside \(confirmation.info.target.display). The newest observed change was \(confirmationTimestamp(confirmation.info.newestMtime)); the rule requires \(confirmationMinimumAge(seconds: confirmation.info.minimumAgeSeconds, nanoseconds: confirmation.info.minimumAgeNanoseconds)) of inactivity, which DUX revalidates before cleanup. The target folder and CACHEDIR.TAG marker remain. Estimated reclaimable space is \(StorageByteFormatter.string(from: confirmation.info.estimatedBytes)); the estimate is not guaranteed. This cannot be undone."
            )
            .accessibilityIdentifier(
                ExplorerAccessibility.snapshotCandidateCleanupConfirmation
            )
        }
#endif
        .accessibilityIdentifier(ExplorerAccessibility.snapshotBrowser)
    }

    private var permanentCleanupAvailability: PermanentCleanupActionAvailability {
        .make(
            policy: model.permanentCleanupPolicy,
            state: model.permanentCleanupPolicyState
        )
    }

    private func confirmationTimestamp(_ value: ExplorerSnapshotTimestamp) -> String {
        let interval = Double(value.secondsSinceUnixEpoch)
            + Double(value.nanoseconds) / 1_000_000_000
        return Date(timeIntervalSince1970: interval).formatted(
            date: .abbreviated,
            time: .shortened
        )
    }

    private func confirmationMinimumAge(seconds: UInt64, nanoseconds: UInt32) -> String {
        let secondsPerDay: UInt64 = 24 * 60 * 60
        if nanoseconds == 0, seconds.isMultiple(of: secondsPerDay) {
            let days = seconds / secondsPerDay
            return "\(days) \(days == 1 ? "day" : "days")"
        }
        guard nanoseconds > 0 else {
            return "\(seconds) seconds"
        }
        var fraction = String(format: "%09u", nanoseconds)
        while fraction.last == "0" {
            fraction.removeLast()
        }
        return "\(seconds).\(fraction) seconds"
    }

    private var header: some View {
        HStack(alignment: .top, spacing: 16) {
            VStack(alignment: .leading, spacing: 5) {
                Text("Snapshot Explorer")
                    .font(.largeTitle.bold())
                Text(
                    "Historical, read-only observations from retained storage scans. A refreshed folder opens as a new standalone snapshot root; names and sizes are not live filesystem authority."
                )
                .foregroundStyle(.secondary)
            }
            Spacer()
            historyMenu
            Button {
                Task { await browser.refreshCurrentSubtree() }
            } label: {
                Label("Rescan This Folder", systemImage: "arrow.triangle.2.circlepath")
            }
            .disabled(!browser.canRefreshCurrentSubtree || model.scanState.phase.isActive)
            .keyboardShortcut(
                KeyEquivalent(ExplorerKeyboardShortcut.scanNow),
                modifiers: [.command]
            )
            .help("Scan the current folder and open the result as a new standalone snapshot root")
            .accessibilityIdentifier(ExplorerAccessibility.snapshotSubtreeRescan)
            .accessibilityHint(
                "Keeps this snapshot visible until the new folder snapshot is fully validated"
            )
            Button {
                Task { await browser.reloadLatest(ifPresented: presentationID) }
            } label: {
                Label("Load Latest Snapshot", systemImage: "arrow.clockwise")
            }
            .disabled(
                browser.phase == .loading
                    || browser.isNavigating
                    || browser.isPaging
                    || browser.isSwitchingSnapshot
            )
            .accessibilityIdentifier(ExplorerAccessibility.snapshotReload)
        }
    }

    @ViewBuilder
    private var subtreeScanStatus: some View {
        if browser.isSubtreeRefreshRunning,
           model.scanState.scope?.isHome == false,
           let scan = ExplorerScanPresentation.make(scanState: model.scanState)
        {
            HStack(alignment: .top, spacing: 12) {
                ProgressView()
                    .controlSize(.small)
                    .accessibilityHidden(true)
                VStack(alignment: .leading, spacing: 3) {
                    Text(verbatim: scan.title)
                        .font(.headline)
                    if let detail = scan.detail {
                        Text(verbatim: detail)
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                    Text("The previous snapshot remains available while this scan runs.")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
                Spacer()
                Button("Cancel") {
                    Task { await model.cancelHomeScan() }
                }
                .disabled(!subtreeCancellationEnabled)
                .accessibilityIdentifier(ExplorerAccessibility.snapshotSubtreeScanCancel)
                .accessibilityHint("Requests cancellation; the previous snapshot remains visible")
            }
            .padding(12)
            .background(.quaternary.opacity(0.45), in: RoundedRectangle(cornerRadius: 10))
            .accessibilityElement(children: .contain)
            .accessibilityIdentifier(ExplorerAccessibility.snapshotSubtreeScanStatus)
        }
    }

    private var subtreeCancellationEnabled: Bool {
        switch model.scanState.phase {
        case .queued, .scanning, .finalizing, .evaluating:
            true
        case .idle, .cancellationRequested, .succeeded, .cancelled, .failed:
            false
        }
    }

    private var historyMenu: some View {
        Menu {
            if browser.isHistoryLoading, browser.historyScans.isEmpty {
                Text("Loading recent scans…")
            } else if browser.historyScans.isEmpty {
                Text(browser.historyFailure == nil ? "No recent scans" : "Recent scans unavailable")
            } else {
                ForEach(browser.historyScans) { scan in
                    Button {
                        Task { await browser.selectHistoricalScan(scan) }
                    } label: {
                        Label {
                            Text(verbatim: historyRowTitle(scan))
                        } icon: {
                            Image(systemName: historyRowSymbol(scan))
                        }
                    }
                    .disabled(!canOpenHistoryRow(scan))
                }
                if browser.historyHasMore {
                    Divider()
                    Text(
                        "Showing the \(ExplorerSnapshotBrowserModel.historyLimit) newest scans; older scans aren’t shown yet."
                    )
                }
                if browser.historyFailure != nil {
                    Divider()
                    Text("Recent scans could not be refreshed. Showing the last confirmed list.")
                }
            }
            Divider()
            Button("Refresh recent scans") {
                Task { await browser.reloadHistory() }
            }
            .disabled(browser.isHistoryLoading)
        } label: {
            Label(selectedSnapshotTitle, systemImage: "clock.arrow.circlepath")
        }
        .disabled(
            browser.phase == .loading
                || browser.isSwitchingSnapshot
                || browser.isNavigating
                || browser.isPaging
        )
        .accessibilityIdentifier(ExplorerAccessibility.snapshotHistory)
        .accessibilityHint("Choose a retained, read-only scan snapshot")
    }

    @ViewBuilder
    private var content: some View {
        switch browser.phase {
        case .idle, .loading:
            Spacer()
            ProgressView("Opening latest snapshot…")
                .frame(maxWidth: .infinity)
            Spacer()
        case let .failed(failure):
            Spacer()
            failureView(failure)
            Spacer()
        case .ready:
            readyContent
        }
    }

    private var readyContent: some View {
        VStack(alignment: .leading, spacing: 12) {
            if browser.isSwitchingSnapshot {
                ProgressView("Opening selected snapshot…")
                    .controlSize(.small)
                    .accessibilityIdentifier(ExplorerAccessibility.snapshotHistoryStatus)
            }
            Picker(
                "Snapshot view",
                selection: Binding(
                    get: { browser.contentMode },
                    set: { mode in Task { await browser.selectContentMode(mode) } }
                )
            ) {
                Text("Browse").tag(ExplorerSnapshotContentMode.browse)
                Text("Changes").tag(ExplorerSnapshotContentMode.changes)
                Text("Candidates").tag(ExplorerSnapshotContentMode.candidates)
                Text("Large Files").tag(ExplorerSnapshotContentMode.largeFiles)
                Text("iCloud Status").tag(ExplorerSnapshotContentMode.iCloudStatus)
                Text("Coverage").tag(ExplorerSnapshotContentMode.coverage)
            }
            .pickerStyle(.segmented)
            .frame(maxWidth: 700)
            .disabled(browser.isSwitchingSnapshot)
            .accessibilityIdentifier(ExplorerAccessibility.snapshotContentMode)

            if let notice = browser.liveActionNotice {
                HStack(alignment: .top, spacing: 10) {
                    Image(systemName: notice.isFailure ? "exclamationmark.triangle.fill" : "checkmark.circle.fill")
                        .foregroundStyle(notice.isFailure ? .orange : .green)
                    Text(verbatim: notice.message)
                        .foregroundStyle(.secondary)
                    Spacer()
                    Button {
                        browser.dismissLiveActionNotice()
                    } label: {
                        Label("Dismiss", systemImage: "xmark")
                            .labelStyle(.iconOnly)
                    }
                    .buttonStyle(.plain)
                }
                .padding(10)
                .background(
                    (notice.isFailure ? Color.orange : Color.green).opacity(0.10),
                    in: RoundedRectangle(cornerRadius: 8)
                )
                .accessibilityIdentifier(ExplorerAccessibility.snapshotLiveActionStatus)
            }

            if let notice = browser.trashNotice {
                HStack(alignment: .top, spacing: 10) {
                    Image(systemName: notice.isFailure ? "exclamationmark.triangle.fill" : "checkmark.circle.fill")
                        .foregroundStyle(notice.isFailure ? .orange : .green)
                    Text(verbatim: notice.message)
                        .foregroundStyle(.secondary)
                    Spacer()
                    Button {
                        browser.dismissTrashNotice()
                    } label: {
                        Label("Dismiss", systemImage: "xmark")
                            .labelStyle(.iconOnly)
                    }
                    .buttonStyle(.plain)
                }
                .padding(10)
                .background(
                    (notice.isFailure ? Color.orange : Color.green).opacity(0.10),
                    in: RoundedRectangle(cornerRadius: 8)
                )
                .accessibilityIdentifier(ExplorerAccessibility.snapshotTrashStatus)
            }

            if let notice = browser.subtreeRefreshNotice {
                HStack(alignment: .top, spacing: 10) {
                    Image(systemName: notice.isFailure ? "exclamationmark.triangle.fill" : "checkmark.circle.fill")
                        .foregroundStyle(notice.isFailure ? .orange : .green)
                    Text(verbatim: notice.message)
                        .foregroundStyle(.secondary)
                    Spacer()
                    Button {
                        browser.dismissSubtreeRefreshNotice()
                    } label: {
                        Label("Dismiss", systemImage: "xmark")
                            .labelStyle(.iconOnly)
                    }
                    .buttonStyle(.plain)
                }
                .padding(10)
                .background(
                    (notice.isFailure ? Color.orange : Color.green).opacity(0.10),
                    in: RoundedRectangle(cornerRadius: 8)
                )
                .accessibilityElement(children: .combine)
                .accessibilityIdentifier(ExplorerAccessibility.snapshotSubtreeScanNotice)
            }

            if let notice = browser.candidateNotice {
                HStack(alignment: .top, spacing: 10) {
                    Image(systemName: notice.isFailure ? "exclamationmark.triangle.fill" : "checkmark.circle.fill")
                        .foregroundStyle(notice.isFailure ? .orange : .green)
                    Text(verbatim: notice.message)
                        .foregroundStyle(.secondary)
                    Spacer()
                    Button {
                        browser.dismissCandidateNotice()
                    } label: {
                        Label("Dismiss", systemImage: "xmark")
                            .labelStyle(.iconOnly)
                    }
                    .buttonStyle(.plain)
                }
                .padding(10)
                .background(
                    (notice.isFailure ? Color.orange : Color.green).opacity(0.10),
                    in: RoundedRectangle(cornerRadius: 8)
                )
                .accessibilityIdentifier(ExplorerAccessibility.snapshotCandidateStatus)
            }

            if browser.contentMode == .browse {
                browseContent
            } else if browser.contentMode == .changes {
                ExplorerSnapshotDiffView(
                    browser: browser,
                    inspectorPresented: $inspectorPresented
                )
            } else if browser.contentMode == .candidates {
                candidatesContent
            } else if browser.contentMode == .largeFiles {
                largeFilesContent
            } else if browser.contentMode == .iCloudStatus {
                iCloudObservationContent
            } else {
                coverageContent
            }
        }
        .inspector(isPresented: Binding(
            get: { browser.contentMode != .coverage && inspectorPresented },
            set: { value in
                if browser.contentMode != .coverage {
                    inspectorPresented = value
                }
            }
        )) {
            if browser.contentMode == .candidates {
                ExplorerCandidateInspectorView(
                    browser: browser,
                    permanentCleanupAvailability: permanentCleanupAvailability,
                    openSettingsDestination: openSettingsDestination,
                    confirmCleanup: { rustTargetCleanupConfirmation = $0 }
                )
                    .inspectorColumnWidth(min: 300, ideal: 380, max: 480)
            } else if browser.contentMode == .changes {
                ExplorerSnapshotDiffInspectorView(browser: browser)
                    .inspectorColumnWidth(min: 280, ideal: 340, max: 440)
            } else if browser.contentMode == .iCloudStatus {
                ExplorerICloudObservationInspectorView(browser: browser)
                    .inspectorColumnWidth(min: 280, ideal: 330, max: 420)
            } else {
                ExplorerSnapshotInspectorView(
                    browser: browser,
                    supplementalPresentation: supplementalPresentation
                )
                    .inspectorColumnWidth(min: 230, ideal: 270, max: 330)
            }
        }
        .overlay {
            if (browser.contentMode == .browse && browser.isNavigating)
                || (browser.contentMode == .changes && browser.isSnapshotDiffNavigating)
            {
                ZStack {
                    Color.black.opacity(0.08)
                    ProgressView(
                        browser.contentMode == .changes
                            ? "Opening comparison…"
                            : "Opening folder…"
                    )
                        .padding(18)
                        .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 10))
                }
            }
        }
    }

    private var candidatesContent: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack {
                Label(
                    "Review deterministic suggestions and inspect exact historical evidence. A supported plan preview performs live checks but cannot approve or remove anything.",
                    systemImage: "checkmark.shield"
                )
                .font(.callout)
                .foregroundStyle(.secondary)
                Spacer()
                Button {
                    Task { await browser.reloadCandidates() }
                } label: {
                    Label("Refresh candidates", systemImage: "arrow.clockwise")
                }
                .disabled(browser.isCandidateLoading || browser.isSwitchingSnapshot)
            }

            if let failure = browser.candidateFailure {
                ContentUnavailableView {
                    Label(failure.title, systemImage: "exclamationmark.triangle")
                } description: {
                    Text(verbatim: failure.detail)
                } actions: {
                    Button("Try Again") {
                        Task { await browser.reloadCandidates() }
                    }
                }
            } else if browser.isCandidateLoading {
                ProgressView("Loading review candidates…")
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else if let page = browser.candidatePage {
                if page.candidates.isEmpty {
                    ContentUnavailableView(
                        "No candidates found",
                        systemImage: "checkmark.circle",
                        description: Text("This snapshot has no deterministic review candidates.")
                    )
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                } else {
                    candidateFindingGroups(page)
                    candidatesTable(page.candidates)
                    candidatePageControls(page)
                }
            }
        }
        .accessibilityIdentifier(ExplorerAccessibility.snapshotCandidates)
    }

    private func candidateFindingGroups(
        _ page: ExplorerCandidateSummaryPage
    ) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(alignment: .firstTextBaseline) {
                Text("Finding groups")
                    .font(.headline)
                Spacer()
                Text("Current page · estimates are not summed")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            ScrollView(.horizontal) {
                HStack(spacing: 10) {
                    ForEach(page.findingGroups) { group in
                        VStack(alignment: .leading, spacing: 5) {
                            Text(verbatim: group.category.displayName)
                                .font(.headline)
                            Text(verbatim: group.safety.displayName)
                                .font(.subheadline)
                            Text(verbatim: group.action.displayName)
                                .font(.caption)
                                .foregroundStyle(.secondary)
                                .lineLimit(1)
                            HStack {
                                Label(
                                    "\(group.count) \(group.count == 1 ? "finding" : "findings")",
                                    systemImage: "doc.text.magnifyingglass"
                                )
                                if group.blockedCount > 0 {
                                    Label(
                                        "\(group.blockedCount) blocked",
                                        systemImage: "lock.fill"
                                    )
                                    .foregroundStyle(.orange)
                                }
                            }
                            .font(.caption)
                        }
                        .frame(minWidth: 220, alignment: .leading)
                        .padding(12)
                        .background(
                            .quaternary.opacity(0.45),
                            in: RoundedRectangle(cornerRadius: 10)
                        )
                        .accessibilityElement(children: .combine)
                    }
                }
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier(ExplorerAccessibility.snapshotCandidateGroups)
    }

    private func candidatesTable(_ candidates: [ExplorerCandidateSummary]) -> some View {
        Table(
            candidates,
            selection: Binding(
                get: { browser.selectedCandidateID },
                set: { candidateID in
                    Task { await browser.selectCandidate(candidateID) }
                }
            )
        ) {
            TableColumn("Rule") { candidate in
                VStack(alignment: .leading, spacing: 2) {
                    Text(verbatim: candidate.ruleDisplayName)
                        .font(.headline)
                    Text(
                        verbatim:
                        "\(candidate.ruleID) · revision \(candidate.ruleRevision) · \(candidate.category.displayName)"
                    )
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
                .accessibilityElement(children: .combine)
                .accessibilityAction(named: Text("Inspect details")) {
                    Task { await browser.selectCandidate(candidate.candidateID) }
                }
            }
            TableColumn("Observed estimate") { candidate in
                Text(ByteCountFormatter.string(fromByteCount: Int64(min(candidate.estimatedBytes, UInt64(Int64.max))), countStyle: .file))
                    .monospacedDigit()
            }
            TableColumn("Safety") { candidate in
                Text(verbatim: candidate.safety.displayName)
            }
            TableColumn("Paths") { candidate in
                Text(candidate.pathCount, format: .number)
                    .monospacedDigit()
            }
            TableColumn("Status") { candidate in
                Text(verbatim: candidate.status.displayName)
            }
            TableColumn("Review") { candidate in
                Menu("Review") {
                    Button("Select") {
                        Task { await browser.reviewCandidate(candidateID: candidate.candidateID, command: .select) }
                    }
                    .disabled(!candidate.blockers.isEmpty)
                    Button("Clear selection") {
                        Task { await browser.reviewCandidate(candidateID: candidate.candidateID, command: .clearSelection) }
                    }
                    Button("Dismiss") {
                        Task { await browser.reviewCandidate(candidateID: candidate.candidateID, command: .dismiss) }
                    }
                    Button("Restore") {
                        Task { await browser.reviewCandidate(candidateID: candidate.candidateID, command: .restore) }
                    }
                }
                .disabled(browser.isCandidateLoading)
            }
        }
        .accessibilityIdentifier(ExplorerAccessibility.snapshotCandidateTable)
    }

    private func candidatePageControls(
        _ page: ExplorerCandidateSummaryPage
    ) -> some View {
        HStack {
            Text(verbatim: candidatePageStatus(page))
                .font(.callout)
                .foregroundStyle(.secondary)
                .monospacedDigit()
                .accessibilityIdentifier(
                    ExplorerAccessibility.snapshotCandidatePageStatus
                )
            Spacer()
            Button {
                Task { await browser.showPreviousCandidatePage() }
            } label: {
                Label("Previous findings", systemImage: "chevron.left")
            }
            .disabled(!browser.hasPreviousCandidatePage || browser.isCandidateLoading)
            .accessibilityIdentifier(
                ExplorerAccessibility.snapshotCandidatePreviousPage
            )
            Button {
                Task { await browser.showNextCandidatePage() }
            } label: {
                Label("Next findings", systemImage: "chevron.right")
            }
            .disabled(!browser.hasNextCandidatePage || browser.isCandidateLoading)
            .accessibilityIdentifier(
                ExplorerAccessibility.snapshotCandidateNextPage
            )
        }
    }

    private func candidatePageStatus(_ page: ExplorerCandidateSummaryPage) -> String {
        guard !page.candidates.isEmpty else {
            return "No findings"
        }
        let first = Int(page.cursor) + 1
        let last = Int(page.cursor) + page.candidates.count
        return "Showing \(first)–\(last) of \(page.totalCandidates) findings"
    }

    private var browseContent: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack(spacing: 12) {
                Button {
                    Task { await browser.goBack() }
                } label: {
                    Label("Back", systemImage: "chevron.left")
                }
                .disabled(
                    browser.breadcrumbs.count <= 1
                        || browser.isNavigating
                        || browser.isPaging
                        || browser.isSwitchingSnapshot
                )
                .accessibilityIdentifier(ExplorerAccessibility.snapshotBack)
                breadcrumbs
                Spacer(minLength: 16)
                sortPicker
                Button {
                    inspectorPresented.toggle()
                } label: {
                    Label("Inspector", systemImage: "sidebar.right")
                        .labelStyle(.iconOnly)
                }
                .help(inspectorPresented ? "Hide inspector" : "Show inspector")
            }

            if let failure = browser.operationFailure {
                HStack(alignment: .top, spacing: 10) {
                    Image(systemName: "exclamationmark.triangle.fill")
                        .foregroundStyle(.orange)
                    VStack(alignment: .leading, spacing: 2) {
                        Text(verbatim: failure.title)
                            .font(.headline)
                        Text(verbatim: failure.detail)
                            .foregroundStyle(.secondary)
                    }
                    Spacer()
                    Button {
                        browser.dismissOperationFailure()
                    } label: {
                        Label("Dismiss", systemImage: "xmark")
                            .labelStyle(.iconOnly)
                    }
                    .buttonStyle(.plain)
                }
                .padding(10)
                .background(.orange.opacity(0.12), in: RoundedRectangle(cornerRadius: 8))
                .accessibilityIdentifier(ExplorerAccessibility.snapshotError)
            }

            supplementalPresentation.status(
                openSettings: openSettingsDestination
            )

            ExplorerSnapshotTreemapView(
                browser: browser,
                supplementalPresentation: supplementalPresentation
            )
            if browser.nodes.isEmpty, browser.totalChildren == 0 {
                ContentUnavailableView(
                    "Folder is empty",
                    systemImage: "folder",
                    description: Text("This snapshot recorded no direct children here.")
                )
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                snapshotTable
                pageControls
            }
        }
    }

    private var largeFilesContent: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack(spacing: 12) {
                Picker(
                    "Minimum size",
                    selection: Binding(
                        get: { browser.largeFileThreshold },
                        set: { value in Task { await browser.selectLargeFileThreshold(value) } }
                    )
                ) {
                    ForEach(ExplorerSnapshotLargeFileThreshold.allCases, id: \.self) { value in
                        Text(verbatim: value.title).tag(value)
                    }
                }
                .pickerStyle(.menu)
                .accessibilityIdentifier(ExplorerAccessibility.snapshotLargeFileThreshold)

                Picker(
                    "Last modified",
                    selection: Binding(
                        get: { browser.largeFileAge },
                        set: { value in Task { await browser.selectLargeFileAge(value) } }
                    )
                ) {
                    ForEach(ExplorerSnapshotLargeFileAge.allCases, id: \.self) { value in
                        Text(verbatim: value.title).tag(value)
                    }
                }
                .pickerStyle(.menu)
                .accessibilityIdentifier(ExplorerAccessibility.snapshotLargeFileAge)

                Spacer()
                Button {
                    Task { await browser.reloadLargeFiles() }
                } label: {
                    Label("Refresh large files", systemImage: "arrow.clockwise")
                }
                .disabled(browser.isLargeFilesLoading || browser.isSwitchingSnapshot)

                Button {
                    inspectorPresented.toggle()
                } label: {
                    Label("Inspector", systemImage: "sidebar.right")
                        .labelStyle(.iconOnly)
                }
                .help(inspectorPresented ? "Hide inspector" : "Show inspector")
            }
            .disabled(browser.isSwitchingSnapshot)

            Label(
                "Historical observations only — sizes are not reclaimable estimates, and files outside scan coverage are not represented.",
                systemImage: "clock.badge.exclamationmark"
            )
            .font(.callout)
            .foregroundStyle(.secondary)

            if let failure = browser.largeFilesFailure {
                HStack(alignment: .top, spacing: 10) {
                    Image(systemName: "exclamationmark.triangle.fill")
                        .foregroundStyle(.orange)
                    VStack(alignment: .leading, spacing: 2) {
                        Text(verbatim: failure.title)
                            .font(.headline)
                        Text(verbatim: failure.detail)
                            .foregroundStyle(.secondary)
                    }
                    Spacer()
                    Button("Try Again") {
                        Task { await browser.reloadLargeFiles() }
                    }
                }
                .padding(10)
                .background(.orange.opacity(0.12), in: RoundedRectangle(cornerRadius: 8))
                .accessibilityIdentifier(ExplorerAccessibility.snapshotLargeFileStatus)
            } else if browser.isLargeFilesLoading {
                Spacer()
                ProgressView("Finding the largest matching files…")
                    .frame(maxWidth: .infinity)
                    .accessibilityIdentifier(ExplorerAccessibility.snapshotLargeFileStatus)
                Spacer()
            } else if let page = browser.largeFilesPage {
                if page.files.isEmpty {
                    ContentUnavailableView(
                        "No matching files",
                        systemImage: "doc.text.magnifyingglass",
                        description: Text(
                            "This snapshot recorded no files matching the selected size and age filters."
                        )
                    )
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                } else {
                    largeFilesTable(page)
                }
                Text(verbatim: largeFilesStatus(page))
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .monospacedDigit()
                    .accessibilityIdentifier(ExplorerAccessibility.snapshotLargeFileStatus)
            }
        }
    }

    private var iCloudObservationContent: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack(alignment: .top, spacing: 12) {
                Label("Manual iCloud status check", systemImage: "icloud.and.arrow.down")
                    .font(.headline)
                Spacer()
                if browser.canStopICloudObservationBatch {
                    Button("Stop after current check") {
                        browser.stopICloudObservationBatch()
                    }
                    .accessibilityIdentifier(
                        ExplorerAccessibility.snapshotICloudObservationStop
                    )
                } else {
                    Button(iCloudObservationCheckButtonTitle) {
                        Task { await browser.startICloudObservationBatch() }
                    }
                    .disabled(!browser.canStartICloudObservationBatch)
                    .accessibilityIdentifier(
                        ExplorerAccessibility.snapshotICloudObservationCheck
                    )
                }
                Button {
                    inspectorPresented.toggle()
                } label: {
                    Label("Inspector", systemImage: "sidebar.right")
                        .labelStyle(.iconOnly)
                }
                .help(inspectorPresented ? "Hide inspector" : "Show inspector")
            }

            Text(
                "DUX ranks up to 32 regular files in this snapshot folder by historical allocated size. It does not identify them as iCloud files until you explicitly check."
            )
            .font(.callout)
            .foregroundStyle(.secondary)
            .accessibilityIdentifier(
                ExplorerAccessibility.snapshotICloudObservationDisclosure
            )

            Label(
                "Results are sequential, point-in-time metadata observations. Sync favorability and identity readiness are separate; neither creates a cleanup candidate, reclaim estimate, or permission to remove a local copy.",
                systemImage: "hand.raised.fill"
            )
            .font(.callout)
            .foregroundStyle(.secondary)

            if let failure = browser.iCloudObservationSourceFailure {
                ContentUnavailableView {
                    Label(
                        iCloudObservationSourceFailureTitle(failure),
                        systemImage: "exclamationmark.triangle"
                    )
                } description: {
                    Text(verbatim: iCloudObservationSourceFailureDetail(failure))
                } actions: {
                    Button("Try Again") {
                        Task { await browser.reloadICloudObservationSource() }
                    }
                }
                .accessibilityIdentifier(
                    ExplorerAccessibility.snapshotICloudObservationSourceStatus
                )
            } else if browser.isICloudObservationSourceLoading {
                Spacer()
                ProgressView("Ranking historical local allocations…")
                    .frame(maxWidth: .infinity)
                    .accessibilityIdentifier(
                        ExplorerAccessibility.snapshotICloudObservationSourceStatus
                    )
                Spacer()
            } else if let source = browser.iCloudObservationSource {
                if source.targets.isEmpty {
                    ContentUnavailableView(
                        "No allocated regular files",
                        systemImage: "icloud.slash",
                        description: Text(
                            "This snapshot folder contains no complete regular-file observations with known nonzero allocation."
                        )
                    )
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                } else {
                    iCloudObservationTable(source)
                }
                Text(verbatim: iCloudObservationSourceSummary(source))
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .monospacedDigit()
                    .accessibilityIdentifier(
                        ExplorerAccessibility.snapshotICloudObservationSummary
                    )
                Text(verbatim: iCloudObservationBatchStatus)
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .accessibilityIdentifier(
                        ExplorerAccessibility.snapshotICloudObservationBatchStatus
                    )
            }
        }
        .accessibilityIdentifier(ExplorerAccessibility.snapshotICloudObservationView)
    }

    private var coverageContent: some View {
        Group {
            if let failure = browser.coverageFailure {
                ContentUnavailableView {
                    Label(failure.title, systemImage: "exclamationmark.triangle")
                } description: {
                    Text(verbatim: failure.detail)
                } actions: {
                    Button("Try Again") {
                        Task { await browser.reloadCoverage() }
                    }
                }
            } else if browser.isCoverageLoading {
                ProgressView("Loading scan coverage…")
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else if let details = browser.coverageDetails {
                ScrollView {
                    VStack(alignment: .leading, spacing: 18) {
                        coverageSummary(details)
                        if details.issues.isEmpty {
                            if details.coverage == .unknown {
                                ContentUnavailableView(
                                    "No coverage details recorded",
                                    systemImage: "questionmark.diamond",
                                    description: Text(
                                        "This historical scan did not record a coverage measurement or issue details. Unknown coverage is not zero coverage."
                                    )
                                )
                                .frame(maxWidth: .infinity)
                            } else {
                                ContentUnavailableView(
                                    "No scan limitations recorded",
                                    systemImage: "checkmark.shield",
                                    description: Text(
                                        "The selected Home scan completed without recorded coverage issues."
                                    )
                                )
                                .frame(maxWidth: .infinity)
                            }
                        } else {
                            Text("Recorded limitations")
                                .font(.title3.bold())
                            LazyVStack(alignment: .leading, spacing: 10) {
                                ForEach(details.issues) { issue in
                                    coverageIssueRow(issue)
                                }
                            }
                            .accessibilityIdentifier(
                                ExplorerAccessibility.snapshotCoverageIssueList
                            )
                        }
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                }
            }
        }
        .accessibilityIdentifier(ExplorerAccessibility.snapshotCoverageView)
    }

    private func coverageSummary(_ details: ExplorerScanCoverageDetails) -> some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack(alignment: .firstTextBaseline) {
                Label(details.coverage.title, systemImage: details.coverage.symbol)
                    .font(.title2.bold())
                    .accessibilityIdentifier(ExplorerAccessibility.snapshotCoverageStatus)
                Spacer()
                Text(
                    "\(details.totalIssueRecords) records · \(details.totalIssueOccurrences) occurrences"
                )
                .monospacedDigit()
                .foregroundStyle(.secondary)
            }
            if let measuredPermille = details.measuredPermille {
                let fraction = Double(measuredPermille) / 1_000
                ProgressView(value: fraction)
                    .tint(details.coverage == .complete ? .green : .orange)
                    .accessibilityLabel("Measured scan coverage")
                    .accessibilityValue(
                        fraction.formatted(.percent.precision(.fractionLength(1)))
                    )
                    .accessibilityIdentifier(ExplorerAccessibility.snapshotCoverageBar)
                Text(
                    "\(fraction.formatted(.percent.precision(.fractionLength(1)))) of the intended Home scan scope was measured."
                )
                .font(.callout)
            } else {
                Text("No quantitative coverage estimate was recorded for this scan.")
                    .font(.callout)
            }
            Text(
                "This describes what the selected Home scan observed. It is not whole-disk coverage, a current access test, or a reclaimable-space estimate."
            )
            .font(.callout)
            .foregroundStyle(.secondary)
            if details.coverage == .limitedAccess {
                Text(
                    "Broader macOS access may change a future scan. It does not change this historical snapshot."
                )
                .font(.callout)
                .foregroundStyle(.secondary)
            }
        }
        .padding(16)
        .background(.quaternary.opacity(0.35), in: RoundedRectangle(cornerRadius: 12))
    }

    private func coverageIssueRow(_ issue: ExplorerScanCoverageIssue) -> some View {
        HStack(alignment: .top, spacing: 12) {
            Image(systemName: issue.kind.symbol)
                .frame(width: 22)
                .foregroundStyle(.orange)
            VStack(alignment: .leading, spacing: 4) {
                HStack {
                    Text(verbatim: issue.kind.title)
                        .font(.headline)
                    Spacer()
                    Text(
                        issue.occurrenceCount == 1
                            ? "1 occurrence"
                            : "\(issue.occurrenceCount) occurrences"
                    )
                    .monospacedDigit()
                    .foregroundStyle(.secondary)
                }
                Text(verbatim: issue.kind.detail)
                    .font(.callout)
                    .foregroundStyle(.secondary)
                Text(verbatim: issue.locationDisplay)
                    .font(.callout.monospaced())
                    .lineLimit(2)
                    .help(issue.locationDisplay)
            }
        }
        .padding(12)
        .background(.background, in: RoundedRectangle(cornerRadius: 10))
        .overlay {
            RoundedRectangle(cornerRadius: 10).stroke(.quaternary)
        }
        .accessibilityElement(children: .combine)
    }

    private var breadcrumbs: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 5) {
                ForEach(Array(browser.breadcrumbs.enumerated()), id: \.element.id) { index, node in
                    if index > 0 {
                        Image(systemName: "chevron.right")
                            .font(.caption)
                            .foregroundStyle(.tertiary)
                    }
                    Button {
                        Task { await browser.goToBreadcrumb(at: index) }
                    } label: {
                        Text(verbatim: node.name.display)
                            .lineLimit(1)
                    }
                    .buttonStyle(.plain)
                    .font(index == browser.breadcrumbs.indices.last ? .headline : .body)
                    .disabled(
                        index == browser.breadcrumbs.indices.last
                            || browser.isNavigating
                            || browser.isPaging
                    )
                }
            }
        }
        .accessibilityIdentifier(ExplorerAccessibility.snapshotBreadcrumbs)
    }

    private var sortPicker: some View {
        Picker(
            "Sort",
            selection: Binding(
                get: { browser.sort },
                set: { sort in Task { await browser.selectSort(sort) } }
            )
        ) {
            ForEach(ExplorerSnapshotNodeSort.allCases, id: \.self) { sort in
                Text(verbatim: sort.title).tag(sort)
            }
        }
        .pickerStyle(.menu)
        .disabled(browser.isNavigating || browser.isPaging || browser.isSwitchingSnapshot)
        .accessibilityIdentifier(ExplorerAccessibility.snapshotSort)
    }

    private var snapshotTable: some View {
        Table(
            browser.nodes,
            selection: Binding(
                get: { browser.selectedNodeID },
                set: { nodeID in
                    Task { await browser.selectTableNodeForPresentation(nodeID) }
                }
            )
        ) {
            TableColumn("Name") { (node: ExplorerSnapshotNode) in
                HStack(spacing: 8) {
                    Image(systemName: node.kind.symbol)
                        .foregroundStyle(
                            node.kind == .directory ? Color.accentColor : Color.secondary
                    )
                    if node.kind == .directory {
                        Text(verbatim: node.name.display)
                            .lineLimit(1)
                            .onTapGesture(count: 2) {
                                Task { await browser.openDirectory(node) }
                            }
                            .accessibilityHint(
                                "Selects this item. Press Return to open the historical folder."
                            )
                    } else {
                        Text(verbatim: node.name.display)
                            .lineLimit(1)
                    }
                    if node.hasObservationWarning {
                        Image(systemName: "exclamationmark.triangle.fill")
                            .foregroundStyle(.orange)
                            .help(node.flagSummary)
                    }
                    supplementalPresentation.tableDecoration(
                        forObservedNodeID: node.id
                    )
                }
                .accessibilityElement(children: .combine)
                .accessibilityLabel(tableAccessibilityLabel(node))
                .contextMenu {
                    liveActionMenu(node: node, fromLargeFiles: false)
                }
            }
            .width(min: 180, ideal: 280)

            TableColumn("Category") { (node: ExplorerSnapshotNode) in
                ExplorerStorageCategoryLabel(category: node.category)
                    .accessibilityIdentifier(
                        "\(ExplorerAccessibility.snapshotCategoryColumn)-\(node.id)"
                    )
            }
            .width(min: 135, ideal: 165)

            TableColumn("Logical size") { (node: ExplorerSnapshotNode) in
                Text(verbatim: StorageByteFormatter.string(from: node.logicalBytes))
                    .monospacedDigit()
            }
            .width(min: 90, ideal: 105)

            TableColumn("Allocated") { (node: ExplorerSnapshotNode) in
                Text(verbatim: node.allocatedSizeText)
                    .monospacedDigit()
                    .foregroundStyle(node.allocatedBytes == nil ? .secondary : .primary)
            }
            .width(min: 90, ideal: 105)

            TableColumn("Items") { (node: ExplorerSnapshotNode) in
                Text(verbatim: node.itemSummary)
                    .monospacedDigit()
            }
            .width(min: 80, ideal: 95)

            TableColumn("Modified") { (node: ExplorerSnapshotNode) in
                Text(verbatim: node.modifiedText)
                    .foregroundStyle(node.modifiedAt == nil ? .secondary : .primary)
            }
            .width(min: 110, ideal: 135)

            TableColumn("Share") { (node: ExplorerSnapshotNode) in
                ProgressView(value: share(for: node))
                    .progressViewStyle(.linear)
                    .accessibilityLabel("Share of current folder")
                    .accessibilityValue(
                        share(for: node).formatted(.percent.precision(.fractionLength(0)))
                    )
            }
            .width(min: 70, ideal: 90)
        }
        .disabled(browser.isNavigating || browser.isPaging || browser.isSwitchingSnapshot)
        .onKeyPress(.return) {
            guard let selected = browser.selectedNode, selected.kind == .directory else {
                return .ignored
            }
            Task { await browser.openDirectory(selected) }
            return .handled
        }
        .onKeyPress(.delete) {
            guard browser.breadcrumbs.count > 1 else {
                return .ignored
            }
            Task { await browser.goBack() }
            return .handled
        }
        .onKeyPress(.space) {
            guard browser.canQuickLookSelectedLiveItem else {
                return .ignored
            }
            Task { await browser.quickLookLiveItem() }
            return .handled
        }
        .accessibilityIdentifier(ExplorerAccessibility.snapshotTable)
    }

    private func tableAccessibilityLabel(_ node: ExplorerSnapshotNode) -> String {
        node.accessibilitySummary
            + supplementalPresentation.accessibilitySuffix(
                forObservedNodeID: node.id
            )
    }

    private func largeFilesTable(_ page: ExplorerSnapshotLargeFilesPage) -> some View {
        Table(
            page.files,
            selection: Binding(
                get: { browser.selectedNodeID },
                set: { browser.selectLargeFile($0) }
            )
        ) {
            TableColumn("Name") { (file: ExplorerSnapshotLargeFile) in
                HStack(spacing: 8) {
                    Image(systemName: file.node.kind.symbol)
                        .foregroundStyle(.secondary)
                    Text(verbatim: file.node.name.display)
                        .lineLimit(1)
                    if file.node.hasObservationWarning {
                        Image(systemName: "exclamationmark.triangle.fill")
                            .foregroundStyle(.orange)
                            .help(file.node.flagSummary)
                    }
                }
                .accessibilityElement(children: .combine)
                .accessibilityLabel(file.node.accessibilitySummary)
                .contextMenu {
                    liveActionMenu(node: file.node, fromLargeFiles: true)
                }
            }
            .width(min: 170, ideal: 250)

            TableColumn("Category") { (file: ExplorerSnapshotLargeFile) in
                ExplorerStorageCategoryLabel(category: file.node.category)
                    .accessibilityIdentifier(
                        "\(ExplorerAccessibility.snapshotCategoryColumn)-large-\(file.id)"
                    )
            }
            .width(min: 135, ideal: 165)

            TableColumn("Historical location") { (file: ExplorerSnapshotLargeFile) in
                Text(verbatim: file.parentDisplay.isEmpty ? "Top level" : file.parentDisplay)
                    .lineLimit(1)
                    .help(file.parentDisplay.isEmpty ? "Top level" : file.parentDisplay)
            }
            .width(min: 190, ideal: 300)

            TableColumn("Logical size") { (file: ExplorerSnapshotLargeFile) in
                Text(verbatim: StorageByteFormatter.string(from: file.node.logicalBytes))
                    .monospacedDigit()
            }
            .width(min: 90, ideal: 105)

            TableColumn("Allocated") { (file: ExplorerSnapshotLargeFile) in
                Text(verbatim: file.node.allocatedSizeText)
                    .monospacedDigit()
                    .foregroundStyle(file.node.allocatedBytes == nil ? .secondary : .primary)
            }
            .width(min: 90, ideal: 105)

            TableColumn("Modified") { (file: ExplorerSnapshotLargeFile) in
                Text(verbatim: file.node.modifiedText)
                    .foregroundStyle(file.node.modifiedAt == nil ? .secondary : .primary)
            }
            .width(min: 110, ideal: 135)

            TableColumn("Share") { (file: ExplorerSnapshotLargeFile) in
                ProgressView(value: largeFileShare(file, page: page))
                    .progressViewStyle(.linear)
                    .accessibilityLabel("Share of matching logical size")
                    .accessibilityValue(
                        largeFileShare(file, page: page)
                            .formatted(.percent.precision(.fractionLength(0)))
                    )
            }
            .width(min: 70, ideal: 90)
        }
        .disabled(browser.isLargeFilesLoading || browser.isSwitchingSnapshot)
        .onKeyPress(.space) {
            guard browser.canQuickLookSelectedLiveItem else {
                return .ignored
            }
            Task { await browser.quickLookLiveItem() }
            return .handled
        }
        .accessibilityIdentifier(ExplorerAccessibility.snapshotLargeFileTable)
    }

    private func iCloudObservationTable(
        _ source: ExplorerICloudObservationSource
    ) -> some View {
        Table(
            source.targets,
            selection: Binding(
                get: { browser.selectedNodeID },
                set: { browser.selectICloudObservationTarget($0) }
            )
        ) {
            TableColumn("Name") { (target: ExplorerICloudObservationTarget) in
                HStack(spacing: 8) {
                    Image(systemName: "doc")
                        .foregroundStyle(.secondary)
                    Text(verbatim: target.node.name.display)
                        .lineLimit(1)
                }
                .accessibilityElement(children: .combine)
                .accessibilityLabel(target.node.accessibilitySummary)
            }
            .width(min: 170, ideal: 250)

            TableColumn("Historical location") {
                (target: ExplorerICloudObservationTarget) in
                Text(
                    verbatim: target.parentDisplay.isEmpty
                        ? "Top level"
                        : target.parentDisplay
                )
                .lineLimit(1)
                .help(
                    target.parentDisplay.isEmpty
                        ? "Top level"
                        : target.parentDisplay
                )
            }
            .width(min: 180, ideal: 280)

            TableColumn("Allocated in scan") {
                (target: ExplorerICloudObservationTarget) in
                Text(verbatim: target.node.allocatedSizeText)
                    .monospacedDigit()
            }
            .width(min: 105, ideal: 120)

            TableColumn("Logical size") {
                (target: ExplorerICloudObservationTarget) in
                Text(
                    verbatim: StorageByteFormatter.string(
                        from: target.node.logicalBytes
                    )
                )
                .monospacedDigit()
            }
            .width(min: 90, ideal: 105)

            TableColumn("Sync metadata") {
                (target: ExplorerICloudObservationTarget) in
                iCloudObservationStatus(target)
            }
            .width(min: 150, ideal: 190)

            TableColumn("Identity proof") {
                (target: ExplorerICloudObservationTarget) in
                iCloudIdentityStatus(target)
            }
            .width(min: 150, ideal: 190)

            TableColumn("Observed") {
                (target: ExplorerICloudObservationTarget) in
                Text(verbatim: iCloudObservationTime(target))
                    .foregroundStyle(.secondary)
            }
            .width(min: 110, ideal: 135)
        }
        .disabled(browser.isSwitchingSnapshot)
        .accessibilityIdentifier(
            ExplorerAccessibility.snapshotICloudObservationTable
        )
    }

    @ViewBuilder
    private func iCloudObservationStatus(
        _ target: ExplorerICloudObservationTarget
    ) -> some View {
        if browser.checkingICloudObservationNodeID == target.id {
            Label("Checking…", systemImage: "hourglass")
                .foregroundStyle(.secondary)
        } else {
            switch browser.iCloudObservationResults[target.id] {
            case let .observed(assessment):
                if assessment.isEligibleObservation {
                    Label("Favorable", systemImage: "checkmark.circle.fill")
                        .foregroundStyle(.green)
                } else {
                    Label(
                        "\(assessment.blockers.count) blocker\(assessment.blockers.count == 1 ? "" : "s")",
                        systemImage: "nosign"
                    )
                    .foregroundStyle(.orange)
                }
            case let .failed(error):
                Label(
                    iCloudObservationProbeErrorLabel(error),
                    systemImage: "exclamationmark.triangle.fill"
                )
                .foregroundStyle(.orange)
            case nil:
                Text("Not checked")
                    .foregroundStyle(.secondary)
            }
        }
    }

    @ViewBuilder
    private func iCloudIdentityStatus(
        _ target: ExplorerICloudObservationTarget
    ) -> some View {
        if browser.checkingICloudObservationNodeID == target.id {
            Label("Checking…", systemImage: "hourglass")
                .foregroundStyle(.secondary)
        } else {
            switch browser.iCloudObservationResults[target.id] {
            case let .observed(assessment):
                if assessment.isIdentityReady {
                    Label("Ready", systemImage: "checkmark.shield.fill")
                        .foregroundStyle(.green)
                } else {
                    let containerUnsupported = assessment.identityBlockers.contains(
                        .containerIdentityUnsupported
                    )
                    let otherBlockerCount = containerUnsupported
                        ? assessment.identityBlockers.count - 1
                        : 0
                    Label(
                        containerUnsupported
                            ? "Container unsupported"
                                + (otherBlockerCount > 0 ? " + \(otherBlockerCount)" : "")
                            : "\(assessment.identityBlockers.count) blocker\(assessment.identityBlockers.count == 1 ? "" : "s")",
                        systemImage: "exclamationmark.shield.fill"
                    )
                    .foregroundStyle(.orange)
                    .help(
                        assessment.identityBlockers
                            .map(\.displayText)
                            .joined(separator: "\n")
                    )
                }
            case .failed:
                Text("Unavailable")
                    .foregroundStyle(.secondary)
            case nil:
                Text("Not checked")
                    .foregroundStyle(.secondary)
            }
        }
    }

    private func iCloudObservationTime(
        _ target: ExplorerICloudObservationTarget
    ) -> String {
        guard
            case let .observed(assessment) =
                browser.iCloudObservationResults[target.id]
        else {
            return "—"
        }
        return Date(
            timeIntervalSince1970:
                Double(assessment.observedAtUnixMilliseconds) / 1_000
        )
        .formatted(date: .abbreviated, time: .shortened)
    }

    @ViewBuilder
    private func liveActionMenu(
        node: ExplorerSnapshotNode,
        fromLargeFiles: Bool
    ) -> some View {
        let supported = node.kind == .file || node.kind == .directory || node.kind == .symlink
        Button("Reveal in Finder") {
            Task {
                await selectForLiveAction(node.id, fromLargeFiles: fromLargeFiles)
                await browser.revealLiveItem(nodeID: node.id)
            }
        }
        .disabled(!supported)
        Button("Copy Path") {
            Task {
                await selectForLiveAction(node.id, fromLargeFiles: fromLargeFiles)
                await browser.copyLiveItemPath(nodeID: node.id)
            }
        }
        .disabled(!supported)
        Button("Quick Look") {
            Task {
                await selectForLiveAction(node.id, fromLargeFiles: fromLargeFiles)
                await browser.quickLookLiveItem(nodeID: node.id)
            }
        }
        .disabled(node.kind != .file)
        Divider()
        Button("Move to Trash…", role: .destructive) {
            Task {
                await selectForLiveAction(node.id, fromLargeFiles: fromLargeFiles)
                trashConfirmation = browser.makeTrashConfirmation(
                    forSelectedNodeID: node.id
                )
            }
        }
        .disabled(!supported || browser.isTrashLoading)
    }

    private func selectForLiveAction(
        _ nodeID: UInt64,
        fromLargeFiles: Bool
    ) async {
        if fromLargeFiles {
            browser.selectLargeFile(nodeID)
        } else {
            await browser.selectTableNodeForPresentation(nodeID)
        }
    }

    private var pageControls: some View {
        HStack {
            Text(verbatim: pageStatus)
                .foregroundStyle(.secondary)
                .monospacedDigit()
                .accessibilityIdentifier(ExplorerAccessibility.snapshotPageStatus)
            Spacer()
            if browser.isPaging {
                ProgressView()
                    .controlSize(.small)
            }
            Button {
                Task { await browser.showPreviousPage() }
            } label: {
                Label("Previous page", systemImage: "chevron.left")
            }
            .disabled(!browser.hasPreviousPage || browser.isPaging)
            .accessibilityIdentifier(ExplorerAccessibility.snapshotPreviousPage)

            Button {
                Task { await browser.showNextPage() }
            } label: {
                Label("Next page", systemImage: "chevron.right")
            }
            .disabled(!browser.hasNextPage || browser.isPaging)
            .accessibilityIdentifier(ExplorerAccessibility.snapshotNextPage)
        }
    }

    private func failureView(_ failure: ExplorerSnapshotBrowserFailure) -> some View {
        ContentUnavailableView {
            Label(failure.title, systemImage: failure == .noSnapshot ? "internaldrive" : "exclamationmark.triangle")
        } description: {
            Text(verbatim: failure.detail)
        } actions: {
            Button("Try Again") {
                Task { await browser.reloadLatest(ifPresented: presentationID) }
            }
            if failure == .noSnapshot {
                Button("Scan Home") {
                    Task {
                        await model.startHomeScan()
                        guard !Task.isCancelled else {
                            return
                        }
                        await browser.reloadLatest(ifPresented: presentationID)
                    }
                }
            }
        }
        .accessibilityIdentifier(ExplorerAccessibility.snapshotError)
    }

    private var pageStatus: String {
        guard !browser.nodes.isEmpty else {
            return "0 of \(browser.totalChildren)"
        }
        let first = browser.pageOffset + 1
        let last = browser.pageOffset + UInt64(browser.nodes.count)
        return "\(first)–\(last) of \(browser.totalChildren)"
    }

    @ViewBuilder
    private var rustTargetDryRunBanner: some View {
        if browser.rustTargetDryRunState != .idle {
            GroupBox("Dry check — no files changed") {
                VStack(alignment: .leading, spacing: 8) {
                    switch browser.rustTargetDryRunState {
                    case .idle:
                        EmptyView()
                    case let .starting(info):
                        ProgressView("Checking current Rust target…")
                        Text(
                            "Repeating every deterministic safety check for \(info.target.display). This task has no filesystem mutation authority."
                        )
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        dryRunCancelButton
                    case let .observing(_, poll):
                        dryRunPollBanner(poll)
                    case let .startFailed(_, failure):
                        Label(
                            failure.title,
                            systemImage: "exclamationmark.triangle.fill"
                        )
                        .foregroundStyle(.orange)
                        Text(verbatim: failure.detail)
                            .font(.caption)
                            .foregroundStyle(.secondary)
                        Text("No files changed · 0 B freed.")
                            .font(.caption.weight(.semibold))
                        dryRunConsumedPreviewNote
                        dryRunDismissButton
                    case .observationFailed:
                        Label(
                            "Dry-check status unavailable",
                            systemImage: "exclamationmark.triangle.fill"
                        )
                        .foregroundStyle(.orange)
                        Text(
                            "DUX could not continue observing the path-free task status. The task had no permission to change files; inspect Cleanup History before trying again."
                        )
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        Text("No files changed · 0 B freed.")
                            .font(.caption.weight(.semibold))
                        dryRunConsumedPreviewNote
                        dryRunDismissButton
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .accessibilityIdentifier(
                    ExplorerAccessibility.snapshotCandidateDryRunStatus
                )
            }
        }
    }

    @ViewBuilder
    private func dryRunPollBanner(_ poll: ExplorerRustTargetDryRunPoll) -> some View {
        switch poll.phase {
        case .queued, .running:
            ProgressView(
                poll.cancellationRequested
                    ? "Cancelling dry check…"
                    : "Checking current Rust target…"
            )
            Text(
                "This is a point-in-time validation. Closing Explorer does not cancel the core-owned task."
            )
            .font(.caption)
            .foregroundStyle(.secondary)
            if !poll.cancellationRequested {
                dryRunCancelButton
            }
        case .succeeded:
            if let result = poll.result {
                Label(
                    dryRunResultTitle(result.status),
                    systemImage: result.status == .dryRun
                        ? "checkmark.shield.fill"
                        : "exclamationmark.triangle.fill"
                )
                .foregroundStyle(result.status == .dryRun ? .green : .orange)
                Text("No files changed · 0 B freed.")
                    .font(.caption.weight(.semibold))
                Text(
                    result.status == .dryRun
                        ? "The reviewed target would pass the safety checks at that moment. This is not approval for a later cleanup."
                        : "The validation result was recorded without changing the target."
                )
                .font(.caption)
                .foregroundStyle(.secondary)
                Text(verbatim: "History session \(result.sessionID)")
                    .font(.caption.monospaced())
                    .textSelection(.enabled)
            }
            dryRunConsumedPreviewNote
            dryRunDismissButton
        case .failed:
            let failure = poll.failure ?? .internalState
            Label(failure.title, systemImage: "exclamationmark.triangle.fill")
                .foregroundStyle(.orange)
            Text(verbatim: failure.detail)
                .font(.caption)
                .foregroundStyle(.secondary)
            Text("No files changed · 0 B freed.")
                .font(.caption.weight(.semibold))
            dryRunConsumedPreviewNote
            dryRunDismissButton
        case .cancelled:
            Label("Dry check cancelled", systemImage: "stop.circle.fill")
                .foregroundStyle(.orange)
            Text("No files changed · 0 B freed.")
                .font(.caption.weight(.semibold))
            dryRunConsumedPreviewNote
            dryRunDismissButton
        }
    }

    private var dryRunCancelButton: some View {
        Button("Cancel dry check") {
            Task { await browser.cancelRustTargetDryRun() }
        }
        .accessibilityIdentifier(
            ExplorerAccessibility.snapshotCandidateDryRunCancel
        )
    }

    private var dryRunDismissButton: some View {
        Button("Dismiss result") {
            Task { await browser.dismissRustTargetDryRunResult() }
        }
        .accessibilityIdentifier(
            ExplorerAccessibility.snapshotCandidateDryRunDismiss
        )
    }

    private var dryRunConsumedPreviewNote: some View {
        Text(
            "This preview was consumed. Prepare a fresh preview before another dry check or cleanup."
        )
        .font(.caption)
        .foregroundStyle(.secondary)
    }

    private func dryRunResultTitle(_ status: CleanupHistorySessionStatus) -> String {
        switch status {
        case .dryRun:
            "Checks passed at that moment"
        case .rejected:
            "Dry check rejected"
        case .failed:
            "Dry check could not validate the target"
        case .interrupted:
            "Dry check was interrupted"
        case .cancelled:
            "Dry check cancelled"
        case .planned, .running, .recovering, .completed, .partiallyCompleted:
            "Dry-check result rejected"
        }
    }

    private func largeFilesStatus(_ page: ExplorerSnapshotLargeFilesPage) -> String {
        let totalSize = StorageByteFormatter.string(from: page.totalMatchingLogicalBytes)
        let summary = "\(page.totalMatchingFiles) matching files · \(totalSize) observed"
        guard page.hasMore else {
            return "\(summary) · Showing all matches"
        }
        let omitted = page.totalMatchingFiles - UInt64(page.files.count)
        return "\(summary) · Showing the \(page.files.count) largest; \(omitted) additional matches omitted"
    }

    private var iCloudObservationCheckButtonTitle: String {
        switch browser.iCloudObservationBatchPhase {
        case .idle:
            "Check iCloud status"
        case .checking, .stopping:
            "Checking…"
        case .completed, .cancelled, .stopped:
            "Check again"
        }
    }

    private func iCloudObservationSourceSummary(
        _ source: ExplorerICloudObservationSource
    ) -> String {
        let coverage = "\(source.visitedNodeCount) snapshot descendants inspected"
        guard source.hasMore else {
            return "\(source.targets.count) files ranked · \(coverage) · Showing all ranked files"
        }
        let omitted = source.totalRankedFiles - UInt64(source.targets.count)
        return "\(source.totalRankedFiles) files ranked · \(coverage) · Showing \(source.targets.count); \(omitted) omitted by the fixed safety limit"
    }

    private var iCloudObservationBatchStatus: String {
        let syncFavorable = browser.iCloudObservationResults.values.reduce(into: 0) {
            count, result in
            if case let .observed(assessment) = result,
               assessment.isEligibleObservation
            {
                count += 1
            }
        }
        let syncBlocked = browser.iCloudObservationResults.values.reduce(into: 0) {
            count, result in
            if case let .observed(assessment) = result,
               !assessment.isEligibleObservation
            {
                count += 1
            }
        }
        let failed = browser.iCloudObservationResults.values.reduce(into: 0) {
            count, result in
            if case .failed = result {
                count += 1
            }
        }
        let identityReady = browser.iCloudObservationResults.values.reduce(into: 0) {
            count, result in
            if case let .observed(assessment) = result,
               assessment.isIdentityReady
            {
                count += 1
            }
        }
        let identityIncomplete = browser.iCloudObservationResults.values.reduce(into: 0) {
            count, result in
            if case let .observed(assessment) = result,
               !assessment.isIdentityReady
            {
                count += 1
            }
        }
        let counts =
            "Sync: \(syncFavorable) favorable, \(syncBlocked) blocked · "
            + "Identity: \(identityReady) ready, \(identityIncomplete) incomplete · "
            + "\(failed) failed"
        return switch browser.iCloudObservationBatchPhase {
        case .idle:
            "No live metadata checks have run. \(counts)."
        case let .checking(completed, total):
            "Checking serially: \(completed) of \(total) finished. \(counts)."
        case let .stopping(completed, total):
            "Stopping after the current system metadata read: \(completed) of \(total) finished. \(counts)."
        case let .completed(total):
            "Finished \(total) point-in-time checks. \(counts)."
        case let .cancelled(completed, total):
            "Stopped with \(completed) of \(total) checks retained. Unchecked rows are not negative results. \(counts)."
        case let .stopped(error, completed, total):
            "Stopped after \(completed) of \(total): \(iCloudObservationProbeErrorLabel(error)). \(counts)."
        }
    }

    private func iCloudObservationSourceFailureTitle(
        _ error: ExplorerICloudObservationSourceError
    ) -> String {
        switch error {
        case .budgetExceeded:
            "Folder is too large for this check"
        case .expired:
            "Snapshot review expired"
        case .invalidRequest, .invalidResponse:
            "Observation source was rejected"
        case .reviewNotAcquired, .unavailable:
            "Observation source unavailable"
        }
    }

    private func iCloudObservationSourceFailureDetail(
        _ error: ExplorerICloudObservationSourceError
    ) -> String {
        switch error {
        case .budgetExceeded:
            "Use Browse to choose a smaller folder, then return to iCloud Status. No partial source was used."
        case .expired:
            "Reload Explorer to acquire the snapshot again. No files were changed."
        case .invalidRequest, .invalidResponse:
            "DUX rejected inconsistent bounded-source data instead of running live metadata checks."
        case .reviewNotAcquired, .unavailable:
            "The retained snapshot cannot provide this bounded source right now. No files were changed."
        }
    }

    private func iCloudObservationProbeErrorLabel(
        _ error: ExplorerICloudLocalCopyProbeError
    ) -> String {
        switch error {
        case .unavailable:
            "iCloud metadata unavailable"
        case .invalidTarget:
            "Not a checkable regular file"
        case .changedSinceSnapshot:
            "Changed since scan"
        case .unsupported:
            "Unsupported on this system"
        case .failed:
            "Metadata check failed"
        case .invalidResponse:
            "Invalid metadata response"
        }
    }

    private func largeFileShare(
        _ file: ExplorerSnapshotLargeFile,
        page: ExplorerSnapshotLargeFilesPage
    ) -> Double {
        guard page.totalMatchingLogicalBytes > 0 else {
            return 0
        }
        return min(
            Double(file.node.logicalBytes) / Double(page.totalMatchingLogicalBytes),
            1
        )
    }

    private var selectedSnapshotTitle: String {
        guard let scan = browser.selectedHistoricalScan else {
            if browser.isHistoryLoading {
                return "Loading recent scans"
            }
            return browser.isLatestSnapshot ? "Latest available" : "Selected snapshot"
        }
        let date = (scan.completedAt ?? scan.startedAt).formatted(
            date: .abbreviated,
            time: .shortened
        )
        return "Snapshot · \(date)"
    }

    private func canOpenHistoryRow(_ scan: ExplorerHistoricalScan) -> Bool {
        scan.canRequestReview
            && scan.scanID != browser.scanID
            && !browser.unavailableHistoricalScanIDs.contains(scan.scanID)
    }

    private func historyRowTitle(_ scan: ExplorerHistoricalScan) -> String {
        let date = (scan.completedAt ?? scan.startedAt).formatted(
            date: .abbreviated,
            time: .shortened
        )
        var details = [date, historyRowState(scan)]
        if let bytes = scan.counts?.logicalBytes {
            details.append(StorageByteFormatter.string(from: bytes))
        }
        details.append(scan.coverage.title)
        if scan.issueCount > 0 {
            details.append("\(scan.issueCount) issues")
        }
        return details.joined(separator: " · ")
    }

    private func historyRowState(_ scan: ExplorerHistoricalScan) -> String {
        if scan.scanID == browser.scanID {
            return "Current"
        }
        if browser.unavailableHistoricalScanIDs.contains(scan.scanID) {
            return "No longer available"
        }
        switch scan.status {
        case .queued: return "Queued"
        case .running: return "In progress"
        case .failed: return "Failed"
        case .cancelled: return "Cancelled"
        case .interrupted: return "Interrupted"
        case .succeeded:
            return scan.snapshotRecorded ? "Open snapshot" : "No snapshot recorded"
        }
    }

    private func historyRowSymbol(_ scan: ExplorerHistoricalScan) -> String {
        if scan.scanID == browser.scanID { return "checkmark.circle.fill" }
        if browser.unavailableHistoricalScanIDs.contains(scan.scanID) { return "xmark.circle" }
        switch scan.status {
        case .queued: return "clock"
        case .running: return "progress.indicator"
        case .succeeded: return scan.snapshotRecorded ? "internaldrive" : "nosign"
        case .failed: return "exclamationmark.triangle"
        case .cancelled, .interrupted: return "stop.circle"
        }
    }

    private func share(for node: ExplorerSnapshotNode) -> Double {
        guard let parentBytes = browser.currentDirectory?.logicalBytes, parentBytes > 0 else {
            return 0
        }
        return min(Double(node.logicalBytes) / Double(parentBytes), 1)
    }
}

private struct ExplorerICloudObservationInspectorView: View {
    @Bindable var browser: ExplorerSnapshotBrowserModel

    var body: some View {
        Group {
            if
                let node = browser.selectedNode,
                let target = browser.iCloudObservationSource?.targets.first(
                    where: { $0.id == node.id }
                )
            {
                ScrollView {
                    VStack(alignment: .leading, spacing: 14) {
                        Text(verbatim: node.name.display)
                            .font(.title3.bold())
                            .textSelection(.enabled)
                        LabeledContent(
                            "Historical location",
                            value: target.parentDisplay.isEmpty
                                ? "Top level"
                                : target.parentDisplay
                        )
                        LabeledContent(
                            "Allocated in scan",
                            value: node.allocatedBytes.map {
                                StorageByteFormatter.string(from: $0)
                            } ?? "Unavailable"
                        )
                        LabeledContent(
                            "Logical size",
                            value: StorageByteFormatter.string(
                                from: node.logicalBytes
                            )
                        )
                        Divider()
                        observationResult
                        Divider()
                        Label(
                            "Discovery only. No cleanup action is available.",
                            systemImage: "hand.raised.fill"
                        )
                        .font(.callout.weight(.semibold))
                        Text(
                            "Favorable sync metadata and complete identity proof are separate requirements. This check creates no candidate or action. If local-copy removal is added later, the file would stay in iCloud and require a network connection to download again."
                        )
                        .font(.caption)
                        .foregroundStyle(.secondary)
                    }
                    .padding()
                }
            } else {
                ContentUnavailableView(
                    "Select a file",
                    systemImage: "icloud",
                    description: Text(
                        "Choose a ranked row to inspect its historical allocation and point-in-time iCloud result."
                    )
                )
            }
        }
        .accessibilityIdentifier(
            ExplorerAccessibility.snapshotICloudObservationInspector
        )
    }

    @ViewBuilder
    private var observationResult: some View {
        switch browser.selectedICloudObservationResult {
        case let .observed(assessment):
            Text("Sync metadata")
                .font(.headline)
            if assessment.isEligibleObservation {
                Label(
                    "Favorable point-in-time observation",
                    systemImage: "checkmark.circle.fill"
                )
                .foregroundStyle(.green)
            } else {
                Label(
                    "Sync metadata has blockers",
                    systemImage: "nosign"
                )
                .foregroundStyle(.orange)
                ForEach(
                    Array(assessment.blockers.enumerated()),
                    id: \.offset
                ) { _, blocker in
                    Text("• \(blockerTitle(blocker))")
                        .font(.callout)
                }
            }
            Text(verbatim: assessment.reviewDetail)
                .font(.caption)
                .foregroundStyle(.secondary)

            Divider()
            Text("Identity proof")
                .font(.headline)
            Label(
                assessment.identityReadinessTitle,
                systemImage: assessment.isIdentityReady
                    ? "checkmark.shield.fill"
                    : "exclamationmark.shield.fill"
            )
            .foregroundStyle(
                assessment.isIdentityReady ? Color.green : Color.orange
            )
            Text(verbatim: assessment.identityReadinessDetail)
                .font(.caption)
                .foregroundStyle(.secondary)
            ForEach(assessment.identityFactRows) { fact in
                LabeledContent(fact.label) {
                    Text(verbatim: fact.value)
                        .multilineTextAlignment(.trailing)
                }
                .font(.callout)
            }
            if !assessment.identityBlockers.isEmpty {
                VStack(alignment: .leading, spacing: 5) {
                    ForEach(
                        Array(assessment.identityBlockers.enumerated()),
                        id: \.offset
                    ) { _, blocker in
                        Label(blocker.displayText, systemImage: "xmark.shield")
                            .font(.callout)
                    }
                }
                .accessibilityElement(children: .combine)
                .accessibilityLabel(
                    "Identity blockers. "
                        + assessment.identityBlockers
                        .map(\.displayText)
                        .joined(separator: " ")
                )
            }

            Text(
                Date(
                    timeIntervalSince1970:
                        Double(assessment.observedAtUnixMilliseconds) / 1_000
                )
                .formatted(date: .abbreviated, time: .standard)
            )
            .font(.caption)
            .foregroundStyle(.secondary)
        case let .failed(error):
            Label(failureTitle(error), systemImage: "exclamationmark.triangle.fill")
                .foregroundStyle(.orange)
        case nil:
            Text("Not checked")
                .foregroundStyle(.secondary)
        }
    }

    private func failureTitle(
        _ error: ExplorerICloudLocalCopyProbeError
    ) -> String {
        switch error {
        case .unavailable: "iCloud metadata unavailable"
        case .invalidTarget: "Not a checkable regular file"
        case .changedSinceSnapshot: "File changed since this scan"
        case .unsupported: "Unsupported on this system"
        case .failed: "Metadata check failed"
        case .invalidResponse: "Metadata response was rejected"
        }
    }

    private func blockerTitle(
        _ blocker: ExplorerICloudLocalCopyBlockReason
    ) -> String {
        switch blocker {
        case .unsupportedItemKind: "Unsupported item kind"
        case .ubiquityUnknown: "iCloud identity is unknown"
        case .notUbiquitous: "Not identified as an iCloud file"
        case .uploadStateUnknown: "Upload state is unknown"
        case .uploadIncomplete: "Upload is incomplete"
        case .uploadActivityUnknown: "Upload activity is unknown"
        case .uploadInProgress: "Upload is in progress"
        case .uploadErrorUnknown: "Upload error state is unknown"
        case .uploadErrorPresent: "An upload error is present"
        case .conflictStateUnknown: "Conflict state is unknown"
        case .unresolvedConflicts: "The file has unresolved conflicts"
        case .localCopyStateUnknown: "Local-copy state is unknown"
        case .staleLocalCopy: "The local copy is stale"
        case .noLocalCopy: "No local copy is present"
        case .downloadRequestUnknown: "Download-request state is unknown"
        case .downloadRequested: "A download was requested"
        case .downloadActivityUnknown: "Download activity is unknown"
        case .downloadInProgress: "A download is in progress"
        case .downloadErrorUnknown: "Download error state is unknown"
        case .downloadErrorPresent: "A download error is present"
        case .syncExclusionUnknown: "Sync-exclusion state is unknown"
        case .excludedFromSync: "The item is excluded from sync"
        case .allocationUnknown: "Local allocation is unknown"
        case .noLocalAllocation: "No local allocation was recorded"
        case .invalidObservationTime: "Observation time was invalid"
        }
    }
}

private struct ExplorerCandidateInspectorView: View {
    @Bindable var browser: ExplorerSnapshotBrowserModel
    let permanentCleanupAvailability: PermanentCleanupActionAvailability
    let openSettingsDestination: () -> Void
    let confirmCleanup: (ExplorerRustTargetCleanupConfirmation) -> Void

    var body: some View {
        Group {
            if let candidate = browser.selectedCandidate {
                selectedContent(candidate)
            } else {
                ContentUnavailableView(
                    "Select a candidate",
                    systemImage: "list.bullet.rectangle",
                    description: Text(
                        "Choose a row to inspect its exact historical paths, blockers, and deterministic evidence."
                    )
                )
            }
        }
        .accessibilityIdentifier(ExplorerAccessibility.snapshotCandidateInspector)
    }

    private func selectedContent(_ candidate: ExplorerCandidateSummary) -> some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 14) {
                VStack(alignment: .leading, spacing: 4) {
                    Text(verbatim: candidate.ruleDisplayName)
                        .font(.title3.bold())
                        .textSelection(.enabled)
                    Text(
                        verbatim:
                        "\(candidate.ruleID) · revision \(candidate.ruleRevision) · \(candidate.category.displayName)"
                    )
                    .font(.caption)
                    .foregroundStyle(.secondary)
                }

                Label(
                    "Historical discovery evidence. The permanent-safe check below performs deterministic live validation. It does not use AI, approve cleanup, or remove files.",
                    systemImage: "checkmark.shield"
                )
                .font(.callout)
                .foregroundStyle(.secondary)

                Text(verbatim: candidateDetailStatus)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .accessibilityIdentifier(
                        ExplorerAccessibility.snapshotCandidateDetailStatus
                    )

                GroupBox("Candidate") {
                    VStack(alignment: .leading, spacing: 8) {
                        detailRow(
                            "Observed estimate",
                            StorageByteFormatter.string(from: candidate.estimatedBytes)
                        )
                        detailRow("Safety label", candidate.safety.displayName)
                        detailRow("Proposed action", candidate.action.displayName)
                        detailRow("Review status", candidate.status.displayName)
                        detailRow("Paths", "\(candidate.pathCount)")
                        detailRow("Observed", timestamp(candidate.createdAt))
                        detailRow(
                            "Schedule eligible",
                            candidate.ruleScheduleEligible ? "Yes" : "No"
                        )
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                }

                GroupBox("Discovery blockers") {
                    if candidate.blockers.isEmpty {
                        Label(
                            "No blockers were recorded, but this observation is not cleanup approval.",
                            systemImage: "info.circle"
                        )
                        .foregroundStyle(.secondary)
                    } else {
                        VStack(alignment: .leading, spacing: 7) {
                            ForEach(
                                Array(candidate.blockers.enumerated()),
                                id: \.offset
                            ) { _, blocker in
                                Label(blocker.displayName, systemImage: "lock.trianglebadge.exclamationmark")
                                    .foregroundStyle(.orange)
                            }
                        }
                        .frame(maxWidth: .infinity, alignment: .leading)
                    }
                }

                if browser.isSelectedRustTargetPlanReviewCandidate {
                    rustTargetPlanReviewSection(candidate)
                }

                if let failure = browser.candidateDetailFailure {
                    VStack(alignment: .leading, spacing: 8) {
                        Label(failure.title, systemImage: "exclamationmark.triangle.fill")
                            .foregroundStyle(.orange)
                        Text(verbatim: failure.detail)
                            .font(.caption)
                            .foregroundStyle(.secondary)
                        Button("Retry detail") {
                            Task { await browser.reloadSelectedCandidateDetail() }
                        }
                    }
                    .padding(10)
                    .background(Color.orange.opacity(0.10), in: RoundedRectangle(cornerRadius: 8))
                }

                if browser.isCandidateDetailLoading {
                    ProgressView("Loading exact candidate evidence…")
                        .frame(maxWidth: .infinity, alignment: .leading)
                } else {
                    pathsSection
                    evidenceSection
                }
            }
            .padding(14)
        }
    }

    private func rustTargetPlanReviewSection(
        _: ExplorerCandidateSummary
    ) -> some View {
        GroupBox("Permanent-safe plan preview") {
            VStack(alignment: .leading, spacing: 10) {
                switch browser.rustTargetPlanReviewState {
                case .idle:
                    if browser.rustTargetDryRunState != .idle {
                        Label(
                            "The dry-check result is shown above the snapshot browser.",
                            systemImage: "arrow.up.circle"
                        )
                        .foregroundStyle(.secondary)
                    } else if browser.rustTargetCleanupState == .idle {
                        Label(
                            "DUX can revalidate this exact Rust target and prepare a short-lived in-memory preview. Preparing it changes no files and records no approval.",
                            systemImage: "checkmark.shield"
                        )
                        .foregroundStyle(.secondary)
                        Button("Prepare plan preview") {
                            Task { await browser.prepareSelectedRustTargetPlanReview() }
                        }
                        .disabled(!browser.canPrepareRustTargetPlanReview)
                        .accessibilityIdentifier(
                            ExplorerAccessibility.snapshotCandidatePlanReviewPrepare
                        )
                        .accessibilityHint(
                            "Performs deterministic live checks without approving or removing files"
                        )
                    } else {
                        Label(
                            "The confirmed cleanup and its path-free status are shown above the snapshot browser.",
                            systemImage: "arrow.up.circle"
                        )
                        .foregroundStyle(.secondary)
                    }
                case .preparing:
                    ProgressView("Performing deterministic live checks…")
                        .accessibilityIdentifier(
                            ExplorerAccessibility.snapshotCandidatePlanReviewStatus
                        )
                case let .ready(info):
                    Label("Ready for review", systemImage: "checkmark.circle.fill")
                        .foregroundStyle(.green)
                        .accessibilityIdentifier(
                            ExplorerAccessibility.snapshotCandidatePlanReviewStatus
                        )
                    VStack(alignment: .leading, spacing: 7) {
                        detailRow("Mode", info.mode.displayName)
                        detailRow(
                            "Estimated reclaimable",
                            StorageByteFormatter.string(from: info.estimatedBytes)
                        )
                        detailRow("Scope", "\(info.itemCount) candidate · \(info.pathCount) exact target")
                        detailRow("Rule", "\(info.ruleID) revision \(info.ruleRevision)")
                        detailRow("Newest observed change", timestamp(info.newestMtime))
                        detailRow(
                            "Required inactivity",
                            minimumAgeDescription(
                                seconds: info.minimumAgeSeconds,
                                nanoseconds: info.minimumAgeNanoseconds
                            )
                        )
                        detailRow("Created", timestamp(info.createdAt))
                        detailRow("Current until", timestamp(info.effectiveExpiresAt))
                        VStack(alignment: .leading, spacing: 3) {
                            Text("Exact current target")
                                .foregroundStyle(.secondary)
                            Text(verbatim: info.target.display)
                                .font(.system(.callout, design: .monospaced))
                                .textSelection(.enabled)
                                .lineLimit(4)
                        }
                    }
                    .accessibilityElement(children: .combine)
                    .accessibilityLabel(
                        "Permanent-safe plan preview. Estimated \(StorageByteFormatter.string(from: info.estimatedBytes)). Newest observed change \(timestamp(info.newestMtime)); required inactivity \(minimumAgeDescription(seconds: info.minimumAgeSeconds, nanoseconds: info.minimumAgeNanoseconds)). Current until \(timestamp(info.effectiveExpiresAt)). Exact current target \(accessibilityPlanTarget(info.target.display)). Not approved; no files changed."
                    )
                    .accessibilityIdentifier(
                        ExplorerAccessibility.snapshotCandidatePlanReviewSummary
                    )
                    VStack(alignment: .leading, spacing: 6) {
                        ForEach(Array(info.warnings.enumerated()), id: \.offset) { _, warning in
                            Label(warning.displayName, systemImage: "exclamationmark.triangle")
                                .foregroundStyle(.orange)
                        }
                    }
                    .accessibilityIdentifier(
                        ExplorerAccessibility.snapshotCandidatePlanReviewWarnings
                    )
                    Text(
                        "No cleanup has been approved or performed. Closing this preview releases it."
                    )
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    HStack {
                        Button("Run dry check") {
                            Task { await browser.startRustTargetDryRun() }
                        }
                        .accessibilityIdentifier(
                            ExplorerAccessibility.snapshotCandidateDryRunStart
                        )
                        .accessibilityHint(
                            "Repeats all current safety checks and records the result. Changes no files."
                        )
#if DUX_INTERNAL_PERMANENT_SAFE_CLEANUP
                        switch permanentCleanupAvailability {
                        case .enabled:
                            Button("Remove build output…", role: .destructive) {
                                if let confirmation =
                                    browser.makeRustTargetCleanupConfirmation()
                                {
                                    confirmCleanup(confirmation)
                                }
                            }
                            .accessibilityIdentifier(
                                ExplorerAccessibility.snapshotCandidateCleanupPrepare
                            )
                            .accessibilityHint(
                                "Opens a destructive confirmation for this exact reviewed plan"
                            )
                        case .disabled:
                            Button("Enable permanent cleanup in Settings…") {
                                openSettingsDestination()
                            }
                            .accessibilityIdentifier(
                                ExplorerAccessibility.snapshotCandidateCleanupSettings
                            )
                            .accessibilityHint(
                                "Opens the deny-by-default permanent cleanup safety setting"
                            )
                        case .loading:
                            Label(
                                "Loading cleanup safety setting…",
                                systemImage: "hourglass"
                            )
                            .foregroundStyle(.secondary)
                        case .unavailable:
                            Button("Review cleanup safety in Settings…") {
                                openSettingsDestination()
                            }
                            .accessibilityIdentifier(
                                ExplorerAccessibility.snapshotCandidateCleanupSettings
                            )
                            .accessibilityHint(
                                "Opens Settings because cleanup safety could not be verified"
                            )
                        }
#else
                        Label(
                            "Cleanup execution is unavailable in this build",
                            systemImage: "lock.shield"
                        )
                        .foregroundStyle(.secondary)
#endif
                        Button("Check again") {
                            Task { await browser.prepareSelectedRustTargetPlanReview() }
                        }
                        .accessibilityIdentifier(
                            ExplorerAccessibility.snapshotCandidatePlanReviewRefresh
                        )
                        Button("Close preview") {
                            Task { await browser.closeRustTargetPlanReview() }
                        }
                        .accessibilityIdentifier(
                            ExplorerAccessibility.snapshotCandidatePlanReviewClose
                        )
                    }
                case let .failed(error):
                    Label(error.title, systemImage: "exclamationmark.triangle.fill")
                        .foregroundStyle(.orange)
                        .accessibilityIdentifier(
                            ExplorerAccessibility.snapshotCandidatePlanReviewStatus
                        )
                    Text(verbatim: error.detail)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                    Button("Check again") {
                        Task { await browser.prepareSelectedRustTargetPlanReview() }
                    }
                    .disabled(!browser.canPrepareRustTargetPlanReview)
                    .accessibilityIdentifier(
                        ExplorerAccessibility.snapshotCandidatePlanReviewRefresh
                    )
                case .expired:
                    Label("Plan preview expired", systemImage: "clock.badge.exclamationmark")
                        .foregroundStyle(.orange)
                        .accessibilityIdentifier(
                            ExplorerAccessibility.snapshotCandidatePlanReviewStatus
                        )
                    Text(
                        "This preview is no longer current. Prepare a new preview to repeat the live checks."
                    )
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    Button("Prepare new preview") {
                        Task { await browser.prepareSelectedRustTargetPlanReview() }
                    }
                    .disabled(!browser.canPrepareRustTargetPlanReview)
                    .accessibilityIdentifier(
                        ExplorerAccessibility.snapshotCandidatePlanReviewRefresh
                    )
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .accessibilityIdentifier(ExplorerAccessibility.snapshotCandidatePlanReview)
    }

    @ViewBuilder
    private var pathsSection: some View {
        if let page = browser.candidatePathPage {
            GroupBox("Historical paths") {
                VStack(alignment: .leading, spacing: 10) {
                    ForEach(Array(page.paths.enumerated()), id: \.offset) { _, path in
                        VStack(alignment: .leading, spacing: 3) {
                            Text(verbatim: path.display)
                                .font(.system(.callout, design: .monospaced))
                                .textSelection(.enabled)
                                .lineLimit(4)
                            Text(verbatim: path.encoding.displayName)
                                .font(.caption2)
                                .foregroundStyle(.tertiary)
                        }
                        .frame(maxWidth: .infinity, alignment: .leading)
                    }
                    Divider()
                    HStack {
                        Text(
                            verbatim: pageStatus(
                                cursor: page.cursor,
                                count: page.paths.count,
                                total: page.totalPaths
                            )
                        )
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .monospacedDigit()
                        Spacer()
                        if browser.isCandidatePathPaging {
                            ProgressView()
                                .controlSize(.small)
                        }
                        Button {
                            Task { await browser.showPreviousCandidatePathPage() }
                        } label: {
                            Label("Previous paths", systemImage: "chevron.left")
                                .labelStyle(.iconOnly)
                        }
                        .disabled(
                            !browser.hasPreviousCandidatePathPage
                                || browser.isCandidatePathPaging
                        )
                        .accessibilityIdentifier(
                            ExplorerAccessibility.snapshotCandidatePathPrevious
                        )
                        Button {
                            Task { await browser.showNextCandidatePathPage() }
                        } label: {
                            Label("Next paths", systemImage: "chevron.right")
                                .labelStyle(.iconOnly)
                        }
                        .disabled(
                            !browser.hasNextCandidatePathPage
                                || browser.isCandidatePathPaging
                        )
                        .accessibilityIdentifier(
                            ExplorerAccessibility.snapshotCandidatePathNext
                        )
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            .accessibilityIdentifier(ExplorerAccessibility.snapshotCandidatePaths)
        }
    }

    @ViewBuilder
    private var evidenceSection: some View {
        if let page = browser.candidateEvidencePage {
            GroupBox("Deterministic evidence") {
                VStack(alignment: .leading, spacing: 10) {
                    ForEach(page.evidence, id: \.ordinal) { evidence in
                        VStack(alignment: .leading, spacing: 3) {
                            Text(verbatim: evidence.kind.displayName)
                                .font(.callout.weight(.semibold))
                            if let detail = evidenceDetail(evidence) {
                                Text(verbatim: detail)
                                    .font(.caption)
                                    .foregroundStyle(.secondary)
                                    .textSelection(.enabled)
                                    .lineLimit(4)
                            }
                        }
                        .frame(maxWidth: .infinity, alignment: .leading)
                    }
                    Divider()
                    HStack {
                        Text(
                            verbatim: pageStatus(
                                cursor: page.cursor,
                                count: page.evidence.count,
                                total: page.totalEvidence
                            )
                        )
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .monospacedDigit()
                        Spacer()
                        if browser.isCandidateEvidencePaging {
                            ProgressView()
                                .controlSize(.small)
                        }
                        Button {
                            Task { await browser.showPreviousCandidateEvidencePage() }
                        } label: {
                            Label("Previous evidence", systemImage: "chevron.left")
                                .labelStyle(.iconOnly)
                        }
                        .disabled(
                            !browser.hasPreviousCandidateEvidencePage
                                || browser.isCandidateEvidencePaging
                        )
                        .accessibilityIdentifier(
                            ExplorerAccessibility.snapshotCandidateEvidencePrevious
                        )
                        Button {
                            Task { await browser.showNextCandidateEvidencePage() }
                        } label: {
                            Label("Next evidence", systemImage: "chevron.right")
                                .labelStyle(.iconOnly)
                        }
                        .disabled(
                            !browser.hasNextCandidateEvidencePage
                                || browser.isCandidateEvidencePaging
                        )
                        .accessibilityIdentifier(
                            ExplorerAccessibility.snapshotCandidateEvidenceNext
                        )
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            .accessibilityIdentifier(ExplorerAccessibility.snapshotCandidateEvidence)
        }
    }

    private func detailRow(_ label: String, _ value: String) -> some View {
        HStack(alignment: .firstTextBaseline) {
            Text(verbatim: label)
                .foregroundStyle(.secondary)
            Spacer(minLength: 12)
            Text(verbatim: value)
                .multilineTextAlignment(.trailing)
                .textSelection(.enabled)
        }
        .font(.callout)
    }

    private func pageStatus(cursor: UInt16, count: Int, total: UInt16) -> String {
        guard count > 0 else {
            return "0 of \(total)"
        }
        return "\(Int(cursor) + 1)–\(Int(cursor) + count) of \(total)"
    }

    private func accessibilityPlanTarget(_ value: String) -> String {
        let limit = 512
        guard value.count > limit else {
            return value
        }
        return "\(value.prefix(limit))…"
    }

    private var candidateDetailStatus: String {
        if browser.isCandidateDetailLoading {
            return "Loading exact historical paths and deterministic evidence."
        }
        if browser.candidateDetailFailure != nil {
            return "Candidate detail is unavailable. No path or evidence was accepted."
        }
        let paths = browser.candidatePathPage.map {
            "Paths \(pageStatus(cursor: $0.cursor, count: $0.paths.count, total: $0.totalPaths))"
        } ?? "Paths unavailable"
        let evidence = browser.candidateEvidencePage.map {
            "Evidence \(pageStatus(cursor: $0.cursor, count: $0.evidence.count, total: $0.totalEvidence))"
        } ?? "Evidence unavailable"
        if browser.isCandidatePathPaging {
            return "\(paths). \(evidence). Updating paths."
        }
        if browser.isCandidateEvidencePaging {
            return "\(paths). \(evidence). Updating evidence."
        }
        return "\(paths). \(evidence)."
    }

    private func timestamp(_ value: ExplorerSnapshotTimestamp) -> String {
        let interval = Double(value.secondsSinceUnixEpoch)
            + Double(value.nanoseconds) / 1_000_000_000
        return Date(timeIntervalSince1970: interval).formatted(
            date: .abbreviated,
            time: .shortened
        )
    }

    private func evidenceDetail(_ evidence: ExplorerCandidateEvidence) -> String? {
        if let path = evidence.path {
            if let identifier = evidence.identifier {
                return "\(path.display) · \(identifier)"
            }
            return path.display
        }
        if let identifier = evidence.identifier {
            return identifier
        }
        if
            let newestMtime = evidence.newestMtime,
            let minimumAgeSeconds = evidence.minimumAgeSeconds,
            let minimumAgeNanoseconds = evidence.minimumAgeNanoseconds
        {
            return "Newest item \(timestamp(newestMtime)); minimum age \(duration(seconds: minimumAgeSeconds, nanoseconds: minimumAgeNanoseconds))"
        }
        if
            let observedBytes = evidence.observedBytes,
            let minimumBytes = evidence.minimumBytes
        {
            return "\(StorageByteFormatter.string(from: observedBytes)) observed; minimum \(StorageByteFormatter.string(from: minimumBytes))"
        }
        return nil
    }

    private func duration(seconds: UInt64, nanoseconds: UInt32) -> String {
        guard nanoseconds > 0 else {
            return "\(seconds) seconds"
        }
        var fraction = String(format: "%09u", nanoseconds)
        while fraction.last == "0" {
            fraction.removeLast()
        }
        return "\(seconds).\(fraction) seconds"
    }

    private func minimumAgeDescription(seconds: UInt64, nanoseconds: UInt32) -> String {
        let secondsPerDay: UInt64 = 24 * 60 * 60
        if nanoseconds == 0, seconds.isMultiple(of: secondsPerDay) {
            let days = seconds / secondsPerDay
            return "\(days) \(days == 1 ? "day" : "days")"
        }
        return duration(seconds: seconds, nanoseconds: nanoseconds)
    }
}

private extension AppScanCoverage {
    var title: String {
        switch self {
        case .complete: "Complete coverage"
        case .limitedAccess: "Limited access"
        case .partial: "Partial coverage"
        case .unknown: "Unknown coverage"
        }
    }

    var symbol: String {
        switch self {
        case .complete: "checkmark.shield.fill"
        case .limitedAccess: "lock.trianglebadge.exclamationmark"
        case .partial: "chart.pie.fill"
        case .unknown: "questionmark.diamond"
        }
    }
}

private extension ExplorerScanCoverageIssueKind {
    var title: String {
        switch self {
        case .permissionDenied: "Permission denied"
        case .timedOut: "Scan timed out"
        case .differentFilesystem: "Different filesystem"
        case .networkOrVirtualFilesystem: "Network or virtual filesystem"
        case .symlinkSkipped: "Symbolic link skipped"
        case .fileChangedDuringScan: "Changed during scan"
        case .metadataError: "Metadata unavailable"
        case .cancelled: "Scan cancelled"
        case .policyExcluded: "Excluded by scan policy"
        case .depthLimited: "Depth limit reached"
        case .probePoolExhausted: "Probe capacity exhausted"
        case .filesystemBoundaryUnknown: "Filesystem boundary unknown"
        case .issueLimitReached: "Issue detail limit reached"
        }
    }

    var detail: String {
        switch self {
        case .permissionDenied:
            "macOS did not allow this part of the historical scan to be read."
        case .timedOut:
            "DUX stopped waiting for this part of the scan within its fixed time budget."
        case .differentFilesystem:
            "The scan did not cross into a different mounted filesystem."
        case .networkOrVirtualFilesystem:
            "The scan avoided a network-backed or virtual filesystem."
        case .symlinkSkipped:
            "The scan recorded but did not follow a symbolic link."
        case .fileChangedDuringScan:
            "This item changed while the historical scan was measuring it."
        case .metadataError:
            "Required size or file metadata could not be read reliably."
        case .cancelled:
            "The scan ended before all intended work completed."
        case .policyExcluded:
            "The active scan policy intentionally excluded this scope."
        case .depthLimited:
            "The configured traversal-depth budget stopped deeper inspection."
        case .probePoolExhausted:
            "The bounded filesystem-probe pool could not inspect more entries."
        case .filesystemBoundaryUnknown:
            "DUX could not safely determine whether this crossed a filesystem boundary."
        case .issueLimitReached:
            "Additional issue locations were aggregated after the retained detail limit."
        }
    }

    var symbol: String {
        switch self {
        case .permissionDenied: "lock.fill"
        case .timedOut: "clock.badge.exclamationmark"
        case .differentFilesystem, .filesystemBoundaryUnknown: "externaldrive.badge.questionmark"
        case .networkOrVirtualFilesystem: "network"
        case .symlinkSkipped: "link.badge.plus"
        case .fileChangedDuringScan: "arrow.triangle.2.circlepath"
        case .metadataError: "doc.badge.ellipsis"
        case .cancelled: "stop.circle.fill"
        case .policyExcluded: "line.3.horizontal.decrease.circle"
        case .depthLimited: "arrow.down.to.line.compact"
        case .probePoolExhausted: "gauge.with.dots.needle.0percent"
        case .issueLimitReached: "ellipsis.circle.fill"
        }
    }
}

private extension ExplorerSnapshotNodeSort {
    var title: String {
        switch self {
        case .nameAscending: "Name"
        case .logicalBytesDescending: "Logical size"
        case .allocatedBytesDescending: "Allocated size"
        case .modifiedNewest: "Recently modified"
        }
    }
}

private extension ExplorerSnapshotLargeFileThreshold {
    var title: String {
        switch self {
        case .mebibytes100: "100 MiB and larger"
        case .mebibytes500: "500 MiB and larger"
        case .gibibyte1: "1 GiB and larger"
        case .gibibytes5: "5 GiB and larger"
        case .gibibytes10: "10 GiB and larger"
        }
    }
}

private extension ExplorerSnapshotLargeFileAge {
    var title: String {
        switch self {
        case .any: "Any modification date"
        case .days30: "Not modified in 30 days"
        case .days90: "Not modified in 90 days"
        case .year1: "Not modified in 1 year"
        }
    }
}

private extension ExplorerSnapshotNodeKind {
    var title: String {
        switch self {
        case .directory: "Folder"
        case .file: "File"
        case .symlink: "Symbolic link"
        case .other: "Other"
        case .error: "Unavailable item"
        }
    }

    var symbol: String {
        switch self {
        case .directory: "folder.fill"
        case .file: "doc.fill"
        case .symlink: "link"
        case .other: "questionmark.square"
        case .error: "exclamationmark.triangle.fill"
        }
    }
}

private extension ExplorerSnapshotNode {
    var allocatedSizeText: String {
        allocatedBytes.map { StorageByteFormatter.string(from: $0) } ?? "Unavailable"
    }

    var itemSummary: String {
        if kind == .directory {
            return "\(childCount) children · \(fileCount) files"
        }
        return fileCount == 1 ? "1 file" : "\(fileCount) files"
    }

    var modifiedText: String {
        guard
            let modifiedAt,
            modifiedAt.secondsSinceUnixEpoch <= 253_402_300_799
        else {
            return "Unavailable"
        }
        let interval = Double(modifiedAt.secondsSinceUnixEpoch)
            + Double(modifiedAt.nanoseconds) / 1_000_000_000
        return Date(timeIntervalSince1970: interval).formatted(
            date: .abbreviated,
            time: .shortened
        )
    }

    var hasObservationWarning: Bool {
        scanFlags.inaccessible
            || scanFlags.timedOut
            || scanFlags.hardLinkDuplicate
            || scanFlags.mountBoundary
            || kind == .error
    }

    var flagSummary: String {
        var flags: [String] = []
        if scanFlags.inaccessible { flags.append("inaccessible") }
        if scanFlags.timedOut { flags.append("timed out") }
        if scanFlags.hardLinkDuplicate { flags.append("hard-link duplicate") }
        if scanFlags.mountBoundary { flags.append("mount boundary") }
        if kind == .error { flags.append("scan error") }
        return flags.joined(separator: ", ")
    }

    var accessibilitySummary: String {
        var parts = [
            kind.title,
            name.display,
            category.presentation.accessibilityPhrase,
            "logical size \(StorageByteFormatter.string(from: logicalBytes))",
            "allocated size \(allocatedSizeText)",
            itemSummary,
        ]
        if hasObservationWarning {
            parts.append(flagSummary)
        }
        return parts.joined(separator: ", ")
    }
}
