import SwiftUI

private enum ExplorerDiskMapLens: String, CaseIterable, Identifiable {
    case storageType = "Storage Type"
    case cleanupStatus = "Cleanup Status"

    var id: Self { self }
}

private struct ExplorerDiskMapDetailRow: Identifiable {
    let label: String
    let value: String

    var id: String { label }
}

private struct ExplorerDiskMapDetail {
    let title: String
    let bytes: UInt64
    let rows: [ExplorerDiskMapDetailRow]
    let note: String
}

struct ExplorerDiskMapView: View {
    @Bindable var browser: ExplorerSnapshotBrowserModel
    let model: AppModel
    @Binding var showingDirectory: Bool
    let openBrowse: @MainActor () -> Void

    @State private var lens = ExplorerDiskMapLens.storageType
    @State private var hoveredVisual: ExplorerDiskMapVisualID?
    @State private var scanNotice: String?
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.accessibilityDifferentiateWithoutColor) private var differentiateWithoutColor

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            controls
            breadcrumbs
            VStack(spacing: 0) {
                HStack {
                    Label(
                        showingDirectory ? "Allocated space" : "Volume allocation",
                        systemImage: "circle.hexagongrid.fill"
                    )
                    Spacer()
                    Text("Circle area is proportional to allocated space")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                        .minimumScaleFactor(0.75)
                }
                .padding(.horizontal, 14)
                .padding(.vertical, 10)

                Divider()

                mapContent
                    .frame(minHeight: 390, idealHeight: 560)
                    .background(mapSurface)
            }
            .background(.thinMaterial)
            .clipShape(RoundedRectangle(cornerRadius: 14, style: .continuous))
            .overlay {
                RoundedRectangle(cornerRadius: 14, style: .continuous)
                    .strokeBorder(Color.primary.opacity(0.12), lineWidth: 1)
            }
            footer
        }
        .onChange(of: showingDirectory) { _, _ in
            hoveredVisual = nil
        }
        .accessibilityIdentifier(ExplorerAccessibility.diskMap)
    }

    private var mapSurface: some View {
        LinearGradient(
            colors: [
                Color(nsColor: .controlBackgroundColor),
                Color.accentColor.opacity(0.025),
                Color(nsColor: .controlBackgroundColor),
            ],
            startPoint: .topLeading,
            endPoint: .bottomTrailing
        )
    }

    private var controls: some View {
        ViewThatFits(in: .horizontal) {
            HStack(spacing: 12) {
                lensPicker
                cleanupDisclaimer
                Spacer()
                mapActions
            }
            VStack(alignment: .leading, spacing: 8) {
                lensPicker
                mapActions
                cleanupDisclaimer
            }
        }
    }

    private var lensPicker: some View {
        Picker("Map lens", selection: $lens) {
            ForEach(ExplorerDiskMapLens.allCases) { lens in
                Text(lens.rawValue).tag(lens)
            }
        }
        .pickerStyle(.segmented)
        .frame(width: 280)
        .accessibilityIdentifier(ExplorerAccessibility.diskMapLens)
    }

    private var cleanupDisclaimer: some View {
        Text("Display only — cleanup still requires review and revalidation.")
            .font(.caption)
            .foregroundStyle(.secondary)
    }

    private var mapActions: some View {
        HStack(spacing: 8) {
            Button {
                Task { await scanStartupVolume() }
            } label: {
                Label("Scan Disk", systemImage: "internaldrive")
            }
            .disabled(model.scanState.phase.isActive)
            .accessibilityIdentifier(ExplorerAccessibility.diskMapScan)

            Button("Open in Browse") {
                Task { await browser.selectContentMode(.browse) }
                openBrowse()
            }
            .disabled(browser.phase != .ready)
            .accessibilityIdentifier(ExplorerAccessibility.diskMapOpenBrowse)
        }
    }

    private var breadcrumbs: some View {
        ScrollView(.horizontal) {
            HStack(spacing: 6) {
                Button(model.volumeState.snapshot?.displayName ?? "Startup volume") {
                    showingDirectory = false
                }
                .buttonStyle(.borderless)
                if showingDirectory {
                    ForEach(Array(browser.breadcrumbs.enumerated()), id: \.element.id) { index, node in
                        Image(systemName: "chevron.right")
                            .font(.caption2)
                            .foregroundStyle(.tertiary)
                        Button(node.name.display) {
                            Task { await browser.goToBreadcrumb(at: index) }
                        }
                        .buttonStyle(.borderless)
                    }
                }
            }
        }
        .scrollIndicators(.hidden)
        .accessibilityLabel("Disk map path")
        .accessibilityIdentifier(ExplorerAccessibility.diskMapBreadcrumbs)
    }

    @ViewBuilder
    private var mapContent: some View {
        if showingDirectory {
            directoryMap
        } else if let capacity = model.volumeState.snapshot,
                  let envelope = ExplorerDiskMapVolumeEnvelope.make(
                      capacity: capacity,
                      measuredAllocatedBytes: browser.breadcrumbs.first?.allocatedBytes
                  )
        {
            volumeMap(envelope)
        } else {
            ContentUnavailableView(
                "Volume allocation unavailable",
                systemImage: "internaldrive",
                description: Text(
                    "DUX needs ordinary filesystem capacity before it can draw an honest used-space map."
                )
            )
        }
    }

    private func volumeMap(_ envelope: ExplorerDiskMapVolumeEnvelope) -> some View {
        GeometryReader { geometry in
            let bounds = CGRect(origin: .zero, size: geometry.size)
            let stage = squareStage(in: bounds).insetBy(dx: 14, dy: 14)
            let circles = ExplorerDiskMapLayout.circles(
                for: envelope.overviewLayoutItems,
                in: stage,
                inset: 4
            )
            ZStack(alignment: .bottomTrailing) {
                ForEach(circles, id: \.id) { circle in
                    if circle.id == .used {
                        usedVolumeGroup(circle, envelope: envelope)
                    } else {
                        volumeCircle(circle, envelope: envelope)
                    }
                }
                if let detail = volumeHoverDetail(envelope) {
                    detailCard(detail)
                        .frame(width: 292)
                        .padding(18)
                        .transition(.move(edge: .bottom).combined(with: .opacity))
                }
            }
            .onKeyPress(.delete) {
                Task { await zoomOut() }
                return .handled
            }
            .accessibilityElement(children: .contain)
            .accessibilityLabel("Startup volume disk map")
            .accessibilityValue(
                "\(StorageByteFormatter.string(from: envelope.usedBytes)) used, \(StorageByteFormatter.string(from: envelope.availableBytes)) available"
            )
            .animation(reduceMotion ? nil : .easeOut(duration: 0.16), value: hoveredVisual)
        }
    }

    @ViewBuilder
    private func usedVolumeGroup(
        _ circle: ExplorerDiskMapLayoutCircle,
        envelope: ExplorerDiskMapVolumeEnvelope
    ) -> some View {
        let inset = max(12, min(28, circle.radius * 0.075))
        let innerBounds = CGRect(
            x: circle.center.x - circle.radius + inset,
            y: circle.center.y - circle.radius + inset,
            width: max(0, circle.radius * 2 - inset * 2),
            height: max(0, circle.radius * 2 - inset * 2)
        )
        let children = ExplorerDiskMapLayout.circles(
            for: envelope.usedLayoutItems,
            in: innerBounds,
            inset: 8
        )
        let isHovered = hoveredVisual == .used

        Circle()
            .fill(
                LinearGradient(
                    colors: [Color.teal.opacity(0.17), Color.teal.opacity(0.055)],
                    startPoint: .topLeading,
                    endPoint: .bottomTrailing
                )
            )
            .strokeBorder(
                isHovered ? Color.primary.opacity(0.8) : Color.teal.opacity(0.48),
                lineWidth: isHovered ? 2.5 : 1.5
            )
            .frame(width: circle.radius * 2, height: circle.radius * 2)
            .position(circle.center)
            .contentShape(Circle())
            .onHover { hoveredVisual = $0 ? .used : nil }
            .onTapGesture {
                hoveredVisual = .used
                if browser.diskMap != nil, envelope.measuredBytes > 0 {
                    showingDirectory = true
                }
            }
            .help("Used space: \(StorageByteFormatter.string(from: envelope.usedBytes))")

        if circle.radius >= 72 {
            Text("Used · \(StorageByteFormatter.string(from: envelope.usedBytes))")
                .font(.caption.weight(.semibold))
                .foregroundStyle(.secondary)
                .padding(.horizontal, 9)
                .padding(.vertical, 4)
                .background(.ultraThinMaterial, in: Capsule())
                .position(x: circle.center.x, y: circle.center.y - circle.radius + 23)
                .allowsHitTesting(false)
        }

        ForEach(children, id: \.id) { child in
            volumeCircle(child, envelope: envelope)
        }
    }

    @ViewBuilder
    private var directoryMap: some View {
        if browser.isDiskMapLoading || browser.phase == .loading {
            ProgressView("Packing allocated space…")
                .frame(maxWidth: .infinity, maxHeight: .infinity)
        } else if let diskMap = browser.diskMap {
            if diskMap.totalChildAllocatedBytes == 0 {
                ContentUnavailableView(
                    "No allocated size to plot",
                    systemImage: "circle.hexagongrid",
                    description: Text(allocationUnknownDescription(diskMap))
                )
            } else {
                directoryGeometry(diskMap)
            }
        } else if let failure = browser.diskMapFailure {
            ContentUnavailableView(
                failure.title,
                systemImage: "exclamationmark.triangle",
                description: Text("Browse remains available as the textual snapshot view.")
            )
        } else {
            ContentUnavailableView(
                "No saved scan",
                systemImage: "internaldrive.badge.questionmark",
                description: Text("Scan the disk to build its allocated-space map.")
            )
        }
    }

    private func directoryGeometry(_ diskMap: ExplorerSnapshotDiskMap) -> some View {
        GeometryReader { geometry in
            let bounds = CGRect(origin: .zero, size: geometry.size)
            let stage = squareStage(in: bounds)
            let circles = ExplorerDiskMapLayout.circles(
                for: directoryLayoutItems(diskMap),
                in: stage
            )
            ZStack(alignment: .bottomTrailing) {
                Circle()
                    .fill(
                        RadialGradient(
                            colors: [
                                Color.accentColor.opacity(0.085),
                                Color.accentColor.opacity(0.025),
                            ],
                            center: .center,
                            startRadius: 0,
                            endRadius: stage.width / 2
                        )
                    )
                    .strokeBorder(Color.primary.opacity(0.2), lineWidth: 1.25)
                    .frame(width: stage.width, height: stage.height)
                    .position(x: stage.midX, y: stage.midY)
                    .contentShape(Circle())
                    .onTapGesture { Task { await zoomOut() } }
                ForEach(circles, id: \.id) { circle in
                    directoryCircle(circle, diskMap: diskMap)
                }
                if let detail = directoryHoverDetail(diskMap) {
                    detailCard(detail)
                        .frame(width: 304)
                        .padding(18)
                        .transition(.move(edge: .bottom).combined(with: .opacity))
                }
            }
            .onKeyPress(.delete) {
                Task { await zoomOut() }
                return .handled
            }
            .animation(reduceMotion ? nil : .snappy(duration: 0.32), value: browser.currentDirectory?.id)
            .animation(reduceMotion ? nil : .easeOut(duration: 0.16), value: hoveredVisual)
            .animation(reduceMotion ? nil : .easeInOut(duration: 0.24), value: lens)
            .accessibilityElement(children: .contain)
            .accessibilityLabel("Allocated-space map for \(browser.currentDirectory?.name.display ?? "folder")")
        }
    }

    private func volumeCircle(
        _ circle: ExplorerDiskMapLayoutCircle,
        envelope: ExplorerDiskMapVolumeEnvelope
    ) -> some View {
        let detail = volumeDetail(circle.id, envelope: envelope)
        let canZoom = circle.id == .scanned && browser.diskMap != nil
        return Button {
            hoveredVisual = circle.id
            if canZoom { showingDirectory = true }
        } label: {
            circleLabel(
                title: detail.title,
                detail: StorageByteFormatter.string(from: detail.bytes),
                color: volumeColor(circle.id),
                radius: circle.radius,
                selected: false,
                hovered: hoveredVisual == circle.id,
                dashed: circle.id == .unmapped,
                symbol: circle.id == .scanned ? "folder.fill" : nil
            )
        }
        .buttonStyle(.plain)
        .frame(width: circle.radius * 2, height: circle.radius * 2)
        .position(circle.center)
        .onHover { hoveredVisual = $0 ? circle.id : nil }
        .help("\(detail.title): \(StorageByteFormatter.string(from: detail.bytes)). \(detail.note)")
        .accessibilityLabel(detail.title)
        .accessibilityValue(StorageByteFormatter.string(from: detail.bytes))
        .accessibilityHint(canZoom ? "Press Return to zoom into the saved scan." : detail.note)
    }

    @ViewBuilder
    private func directoryCircle(
        _ circle: ExplorerDiskMapLayoutCircle,
        diskMap: ExplorerSnapshotDiskMap
    ) -> some View {
        switch circle.id {
        case let .node(nodeID):
            if let cell = diskMap.cell(nodeID: nodeID), let bytes = cell.node.allocatedBytes {
                Button {
                    hoveredVisual = circle.id
                    if cell.node.kind == .directory {
                        Task { await browser.openDirectory(cell.node) }
                    } else {
                        Task { await browser.selectDiskMapCell(cell) }
                    }
                } label: {
                    circleLabel(
                        title: cell.node.name.display,
                        detail: StorageByteFormatter.string(from: bytes),
                        color: color(for: cell.node.category),
                        radius: circle.radius,
                        selected: browser.selectedNodeID == nodeID,
                        hovered: hoveredVisual == circle.id,
                        dashed: differentiateWithoutColor,
                        symbol: cell.node.kind == .directory ? "folder.fill" : nil
                    )
                }
                .buttonStyle(.plain)
                .frame(width: circle.radius * 2, height: circle.radius * 2)
                .position(circle.center)
                .onHover { hoveredVisual = $0 ? circle.id : nil }
                .help(nodeHelp(cell.node, bytes: bytes, total: diskMap.totalChildAllocatedBytes))
                .accessibilityLabel(cell.node.name.display)
                .accessibilityValue(
                    "\(StorageByteFormatter.string(from: bytes)), \(lensTitle(for: cell.node.category))"
                )
                .accessibilityHint(
                    cell.node.kind == .directory
                        ? "Press Return to zoom into this folder."
                        : "Press Return to select this item."
                )
                .accessibilityIdentifier(
                    ExplorerAccessibility.diskMapCell(nodeID: cell.node.id)
                )
            }
        case .other:
            Button {
                hoveredVisual = .other
            } label: {
                circleLabel(
                    title: "Other",
                    detail: StorageByteFormatter.string(from: diskMap.otherAllocatedBytes),
                    color: .secondary,
                    radius: circle.radius,
                    selected: false,
                    hovered: hoveredVisual == .other,
                    dashed: true,
                    symbol: "ellipsis"
                )
            }
            .buttonStyle(.plain)
            .frame(width: circle.radius * 2, height: circle.radius * 2)
            .position(circle.center)
            .onHover { hoveredVisual = $0 ? .other : nil }
            .help("\(diskMap.otherChildCount) smaller children; open Browse to inspect them.")
            .accessibilityLabel("Other smaller children")
            .accessibilityValue(StorageByteFormatter.string(from: diskMap.otherAllocatedBytes))
            .accessibilityIdentifier(ExplorerAccessibility.diskMapOther)
        case .used, .scanned, .available, .unmapped:
            EmptyView()
        }
    }

    private func circleLabel(
        title: String,
        detail: String,
        color: Color,
        radius: CGFloat,
        selected: Bool,
        hovered: Bool,
        dashed: Bool,
        symbol: String? = nil
    ) -> some View {
        ZStack {
            Circle().fill(
                LinearGradient(
                    colors: [
                        color.opacity(selected ? 0.5 : hovered ? 0.4 : 0.3),
                        color.opacity(selected ? 0.3 : hovered ? 0.25 : 0.16),
                    ],
                    startPoint: .topLeading,
                    endPoint: .bottomTrailing
                )
            )
            Circle().strokeBorder(
                selected
                    ? Color.accentColor
                    : hovered ? Color.primary.opacity(0.78) : color.opacity(0.82),
                style: StrokeStyle(
                    lineWidth: selected ? 3.5 : hovered ? 2.4 : 1.5,
                    dash: dashed ? [7, 4] : []
                )
            )
            if radius >= 28 {
                VStack(spacing: radius >= 54 ? 3 : 1) {
                    if let symbol, radius >= 54 {
                        Image(systemName: symbol)
                            .font(radius >= 78 ? .title3 : .caption)
                            .foregroundStyle(.secondary)
                    }
                    Text(verbatim: title)
                        .font(radius >= 62 ? .headline : .caption.bold())
                        .lineLimit(radius >= 62 ? 2 : 1)
                        .minimumScaleFactor(0.72)
                    if radius >= 42 {
                        Text(verbatim: detail)
                            .font(.caption.monospacedDigit())
                            .foregroundStyle(.secondary)
                    }
                }
                .multilineTextAlignment(.center)
                .padding(max(5, radius * 0.18))
            }
        }
        .contentShape(Circle())
        .scaleEffect(selected ? 1.035 : hovered ? 1.018 : 1)
        .shadow(
            color: selected
                ? Color.accentColor.opacity(0.32)
                : hovered ? Color.black.opacity(0.2) : .clear,
            radius: selected ? 12 : hovered ? 7 : 0,
            y: selected ? 4 : hovered ? 3 : 0
        )
        .animation(reduceMotion ? nil : .easeOut(duration: 0.14), value: selected)
        .animation(reduceMotion ? nil : .easeOut(duration: 0.14), value: hovered)
    }

    private var footer: some View {
        VStack(alignment: .leading, spacing: 4) {
            if let scanNotice {
                Text(scanNotice)
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            ViewThatFits(in: .horizontal) {
                HStack {
                    zoomInstructions
                    Spacer()
                    allocationFootnote
                }
                VStack(alignment: .leading, spacing: 2) {
                    zoomInstructions
                    allocationFootnote
                }
            }
            .font(.caption)
            .foregroundStyle(.secondary)
        }
    }

    private var zoomInstructions: some View {
        Text("Click a folder to zoom. Click the background or press Backspace to move up.")
    }

    @ViewBuilder
    private var allocationFootnote: some View {
        if showingDirectory, let diskMap = browser.diskMap {
            Text(allocationUnknownDescription(diskMap))
        }
    }

    private func scanStartupVolume() async {
        let name = model.volumeState.snapshot?.displayName ?? "Startup volume"
        scanNotice = "Scanning \(name)…"
        let outcome = await model.startStartupVolumeScan(displayName: name)
        switch outcome {
        case .succeeded:
            scanNotice = "Scan complete. Loading the newest allocated-space snapshot…"
            await browser.reloadLatest()
            showingDirectory = browser.diskMap != nil
        case .cancelled:
            scanNotice = "Disk scan cancelled."
        case let .failed(failure):
            scanNotice = "Disk scan could not finish (\(String(describing: failure)))."
        case .superseded:
            scanNotice = "A newer scan request replaced this one."
        }
    }

    private func zoomOut() async {
        guard showingDirectory else { return }
        if browser.breadcrumbs.count > 1 {
            await browser.goBack()
        } else {
            showingDirectory = false
        }
    }

    private func directoryLayoutItems(
        _ diskMap: ExplorerSnapshotDiskMap
    ) -> [ExplorerDiskMapLayoutItem] {
        var items = diskMap.cells.compactMap { cell -> ExplorerDiskMapLayoutItem? in
            guard let bytes = cell.node.allocatedBytes, bytes > 0 else { return nil }
            return ExplorerDiskMapLayoutItem(id: .node(cell.id), allocatedBytes: bytes)
        }
        if diskMap.otherAllocatedBytes > 0 {
            items.append(.init(id: .other, allocatedBytes: diskMap.otherAllocatedBytes))
        }
        return items
    }

    private func squareStage(in bounds: CGRect) -> CGRect {
        let side = min(bounds.width, bounds.height)
        return CGRect(
            x: bounds.midX - side / 2,
            y: bounds.midY - side / 2,
            width: side,
            height: side
        )
    }

    private func volumeColor(_ id: ExplorerDiskMapVisualID) -> Color {
        switch id {
        case .used: .teal
        case .scanned: .teal
        case .available: .green
        case .unmapped: .secondary
        case .other, .node: .secondary
        }
    }

    private func color(for category: ExplorerStorageCategory) -> Color {
        switch lens {
        case .storageType:
            category.presentation.palette.color
        case .cleanupStatus:
            switch category {
            case .protectedSystemData: .red
            case .unclassified, .unknownStorage: .gray
            case .developerArtifact, .applicationCache, .browserCache,
                 .logAndDiagnostic, .deviceAndSimulatorData:
                .orange
            case .installerAndDownload, .cloudFile, .largeReviewItem:
                .blue
            }
        }
    }

    private func lensTitle(for category: ExplorerStorageCategory) -> String {
        switch lens {
        case .storageType:
            category.presentation.title
        case .cleanupStatus:
            switch category {
            case .protectedSystemData: "Protected"
            case .unclassified, .unknownStorage: "Unclassified"
            case .developerArtifact, .applicationCache, .browserCache,
                 .logAndDiagnostic, .deviceAndSimulatorData:
                "Potentially reclaimable"
            case .installerAndDownload, .cloudFile, .largeReviewItem:
                "Selective review"
            }
        }
    }

    private func volumeDetail(
        _ id: ExplorerDiskMapVisualID,
        envelope: ExplorerDiskMapVolumeEnvelope
    ) -> ExplorerDiskMapDetail {
        switch id {
        case .used:
            ExplorerDiskMapDetail(
                title: "Used space",
                bytes: envelope.usedBytes,
                rows: [
                    .init(label: "Share of volume", value: percent(envelope.usedBytes, of: envelope.totalBytes)),
                    .init(label: "Source", value: "Filesystem capacity"),
                    .init(label: "Contains", value: "Measured + unmapped"),
                ],
                note: browser.diskMap == nil
                    ? "Scan the disk to explore the measured hierarchy."
                    : "Click to zoom into the latest measured snapshot."
            )
        case .scanned:
            ExplorerDiskMapDetail(
                title: "Measured by latest scan",
                bytes: envelope.measuredBytes,
                rows: [
                    .init(label: "Share of used", value: percent(envelope.measuredBytes, of: envelope.usedBytes)),
                    .init(label: "Type", value: "Saved snapshot"),
                    .init(label: "Coverage", value: "Measured allocation"),
                ],
                note: "Click to zoom into the saved folder hierarchy."
            )
        case .available:
            ExplorerDiskMapDetail(
                title: "Available",
                bytes: envelope.availableBytes,
                rows: [
                    .init(label: "Share of volume", value: percent(envelope.availableBytes, of: envelope.totalBytes)),
                    .init(label: "Source", value: "Filesystem reported"),
                    .init(label: "Status", value: "Available to use"),
                ],
                note: "Ordinary filesystem availability reported for this volume."
            )
        case .unmapped:
            ExplorerDiskMapDetail(
                title: "Used, not mapped",
                bytes: envelope.unmappedBytes,
                rows: [
                    .init(label: "Share of used", value: percent(envelope.unmappedBytes, of: envelope.usedBytes)),
                    .init(label: "Type", value: "Outside saved scan"),
                    .init(label: "Cleanup", value: "Unclassified"),
                ],
                note: "Used volume space outside the latest scan's measured allocation."
            )
        case .other, .node:
            ExplorerDiskMapDetail(title: "Other", bytes: 0, rows: [], note: "")
        }
    }

    private func volumeHoverDetail(
        _ envelope: ExplorerDiskMapVolumeEnvelope
    ) -> ExplorerDiskMapDetail? {
        guard let hoveredVisual else { return nil }
        return volumeDetail(hoveredVisual, envelope: envelope)
    }

    private func directoryHoverDetail(
        _ diskMap: ExplorerSnapshotDiskMap
    ) -> ExplorerDiskMapDetail? {
        let focusedVisual = hoveredVisual ?? browser.selectedNodeID.map(ExplorerDiskMapVisualID.node)
        guard let focusedVisual else { return nil }
        switch focusedVisual {
        case let .node(id):
            guard let cell = diskMap.cell(nodeID: id), let bytes = cell.node.allocatedBytes else {
                return nil
            }
            return ExplorerDiskMapDetail(
                title: cell.node.name.display,
                bytes: bytes,
                rows: [
                    .init(label: "Share of parent", value: percent(bytes, of: diskMap.totalChildAllocatedBytes)),
                    .init(label: "Type", value: cell.node.category.presentation.title),
                    .init(label: "Cleanup", value: cleanupTitle(for: cell.node.category)),
                    .init(label: "Item", value: nodeKindTitle(cell.node.kind)),
                ],
                note: cell.node.kind == .directory
                    ? "Click to zoom into this folder."
                    : "Click to keep this item selected; open Browse for file actions."
            )
        case .other:
            return ExplorerDiskMapDetail(
                title: "Other",
                bytes: diskMap.otherAllocatedBytes,
                rows: [
                    .init(label: "Share of parent", value: percent(diskMap.otherAllocatedBytes, of: diskMap.totalChildAllocatedBytes)),
                    .init(label: "Items", value: diskMap.otherChildCount.formatted()),
                    .init(label: "Type", value: "Smaller allocations"),
                ],
                note: "Open Browse to inspect the aggregated children individually."
            )
        case .used, .scanned, .available, .unmapped:
            return nil
        }
    }

    private func detailCard(_ detail: ExplorerDiskMapDetail) -> some View {
        VStack(alignment: .leading, spacing: 9) {
            Text(verbatim: detail.title)
                .font(.headline)
                .lineLimit(2)
            Text(StorageByteFormatter.string(from: detail.bytes))
                .font(.title3.monospacedDigit().weight(.medium))
            VStack(spacing: 4) {
                ForEach(detail.rows) { row in
                    HStack(alignment: .firstTextBaseline, spacing: 12) {
                        Text(verbatim: row.label)
                            .foregroundStyle(.secondary)
                        Spacer(minLength: 8)
                        Text(verbatim: row.value)
                            .multilineTextAlignment(.trailing)
                    }
                }
            }
            .font(.caption)
            Divider()
            Text(verbatim: detail.note)
                .font(.caption)
                .foregroundStyle(.secondary)
        }
        .padding(14)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 12, style: .continuous))
        .overlay {
            RoundedRectangle(cornerRadius: 12, style: .continuous)
                .strokeBorder(Color.primary.opacity(0.14), lineWidth: 1)
        }
        .shadow(color: .black.opacity(0.3), radius: 18, y: 8)
        .allowsHitTesting(false)
    }

    private func cleanupTitle(for category: ExplorerStorageCategory) -> String {
        switch category {
        case .protectedSystemData: "Protected"
        case .unclassified, .unknownStorage: "Unclassified"
        case .developerArtifact, .applicationCache, .browserCache,
             .logAndDiagnostic, .deviceAndSimulatorData:
            "Potentially reclaimable"
        case .installerAndDownload, .cloudFile, .largeReviewItem:
            "Selective review"
        }
    }

    private func nodeKindTitle(_ kind: ExplorerSnapshotNodeKind) -> String {
        switch kind {
        case .directory: "Folder"
        case .file: "File"
        case .symlink: "Symbolic link"
        case .other: "Other"
        case .error: "Unavailable"
        }
    }

    private func nodeHelp(
        _ node: ExplorerSnapshotNode,
        bytes: UInt64,
        total: UInt64
    ) -> String {
        "\(node.name.display) · \(StorageByteFormatter.string(from: bytes)) · \(percent(bytes, of: total)) · \(lensTitle(for: node.category))"
    }

    private func percent(_ bytes: UInt64, of total: UInt64) -> String {
        guard total > 0 else { return "0%" }
        return (Double(bytes) / Double(total)).formatted(.percent.precision(.fractionLength(1)))
    }

    private func allocationUnknownDescription(_ diskMap: ExplorerSnapshotDiskMap) -> String {
        var parts: [String] = []
        if diskMap.unknownAllocatedChildCount > 0 {
            parts.append("\(diskMap.unknownAllocatedChildCount) allocation-unknown")
        }
        if diskMap.zeroAllocatedChildCount > 0 {
            parts.append("\(diskMap.zeroAllocatedChildCount) zero-allocation")
        }
        return parts.isEmpty
            ? "All recorded child allocation is represented."
            : "Not plotted by area: \(parts.joined(separator: ", "))."
    }
}
