import SwiftUI

enum SnapshotRetentionCapAccessibility {
    static let status = "snapshot-retention-cap-status"
    static let field = "snapshot-retention-cap-gib"
    static let save = "snapshot-retention-cap-save"
    static let reset = "snapshot-retention-cap-reset"
    static let reload = "snapshot-retention-cap-reload"
    static let error = "snapshot-retention-cap-error"
    static let progress = "snapshot-retention-cap-progress"

    static let allControlIdentifiers = [
        status,
        field,
        save,
        reset,
        reload,
        error,
        progress,
    ]
}

struct SnapshotRetentionCapSettingsView: View {
    @Bindable var settings: SnapshotRetentionCapSettingsModel

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Label("Snapshot storage limit", systemImage: "externaldrive.fill")
                .font(.headline)

            Text(
                "DUX keeps checksummed scan snapshots for Explorer history. This limit "
                    + "controls later idle retention; saving it does not immediately remove "
                    + "anything and never touches files outside DUX’s private storage."
            )
            .foregroundStyle(.secondary)

            if let current = settings.settings {
                HStack {
                    LabeledContent("Current limit") {
                        Text(StorageByteFormatter.string(from: current.capBytes))
                    }
                    LabeledContent("Source") {
                        Text(current.source == .default ? "DUX default" : "Customized")
                    }
                }
                .accessibilityElement(children: .combine)
                .accessibilityIdentifier(SnapshotRetentionCapAccessibility.status)

                if let milliseconds = current.updatedAtUnixMilliseconds {
                    LabeledContent("Updated") {
                        Text(
                            Date(timeIntervalSince1970: Double(milliseconds) / 1_000),
                            format: .dateTime
                        )
                    }
                }
            }

            HStack {
                TextField("Snapshot limit", text: $settings.draft.gib)
                    .frame(width: 140)
                    .accessibilityLabel("Snapshot storage limit in GiB")
                    .accessibilityHint(
                        "Enter an exact non-negative value; zero is a retention policy, not a clear command"
                    )
                    .accessibilityIdentifier(SnapshotRetentionCapAccessibility.field)
                Text("GiB")
                    .foregroundStyle(.secondary)

                Button("Save limit") {
                    Task { await settings.save() }
                }
                .disabled(settings.state.isBusy || settings.settings == nil)
                .accessibilityIdentifier(SnapshotRetentionCapAccessibility.save)
                .accessibilityHint(
                    "Stores the exact limit without running snapshot retention"
                )

                Button("Restore DUX default") {
                    Task { await settings.reset() }
                }
                .disabled(settings.state.isBusy || settings.settings == nil)
                .accessibilityIdentifier(SnapshotRetentionCapAccessibility.reset)
                .accessibilityHint(
                    "Restores the limit supplied by this DUX version without removing snapshots"
                )

                if settings.state.isBusy {
                    ProgressView()
                        .controlSize(.small)
                        .accessibilityIdentifier(SnapshotRetentionCapAccessibility.progress)
                        .accessibilityLabel(
                            settings.state == .loading
                                ? "Loading snapshot storage limit"
                                : settings.state == .resetting
                                    ? "Restoring snapshot storage limit"
                                    : "Saving snapshot storage limit"
                        )
                }
            }

            Text(
                "Protected snapshots, active or uncertain temporary files, and maintenance "
                    + "debt may keep usage above this value. A zero limit does not mean "
                    + "“clear now”; retention still preserves every safety invariant."
            )
            .font(.caption)
            .foregroundStyle(.secondary)

            if case let .failed(failure) = settings.state {
                Label(Self.message(for: failure), systemImage: "exclamationmark.triangle")
                    .foregroundStyle(.red)
                    .accessibilityIdentifier(SnapshotRetentionCapAccessibility.error)
            }

            if settings.requiresAuthoritativeReload || settings.state.isFailed {
                Button(
                    settings.requiresAuthoritativeReload
                        ? "Reload current limit"
                        : "Try loading again"
                ) {
                    Task { await settings.load(force: true) }
                }
                .disabled(settings.state.isBusy)
                .accessibilityIdentifier(SnapshotRetentionCapAccessibility.reload)
                .accessibilityHint(
                    "Reads the authoritative setting after an uncertain result; it does not retry the write"
                )
            }
        }
    }

    static func message(for failure: SnapshotRetentionCapFailure) -> String {
        switch failure {
        case .draft:
            String(
                localized:
                    "Enter an exact non-negative GiB value without grouping separators."
            )
        case let .service(error):
            switch error {
            case .closed:
                String(localized: "The storage engine session is closed.")
            case .invalidRecordVersion, .incompatibleSchema:
                String(
                    localized:
                        "This snapshot-limit format is incompatible with the current app."
                )
            case .retryable, .invalidClock:
                String(
                    localized:
                        "The snapshot limit could not be changed safely. Try again."
                )
            case .outcomeUnknown:
                String(
                    localized:
                        "The write result is unknown. Reload the current limit before making another change."
                )
            case .unsafeStorage, .budgetExceeded, .corruptData, .unavailable,
                 .internalState, .invalidResponse:
                String(
                    localized:
                        "Snapshot-limit settings are unavailable. The last confirmed value remains displayed."
                )
            }
        case .unexpected:
            String(
                localized:
                    "Snapshot-limit settings are unavailable. The last confirmed value remains displayed."
            )
        }
    }
}
