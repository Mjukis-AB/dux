import SwiftUI

/// App-global observation and controls for one confirmed permanent-safe task.
/// The browser owns the opaque task; this view receives only path-free state
/// plus narrow stop/dismiss controls and can navigate to read-only history,
/// exactly when correlation is present.
struct ExplorerRustTargetCleanupStatusView: View {
    let cleanupState: ExplorerRustTargetCleanupState
    let historyFinalizationInProgress: Bool
    let cancelCleanup: () async -> Void
    let dismissResult: () async -> Void
    let openHistory: (String?) -> Void

    var body: some View {
        if cleanupState != .idle {
            GroupBox("Permanent-safe cleanup") {
                VStack(alignment: .leading, spacing: 8) {
                    switch cleanupState {
                    case .idle:
                        EmptyView()
                    case .starting:
                        ProgressView("Starting confirmed cleanup…")
                        Text(
                            "The exact reviewed plan has been consumed. DUX is repeating deterministic checks before any filesystem effect."
                        )
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        cleanupCancelButton
                    case let .observing(_, poll):
                        cleanupPollContent(poll)
                    case let .startFailed(_, failure):
                        Label(
                            failure.title,
                            systemImage: "exclamationmark.triangle.fill"
                        )
                        .foregroundStyle(.orange)
                        Text(verbatim: failure.detail)
                            .font(.caption)
                            .foregroundStyle(.secondary)
                        Text(
                            "The reviewed capability was consumed. No automatic retry is available."
                        )
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        genericHistoryLink
                        cleanupDismissButton
                    case .observationFailed:
                        Label(
                            "Cleanup status unavailable",
                            systemImage: "exclamationmark.triangle.fill"
                        )
                        .foregroundStyle(.orange)
                        Text(
                            "DUX rejected an invalid path-free task response. Do not retry; restart DUX and inspect Cleanup History."
                        )
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        genericHistoryLink
                        cleanupDismissButton
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .accessibilityIdentifier(
                    ExplorerAccessibility.snapshotCandidateCleanupStatus
                )
            }
        }
    }

    @ViewBuilder
    private func cleanupPollContent(
        _ poll: ExplorerRustTargetCleanupPoll
    ) -> some View {
        switch poll.phase {
        case .queued, .running:
            ProgressView(
                poll.cancellationRequested
                    ? "Stopping remaining work…"
                    : "Removing validated build output…"
            )
            Text(
                poll.cancellationRequested
                    ? "Cancellation is recorded. A filesystem operation already in progress may still need to settle."
                    : "Closing Explorer does not cancel or retry this core-owned operation."
            )
            .font(.caption)
            .foregroundStyle(.secondary)
            if !poll.cancellationRequested {
                cleanupCancelButton
            }
        case .succeeded:
            if let result = poll.result {
                Label(
                    cleanupResultTitle(result.status),
                    systemImage: result.status == .completed
                        ? "checkmark.circle.fill"
                        : "exclamationmark.triangle.fill"
                )
                .foregroundStyle(result.status == .completed ? .green : .orange)
                cleanupResultSummary(result)
                cleanupHistoryLink(result, recovery: false)
            } else {
                genericHistoryLink
            }
            cleanupDismissButton
        case .failed:
            let failure = poll.failure ?? .internalState
            Label(failure.title, systemImage: "exclamationmark.triangle.fill")
                .foregroundStyle(.orange)
            Text(verbatim: failure.detail)
                .font(.caption)
                .foregroundStyle(.secondary)
            if let result = poll.result {
                cleanupHistoryLink(result, recovery: true)
            } else {
                genericHistoryLink
            }
            Text("No retry is available from this result.")
                .font(.caption)
                .foregroundStyle(.secondary)
            cleanupDismissButton
        case .cancelled:
            Label("Cleanup stopped", systemImage: "stop.circle.fill")
                .foregroundStyle(.orange)
            if let result = poll.result {
                cleanupResultSummary(result)
                cleanupHistoryLink(result, recovery: false)
            } else {
                Text(
                    "Cleanup History is the authoritative record of any work that settled before cancellation."
                )
                .font(.caption)
                .foregroundStyle(.secondary)
                genericHistoryLink
            }
            cleanupDismissButton
        }
    }

    private var cleanupCancelButton: some View {
        Button("Stop remaining work", role: .destructive) {
            Task { await cancelCleanup() }
        }
        .accessibilityIdentifier(
            ExplorerAccessibility.snapshotCandidateCleanupCancel
        )
    }

    private var cleanupDismissButton: some View {
        Group {
            if historyFinalizationInProgress {
                ProgressView("Updating Cleanup History…")
                    .font(.caption)
            } else {
                Button("Dismiss result") {
                    Task { await dismissResult() }
                }
                .accessibilityIdentifier(
                    ExplorerAccessibility.snapshotCandidateCleanupDismiss
                )
            }
        }
    }

    private func cleanupHistoryLink(
        _ result: ExplorerRustTargetCleanupResult,
        recovery: Bool
    ) -> some View {
        VStack(alignment: .leading, spacing: 5) {
            Text(
                verbatim: "\(recovery ? "Recovery history session" : "History session") \(result.sessionID)"
            )
            .font(.caption.monospaced())
            .textSelection(.enabled)
            Button("View this session in Cleanup History") {
                openHistory(result.sessionID)
            }
            .accessibilityIdentifier(
                ExplorerAccessibility.snapshotCandidateCleanupHistory
            )
            .accessibilityHint(
                "Opens this path-free outcome record without retrying cleanup"
            )
        }
    }

    private var genericHistoryLink: some View {
        Button("Open Cleanup History") {
            openHistory(nil)
        }
        .accessibilityIdentifier(
            ExplorerAccessibility.snapshotCandidateCleanupHistory
        )
        .accessibilityHint(
            "Opens read-only cleanup outcomes without retrying cleanup"
        )
    }

    private func cleanupResultSummary(
        _ result: ExplorerRustTargetCleanupResult
    ) -> some View {
        Text(
            "Removed \(result.removedEntries) entries · \(StorageByteFormatter.string(from: result.removedLogicalBytes)) logical · available-space change \(cleanupCapacityDelta(result.verifiedCapacityDeltaBytes))."
        )
        .font(.caption)
        .foregroundStyle(.secondary)
    }

    private func cleanupResultTitle(
        _ status: CleanupHistorySessionStatus
    ) -> String {
        switch status {
        case .completed: "Cleanup completed"
        case .partiallyCompleted: "Cleanup partially completed"
        case .failed: "Cleanup finished with failures"
        case .interrupted: "Cleanup was interrupted"
        case .rejected: "Cleanup was rejected"
        case .planned, .running, .recovering, .cancelled, .dryRun:
            "Cleanup result rejected"
        }
    }

    private func cleanupCapacityDelta(_ delta: Int64?) -> String {
        guard let delta else {
            return "not verified"
        }
        let formatted = StorageByteFormatter.string(from: delta.magnitude)
        if delta > 0 {
            return "+\(formatted)"
        }
        if delta < 0 {
            return "−\(formatted)"
        }
        return "no measured change"
    }
}
