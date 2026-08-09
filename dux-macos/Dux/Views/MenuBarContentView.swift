import SwiftUI

struct MenuBarContentView: View {
    @Environment(\.openWindow) private var openWindow

    let model: AppModel

    var body: some View {
        let presentation = MenuBarPopoverPresentation.make(
            volumeState: model.volumeState,
            scanState: model.scanState,
            trend: model.capacityTrend,
            pressureHistory: model.pressureHistoryState.history
        )
        let emergencyRecovery = ExplorerEmergencyRecoveryPresentation.make(
            volumeState: model.volumeState,
            targetedState: model.targetedReclaimScanState
        )

        VStack(alignment: .leading, spacing: 14) {
#if DUX_CLEANUP_QUALIFICATION
            CleanupQualificationNotice(compact: true)
            Divider()
#endif

            if model.showsStorageAccessIntroduction {
                storageAccessIntroduction
                Divider()
            }

            volumeSummary(presentation.volume, actions: presentation.actions)

            if let scan = presentation.scan {
                scanSummary(scan, actions: presentation.actions)
            }

            if !emergencyRecovery.cards.isEmpty {
                emergencyRecoverySummary(emergencyRecovery)
            }

            if model.volumeState.snapshot.map({
                $0.pressure == .warning || $0.pressure == .critical
            }) == true || model.targetedReclaimScanState.isActive
            {
                targetedReclaimSummary(
                    TargetedReclaimScanPresentation.make(
                        model.targetedReclaimScanState
                    )
                )
            }

            Divider()

            Button {
                AppActivation.openExplorer(using: openWindow)
            } label: {
                Label("Open Explorer", systemImage: "rectangle.on.rectangle")
                    .frame(maxWidth: .infinity)
            }
            .buttonStyle(.borderedProminent)
            .controlSize(.large)
            .keyboardShortcut(
                KeyEquivalent(MenuBarPopoverKeyboardShortcut.openExplorer),
                modifiers: [.command]
            )
            .accessibilityIdentifier(MenuBarPopoverAccessibility.openExplorer)
            .accessibilityHint("Opens or focuses the Storage Explorer window")

            HStack(spacing: 10) {
                if presentation.actions.showScanNow {
                    Button("Scan now") {
                        Task { await model.startHomeScan() }
                    }
                    .disabled(!presentation.actions.scanNowEnabled)
                    .keyboardShortcut(
                        KeyEquivalent(MenuBarPopoverKeyboardShortcut.scanNow),
                        modifiers: [.command]
                    )
                    .accessibilityIdentifier(MenuBarPopoverAccessibility.scanNow)
                    .accessibilityHint("Scans the Home folder without changing files")
                }

                Button("Settings…") {
                    model.requestExplorerDestination(.settings)
                    AppActivation.openExplorer(using: openWindow)
                }
                .keyboardShortcut(
                    KeyEquivalent(MenuBarPopoverKeyboardShortcut.settings),
                    modifiers: [.command]
                )
                .accessibilityIdentifier(MenuBarPopoverAccessibility.settings)

                Spacer(minLength: 6)

                Button("Quit DUX") {
                    AppActivation.quit()
                }
                .keyboardShortcut(
                    KeyEquivalent(MenuBarPopoverKeyboardShortcut.quit),
                    modifiers: [.command]
                )
                .accessibilityIdentifier(MenuBarPopoverAccessibility.quit)
            }
        }
        .padding(16)
        .frame(width: 372)
        .accessibilityIdentifier(MenuBarPopoverAccessibility.root)
    }

    private var storageAccessIntroduction: some View {
        VStack(alignment: .leading, spacing: 8) {
            Label("Understand your storage locally", systemImage: "hand.raised.fill")
                .font(.headline)
            Text(
                String(
                    localized:
                        "Home scans run locally and never change your files. DUX starts with the access your account already has; Full Disk Access is optional."
                )
            )
            .font(.caption)
            .foregroundStyle(.secondary)
            Text(
                String(
                    localized:
                        "When you scan, DUX reports limited coverage instead of pretending it saw everything."
                )
            )
            .font(.caption)
            .foregroundStyle(.secondary)
            Button("Continue") {
                model.acknowledgeStorageAccessIntroduction()
            }
            .accessibilityIdentifier(StorageAccessAccessibility.introductionContinue)
            .accessibilityHint("Dismisses this introduction without starting a scan")
        }
        .padding(10)
        .background(.quaternary, in: RoundedRectangle(cornerRadius: 9))
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier(StorageAccessAccessibility.introduction)
    }

