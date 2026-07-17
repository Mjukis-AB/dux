import SwiftUI

enum MenuBarLabelAccessibility {
    static let picker = "menu-bar-label-mode"
}

enum DiskPressurePolicyAccessibility {
    static let criticalGiB = "pressure-policy-critical-gib"
    static let criticalPercent = "pressure-policy-critical-percent"
    static let warningGiB = "pressure-policy-warning-gib"
    static let warningPercent = "pressure-policy-warning-percent"
    static let recoveryGiB = "pressure-policy-recovery-gib"
    static let recoveryPercent = "pressure-policy-recovery-percent"
    static let save = "pressure-policy-save"
    static let reset = "pressure-policy-reset"
    static let error = "pressure-policy-error"
    static let progress = "pressure-policy-progress"

    static let fieldIdentifiers = [
        criticalGiB,
        criticalPercent,
        warningGiB,
        warningPercent,
        recoveryGiB,
        recoveryPercent,
    ]

    static let allControlIdentifiers = fieldIdentifiers + [save, reset, error, progress]
}

struct DuxSettingsView: View {
    let model: AppModel

    var body: some View {
        @Bindable var model = model

        Form {
            Section("Application") {
                LabeledContent("App mode") {
                    Text("Menu bar helper")
                }
                Picker("Menu bar label", selection: $model.menuBarLabelMode) {
                    ForEach(MenuBarLabelMode.allCases) { mode in
                        Text(verbatim: mode.localizedTitle())
                            .tag(mode)
                    }
                }
                .accessibilityIdentifier(MenuBarLabelAccessibility.picker)
                .accessibilityHint("Changes the menu bar status immediately")
                Text(
                    "Free-space labels use startup-disk capacity available for important use, "
                        + "with filesystem availability as a clearly disclosed fallback."
                )
                .font(.caption)
                .foregroundStyle(.secondary)
                Text("Closing Explorer keeps DUX available from the menu bar.")
                    .foregroundStyle(.secondary)
            }

            Section("Disk pressure") {
                Text(
                    "DUX enters each pressure level when available storage reaches the "
                        + "smaller of its GiB and percentage limits."
                )
                .foregroundStyle(.secondary)

                if let policy = model.diskPressurePolicy {
                    LabeledContent("Policy source") {
                        Text(policy.source == .default ? "DUX defaults" : "Customized")
                    }
                    LabeledContent("Policy revision") {
                        Text(verbatim: String(policy.revision))
                    }
                    if let milliseconds = policy.updatedAtUnixMilliseconds {
                        LabeledContent("Policy updated") {
                            Text(
                                Date(timeIntervalSince1970: Double(milliseconds) / 1_000),
                                format: .dateTime
                            )
                        }
                    }
                }

                Grid(alignment: .leading, horizontalSpacing: 12, verticalSpacing: 10) {
                    GridRow {
                        Text("")
                        Text("GiB")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                        Text("Percent")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                    policyRow(
                        "Critical",
                        gib: $model.diskPressurePolicyDraft.criticalGiB,
                        percent: $model.diskPressurePolicyDraft.criticalPercent,
                        gibIdentifier: DiskPressurePolicyAccessibility.criticalGiB,
                        percentIdentifier: DiskPressurePolicyAccessibility.criticalPercent
                    )
                    policyRow(
                        "Warning",
                        gib: $model.diskPressurePolicyDraft.warningGiB,
                        percent: $model.diskPressurePolicyDraft.warningPercent,
                        gibIdentifier: DiskPressurePolicyAccessibility.warningGiB,
                        percentIdentifier: DiskPressurePolicyAccessibility.warningPercent
                    )
                    policyRow(
                        "Recovery margin",
                        gib: $model.diskPressurePolicyDraft.recoveryGiB,
                        percent: $model.diskPressurePolicyDraft.recoveryPercent,
                        gibIdentifier: DiskPressurePolicyAccessibility.recoveryGiB,
                        percentIdentifier: DiskPressurePolicyAccessibility.recoveryPercent
                    )
                }
                .accessibilityElement(children: .contain)
                .accessibilityLabel("Disk pressure thresholds")

                Text(
                    "Recovery requires available storage to clear the active boundary by "
                        + "both recovery margins. Values are stored exactly; DUX never rounds them."
                )
                .font(.caption)
                .foregroundStyle(.secondary)

                Text(
                    "These settings change classification only. They do not grant cleanup, "
                        + "notification, scan, or scheduling authority."
                )
                .font(.caption)
                .foregroundStyle(.secondary)

                if case let .failed(failure) = model.diskPressurePolicyState {
                    Label(Self.message(for: failure), systemImage: "exclamationmark.triangle")
                        .foregroundStyle(.red)
                        .accessibilityIdentifier(DiskPressurePolicyAccessibility.error)
                }

                HStack {
                    Button("Save thresholds") {
                        Task { await model.saveDiskPressurePolicy() }
                    }
                    .keyboardShortcut(.defaultAction)
                    .disabled(
                        model.diskPressurePolicyState.isBusy
                            || model.diskPressurePolicy == nil
                    )
                    .accessibilityIdentifier(DiskPressurePolicyAccessibility.save)
                    .accessibilityHint("Validates and stores these exact thresholds")

                    Button("Restore DUX defaults") {
                        Task { await model.resetDiskPressurePolicy() }
                    }
                    .disabled(
                        model.diskPressurePolicyState.isBusy
                            || model.diskPressurePolicy == nil
                    )
                    .accessibilityIdentifier(DiskPressurePolicyAccessibility.reset)
                    .accessibilityHint("Restores the thresholds supplied by this DUX version")

                    if model.diskPressurePolicyState.isBusy {
                        ProgressView()
                            .controlSize(.small)
                            .accessibilityIdentifier(DiskPressurePolicyAccessibility.progress)
                            .accessibilityLabel(
                                model.diskPressurePolicyState == .loading
                                    ? "Loading disk pressure thresholds"
                                    : model.diskPressurePolicyState == .resetting
                                        ? "Restoring disk pressure thresholds"
                                        : "Saving disk pressure thresholds"
                            )
                    }
                }
            }

            Section("Engine") {
                switch model.engineState {
                case .idle, .loading:
                    LabeledContent("Status") {
                        Text("Connecting…")
                    }
                case let .loaded(result):
                    LabeledContent("Status") {
                        Text("Ready")
                    }
                    LabeledContent("Library version") {
                        Text(verbatim: result.libraryVersion)
                    }
                    LabeledContent("FFI contract") {
                        Text(verbatim: String(result.ffiContractVersion))
                    }
                case .failed:
                    LabeledContent("Status") {
                        Text("Unavailable")
                    }
                }
            }
        }
        .formStyle(.grouped)
        .frame(width: 620, height: 610)
        .task {
            await model.loadDiskPressurePolicy()
            await model.loadInitialState()
        }
    }

    private func policyRow(
        _ title: LocalizedStringKey,
        gib: Binding<String>,
        percent: Binding<String>,
        gibIdentifier: String,
        percentIdentifier: String
    ) -> some View {
        GridRow {
            Text(title)
            TextField("GiB", text: gib)
                .frame(width: 120)
                .accessibilityLabel(Text(title) + Text(" available GiB"))
                .accessibilityHint("Enter an exact value greater than zero")
                .accessibilityIdentifier(gibIdentifier)
            TextField("Percent", text: percent)
                .frame(width: 120)
                .accessibilityLabel(Text(title) + Text(" available percent"))
                .accessibilityHint(
                    "Enter an exact value greater than zero and no more than 100 percent"
                )
                .accessibilityIdentifier(percentIdentifier)
        }
    }

    static func message(for failure: DiskPressurePolicyFailure) -> String {
        switch failure {
        case let .draft(error):
            switch error {
            case let .invalidNumber(field):
                String(
                    localized: "Enter an exact non-negative number for \(fieldName(field))."
                )
            case let .zero(field):
                String(localized: "\(fieldName(field)) must be greater than zero.")
            case let .outOfRange(field):
                String(localized: "\(fieldName(field)) must be no more than 100 percent.")
            }
        case let .service(error):
            switch error {
            case .warningBytesBelowCritical, .warningBasisPointsBelowCritical:
                String(
                    localized: "Warning limits must be greater than or equal to Critical limits."
                )
            case .warningThresholdMatchesCritical:
                String(localized: "Warning and Critical limits cannot both be identical.")
            case .thresholdBytesZero, .recoveryBytesZero:
                String(localized: "GiB values must be greater than zero.")
            case .thresholdBasisPointsOutOfRange, .recoveryBasisPointsOutOfRange:
                String(
                    localized: "Percentage values must be greater than zero and no more than 100."
                )
            case .revisionExhausted:
                String(
                    localized: "The policy revision limit has been reached. Restarting will not change it."
                )
            case .closed:
                String(localized: "The storage engine session is closed.")
            case .invalidRecordVersion, .incompatibleSchema:
                String(localized: "This DUX policy format is incompatible with the current app.")
            case .retryable, .invalidClock, .outcomeUnknown:
                String(localized: "The policy could not be saved safely. Try again.")
            case .unsafeStorage, .corruptData, .unavailable, .internalState,
                 .invalidResponse:
                String(
                    localized: "Disk pressure settings are unavailable. Your last saved policy is unchanged."
                )
            }
        case .unexpected:
            String(
                localized: "Disk pressure settings are unavailable. Your last saved policy is unchanged."
            )
        }
    }

    private static func fieldName(_ field: DiskPressurePolicyField) -> String {
        switch field {
        case .criticalGiB: String(localized: "Critical GiB")
        case .criticalPercent: String(localized: "Critical percent")
        case .warningGiB: String(localized: "Warning GiB")
        case .warningPercent: String(localized: "Warning percent")
        case .recoveryGiB: String(localized: "Recovery GiB")
        case .recoveryPercent: String(localized: "Recovery percent")
        }
    }
}
