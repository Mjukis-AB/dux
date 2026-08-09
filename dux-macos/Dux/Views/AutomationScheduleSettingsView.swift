import SwiftUI

enum AutomationScheduleAccessibility {
    static let section = "automation-schedules-section"
    static let globalStatus = "automation-schedules-global-status"
    static let executionStatus = "automation-schedules-execution-status"
    static let eligibleRuleCount = "automation-schedules-eligible-rule-count"
    static let draftCount = "automation-schedules-draft-count"
    static let defaults = "automation-schedules-defaults"
    static let safetyDisclosure = "automation-schedules-safety-disclosure"
    static let draftList = "automation-schedules-draft-list"
    static let progress = "automation-schedules-progress"
    static let error = "automation-schedules-error"
    static let reload = "automation-schedules-reload"
    static let draftRowPrefix = "automation-schedules-draft-"

    static let allStaticIdentifiers = [
        section,
        globalStatus,
        executionStatus,
        eligibleRuleCount,
        draftCount,
        defaults,
        safetyDisclosure,
        draftList,
        progress,
        error,
        reload,
    ]

    static func draftRow(_ index: Int) -> String {
        draftRowPrefix + String(index)
    }
}

struct AutomationScheduleSettingsView: View {
    @Bindable var settings: AutomationScheduleSettingsModel

    var body: some View {
        Section("Automations") {
            VStack(alignment: .leading, spacing: 12) {
                Label("Automations are off", systemImage: "lock.shield.fill")
                    .font(.headline)
                    .foregroundStyle(.secondary)

                Text(
                    "DUX does not schedule cleanup in this build. A disabled draft records "
                        + "preferences only: it grants no cleanup permission and cannot run "
                        + "by itself."
                )
                .foregroundStyle(.secondary)
                .accessibilityIdentifier(
                    AutomationScheduleAccessibility.safetyDisclosure
                )

                if let overview = settings.overview {
                    statusGrid(overview)

                    Text(
                        "Eligible-rule count reflects shipped rule policy only. Manual "
                            + "history, current candidate evidence, and activity checks are "
                            + "separate gates for a later release."
                    )
                    .font(.caption)
                    .foregroundStyle(.secondary)

                    defaultsCard

                    if overview.disabledDrafts.isEmpty {
                        ContentUnavailableView(
                            "No automation drafts",
                            systemImage: "calendar.badge.minus",
                            description: Text(
                                "There are no disabled schedule drafts. DUX currently has "
                                    + "zero shipped rules marked and structurally valid for "
                                    + "scheduling."
                            )
                        )
                    } else {
                        VStack(alignment: .leading, spacing: 8) {
                            Text("Disabled drafts")
                                .font(.headline)
                            ForEach(
                                Array(overview.disabledDrafts.enumerated()),
                                id: \.element.id
                            ) { index, draft in
                                draftCard(draft, index: index)
                            }
                        }
                        .accessibilityElement(children: .contain)
                        .accessibilityIdentifier(
                            AutomationScheduleAccessibility.draftList
                        )
                    }
                }

                if settings.state.isLoading {
                    ProgressView("Loading automation drafts…")
                        .controlSize(.small)
                        .accessibilityIdentifier(
                            AutomationScheduleAccessibility.progress
                        )
                }

                if case let .failed(failure) = settings.state {
                    Label(Self.message(for: failure), systemImage: "exclamationmark.triangle")
                        .foregroundStyle(.red)
                        .accessibilityIdentifier(
                            AutomationScheduleAccessibility.error
                        )

                    Button("Try loading drafts again") {
                        Task { await settings.load(force: true) }
                    }
                    .accessibilityIdentifier(
                        AutomationScheduleAccessibility.reload
                    )
                    .accessibilityHint(
                        "Reads disabled drafts only and never starts cleanup"
                    )
                }

                Text(
                    "A later automation release must re-plan and revalidate every run. Only "
                        + "shipped SafeRegenerable rules with repeated successful manual "
                        + "history can become eligible; AI can never approve or run cleanup."
                )
                .font(.caption)
                .foregroundStyle(.secondary)
            }
            .accessibilityElement(children: .contain)
            .accessibilityIdentifier(AutomationScheduleAccessibility.section)
        }
    }