    @ViewBuilder
    private func volumeSummary(
        _ volume: MenuBarPopoverVolumePresentation,
        actions: MenuBarPopoverActionMatrix
    ) -> some View {
        switch volume {
        case let .loading(message):
            HStack(spacing: 10) {
                ProgressView()
                    .controlSize(.small)
                    .accessibilityLabel(Text(verbatim: message))
                Text(verbatim: message)
                    .foregroundStyle(.secondary)
            }
            .accessibilityElement(children: .combine)
            .accessibilityIdentifier(MenuBarPopoverAccessibility.volumeSummary)

        case let .snapshot(snapshot, status):
            VStack(alignment: .leading, spacing: 9) {
                HStack(alignment: .firstTextBaseline, spacing: 10) {
                    Text(verbatim: snapshot.volumeName)
                        .font(.subheadline.weight(.semibold))
                        .lineLimit(1)
                        .truncationMode(.middle)
                        .help(snapshot.volumeName)
                        .accessibilityIdentifier(MenuBarPopoverAccessibility.volumeName)

                    Spacer(minLength: 8)

                    DiskPressureBadge(pressure: snapshot.pressure)
                        .accessibilityIdentifier(MenuBarPopoverAccessibility.pressure)
                }

                VStack(alignment: .leading, spacing: 2) {
                    Text(verbatim: snapshot.availableHeadline)
                        .font(.system(size: 26, weight: .bold, design: .rounded))
                        .monospacedDigit()
                        .foregroundStyle(DuxTheme.tint(for: snapshot.pressure).gradient)
                        .accessibilityIdentifier(MenuBarPopoverAccessibility.available)
                        .accessibilitySortPriority(snapshot.isCritical ? 2 : 0)
                    Text(verbatim: snapshot.totalText)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .monospacedDigit()
                        .accessibilityIdentifier(MenuBarPopoverAccessibility.total)
                }

                ProgressView(value: snapshot.availableFraction)
                    .progressViewStyle(.linear)
                    .tint(DuxTheme.tint(for: snapshot.pressure))
                    .accessibilityIdentifier(MenuBarPopoverAccessibility.capacityBar)
                    .accessibilityLabel(Text(verbatim: snapshot.availabilityBasisText))
                    .accessibilityValue(Text(verbatim: snapshot.availablePercentText))

                HStack(alignment: .firstTextBaseline, spacing: 8) {
                    Text(verbatim: snapshot.availabilityBasisText)
                    Spacer(minLength: 8)
                    Text(verbatim: snapshot.availablePercentText)
                        .monospacedDigit()
                }
                .font(.caption)
                .foregroundStyle(.secondary)

                Text(verbatim: snapshot.freshnessText)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .accessibilityIdentifier(MenuBarPopoverAccessibility.freshness)

                if let trend = snapshot.trend {
                    trendSummary(trend)
                }

                if let activePressurePeriod = snapshot.activePressurePeriodText {
                    Label(activePressurePeriod, systemImage: "clock.badge.exclamationmark")
                        .font(.caption)
                        .foregroundStyle(
                            snapshot.pressure == .critical ? Color.red : Color.orange
                        )
                        .accessibilityIdentifier(
                            MenuBarPopoverAccessibility.activePressurePeriod
                        )
                }

                if let status {
                    capacityStatus(status, actions: actions)
                }
            }
            .accessibilityElement(children: .contain)
            .accessibilityIdentifier(MenuBarPopoverAccessibility.volumeSummary)
            .accessibilityLabel(Text(verbatim: snapshot.accessibilitySummary))

        case let .failed(title, detail):
            VStack(alignment: .leading, spacing: 8) {
                Label(title, systemImage: "exclamationmark.triangle.fill")
                    .fontWeight(.semibold)
                    .foregroundStyle(.red)
                Text(verbatim: detail)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                if actions.showCapacityRetry {
                    capacityRetryButton
                }
            }
            .accessibilityElement(children: .contain)
            .accessibilityIdentifier(MenuBarPopoverAccessibility.volumeSummary)
        }
    }

