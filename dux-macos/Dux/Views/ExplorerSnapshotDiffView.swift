import SwiftUI

struct ExplorerSnapshotDiffView: View {
    @Bindable var browser: ExplorerSnapshotBrowserModel
    @Binding var inspectorPresented: Bool

    private let disclosure =
        "Logical-size observations, not verified capacity change or reclaimable space."

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            disclosureBanner
            switch browser.snapshotDiffPhase {
            case .idle, .loading:
                Spacer()
                ProgressView("Comparing retained snapshots…")
                    .frame(maxWidth: .infinity)
                    .accessibilityIdentifier(ExplorerAccessibility.snapshotChangesStatus)
                Spacer()
            case let .failed(failure):
                Spacer()
                ContentUnavailableView {
                    Label(failure.title, systemImage: "arrow.left.arrow.right.circle")
                } description: {
                    Text(verbatim: failure.detail)
                } actions: {
                    Button("Try Again") {
                        Task { await browser.retrySnapshotDiffReview() }
                    }
                }
                .accessibilityIdentifier(ExplorerAccessibility.snapshotChangesStatus)
                Spacer()
            case .ready:
                readyContent
            }
        }
        .accessibilityIdentifier(ExplorerAccessibility.snapshotChanges)
    }

    private var disclosureBanner: some View {
        Label(disclosure, systemImage: "clock.badge.exclamationmark")
            .font(.callout.weight(.medium))
            .foregroundStyle(.secondary)
            .padding(10)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(.quaternary.opacity(0.45), in: RoundedRectangle(cornerRadius: 10))
            .accessibilityIdentifier(ExplorerAccessibility.snapshotChangesDisclosure)
    }

    @ViewBuilder
    private var readyContent: some View {
        if let info = browser.snapshotDiffInfo,
           let directory = browser.currentSnapshotDiffDirectory,
           let page = browser.snapshotDiffPage
        {
            comparisonHeader(info)
            comparisonSummary(directory: directory, page: page)
            navigationBar
            if let failure = browser.snapshotDiffOperationFailure {
                operationFailure(failure)
            }
            ExplorerSnapshotDiffTreemapView(browser: browser)
            if page.nodes.isEmpty, page.totalChildren == 0 {
                ContentUnavailableView(
                    "No recorded children",
                    systemImage: "equal.circle",
                    description: Text(
                        "Neither retained snapshot recorded children in this folder."
                    )
                )
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                changesTable
                pageControls(page)
            }
        } else {
            ContentUnavailableView(
                "Comparison incomplete",
                systemImage: "exclamationmark.triangle",
                description: Text("DUX did not publish a partial comparison.")
            )
        }
    }

    private func comparisonHeader(_ info: ExplorerSnapshotDiffInfo) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(alignment: .firstTextBaseline) {
                Label("Observed changes", systemImage: "arrow.left.arrow.right")
                    .font(.title2.bold())
                Spacer()
                Text(
                    "\(format(info.baselineCompletedAt)) → \(format(info.currentCompletedAt))"
                )
                .font(.headline.monospacedDigit())
            }
            .accessibilityElement(children: .combine)
            .accessibilityLabel(
                "Comparing older snapshot \(format(info.baselineCompletedAt)) to newer snapshot \(format(info.currentCompletedAt))"
            )
            .accessibilityIdentifier(ExplorerAccessibility.snapshotChangesTimeline)

            HStack(spacing: 12) {
                coverageCard(
                    title: "Older",
                    scanID: info.baselineScanID,
                    coverage: info.baselineCoverage
                )
                coverageCard(
                    title: "Newer",
                    scanID: info.currentScanID,
                    coverage: info.currentCoverage
                )
            }
        }
    }

    private func coverageCard(
        title: String,
        scanID: String,
        coverage: ExplorerSnapshotDiffCoverage
    ) -> some View {
        VStack(alignment: .leading, spacing: 3) {
            Text(title)
                .font(.caption.weight(.semibold))
                .foregroundStyle(.secondary)
            Text(verbatim: coverage.title)
                .font(.subheadline.weight(.medium))
            Text(verbatim: coverage.detail)
                .font(.caption)
                .foregroundStyle(.secondary)
            Text(verbatim: scanID)
                .font(.caption2.monospaced())
                .foregroundStyle(.tertiary)
                .lineLimit(1)
        }
        .padding(10)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(.quaternary.opacity(0.35), in: RoundedRectangle(cornerRadius: 8))
        .accessibilityElement(children: .combine)
    }

    private func comparisonSummary(
        directory: ExplorerSnapshotDiffNode,
        page: ExplorerSnapshotDiffNodePage
    ) -> some View {
        HStack(spacing: 10) {
            summaryCard(
                "Current",
                value: directory.currentSizeText,
                symbol: "internaldrive"
            )
            summaryCard(
                "Previous",
                value: directory.baselineSizeText,
                symbol: "clock"
            )
            summaryCard(
                "Observed growth",
                value: "+\(StorageByteFormatter.string(from: page.totalGrowthBytes))",
                symbol: "arrow.up",
                tint: .orange
            )
            summaryCard(
                "Observed shrinkage",
                value: "−\(StorageByteFormatter.string(from: page.totalShrinkageBytes))",
                symbol: "arrow.down",
                tint: .blue
            )
        }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier(ExplorerAccessibility.snapshotChangesSummary)
    }

    private func summaryCard(
        _ title: String,
        value: String,
        symbol: String,
        tint: Color = .secondary
    ) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            Label(title, systemImage: symbol)
                .font(.caption.weight(.semibold))
                .foregroundStyle(tint)
            Text(verbatim: value)
                .font(.headline.monospacedDigit())
                .lineLimit(1)
                .minimumScaleFactor(0.75)
        }
        .padding(10)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(tint.opacity(0.08), in: RoundedRectangle(cornerRadius: 8))
        .accessibilityElement(children: .combine)
    }

    private var navigationBar: some View {
        HStack(spacing: 10) {
            Button {
                Task { await browser.goBackSnapshotDiff() }
            } label: {
                Label("Back", systemImage: "chevron.left")
            }
            .disabled(
                browser.snapshotDiffBreadcrumbs.count <= 1
                    || browser.isSnapshotDiffNavigating
                    || browser.isSnapshotDiffPaging
            )
            .accessibilityIdentifier(ExplorerAccessibility.snapshotChangesBack)

            ScrollView(.horizontal) {
                HStack(spacing: 5) {
                    ForEach(Array(browser.snapshotDiffBreadcrumbs.enumerated()), id: \.element.id) {
                        index, node in
                        if index > 0 {
                            Image(systemName: "chevron.right")
                                .font(.caption2)
                                .foregroundStyle(.tertiary)
                                .accessibilityHidden(true)
                        }
                        Button(node.name.display) {
                            Task { await browser.openSnapshotDiffBreadcrumb(at: index) }
                        }
                        .buttonStyle(.plain)
                        .disabled(index == browser.snapshotDiffBreadcrumbs.count - 1)
                    }
                }
            }
            .accessibilityIdentifier(ExplorerAccessibility.snapshotChangesBreadcrumbs)

            Spacer(minLength: 12)
            Picker(
                "Sort changes",
                selection: Binding(
                    get: { browser.snapshotDiffSort },
                    set: { value in Task { await browser.selectSnapshotDiffSort(value) } }
                )
            ) {
                ForEach(ExplorerSnapshotDiffSort.allCases, id: \.self) {
                    Text(verbatim: $0.title).tag($0)
                }
            }
            .pickerStyle(.menu)
            .disabled(browser.isSnapshotDiffNavigating || browser.isSnapshotDiffPaging)
            .accessibilityIdentifier(ExplorerAccessibility.snapshotChangesSort)

            Button {
                inspectorPresented.toggle()
            } label: {
                Label("Inspector", systemImage: "sidebar.right")
                    .labelStyle(.iconOnly)
            }
            .help(inspectorPresented ? "Hide inspector" : "Show inspector")
        }
    }

    private func operationFailure(_ failure: ExplorerSnapshotDiffFailure) -> some View {
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
            Button("Prepare Again") {
                Task { await browser.retrySnapshotDiffReview() }
            }
        }
        .padding(10)
        .background(.orange.opacity(0.10), in: RoundedRectangle(cornerRadius: 8))
        .accessibilityIdentifier(ExplorerAccessibility.snapshotChangesStatus)
    }

    private var changesTable: some View {
        Table(
            browser.snapshotDiffNodes,
            selection: Binding(
                get: { browser.selectedSnapshotDiffNodeID },
                set: { browser.selectSnapshotDiffNode($0) }
            )
        ) {
            TableColumn("Name") { (node: ExplorerSnapshotDiffNode) in
                HStack(spacing: 7) {
                    Image(systemName: nodeKindSymbol(node.kind))
                        .foregroundStyle(
                            node.kind == .directory ? Color.accentColor : Color.secondary
                        )
                    Text(verbatim: node.name.display)
                        .lineLimit(1)
                        .onTapGesture(count: 2) {
                            guard node.canDescend else { return }
                            Task { await browser.openSnapshotDiffDirectory(node) }
                        }
                    if node.hasObservationWarning {
                        Image(systemName: "exclamationmark.triangle.fill")
                            .foregroundStyle(.orange)
                            .help("One or both scans recorded an observation warning")
                    }
                }
                .accessibilityElement(children: .ignore)
                .accessibilityLabel(node.accessibilitySummary)
                .accessibilityHint(
                    node.canDescend
                        ? "Press Return to open this historical comparison."
                        : "Selects this read-only comparison."
                )
            }
            .width(min: 170, ideal: 270)

            TableColumn("Change") { (node: ExplorerSnapshotDiffNode) in
                Label {
                    Text(verbatim: node.logicalChange.display)
                        .monospacedDigit()
                } icon: {
                    Text(verbatim: node.change.symbol)
                        .fontWeight(.bold)
                }
                .foregroundStyle(changeStyle(node.change).color)
                .accessibilityLabel(
                    "\(node.change.title), \(node.logicalChange.display)"
                )
            }
            .width(min: 105, ideal: 125)

            TableColumn("Current") { (node: ExplorerSnapshotDiffNode) in
                Text(verbatim: node.currentSizeText)
                    .monospacedDigit()
                    .foregroundStyle(node.currentLogicalBytes == nil ? .secondary : .primary)
            }
            .width(min: 95, ideal: 115)

            TableColumn("Previous") { (node: ExplorerSnapshotDiffNode) in
                Text(verbatim: node.baselineSizeText)
                    .monospacedDigit()
                    .foregroundStyle(node.baselineLogicalBytes == nil ? .secondary : .primary)
            }
            .width(min: 95, ideal: 115)

            TableColumn("State") { (node: ExplorerSnapshotDiffNode) in
                Label(node.change.title, systemImage: node.change.systemImage)
                    .foregroundStyle(changeStyle(node.change).color)
            }
            .width(min: 115, ideal: 145)

            TableColumn("Category") { (node: ExplorerSnapshotDiffNode) in
                ExplorerStorageCategoryLabel(category: node.category)
            }
            .width(min: 135, ideal: 165)
        }
        .disabled(browser.isSnapshotDiffNavigating || browser.isSnapshotDiffPaging)
        .onKeyPress(.return) {
            guard let node = browser.selectedSnapshotDiffNode, node.canDescend else {
                return .ignored
            }
            Task { await browser.openSnapshotDiffDirectory(node) }
            return .handled
        }
        .onKeyPress(.delete) {
            guard browser.snapshotDiffBreadcrumbs.count > 1 else {
                return .ignored
            }
            Task { await browser.goBackSnapshotDiff() }
            return .handled
        }
        .accessibilityIdentifier(ExplorerAccessibility.snapshotChangesTable)
    }

    private func pageControls(_ page: ExplorerSnapshotDiffNodePage) -> some View {
        HStack {
            Text(verbatim: pageStatus(page))
                .font(.callout.monospacedDigit())
                .foregroundStyle(.secondary)
                .accessibilityIdentifier(ExplorerAccessibility.snapshotChangesPageStatus)
            Spacer()
            Button {
                Task { await browser.showPreviousSnapshotDiffPage() }
            } label: {
                Label("Previous changes", systemImage: "chevron.left")
            }
            .disabled(!browser.hasPreviousSnapshotDiffPage || browser.isSnapshotDiffPaging)
            .accessibilityIdentifier(ExplorerAccessibility.snapshotChangesPreviousPage)
            Button {
                Task { await browser.showNextSnapshotDiffPage() }
            } label: {
                Label("Next changes", systemImage: "chevron.right")
            }
            .disabled(!browser.hasNextSnapshotDiffPage || browser.isSnapshotDiffPaging)
            .accessibilityIdentifier(ExplorerAccessibility.snapshotChangesNextPage)
        }
    }

    private func pageStatus(_ page: ExplorerSnapshotDiffNodePage) -> String {
        guard !page.nodes.isEmpty else {
            return "0 of \(page.totalChildren)"
        }
        return "\(page.offset + 1)–\(page.offset + UInt64(page.nodes.count)) of \(page.totalChildren)"
    }

    private func format(_ date: Date) -> String {
        date.formatted(date: .abbreviated, time: .shortened)
    }
}

