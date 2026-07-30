import SwiftUI

struct ExplorerSnapshotTreemapView: View {
    @Bindable var browser: ExplorerSnapshotBrowserModel
    @Environment(\.accessibilityDifferentiateWithoutColor) private var differentiateWithoutColor

    var body: some View {
        GroupBox {
            Group {
                if browser.isTreemapLoading {
                    ProgressView("Building bounded view…")
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                } else if let treemap = browser.treemap {
                    treemapContent(treemap)
                } else if let failure = browser.treemapFailure {
                    VStack(spacing: 8) {
                        Label(failure.title, systemImage: "chart.treemap")
                            .font(.headline)
                        Text("The sortable table remains available as the complete textual view.")
                            .foregroundStyle(.secondary)
                            .multilineTextAlignment(.center)
                        Button("Dismiss") {
                            browser.dismissTreemapFailure()
                        }
                    }
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                }
            }
            .frame(minHeight: 170, idealHeight: 220, maxHeight: 260)
        } label: {
            HStack {
                Label("Logical size", systemImage: "chart.treemap")
                Spacer()
                Text("Area represents historical logical bytes")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
        .accessibilityIdentifier(ExplorerAccessibility.snapshotTreemap)
    }

    @ViewBuilder
    private func treemapContent(_ treemap: ExplorerSnapshotTreemap) -> some View {
        if treemap.totalChildLogicalBytes == 0 {
            ContentUnavailableView(
                "No logical size to plot",
                systemImage: "chart.treemap",
                description: Text(
                    "This folder has \(treemap.totalChildren) recorded children, all with zero logical bytes."
                )
            )
        } else {
            GeometryReader { geometry in
                let rectangles = ExplorerTreemapLayout.rectangles(
                    for: layoutItems(treemap),
                    in: CGRect(origin: .zero, size: geometry.size)
                )
                ZStack(alignment: .topLeading) {
                    ForEach(rectangles, id: \.id) { rectangle in
                        cell(for: rectangle, treemap: treemap)
                    }
                }
            }
            .accessibilityElement(children: .contain)
            .accessibilityLabel("Logical-size treemap")
            .accessibilityValue(treemap.accessibilitySummary)
            ExplorerStorageCategoryLegend(categories: representedCategories(in: treemap))
            if treemap.zeroLogicalChildCount > 0 {
                Text(
                    "\(treemap.zeroLogicalChildCount) zero-size children are counted in Other and remain available in the table."
                )
                .font(.caption)
                .foregroundStyle(.secondary)
            }
        }
    }

    private func layoutItems(_ treemap: ExplorerSnapshotTreemap) -> [ExplorerTreemapLayoutItem] {
        var items = treemap.cells.map {
            ExplorerTreemapLayoutItem(id: .node($0.id), logicalBytes: $0.node.logicalBytes)
        }
        if treemap.otherLogicalBytes > 0 {
            items.append(
                ExplorerTreemapLayoutItem(id: .other, logicalBytes: treemap.otherLogicalBytes)
            )
        }
        return items
    }

    @ViewBuilder
    private func cell(
        for rectangle: ExplorerTreemapLayoutRectangle,
        treemap: ExplorerSnapshotTreemap
    ) -> some View {
        let rect = rectangle.rect.insetBy(dx: 1.5, dy: 1.5)
        switch rectangle.id {
        case let .node(nodeID):
            if let cell = treemap.cell(nodeID: nodeID) {
                Button {
                    Task { await browser.selectTreemapCell(cell) }
                } label: {
                    visualLabel(
                        title: cell.node.name.display,
                        detail: StorageByteFormatter.string(from: cell.node.logicalBytes),
                        symbol: cell.node.kind.treemapSymbol,
                        category: cell.node.category,
                        rect: rect,
                        selected: browser.selectedNodeID == nodeID,
                        tint: cell.node.category.presentation.palette.color
                    )
                }
                .buttonStyle(.plain)
                .frame(width: max(rect.width, 0), height: max(rect.height, 0))
                .position(x: rect.midX, y: rect.midY)
                .help(cell.node.treemapAccessibilitySummary(total: treemap.totalChildLogicalBytes))
                .accessibilityLabel(
                    cell.node.treemapAccessibilitySummary(total: treemap.totalChildLogicalBytes)
                )
                .accessibilityAddTraits(
                    browser.selectedNodeID == nodeID ? .isSelected : []
                )
                .accessibilityHint(
                    cell.node.kind == .directory
                        ? "Selects this item. Press Return in the table to open the folder."
                        : "Selects this historical item."
                )
                .accessibilityIdentifier(ExplorerAccessibility.snapshotTreemapCell(nodeID: nodeID))
            }
        case .other:
            Button {
                browser.selectOther()
            } label: {
                visualLabel(
                    title: "Other",
                    detail: StorageByteFormatter.string(from: treemap.otherLogicalBytes),
                    symbol: "ellipsis",
                    category: nil,
                    rect: rect,
                    selected: browser.isOtherSelected,
                    tint: .secondary
                )
            }
            .buttonStyle(.plain)
            .frame(width: max(rect.width, 0), height: max(rect.height, 0))
            .position(x: rect.midX, y: rect.midY)
            .help(
                "\(treemap.otherChildCount) smaller children omitted from this treemap; category details are available in the table"
            )
            .accessibilityLabel(
                treemap.otherAccessibilitySummary
            )
            .accessibilityHint("Selects the aggregate. It cannot be opened as a folder.")
            .accessibilityAddTraits(browser.isOtherSelected ? .isSelected : [])
            .accessibilityIdentifier(ExplorerAccessibility.snapshotTreemapOther)
        case .otherGrowth, .otherShrinkage:
            EmptyView()
        }
    }

    private func visualLabel(
        title: String,
        detail: String,
        symbol: String,
        category: ExplorerStorageCategory?,
        rect: CGRect,
        selected: Bool,
        tint: Color
    ) -> some View {
        ZStack(alignment: .topLeading) {
            RoundedRectangle(cornerRadius: 6)
                .fill(tint.opacity(selected ? 0.36 : 0.18))
            RoundedRectangle(cornerRadius: 6)
                .strokeBorder(
                    selected ? Color.accentColor : tint.opacity(0.55),
                    style: StrokeStyle(
                        lineWidth: selected ? 3 : 1,
                        dash: differentiateWithoutColor
                            ? category?.presentation.palette.strokeDash ?? [1, 3]
                            : []
                    )
                )
            if rect.width >= 42, rect.height >= 30 {
                VStack(alignment: .leading, spacing: 2) {
                    HStack(spacing: 4) {
                        Image(systemName: symbol)
                        if rect.width >= 82 {
                            Text(verbatim: title)
                                .lineLimit(1)
                        }
                    }
                    .font(.caption.bold())
                    if rect.width >= 76, rect.height >= 50 {
                        Text(verbatim: detail)
                            .font(.caption2)
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                    }
                    if let category, rect.width >= 105, rect.height >= 68 {
                        Label(
                            category.presentation.title,
                            systemImage: category.presentation.symbol
                        )
                        .font(.caption2)
                        .lineLimit(1)
                    }
                }
                .padding(6)
            }
            if let category, rect.width >= 44, rect.height >= 44 {
                Image(systemName: category.presentation.symbol)
                    .font(.caption2.bold())
                    .foregroundStyle(.primary)
                    .padding(4)
                    .background(.regularMaterial, in: Circle())
                    .padding(4)
                    .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .bottomLeading)
                    .accessibilityHidden(true)
            }
            if selected, rect.width >= 20, rect.height >= 20 {
                Image(systemName: "checkmark.circle.fill")
                    .symbolRenderingMode(.palette)
                    .foregroundStyle(.white, Color.accentColor)
                    .padding(4)
                    .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topTrailing)
                    .accessibilityHidden(true)
            }
        }
        .clipShape(RoundedRectangle(cornerRadius: 6))
        .contentShape(Rectangle())
    }