    private func trendSummary(_ trend: MenuBarPopoverTrendPresentation) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 12) {
                if let change = trend.change24hText {
                    Label("24h \(change)", systemImage: "arrow.left.arrow.right")
                }
                if let change = trend.change7dText {
                    Label("7d \(change)", systemImage: "calendar")
                }
            }
            .font(.caption.monospacedDigit())
            .foregroundStyle(.secondary)

            if trend.chartFractions.count >= 2 {
                ZStack {
                    TrendSparklineArea(fractions: trend.chartFractions)
                        .fill(
                            LinearGradient(
                                colors: [Color.accentColor.opacity(0.25), .clear],
                                startPoint: .top,
                                endPoint: .bottom
                            )
                        )
                    TrendSparkline(fractions: trend.chartFractions)
                        .stroke(
                            Color.accentColor,
                            style: StrokeStyle(lineWidth: 1.5, lineCap: .round)
                        )
                }
                .frame(height: 28)
                .accessibilityIdentifier(MenuBarPopoverAccessibility.trendChart)
                .accessibilityLabel(Text("30-day available-space trend"))
                .accessibilityValue(Text(verbatim: trend.accessibilitySummary))
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier(MenuBarPopoverAccessibility.trend)
        .accessibilityLabel(Text("Capacity trend"))
        .accessibilityValue(Text(verbatim: trend.accessibilitySummary))
    }

    private func capacityStatus(
        _ status: MenuBarPopoverCapacityStatus,
        actions: MenuBarPopoverActionMatrix
    ) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: 8) {
            if status.showsProgress {
                ProgressView()
                    .controlSize(.small)
                    .accessibilityHidden(true)
            } else {
                Image(systemName: "exclamationmark.triangle")
                    .foregroundStyle(.orange)
                    .accessibilityHidden(true)
            }
            Text(verbatim: status.message)
                .font(.caption)
                .foregroundStyle(status.style == .stale ? .orange : .secondary)
            Spacer(minLength: 6)
            if actions.showCapacityRetry {
                capacityRetryButton
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier(MenuBarPopoverAccessibility.capacityStatus)
    }

    private var capacityRetryButton: some View {
        Button("Try again") {
            Task { await model.refreshVolumeCapacity() }
        }
        .controlSize(.small)
        .accessibilityIdentifier(MenuBarPopoverAccessibility.capacityRetry)
        .accessibilityHint("Checks startup-disk capacity again without starting a scan")
    }

    private func targetedReclaimSummary(
        _ focused: TargetedReclaimScanPresentation
    ) -> some View {
        VStack(alignment: .leading, spacing: 7) {
            HStack(alignment: .firstTextBaseline, spacing: 8) {
                Image(systemName: focused.symbol)
                    .foregroundStyle(focused.tone == .failure ? Color.red : Color.accentColor)
                    .accessibilityHidden(true)
                Text(focused.title)
                    .font(.subheadline.weight(.semibold))
                    .lineLimit(1)
                Spacer(minLength: 6)
                if focused.canCancel {
                    Button("Stop") {
                        Task { await model.cancelTargetedReclaimScan() }
                    }
                    .controlSize(.small)
                }
            }
            Text(focused.detail)
                .font(.caption)
                .foregroundStyle(.secondary)
                .lineLimit(2)
            if let progress = focused.progressValue,
               let label = focused.progressLabel
            {
                ProgressView(value: progress)
                    .accessibilityLabel("Focused recovery scan")
                    .accessibilityValue(label)
                    .accessibilityIdentifier(
                        MenuBarPopoverAccessibility.targetedReclaimProgress
                    )
            }
        }
        .padding(10)
        .background(.quaternary, in: RoundedRectangle(cornerRadius: 9))
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier(MenuBarPopoverAccessibility.targetedReclaim)
    }

    private func emergencyRecoverySummary(
        _ recovery: ExplorerEmergencyRecoveryPresentation
    ) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(alignment: .firstTextBaseline, spacing: 8) {
                Label("Recovery order", systemImage: "list.number")
                    .font(.subheadline.weight(.semibold))
                Spacer()
                Text(recovery.compactEvidenceLabel)
                    .font(.caption2.weight(.semibold))
                    .foregroundStyle(
                        recovery.evidenceState == .current ? .primary : .secondary
                    )
            }
            Text(recovery.statusText)
                .font(.caption)
                .foregroundStyle(.secondary)
                .lineLimit(2)
            Text(recovery.freshnessText)
                .font(.caption2)
                .foregroundStyle(.tertiary)
            ForEach(recovery.menuBarCards) { card in
                HStack(alignment: .firstTextBaseline, spacing: 8) {
                    Text("\(card.lane.rawValue)")
                        .font(.caption.bold().monospacedDigit())
                        .foregroundStyle(.secondary)
                    Text(card.title)
                        .font(.caption)
                        .lineLimit(1)
                }
                .accessibilityElement(children: .combine)
                .accessibilityLabel(card.accessibilitySummary)
                .accessibilityIdentifier(
                    MenuBarPopoverAccessibility.emergencyRecoveryCard(rank: card.rank)
                )
            }
            Button("Review recovery order") {
                model.requestExplorerDestination(.recommendations)
                AppActivation.openExplorer(using: openWindow)
            }
            .controlSize(.small)
            .accessibilityIdentifier(
                MenuBarPopoverAccessibility.emergencyRecoveryAction
            )
            .accessibilityHint(
                "Opens the evidence-backed read-only recovery order in Explorer"
            )
        }
        .padding(10)
        .background(.quaternary, in: RoundedRectangle(cornerRadius: 9))
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier(MenuBarPopoverAccessibility.emergencyRecovery)
    }

    private func scanSummary(
        _ scan: MenuBarPopoverScanPresentation,
        actions: MenuBarPopoverActionMatrix
    ) -> some View {
        HStack(alignment: .top, spacing: 10) {
            scanIcon(for: scan)

            VStack(alignment: .leading, spacing: 4) {
                Text(verbatim: scan.title)
                    .font(.subheadline.weight(.semibold))
                if let detail = scan.detail {
                    Text(verbatim: detail)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
                if scan.showsIndeterminateProgress {
                    ProgressView()
                        .progressViewStyle(.linear)
                        .accessibilityIdentifier(MenuBarPopoverAccessibility.scanProgress)
                        .accessibilityLabel(Text(verbatim: scan.title))
                        .accessibilityValue(
                            Text(verbatim: scan.progressAccessibilityValue ?? "")
                        )
                }
            }

            Spacer(minLength: 6)

            if actions.showScanCancel {
                Button("Cancel") {
                    Task { await model.cancelHomeScan() }
                }
                .controlSize(.small)
                .disabled(!actions.scanCancelEnabled)
                .keyboardShortcut(
                    KeyEquivalent(MenuBarPopoverKeyboardShortcut.cancelScan),
                    modifiers: [.command]
                )
                .accessibilityIdentifier(MenuBarPopoverAccessibility.scanCancel)
                .accessibilityHint("Stops the scan; previous results remain available")
            }
        }
        .padding(10)
        .background(.quaternary, in: RoundedRectangle(cornerRadius: 9))
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier(MenuBarPopoverAccessibility.scanStatus)
    }

    @ViewBuilder
    private func scanIcon(for scan: MenuBarPopoverScanPresentation) -> some View {
        switch scan.style {
        case .progress:
            ProgressView()
                .controlSize(.small)
                .accessibilityHidden(true)
        case .success:
            Image(systemName: "checkmark.circle.fill")
                .foregroundStyle(.green)
                .accessibilityHidden(true)
        case .cancelled:
            Image(systemName: "stop.circle")
                .foregroundStyle(.secondary)
                .accessibilityHidden(true)
        case .failure:
            Image(systemName: "exclamationmark.triangle.fill")
                .foregroundStyle(.red)
                .accessibilityHidden(true)
        }
    }
}

private struct TrendSparkline: Shape {
    let fractions: [Double]

    func path(in rect: CGRect) -> Path {
        guard fractions.count >= 2 else { return Path() }
        let width = rect.width
        let height = rect.height
        let step = width / CGFloat(fractions.count - 1)
        var path = Path()
        for (index, fraction) in fractions.enumerated() {
            let clamped = max(0.0, min(1.0, fraction))
            let point = CGPoint(
                x: CGFloat(index) * step,
                y: height * (1 - CGFloat(clamped))
            )
            if index == 0 {
                path.move(to: point)
            } else {
                path.addLine(to: point)
            }
        }
        return path
    }
}

private struct TrendSparklineArea: Shape {
    let fractions: [Double]

    func path(in rect: CGRect) -> Path {
        var path = TrendSparkline(fractions: fractions).path(in: rect)
        guard !path.isEmpty else { return path }
        path.addLine(to: CGPoint(x: rect.maxX, y: rect.maxY))
        path.addLine(to: CGPoint(x: rect.minX, y: rect.maxY))
        path.closeSubpath()
        return path
    }
}