struct ExplorerSnapshotDiffTreemapView: View {
    @Bindable var browser: ExplorerSnapshotBrowserModel
    @Environment(\.accessibilityDifferentiateWithoutColor) private var differentiateWithoutColor
    @Environment(\.colorSchemeContrast) private var contrast

    var body: some View {
        GroupBox {
            if let treemap = browser.snapshotDiffTreemap {
                treemapContent(treemap)
            }
        } label: {
            HStack {
                Label("Change magnitude", systemImage: "chart.treemap")
                Spacer()
                Text("Area represents absolute observed change")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
        .accessibilityIdentifier(ExplorerAccessibility.snapshotChangesTreemap)
    }

    @ViewBuilder
    private func treemapContent(_ treemap: ExplorerSnapshotDiffTreemap) -> some View {
        if treemap.changedChildCount == 0 {
            ContentUnavailableView(
                "No size changes to plot",
                systemImage: "equal.circle",
                description: Text(
                    "\(treemap.unchangedChildCount) unchanged rows remain available in the table."
                )
            )
            .frame(minHeight: 150, idealHeight: 190)
        } else {
            GeometryReader { geometry in
                let rectangles = ExplorerTreemapLayout.rectangles(
                    for: layoutItems(treemap),
                    in: CGRect(origin: .zero, size: geometry.size)
                )
                ZStack(alignment: .topLeading) {
                    ForEach(rectangles, id: \.id) { rectangle in
                        cell(rectangle, treemap: treemap)
                    }
                }
            }
            .frame(minHeight: 170, idealHeight: 220, maxHeight: 260)
            .accessibilityElement(children: .contain)
            .accessibilityLabel("Observed change magnitude treemap")
            legend
        }
    }

    private func layoutItems(
        _ treemap: ExplorerSnapshotDiffTreemap
    ) -> [ExplorerTreemapLayoutItem] {
        var items = treemap.cells.map {
            ExplorerTreemapLayoutItem(
                id: .node($0.id),
                logicalBytes: $0.node.logicalChange.magnitudeBytes
            )
        }
        if treemap.otherGrowthBytes > 0 {
            items.append(
                ExplorerTreemapLayoutItem(
                    id: .otherGrowth,
                    logicalBytes: treemap.otherGrowthBytes
                )
            )
        }
        if treemap.otherShrinkageBytes > 0 {
            items.append(
                ExplorerTreemapLayoutItem(
                    id: .otherShrinkage,
                    logicalBytes: treemap.otherShrinkageBytes
                )
            )
        }
        return items
    }

    @ViewBuilder
    private func cell(
        _ rectangle: ExplorerTreemapLayoutRectangle,
        treemap: ExplorerSnapshotDiffTreemap
    ) -> some View {
        let rect = rectangle.rect.insetBy(dx: 1.5, dy: 1.5)
        switch rectangle.id {
        case let .node(nodeID):
            if let cell = treemap.cell(nodeID: nodeID) {
                let style = changeStyle(cell.node.change)
                Button {
                    browser.selectSnapshotDiffTreemapCell(cell)
                } label: {
                    visualLabel(
                        title: cell.node.name.display,
                        detail: "\(cell.node.change.symbol) \(cell.node.logicalChange.display)",
                        systemImage: cell.node.change.systemImage,
                        style: style,
                        rect: rect,
                        selected: browser.selectedSnapshotDiffNodeID == nodeID
                    )
                }
                .buttonStyle(.plain)
                .frame(width: max(rect.width, 0), height: max(rect.height, 0))
                .position(x: rect.midX, y: rect.midY)
                .help(cell.node.accessibilitySummary)
                .accessibilityLabel(cell.node.accessibilitySummary)
                .accessibilityHint(
                    cell.node.canDescend
                        ? "Selects this comparison. Press Return in the table to open it."
                        : "Selects this read-only comparison."
                )
                .accessibilityAddTraits(
                    browser.selectedSnapshotDiffNodeID == nodeID ? .isSelected : []
                )
                .accessibilityIdentifier(
                    ExplorerAccessibility.snapshotChangesTreemapCell(nodeID: nodeID)
                )
            }
        case .otherGrowth:
            aggregateCell(
                title: "Other Growth",
                symbol: "↑",
                bytes: treemap.otherGrowthBytes,
                count: treemap.otherGrowthChildCount,
                style: changeStyle(.grew),
                selected: browser.isSnapshotDiffOtherGrowthSelected,
                identifier: ExplorerAccessibility.snapshotChangesOtherGrowth,
                rect: rect
            ) {
                browser.selectSnapshotDiffOtherGrowth()
            }
        case .otherShrinkage:
            aggregateCell(
                title: "Other Shrinkage",
                symbol: "↓",
                bytes: treemap.otherShrinkageBytes,
                count: treemap.otherShrinkageChildCount,
                style: changeStyle(.shrank),
                selected: browser.isSnapshotDiffOtherShrinkageSelected,
                identifier: ExplorerAccessibility.snapshotChangesOtherShrinkage,
                rect: rect
            ) {
                browser.selectSnapshotDiffOtherShrinkage()
            }
        case .other:
            EmptyView()
        }
    }

    private func aggregateCell(
        title: String,
        symbol: String,
        bytes: UInt64,
        count: UInt64,
        style: SnapshotDiffVisualStyle,
        selected: Bool,
        identifier: String,
        rect: CGRect,
        action: @escaping () -> Void
    ) -> some View {
        Button(action: action) {
            visualLabel(
                title: title,
                detail: "\(symbol) \(StorageByteFormatter.string(from: bytes))",
                systemImage: "ellipsis",
                style: style,
                rect: rect,
                selected: selected
            )
        }
        .buttonStyle(.plain)
        .frame(width: max(rect.width, 0), height: max(rect.height, 0))
        .position(x: rect.midX, y: rect.midY)
        .accessibilityLabel(
            "\(title), \(count) \(count == 1 ? "item" : "items"), \(StorageByteFormatter.string(from: bytes))"
        )
        .accessibilityHint("Selects this aggregate. It cannot be opened.")
        .accessibilityAddTraits(selected ? .isSelected : [])
        .accessibilityIdentifier(identifier)
    }

    private func visualLabel(
        title: String,
        detail: String,
        systemImage: String,
        style: SnapshotDiffVisualStyle,
        rect: CGRect,
        selected: Bool
    ) -> some View {
        ZStack(alignment: .topLeading) {
            RoundedRectangle(cornerRadius: 6)
                .fill(style.color.opacity(selected ? 0.36 : 0.18))
            RoundedRectangle(cornerRadius: 6)
                .strokeBorder(
                    selected ? Color.accentColor : style.color.opacity(0.8),
                    style: StrokeStyle(
                        lineWidth: selected || contrast == .increased ? 3 : 1.5,
                        dash: style.dash
                    )
                )
            if rect.width >= 42, rect.height >= 30 {
                VStack(alignment: .leading, spacing: 2) {
                    HStack(spacing: 4) {
                        Image(systemName: systemImage)
                        if rect.width >= 82 {
                            Text(verbatim: title)
                                .lineLimit(1)
                        }
                    }
                    .font(.caption.bold())
                    if rect.width >= 76, rect.height >= 50 {
                        Text(verbatim: detail)
                            .font(.caption2.monospacedDigit())
                            .lineLimit(1)
                    }
                    if differentiateWithoutColor, rect.width >= 86, rect.height >= 68 {
                        Text(verbatim: style.patternName)
                            .font(.caption2)
                            .foregroundStyle(.secondary)
                    }
                }
                .padding(6)
            }
        }
        .clipShape(RoundedRectangle(cornerRadius: 6))
        .contentShape(Rectangle())
    }

    private var legend: some View {
        HStack(spacing: 14) {
            legendItem(.grew)
            legendItem(.shrank)
            legendItem(.added)
            legendItem(.removed)
            legendItem(.replaced)
            Spacer()
            Text("Unchanged rows stay in the table")
                .font(.caption)
                .foregroundStyle(.secondary)
        }
        .font(.caption)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier(ExplorerAccessibility.snapshotChangesLegend)
    }

    private func legendItem(_ change: ExplorerSnapshotDiffChange) -> some View {
        let style = changeStyle(change)
        return Label {
            Text("\(change.symbol) \(change.title)")
        } icon: {
            Image(systemName: change.systemImage)
        }
        .foregroundStyle(style.color)
        .accessibilityLabel("\(change.symbol) \(change.title), \(style.patternName)")
    }
}

struct ExplorerSnapshotDiffInspectorView: View {
    @Bindable var browser: ExplorerSnapshotBrowserModel

    private let disclosure =
        "Logical-size observations, not verified capacity change or reclaimable space."

    var body: some View {
        GroupBox("Changes Inspector") {
            ScrollView {
                if let node = browser.selectedSnapshotDiffNode {
                    VStack(alignment: .leading, spacing: 12) {
                        Label(node.name.display, systemImage: nodeKindSymbol(node.kind))
                            .font(.headline)
                            .textSelection(.enabled)
                        Label(
                            "\(node.change.symbol) \(node.change.title)",
                            systemImage: node.change.systemImage
                        )
                        .foregroundStyle(changeStyle(node.change).color)
                        Divider()
                        row("Observed change", node.logicalChange.display)
                        row("Current logical size", node.currentSizeText)
                        row("Previous logical size", node.baselineSizeText)
                        row("Current kind", kindTitle(node.currentKind))
                        row("Previous kind", kindTitle(node.baselineKind))
                        LabeledContent("Category") {
                            ExplorerStorageCategoryLabel(category: node.category)
                        }
                        row(
                            "Current allocated",
                            node.currentAllocatedBytes.map(StorageByteFormatter.string(from:))
                                ?? "Unavailable"
                        )
                        row(
                            "Previous allocated",
                            node.baselineAllocatedBytes.map(StorageByteFormatter.string(from:))
                                ?? "Unavailable"
                        )
                        if node.hasObservationWarning {
                            Label(
                                "One or both snapshots recorded an observation warning.",
                                systemImage: "exclamationmark.triangle.fill"
                            )
                            .foregroundStyle(.orange)
                        }
                        Divider()
                        Label(disclosure, systemImage: "lock.shield")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                        Text(
                            "This inspector is historical and read-only. It has no Finder, path, Trash, cleanup, candidate, plan, AI, or execution capability."
                        )
                        .font(.caption)
                        .foregroundStyle(.secondary)
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                } else if browser.isSnapshotDiffOtherGrowthSelected,
                          let treemap = browser.snapshotDiffTreemap
                {
                    aggregate(
                        title: "Other Growth",
                        symbol: "↑",
                        count: treemap.otherGrowthChildCount,
                        bytes: treemap.otherGrowthBytes,
                        tint: .orange
                    )
                } else if browser.isSnapshotDiffOtherShrinkageSelected,
                          let treemap = browser.snapshotDiffTreemap
                {
                    aggregate(
                        title: "Other Shrinkage",
                        symbol: "↓",
                        count: treemap.otherShrinkageChildCount,
                        bytes: treemap.otherShrinkageBytes,
                        tint: .blue
                    )
                } else {
                    VStack(alignment: .leading, spacing: 10) {
                        Label("Select a change", systemImage: "cursorarrow.click")
                            .font(.headline)
                        Text(
                            "Choose a table row or treemap cell to compare its two historical observations."
                        )
                        .foregroundStyle(.secondary)
                        Text(disclosure)
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                }
            }
        }
        .accessibilityIdentifier(ExplorerAccessibility.snapshotChangesInspector)
    }

    private func aggregate(
        title: String,
        symbol: String,
        count: UInt64,
        bytes: UInt64,
        tint: Color
    ) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("\(symbol) \(title)")
                .font(.headline)
                .foregroundStyle(tint)
            row("Items", count.formatted())
            row("Observed magnitude", StorageByteFormatter.string(from: bytes))
            Text("This aggregate contains omitted treemap cells; every row remains in the table.")
                .foregroundStyle(.secondary)
            Divider()
            Text(disclosure)
                .font(.caption)
                .foregroundStyle(.secondary)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private func row(_ label: String, _ value: String) -> some View {
        LabeledContent(label) {
            Text(verbatim: value)
                .multilineTextAlignment(.trailing)
                .textSelection(.enabled)
        }
    }
}

struct SnapshotDiffVisualStyle {
    let color: Color
    let dash: [CGFloat]
    let patternName: String
}

private func changeStyle(
    _ change: ExplorerSnapshotDiffChange
) -> SnapshotDiffVisualStyle {
    switch change {
    case .grew:
        SnapshotDiffVisualStyle(color: .orange, dash: [], patternName: "solid")
    case .shrank:
        SnapshotDiffVisualStyle(color: .blue, dash: [7, 3], patternName: "long dash")
    case .added:
        SnapshotDiffVisualStyle(color: .purple, dash: [2, 3], patternName: "dot")
    case .removed:
        SnapshotDiffVisualStyle(color: .gray, dash: [10, 3, 2, 3], patternName: "dash dot")
    case .replaced:
        SnapshotDiffVisualStyle(color: .pink, dash: [4, 2, 1, 2], patternName: "alternating")
    case .unchanged:
        SnapshotDiffVisualStyle(color: .secondary, dash: [1, 4], patternName: "light dot")
    }
}

private func nodeKindSymbol(_ kind: ExplorerSnapshotNodeKind) -> String {
    switch kind {
    case .directory: "folder.fill"
    case .file: "doc.fill"
    case .symlink: "link"
    case .other: "questionmark.square"
    case .error: "exclamationmark.triangle.fill"
    }
}

private func kindTitle(_ kind: ExplorerSnapshotNodeKind?) -> String {
    guard let kind else {
        return "Not observed"
    }
    return switch kind {
    case .directory: "Folder"
    case .file: "File"
    case .symlink: "Symbolic link"
    case .other: "Other"
    case .error: "Scan error"
    }
}
