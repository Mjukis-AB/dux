import Foundation

enum ExplorerEmergencyRecoveryTone: Equatable, Sendable {
    case critical
    case warning
    case neutral
}

enum ExplorerEmergencyRecoveryEvidenceState: Equatable, Sendable {
    case current
    case updating
    case earlier
    case unavailable
}

enum ExplorerEmergencyRecoveryAction: Equatable, Sendable {
    case reviewCandidates(scanID: String)
    case exploreSnapshot(scanID: String)
    case reviewCoverage(scanID: String)
}

struct ExplorerEmergencyRecoverySourceAction: Equatable, Identifiable, Sendable {
    var id: String {
        "\(rootOrdinal):\(scanID)"
    }

    let rootOrdinal: UInt16
    let scanID: String
    let label: String
    let accessibilitySummary: String
    let action: ExplorerEmergencyRecoveryAction
}

struct ExplorerEmergencyRecoveryCard: Equatable, Identifiable, Sendable {
    var id: String { accessibilityKey }

    let rank: UInt16
    let lane: AppEmergencyRecoveryLane
    let accessibilityKey: String
    let title: String
    let detail: String
    let observationText: String
    let accessibilitySummary: String
    let symbol: String
    let actions: [ExplorerEmergencyRecoverySourceAction]
}

/// Native copy and navigation for the core-owned emergency ordering. This
/// layer preserves Rust's rank verbatim and never derives a new candidate,
/// byte total, safety decision, or cleanup action.
struct ExplorerEmergencyRecoveryPresentation: Equatable, Sendable {
    let tone: ExplorerEmergencyRecoveryTone
    let evidenceState: ExplorerEmergencyRecoveryEvidenceState
    let title: String
    let detail: String
    let availableText: String?
    let freshnessText: String
    let statusText: String
    let symbol: String
    let cards: [ExplorerEmergencyRecoveryCard]
    let limitationsText: String

    /// The popover mirrors, but never reorders, the first three core-ranked
    /// groups. Explorer retains the complete bounded list.
    var menuBarCards: [ExplorerEmergencyRecoveryCard] {
        Array(cards.prefix(3))
    }

    var compactEvidenceLabel: String {
        switch evidenceState {
        case .current: "Current"
        case .updating: "Updating"
        case .earlier: "Earlier"
        case .unavailable: "Unavailable"
        }
    }

    static func make(
        volumeState: VolumeCapacityState,
        targetedState: TargetedReclaimScanState
    ) -> Self {
        guard let snapshot = volumeState.snapshot else {
            return Self(
                tone: .neutral,
                evidenceState: .unavailable,
                title: "Recovery guidance unavailable",
                detail: "Refresh startup-disk capacity before DUX orders recovery observations.",
                availableText: nil,
                freshnessText: "No confirmed capacity observation",
                statusText: "No recovery order is being shown.",
                symbol: "questionmark.circle",
                cards: [],
                limitationsText: limitations
            )
        }

        let capacityState = capacityEvidenceState(volumeState)
        let pressureTone: ExplorerEmergencyRecoveryTone = switch snapshot.pressure {
        case .critical: .critical
        case .warning: .warning
        case .healthy, .unknown: .neutral
        }
        let title = switch snapshot.pressure {
        case .critical: "Critical storage pressure"
        case .warning: "Low storage warning"
        case .healthy: "Storage is healthy"
        case .unknown: "Storage pressure is unknown"
        }
        let detail = switch snapshot.pressure {
        case .critical:
            "Review deterministic observations in DUX’s fixed recovery order."
        case .warning:
            "Plan recovery before available space reaches the Critical threshold."
        case .healthy:
            "Explore storage when useful; emergency recovery ordering is not active."
        case .unknown:
            "DUX cannot infer urgency from this capacity observation."
        }

        let matched = matchedOrdering(
            snapshot: snapshot,
            targetedState: targetedState
        )
        let evidenceState: ExplorerEmergencyRecoveryEvidenceState
        let statusText: String
        let cards: [ExplorerEmergencyRecoveryCard]
        if snapshot.pressure != .critical {
            evidenceState = capacityState == .earlier ? .earlier : .unavailable
            statusText = snapshot.pressure == .warning
                ? "Focused read-only discovery is collecting evidence. Emergency ordering activates only for an exact Critical observation."
                : "Emergency ordering is inactive."
            cards = []
        } else if let matched {
            evidenceState = combinedEvidenceState(
                capacity: capacityState,
                targeted: matched.state
            )
            cards = matched.ordering.groups.map(card)
            let hasIncompleteEvidence =
                matched.ordering.unavailableRootCount > 0
                    || matched.ordering.candidateEvaluatedRootCount
                        < matched.ordering.observedRootCount
            statusText = switch evidenceState {
            case .current:
                if hasIncompleteEvidence {
                    "The exact Critical pass is incomplete: \(matched.ordering.observedRootCount) of \(matched.ordering.context.rootCount) locations have current scan evidence, and \(matched.ordering.candidateEvaluatedRootCount) have complete candidate evaluation."
                } else if cards.isEmpty {
                    "No supported recovery findings were observed across the complete exact Critical pass."
                } else {
                    "\(cards.count) evidence-backed recovery option\(cards.count == 1 ? "" : "s") from the complete exact Critical pass."
                }
            case .updating:
                "Earlier exact observations are shown while DUX revalidates the Critical pass."
            case .earlier:
                "These observations are retained for review, but are not confirmed current."
            case .unavailable:
                "No exact Critical recovery order is available."
            }
        } else {
            evidenceState = targetedState.isActive ? .updating : .unavailable
            statusText = targetedState.isActive
                ? "DUX is building an exact, read-only Critical recovery order."
                : "No exact Critical recovery order is available yet."
            cards = []
        }
        let symbol = switch snapshot.pressure {
        case .critical: "exclamationmark.octagon.fill"
        case .warning: "exclamationmark.triangle.fill"
        case .healthy: "checkmark.circle.fill"
        case .unknown: "questionmark.circle"
        }

        return Self(
            tone: pressureTone,
            evidenceState: evidenceState,
            title: title,
            detail: detail,
            availableText: "\(byteCount(snapshot.effectiveAvailableBytes)) available",
            freshnessText: freshness(snapshot: snapshot, state: volumeState),
            statusText: statusText,
            symbol: symbol,
            cards: cards,
            limitationsText: limitations
        )
    }

