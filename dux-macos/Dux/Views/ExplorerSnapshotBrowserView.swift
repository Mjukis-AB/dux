import Foundation
import SwiftUI

struct ExplorerSnapshotBrowserView: View {
    @Bindable var browser: ExplorerSnapshotBrowserModel
    @State private var presentationID = UUID()
    @State private var inspectorPresented = true
    @State private var trashConfirmationNode: ExplorerSnapshotNode?
    let model: AppModel

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            header
            subtreeScanStatus
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
        .confirmationDialog(
            "Move item to Trash?",
            isPresented: Binding(
                get: { trashConfirmationNode != nil },
                set: { if !$0 { trashConfirmationNode = nil } }
            ),
            presenting: trashConfirmationNode
        ) { node in
            Button("Move \(node.name.display) to Trash", role: .destructive) {
                let nodeID = node.id
                trashConfirmationNode = nil
                Task { await browser.trashSelectedItem(nodeID: nodeID) }
            }
            Button("Cancel", role: .cancel) { trashConfirmationNode = nil }
        } message: { _ in
            Text("DUX will revalidate this reviewed item and record a one-shot operation. Empty Trash separately to reclaim disk space.")
        }
        .accessibilityIdentifier(ExplorerAccessibility.snapshotBrowser)
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
                Text("Candidates").tag(ExplorerSnapshotContentMode.candidates)
                Text("Large Files").tag(ExplorerSnapshotContentMode.largeFiles)
                Text("Coverage").tag(ExplorerSnapshotContentMode.coverage)
            }
            .pickerStyle(.segmented)
            .frame(maxWidth: 420)
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
            } else if browser.contentMode == .candidates {
                candidatesContent
            } else if browser.contentMode == .largeFiles {
                largeFilesContent
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
            ExplorerSnapshotInspectorView(browser: browser)
                .inspectorColumnWidth(min: 230, ideal: 270, max: 330)
        }
        .overlay {
            if browser.contentMode == .browse, browser.isNavigating {
                ZStack {
                    Color.black.opacity(0.08)
                    ProgressView("Opening folder…")
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
                    "Review-only suggestions. These statuses do not create plans or remove files.",
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
                    candidatesTable(page.candidates)
                }
            }
        }
        .accessibilityIdentifier(ExplorerAccessibility.snapshotCandidates)
    }

    private func candidatesTable(_ candidates: [ExplorerCandidateSummary]) -> some View {
        Table(candidates) {
            TableColumn("Rule") { candidate in
                VStack(alignment: .leading, spacing: 2) {
                    Text(verbatim: candidate.ruleID)
                        .font(.headline)
                    Text(verbatim: "Revision (candidate.ruleRevision) · (candidate.category.displayName)")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }
            TableColumn("Observed size") { candidate in
                Text(ByteCountFormatter.string(fromByteCount: Int64(min(candidate.estimatedBytes, UInt64(Int64.max))), countStyle: .file))
                    .monospacedDigit()
            }
            TableColumn("Safety") { candidate in
                Text(verbatim: candidate.safety.displayName)
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

            ExplorerSnapshotTreemapView(browser: browser)
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
                set: { browser.selectTableNode($0) }
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
                }
                .accessibilityElement(children: .combine)
                .accessibilityLabel(node.accessibilitySummary)
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

    @ViewBuilder
    private func liveActionMenu(
        node: ExplorerSnapshotNode,
        fromLargeFiles: Bool
    ) -> some View {
        let supported = node.kind == .file || node.kind == .directory || node.kind == .symlink
        Button("Reveal in Finder") {
            selectForLiveAction(node.id, fromLargeFiles: fromLargeFiles)
            Task { await browser.revealLiveItem(nodeID: node.id) }
        }
        .disabled(!supported)
        Button("Copy Path") {
            selectForLiveAction(node.id, fromLargeFiles: fromLargeFiles)
            Task { await browser.copyLiveItemPath(nodeID: node.id) }
        }
        .disabled(!supported)
        Button("Quick Look") {
            selectForLiveAction(node.id, fromLargeFiles: fromLargeFiles)
            Task { await browser.quickLookLiveItem(nodeID: node.id) }
        }
        .disabled(node.kind != .file)
        Divider()
        Button("Move to Trash…", role: .destructive) {
            selectForLiveAction(node.id, fromLargeFiles: fromLargeFiles)
            trashConfirmationNode = node
        }
        .disabled(!supported || browser.isTrashLoading)
    }

    private func selectForLiveAction(_ nodeID: UInt64, fromLargeFiles: Bool) {
        if fromLargeFiles {
            browser.selectLargeFile(nodeID)
        } else {
            browser.selectTableNode(nodeID)
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

    private func largeFilesStatus(_ page: ExplorerSnapshotLargeFilesPage) -> String {
        let totalSize = StorageByteFormatter.string(from: page.totalMatchingLogicalBytes)
        let summary = "\(page.totalMatchingFiles) matching files · \(totalSize) observed"
        guard page.hasMore else {
            return "\(summary) · Showing all matches"
        }
        let omitted = page.totalMatchingFiles - UInt64(page.files.count)
        return "\(summary) · Showing the \(page.files.count) largest; \(omitted) additional matches omitted"
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