    private func representedCategories(
        in treemap: ExplorerSnapshotTreemap
    ) -> [ExplorerStorageCategory] {
        let represented = Set(treemap.cells.map(\.node.category))
        return ExplorerStorageCategory.allCases.filter(represented.contains)
    }
}

struct ExplorerSnapshotInspectorView: View {
    @Bindable var browser: ExplorerSnapshotBrowserModel
    @State private var trashConfirmationNode: ExplorerSnapshotNode?

    var body: some View {
        GroupBox("Inspector") {
            if let node = browser.selectedNode {
                ScrollView {
                    VStack(alignment: .leading, spacing: 12) {
                        Label(node.name.display, systemImage: node.kind.treemapSymbol)
                            .font(.headline)
                            .textSelection(.enabled)
                        Label("Historical observation", systemImage: "clock.arrow.circlepath")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                        Divider()
                        inspectorRow("Kind", node.kind.inspectorTitle)
                        LabeledContent("Category") {
                            ExplorerStorageCategoryLabel(category: node.category)
                        }
                        .accessibilityElement(children: .ignore)
                        .accessibilityLabel("Category")
                        .accessibilityValue(node.category.presentation.accessibilityPhrase)
                        .accessibilityIdentifier(ExplorerAccessibility.snapshotInspectorCategory)
                        Text(
                            "Deterministic historical classification. It does not mean this item is safe to remove."
                        )
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        inspectorRow(
                            "Logical size",
                            StorageByteFormatter.string(from: node.logicalBytes)
                        )
                        inspectorRow(
                            "Allocated size",
                            node.allocatedBytes.map(StorageByteFormatter.string(from:))
                                ?? "Unavailable"
                        )
                        inspectorRow("Children", node.childCount.formatted())
                        inspectorRow("Files", node.fileCount.formatted())
                        inspectorRow("Modified", node.modifiedAt.inspectorText)
                        inspectorRow("Accessed", node.accessedAt.inspectorText)
                        if node.hasTreemapWarning {
                            Divider()
                            Label(node.treemapWarningText, systemImage: "exclamationmark.triangle.fill")
                                .foregroundStyle(.orange)
                        }
                        if node.kind == .file {
                            Divider()
                            iCloudLocalCopyReview(node: node)
                        }
                        Divider()
                        VStack(alignment: .leading, spacing: 8) {
                            Button {
                                Task { await browser.revealLiveItem() }
                            } label: {
                                Label("Reveal in Finder", systemImage: "folder")
                            }
                            .disabled(!browser.canRevealSelectedLiveItem)
                            .accessibilityIdentifier(ExplorerAccessibility.snapshotRevealInFinder)

                            Button {
                                Task { await browser.copyLiveItemPath() }
                            } label: {
                                Label("Copy Path", systemImage: "doc.on.doc")
                            }
                            .disabled(!browser.canCopySelectedLivePath)
                            .accessibilityIdentifier(ExplorerAccessibility.snapshotCopyPath)

                            Button {
                                Task { await browser.quickLookLiveItem() }
                            } label: {
                                Label("Quick Look", systemImage: "eye")
                            }
                            .disabled(!browser.canQuickLookSelectedLiveItem)
                            .accessibilityIdentifier(ExplorerAccessibility.snapshotQuickLook)

                            Button {
                                trashConfirmationNode = node
                            } label: {
                                Label("Move to Trash…", systemImage: "trash")
                            }
                            .disabled(!browser.canTrashSelectedItem)
                            .accessibilityIdentifier(ExplorerAccessibility.snapshotMoveToTrash)
                        }
                        if browser.isLiveActionLoading {
                            ProgressView("Validating current item…")
                                .controlSize(.small)
                        }
                        Text(
                            "These actions freshly match the current item and every folder to this snapshot. Quick Look shows current contents; cleanup authority is never granted."
                        )
                        .font(.caption)
                        .foregroundStyle(.secondary)
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                }
            } else if browser.isOtherSelected, let treemap = browser.treemap {
                VStack(alignment: .leading, spacing: 12) {
                    Label("Other", systemImage: "ellipsis")
                        .font(.headline)
                    Text("Mixed or unclassified smaller items. This is a read-only aggregate, not a filesystem item.")
                        .foregroundStyle(.secondary)
                    Divider()
                    inspectorRow("Children", treemap.otherChildCount.formatted())
                    inspectorRow(
                        "Logical size",
                        StorageByteFormatter.string(from: treemap.otherLogicalBytes)
                    )
                    inspectorRow("Zero-size children", treemap.zeroLogicalChildCount.formatted())
                }
                .frame(maxWidth: .infinity, alignment: .leading)
            } else {
                ContentUnavailableView(
                    "Select an item",
                    systemImage: "sidebar.right",
                    description: Text("Choose a treemap cell or table row to inspect its saved facts.")
                )
            }
        }
        .frame(minWidth: 230, idealWidth: 270, maxWidth: 330, maxHeight: .infinity)
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
        .accessibilityIdentifier(ExplorerAccessibility.snapshotInspector)
    }

    @ViewBuilder
    private func iCloudLocalCopyReview(node: ExplorerSnapshotNode) -> some View {
        VStack(alignment: .leading, spacing: 9) {
            Label("iCloud local copy", systemImage: "icloud")
                .font(.headline)
            Text(
                "Check fresh, point-in-time iCloud metadata for this exact selected file. The allocation below remains the historical value observed in the scan."
            )
            .font(.caption)
            .foregroundStyle(.secondary)

            Button {
                Task { await browser.checkSelectedICloudLocalCopy(nodeID: node.id) }
            } label: {
                Label(
                    browser.iCloudLocalCopyReviewState == .idle
                        ? "Check iCloud Status"
                        : "Check Again",
                    systemImage: "arrow.clockwise"
                )
            }
            .disabled(!browser.canCheckSelectedICloudLocalCopy)
            .accessibilityIdentifier(
                ExplorerAccessibility.snapshotICloudLocalCopyCheck
            )

            switch browser.iCloudLocalCopyReviewState {
            case .idle:
                Text("No iCloud metadata has been checked for this selection.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            case .checking:
                ProgressView("Checking current iCloud status…")
                    .controlSize(.small)
            case let .failed(error):
                VStack(alignment: .leading, spacing: 4) {
                    Label(error.title, systemImage: "exclamationmark.triangle.fill")
                        .font(.subheadline.bold())
                        .foregroundStyle(.orange)
                    Text(verbatim: error.detail)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            case let .observed(assessment):
                VStack(alignment: .leading, spacing: 8) {
                    VStack(alignment: .leading, spacing: 4) {
                        Text("Sync metadata")
                            .font(.caption.weight(.semibold))
                            .foregroundStyle(.secondary)
                        Label(
                            assessment.reviewTitle,
                            systemImage: assessment.isEligibleObservation
                                ? "checkmark.circle.fill"
                                : "exclamationmark.triangle.fill"
                        )
                        .font(.subheadline.bold())
                        .foregroundStyle(
                            assessment.isEligibleObservation ? Color.green : Color.orange
                        )
                        Text(verbatim: assessment.reviewDetail)
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                    .accessibilityElement(children: .combine)
                    .accessibilityLabel(
                        "Sync metadata. \(assessment.reviewTitle). \(assessment.reviewDetail)"
                    )

                    VStack(alignment: .leading, spacing: 4) {
                        Text("Identity proof")
                            .font(.caption.weight(.semibold))
                            .foregroundStyle(.secondary)
                        Label(
                            assessment.identityReadinessTitle,
                            systemImage: assessment.isIdentityReady
                                ? "checkmark.shield.fill"
                                : "exclamationmark.shield.fill"
                        )
                        .font(.subheadline.bold())
                        .foregroundStyle(
                            assessment.isIdentityReady ? Color.green : Color.orange
                        )
                        Text(verbatim: assessment.identityReadinessDetail)
                            .font(.caption)
                            .foregroundStyle(.secondary)
                        ForEach(
                            Array(assessment.identityBlockers.enumerated()),
                            id: \.offset
                        ) { _, blocker in
                            Label(
                                blocker.displayText,
                                systemImage: "xmark.shield"
                            )
                            .font(.caption)
                        }
                    }
                    .accessibilityElement(children: .combine)
                    .accessibilityLabel(
                        "Identity proof. \(assessment.identityReadinessTitle). "
                            + "\(assessment.identityReadinessDetail) "
                            + assessment.identityBlockers
                            .map(\.displayText)
                            .joined(separator: " ")
                    )

                    inspectorRow(
                        "Allocated in scan",
                        StorageByteFormatter.string(from: assessment.localAllocatedBytes)
                    )
                    inspectorRow(
                        "Metadata observed",
                        assessment.observedAt.formatted(
                            date: .abbreviated,
                            time: .standard
                        )
                    )
                    DisclosureGroup("Observed facts") {
                        VStack(alignment: .leading, spacing: 5) {
                            ForEach(assessment.factRows) { fact in
                                inspectorRow(LocalizedStringKey(fact.label), fact.value)
                            }
                        }
                        .padding(.top, 5)
                    }
                    DisclosureGroup("Identity facts") {
                        VStack(alignment: .leading, spacing: 5) {
                            ForEach(assessment.identityFactRows) { fact in
                                inspectorRow(LocalizedStringKey(fact.label), fact.value)
                            }
                        }
                        .padding(.top, 5)
                    }
                    if !assessment.blockers.isEmpty {
                        DisclosureGroup("Why sync metadata is blocked") {
                            VStack(alignment: .leading, spacing: 5) {
                                ForEach(
                                    Array(assessment.blockers.enumerated()),
                                    id: \.offset
                                ) { _, blocker in
                                    Label(
                                        blocker.displayText,
                                        systemImage: "xmark.circle"
                                    )
                                    .font(.caption)
                                }
                            }
                            .padding(.top, 5)
                        }
                    }
                    VStack(alignment: .leading, spacing: 5) {
                        Text(verbatim: ExplorerICloudLocalCopyDisclosure.action)
                            .font(.subheadline.bold())
                        Label(
                            ExplorerICloudLocalCopyDisclosure.retention,
                            systemImage: "icloud"
                        )
                        Label(
                            ExplorerICloudLocalCopyDisclosure.redownload,
                            systemImage: "network"
                        )
                        Text(verbatim: ExplorerICloudLocalCopyDisclosure.unavailable)
                        .foregroundStyle(.secondary)
                    }
                    .font(.caption)
                    .padding(9)
                    .background(
                        .quaternary.opacity(0.45),
                        in: RoundedRectangle(cornerRadius: 8)
                    )
                    .accessibilityElement(children: .combine)
                    .accessibilityLabel(
                        "\(ExplorerICloudLocalCopyDisclosure.action). \(ExplorerICloudLocalCopyDisclosure.retention). \(ExplorerICloudLocalCopyDisclosure.redownload). No cleanup action is available yet."
                    )
                }
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier(
            ExplorerAccessibility.snapshotICloudLocalCopyReview
        )
    }

    private func inspectorRow(_ label: LocalizedStringKey, _ value: String) -> some View {
        LabeledContent(label) {
            Text(verbatim: value)
                .multilineTextAlignment(.trailing)
                .textSelection(.enabled)
        }
    }
}

private extension ExplorerSnapshotNodeKind {
    var treemapSymbol: String {
        switch self {
        case .directory: "folder.fill"
        case .file: "doc.fill"
        case .symlink: "link"
        case .other: "questionmark.square"
        case .error: "exclamationmark.triangle.fill"
        }
    }

    var inspectorTitle: String {
        switch self {
        case .directory: "Folder"
        case .file: "File"
        case .symlink: "Symbolic link"
        case .other: "Other"
        case .error: "Unavailable item"
        }
    }
}

extension ExplorerSnapshotNode {
    func treemapAccessibilitySummary(total: UInt64) -> String {
        let share = total == 0 ? 0 : Double(logicalBytes) / Double(total)
        var parts = [
            inspectorTitle,
            name.display,
            category.presentation.accessibilityPhrase,
            "logical size \(StorageByteFormatter.string(from: logicalBytes))",
            "\(share.formatted(.percent.precision(.fractionLength(1)))) of the current folder",
        ]
        if hasTreemapWarning {
            parts.append(treemapWarningText)
        }
        return parts.joined(separator: ", ")
    }

    var inspectorTitle: String { kind.inspectorTitle }

    var hasTreemapWarning: Bool {
        scanFlags.inaccessible || scanFlags.timedOut || scanFlags.hardLinkDuplicate
            || scanFlags.mountBoundary || kind == .error
    }

    var treemapWarningText: String {
        var values: [String] = []
        if scanFlags.inaccessible { values.append("inaccessible") }
        if scanFlags.timedOut { values.append("timed out") }
        if scanFlags.hardLinkDuplicate { values.append("hard-link duplicate") }
        if scanFlags.mountBoundary { values.append("mount boundary") }
        if kind == .error { values.append("scan error") }
        return values.joined(separator: ", ")
    }
}

extension ExplorerSnapshotTreemap {
    var accessibilitySummary: String {
        var parts = [
            "\(cells.count) represented items",
            "\(totalChildren) total children",
            "logical size \(StorageByteFormatter.string(from: totalChildLogicalBytes))",
        ]
        if otherChildCount > 0 {
            parts.append("\(otherChildCount) items in Other; categories are not summarized")
        }
        return parts.joined(separator: ", ")
    }

    var otherAccessibilitySummary: String {
        let share = totalChildLogicalBytes == 0
            ? 0
            : Double(otherLogicalBytes) / Double(totalChildLogicalBytes)
        return [
            "Other",
            "categories not summarized",
            "\(otherChildCount) children",
            "logical size \(StorageByteFormatter.string(from: otherLogicalBytes))",
            "\(share.formatted(.percent.precision(.fractionLength(1)))) of the current folder",
        ].joined(separator: ", ")
    }
}

private extension Optional where Wrapped == ExplorerSnapshotTimestamp {
    var inspectorText: String {
        guard let timestamp = self, timestamp.secondsSinceUnixEpoch <= 253_402_300_799 else {
            return "Unavailable"
        }
        let interval = Double(timestamp.secondsSinceUnixEpoch)
            + Double(timestamp.nanoseconds) / 1_000_000_000
        return Date(timeIntervalSince1970: interval).formatted(
            date: .abbreviated,
            time: .standard
        )
    }
}