    private struct MatchedOrdering {
        let ordering: AppEmergencyRecoveryOrdering
        let state: ExplorerEmergencyRecoveryEvidenceState
    }

    private static func matchedOrdering(
        snapshot: VolumeCapacitySnapshot,
        targetedState: TargetedReclaimScanState
    ) -> MatchedOrdering? {
        guard
            snapshot.pressure == .critical,
            let stableVolumeID = snapshot.stableVolumeID,
            let batch = targetedState.batch,
            let ordering = batch.emergencyRecovery,
            ordering.policyRevision == AppEmergencyRecoveryOrdering.supportedPolicyRevision,
            ordering.hasValidPresentationShape,
            ordering.context == batch.context,
            ordering.context.pressure == .critical,
            ordering.context.stableVolumeID == stableVolumeID,
            ordering.context.capacityAnchorAt == snapshot.sampledAt
        else {
            return nil
        }
        let state: ExplorerEmergencyRecoveryEvidenceState = switch targetedState {
        case .completed: .current
        case .checking, .scanning: .updating
        case .deferred, .cancelled, .failed: .earlier
        case .idle, .noEligibleRoots: .unavailable
        }
        return MatchedOrdering(ordering: ordering, state: state)
    }

    private static func capacityEvidenceState(
        _ state: VolumeCapacityState
    ) -> ExplorerEmergencyRecoveryEvidenceState {
        switch state {
        case .loaded: .current
        case .refreshing: .updating
        case .stale: .earlier
        case .idle, .loading, .failed: .unavailable
        }
    }

    private static func combinedEvidenceState(
        capacity: ExplorerEmergencyRecoveryEvidenceState,
        targeted: ExplorerEmergencyRecoveryEvidenceState
    ) -> ExplorerEmergencyRecoveryEvidenceState {
        if capacity == .unavailable || targeted == .unavailable {
            return .unavailable
        }
        if capacity == .earlier || targeted == .earlier {
            return .earlier
        }
        if capacity == .updating || targeted == .updating {
            return .updating
        }
        return .current
    }