    @ViewBuilder
    private func statusGrid(_ overview: AutomationScheduleOverviewModel) -> some View {
        Grid(alignment: .leading, horizontalSpacing: 18, verticalSpacing: 6) {
            GridRow {
                LabeledContent("Global switch") {
                    Text(overview.globalEnabled ? "On" : "Off")
                }
                .accessibilityIdentifier(
                    AutomationScheduleAccessibility.globalStatus
                )

                LabeledContent("Execution") {
                    Text(overview.executionAvailable ? "Available" : "Unavailable")
                }
                .accessibilityIdentifier(
                    AutomationScheduleAccessibility.executionStatus
                )
            }

            GridRow {
                LabeledContent("Eligible rules") {
                    Text(verbatim: String(overview.eligibleRuleCount))
                }
                .accessibilityIdentifier(
                    AutomationScheduleAccessibility.eligibleRuleCount
                )

                LabeledContent("Disabled drafts") {
                    Text(verbatim: String(overview.disabledDrafts.count))
                }
                .accessibilityIdentifier(
                    AutomationScheduleAccessibility.draftCount
                )
            }
        }
    }

    private var defaultsCard: some View {
        GroupBox("Safe draft defaults") {
            Grid(alignment: .leading, horizontalSpacing: 16, verticalSpacing: 5) {
                GridRow {
                    LabeledContent("Cadence") {
                        Text(AutomationScheduleDefaults.cadence.displayName)
                    }
                    LabeledContent("Minimum age") {
                        Text(
                            Self.duration(
                                AutomationScheduleDefaults.minimumAgeSeconds
                            )
                        )
                    }
                }
                GridRow {
                    LabeledContent("Maximum per run") {
                        Text(
                            StorageByteFormatter.string(
                                from: AutomationScheduleDefaults.maximumBytesPerRun
                            )
                        )
                    }
                    LabeledContent("Pre-run notices") {
                        Text(
                            "First \(AutomationScheduleDefaults.notificationRuns) runs"
                        )
                    }
                }
                GridRow {
                    LabeledContent("Run approval") {
                        Text(
                            AutomationScheduleDefaults.confirmationMode.displayName
                        )
                    }
                }
            }
        }
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier(AutomationScheduleAccessibility.defaults)
    }

    private func draftCard(
        _ draft: AutomationScheduleDraftModel,
        index: Int
    ) -> some View {
        GroupBox {
            Grid(alignment: .leading, horizontalSpacing: 16, verticalSpacing: 5) {
                GridRow {
                    Label("Disabled draft", systemImage: "pause.circle.fill")
                        .font(.headline)
                    Text(draft.scope.displayName)
                        .foregroundStyle(.secondary)
                }
                GridRow {
                    LabeledContent("Cadence") {
                        Text(draft.cadence.displayName)
                    }
                    LabeledContent("Minimum age") {
                        Text(Self.duration(draft.minimumAgeSeconds))
                    }
                }
                GridRow {
                    LabeledContent("Minimum reclaimable") {
                        Text(
                            StorageByteFormatter.string(
                                from: draft.minimumReclaimableBytes
                            )
                        )
                    }
                    LabeledContent("Maximum per run") {
                        Text(
                            StorageByteFormatter.string(
                                from: draft.maximumBytesPerRun
                            )
                        )
                    }
                }
                GridRow {
                    LabeledContent("Pre-run notices left") {
                        Text(
                            draft.notifyBeforeRun
                                ? String(draft.notifyBeforeRunsRemaining)
                                : "Off"
                        )
                    }
                    LabeledContent("Run approval") {
                        Text(draft.confirmationMode.displayName)
                    }
                }
                GridRow {
                    LabeledContent("Rule exclusions") {
                        Text(verbatim: String(draft.exclusions.count))
                    }
                    LabeledContent("Draft revision") {
                        Text(verbatim: String(draft.revision))
                    }
                }
            }
        }
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier(AutomationScheduleAccessibility.draftRow(index))
        .accessibilityLabel(
            "Disabled automation draft for \(draft.scope.displayName)"
        )
        .accessibilityHint(
            "This read-only draft cannot schedule or execute cleanup"
        )
    }

    static func duration(_ seconds: UInt64) -> String {
        let day: UInt64 = 24 * 60 * 60
        if seconds.isMultiple(of: day) {
            let days = seconds / day
            return days == 1 ? "1 day" : "\(days) days"
        }
        let hour: UInt64 = 60 * 60
        if seconds.isMultiple(of: hour) {
            let hours = seconds / hour
            return hours == 1 ? "1 hour" : "\(hours) hours"
        }
        return "\(seconds) seconds"
    }

    static func message(for failure: AutomationScheduleSettingsFailure) -> String {
        switch failure {
        case let .service(error):
            switch error {
            case .unavailable:
                "Automation drafts are unavailable. No cleanup was scheduled."
            case .incompatibleSchema:
                "This automation-draft format is incompatible with the current app."
            case .invalidResponse:
                "The storage engine returned an invalid automation-draft response."
            }
        case .model:
            "An unsafe or invalid automation draft was rejected."
        case .unexpected:
            "Automation drafts could not be loaded. No cleanup was scheduled."
        }
    }
}