    private static func card(
        _ group: AppEmergencyRecoveryGroup
    ) -> ExplorerEmergencyRecoveryCard {
        let key = "\(group.lane.rawValue)-\(group.rank)"
        switch group.lane {
        case .staleSafeRegenerable:
            let count = group.candidateCount ?? 0
            let blocked = group.blockedCandidateCount ?? 0
            let rule = ruleName(group.ruleID, revision: group.ruleRevision)
            let countText = "\(count) finding\(count == 1 ? "" : "s")"
            let blockedText = blocked == 0
                ? "Review is still required; classification is not cleanup approval."
                : "\(blocked) remain blocked by missing or protected evidence."
            return ExplorerEmergencyRecoveryCard(
                rank: group.rank,
                lane: group.lane,
                accessibilityKey: key,
                title: "Stale regenerable findings · \(rule)",
                detail: "\(countText.capitalized) matched deterministic age and regenerable-type rules. \(blockedText) No reclaimable-space total is claimed.",
                observationText: observationText(group),
                accessibilitySummary: "Recovery step \(group.lane.rawValue). \(rule). \(countText). \(blockedText) Review only.",
                symbol: "shippingbox.and.arrow.backward",
                actions: group.sources.map {
                    sourceAction(
                        $0,
                        lane: group.lane,
                        label: "Review \($0.candidateCount ?? 0) finding\(($0.candidateCount ?? 0) == 1 ? "" : "s")"
                    )
                }
            )
        case .guidedExploration:
            return ExplorerEmergencyRecoveryCard(
                rank: group.rank,
                lane: group.lane,
                accessibilityKey: key,
                title: "Explore measured storage",
                detail: "Drill into exact read-only snapshots to understand where storage is concentrated. DUX does not preselect anything.",
                observationText: observationText(group),
                accessibilitySummary: "Recovery step \(group.lane.rawValue). Guided storage exploration from \(group.sources.count) exact scan\(group.sources.count == 1 ? "" : "s"). Read only.",
                symbol: "square.grid.2x2",
                actions: group.sources.map {
                    sourceAction($0, lane: group.lane, label: "Explore snapshot")
                }
            )
        case .permissionGap:
            let count = group.permissionIssueCount ?? 0
            let unavailable = group.unavailableRootCount
            var facts = [String]()
            if count > 0 {
                facts.append("\(count) coverage issue\(count == 1 ? "" : "s")")
            }
            if unavailable > 0 {
                facts.append(
                    "\(unavailable) unavailable location\(unavailable == 1 ? "" : "s")"
                )
            }
            let countText = facts.joined(separator: " and ")
            return ExplorerEmergencyRecoveryCard(
                rank: group.rank,
                lane: group.lane,
                accessibilityKey: key,
                title: "Coverage and permission gaps",
                detail: "\(countText) may hide storage from the measured view. DUX does not estimate unseen bytes.",
                observationText: observationText(group),
                accessibilitySummary: "Recovery step \(group.lane.rawValue). \(countText). Hidden storage is not estimated.",
                symbol: "lock.trianglebadge.exclamationmark",
                actions: group.sources.map {
                    sourceAction($0, lane: group.lane, label: "Review coverage")
                }
            )
        case .evictableCloud, .trashInformation, .reviewableInstallerArchive, .largeFile:
            // Policy revision 1 rejects these lanes in the adapter because no
            // authoritative source exists yet.
            preconditionFailure("unsupported emergency recovery lane reached presentation")
        }
    }

    private static func sourceAction(
        _ source: AppEmergencyRecoverySource,
        lane: AppEmergencyRecoveryLane,
        label: String
    ) -> ExplorerEmergencyRecoverySourceAction {
        let action: ExplorerEmergencyRecoveryAction = switch lane {
        case .staleSafeRegenerable:
            .reviewCandidates(scanID: source.scanID)
        case .guidedExploration:
            .exploreSnapshot(scanID: source.scanID)
        case .permissionGap:
            .reviewCoverage(scanID: source.scanID)
        case .evictableCloud, .trashInformation, .reviewableInstallerArchive, .largeFile:
            preconditionFailure("unsupported emergency recovery lane reached navigation")
        }
        return ExplorerEmergencyRecoverySourceAction(
            rootOrdinal: source.rootOrdinal,
            scanID: source.scanID,
            label: label,
            accessibilitySummary: "\(label). Observation from \(source.observedAt.formatted(date: .abbreviated, time: .shortened)). Opens the exact read-only scan.",
            action: action
        )
    }

    private static func observationText(
        _ group: AppEmergencyRecoveryGroup
    ) -> String {
        guard let observedAt = group.newestObservationAt else {
            return "No exact scan observation is available for these locations"
        }
        return "Newest observation \(observedAt.formatted(date: .abbreviated, time: .shortened))"
    }

    private static func ruleName(
        _ ruleID: String?,
        revision: UInt32?
    ) -> String {
        switch (ruleID, revision) {
        case ("developer.homebrew.cache", 1): "Homebrew downloads"
        case ("developer.python.pip_cache", 1): "pip package cache"
        case ("developer.rust.target", 3): "Rust build output"
        case ("developer.python.pycache", 2): "Python bytecode cache"
        case let (ruleID?, _): ruleID
        case (nil, _): "deterministic finding"
        }
    }

    private static func freshness(
        snapshot: VolumeCapacitySnapshot,
        state: VolumeCapacityState
    ) -> String {
        let observed = snapshot.sampledAt.formatted(date: .abbreviated, time: .shortened)
        return switch state {
        case .loaded:
            "Capacity observed \(observed)"
        case .refreshing:
            "Last confirmed \(observed); refreshing"
        case .stale:
            "Last confirmed \(observed); refresh failed"
        case .idle, .loading, .failed:
            "Capacity confirmation unavailable"
        }
    }

    private static func byteCount(_ bytes: UInt64) -> String {
        ByteCountFormatter.string(
            fromByteCount: Int64(min(bytes, UInt64(Int64.max))),
            countStyle: .file
        )
    }

    private static let limitations =
        "Only options supported by exact current observations are shown. Counts can overlap and are not guaranteed reclaimable space. This guide only opens read-only review; it never performs cleanup."
}
