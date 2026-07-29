@testable import DUX
import Foundation
import XCTest

final class ExplorerEmergencyRecoveryPresentationTests: XCTestCase {
    func testExactCriticalOrderingPreservesCoreRankAndReviewOnlyCopy() {
        let snapshot = recoverySnapshot(pressure: .critical)
        let context = recoveryContext(anchorAt: snapshot.sampledAt)
        let ordering = recoveryOrdering(context: context)
        let batch = recoveryBatch(context: context, ordering: ordering)

        let presentation = ExplorerEmergencyRecoveryPresentation.make(
            volumeState: .loaded(snapshot),
            targetedState: .completed(batch)
        )

        XCTAssertEqual(presentation.tone, .critical)
        XCTAssertEqual(presentation.evidenceState, .current)
        XCTAssertEqual(presentation.compactEvidenceLabel, "Current")
        XCTAssertEqual(
            presentation.cards.map(\.lane),
            [.staleSafeRegenerable, .guidedExploration, .permissionGap]
        )
        XCTAssertEqual(presentation.cards.map(\.rank), [0, 1, 2])
        XCTAssertTrue(presentation.cards[0].accessibilitySummary.hasPrefix("Recovery step 2."))
        XCTAssertTrue(presentation.cards[1].accessibilitySummary.hasPrefix("Recovery step 6."))
        XCTAssertTrue(presentation.cards[2].accessibilitySummary.hasPrefix("Recovery step 7."))
        XCTAssertEqual(presentation.menuBarCards, presentation.cards)
        XCTAssertTrue(presentation.cards[0].detail.contains("2 remain blocked"))
        XCTAssertTrue(presentation.cards[0].detail.contains("No reclaimable-space total"))
        XCTAssertFalse(presentation.statusText.localizedCaseInsensitiveContains("can free"))
        XCTAssertFalse(presentation.limitationsText.localizedCaseInsensitiveContains("safe to clean"))
        XCTAssertEqual(
            presentation.cards[0].actions.map(\.action),
            [.reviewCandidates(scanID: "scan:targeted:known-user-cache:one")]
        )
        XCTAssertEqual(
            presentation.cards[1].actions.map(\.action),
            [.exploreSnapshot(scanID: "scan:targeted:known-user-cache:one")]
        )
        XCTAssertEqual(
            presentation.cards[2].actions.map(\.action),
            [.reviewCoverage(scanID: "scan:targeted:known-user-cache:one")]
        )
    }

    func testForeignAnchorNeverPresentsRetainedCriticalCardsAsCurrent() {
        let snapshot = recoverySnapshot(pressure: .critical)
        let oldContext = recoveryContext(
            anchorAt: snapshot.sampledAt.addingTimeInterval(-60)
        )
        let batch = recoveryBatch(
            context: oldContext,
            ordering: recoveryOrdering(context: oldContext)
        )

        let presentation = ExplorerEmergencyRecoveryPresentation.make(
            volumeState: .loaded(snapshot),
            targetedState: .completed(batch)
        )

        XCTAssertEqual(presentation.evidenceState, .unavailable)
        XCTAssertTrue(presentation.cards.isEmpty)
        XCTAssertEqual(
            presentation.statusText,
            "No exact Critical recovery order is available yet."
        )
    }

    func testWarningDoesNotReuseCriticalOrdering() {
        let critical = recoverySnapshot(pressure: .critical)
        let context = recoveryContext(anchorAt: critical.sampledAt)
        let batch = recoveryBatch(
            context: context,
            ordering: recoveryOrdering(context: context)
        )
        let warning = recoverySnapshot(
            pressure: .warning,
            sampledAt: critical.sampledAt
        )

        let presentation = ExplorerEmergencyRecoveryPresentation.make(
            volumeState: .loaded(warning),
            targetedState: .completed(batch)
        )

        XCTAssertEqual(presentation.tone, .warning)
        XCTAssertEqual(presentation.evidenceState, .unavailable)
        XCTAssertTrue(presentation.cards.isEmpty)
        XCTAssertTrue(presentation.statusText.contains("only for an exact Critical"))
    }

    func testRefreshingAndStaleCapacityNeverClaimCurrentEvidence() {
        let snapshot = recoverySnapshot(pressure: .critical)
        let context = recoveryContext(anchorAt: snapshot.sampledAt)
        let batch = recoveryBatch(
            context: context,
            ordering: recoveryOrdering(context: context)
        )

        let refreshing = ExplorerEmergencyRecoveryPresentation.make(
            volumeState: .refreshing(snapshot),
            targetedState: .completed(batch)
        )
        let stale = ExplorerEmergencyRecoveryPresentation.make(
            volumeState: .stale(snapshot, .unavailable),
            targetedState: .completed(batch)
        )

        XCTAssertEqual(refreshing.evidenceState, .updating)
        XCTAssertEqual(refreshing.compactEvidenceLabel, "Updating")
        XCTAssertTrue(refreshing.freshnessText.contains("refreshing"))
        XCTAssertEqual(stale.evidenceState, .earlier)
        XCTAssertEqual(stale.compactEvidenceLabel, "Earlier")
        XCTAssertTrue(stale.freshnessText.contains("refresh failed"))
    }

    func testFailedTargetedRefreshMarksRetainedOrderingEarlier() {
        let snapshot = recoverySnapshot(pressure: .critical)
        let context = recoveryContext(anchorAt: snapshot.sampledAt)
        let batch = recoveryBatch(
            context: context,
            ordering: recoveryOrdering(context: context)
        )

        let presentation = ExplorerEmergencyRecoveryPresentation.make(
            volumeState: .loaded(snapshot),
            targetedState: .failed(.unavailable, previous: batch)
        )

        XCTAssertEqual(presentation.evidenceState, .earlier)
        XCTAssertEqual(presentation.cards.count, 3)
        XCTAssertTrue(presentation.statusText.contains("not confirmed current"))
    }

    func testNoCapacityDoesNotInferUrgencyOrRenderCards() {
        let presentation = ExplorerEmergencyRecoveryPresentation.make(
            volumeState: .failed(.unavailable),
            targetedState: .idle
        )

        XCTAssertEqual(presentation.tone, .neutral)
        XCTAssertEqual(presentation.evidenceState, .unavailable)
        XCTAssertNil(presentation.availableText)
        XCTAssertTrue(presentation.cards.isEmpty)
    }

    func testIncompleteEvidenceIsNotPresentedAsZeroFindings() {
        let snapshot = recoverySnapshot(pressure: .critical)
        let context = recoveryContext(anchorAt: snapshot.sampledAt)
        let ordering = AppEmergencyRecoveryOrdering(
            policyRevision: 1,
            context: context,
            observedRootCount: 0,
            candidateEvaluatedRootCount: 0,
            unavailableRootCount: 1,
            groups: [
                AppEmergencyRecoveryGroup(
                    rank: 0,
                    lane: .permissionGap,
                    ruleID: nil,
                    ruleRevision: nil,
                    category: nil,
                    unavailableRootCount: 1,
                    sources: []
                ),
            ]
        )

        let presentation = ExplorerEmergencyRecoveryPresentation.make(
            volumeState: .loaded(snapshot),
            targetedState: .completed(
                recoveryBatch(context: context, ordering: ordering)
            )
        )

        XCTAssertEqual(presentation.evidenceState, .current)
        XCTAssertEqual(presentation.cards.count, 1)
        XCTAssertTrue(presentation.cards[0].detail.contains("1 unavailable location"))
        XCTAssertTrue(presentation.cards[0].actions.isEmpty)
        XCTAssertTrue(presentation.statusText.contains("is incomplete"))
        XCTAssertFalse(presentation.statusText.contains("No supported"))
    }

    func testReorderedOrUnsupportedCoreShapeFailsClosed() {
        let snapshot = recoverySnapshot(pressure: .critical)
        let context = recoveryContext(anchorAt: snapshot.sampledAt)
        let valid = recoveryOrdering(context: context)
        let reordered = AppEmergencyRecoveryOrdering(
            policyRevision: 1,
            context: context,
            observedRootCount: 1,
            candidateEvaluatedRootCount: 1,
            unavailableRootCount: 0,
            groups: valid.groups.reversed().enumerated().map { index, group in
                AppEmergencyRecoveryGroup(
                    rank: UInt16(index),
                    lane: group.lane,
                    ruleID: group.ruleID,
                    ruleRevision: group.ruleRevision,
                    category: group.category,
                    unavailableRootCount: group.unavailableRootCount,
                    sources: group.sources
                )
            }
        )
        let unsupported = AppEmergencyRecoveryOrdering(
            policyRevision: 1,
            context: context,
            observedRootCount: 1,
            candidateEvaluatedRootCount: 1,
            unavailableRootCount: 0,
            groups: [
                AppEmergencyRecoveryGroup(
                    rank: 0,
                    lane: .largeFile,
                    ruleID: nil,
                    ruleRevision: nil,
                    category: nil,
                    unavailableRootCount: 0,
                    sources: [
                        AppEmergencyRecoverySource(
                            rootOrdinal: 0,
                            scanID: "scan:unsupported",
                            observedAt: context.capacityAnchorAt,
                            candidateCount: nil,
                            blockedCandidateCount: nil,
                            permissionIssueCount: nil
                        ),
                    ]
                ),
            ]
        )

        XCTAssertFalse(reordered.hasValidPresentationShape)
        XCTAssertFalse(unsupported.hasValidPresentationShape)
        for ordering in [reordered, unsupported] {
            let presentation = ExplorerEmergencyRecoveryPresentation.make(
                volumeState: .loaded(snapshot),
                targetedState: .completed(
                    recoveryBatch(context: context, ordering: ordering)
                )
            )
            XCTAssertEqual(presentation.evidenceState, .unavailable)
            XCTAssertTrue(presentation.cards.isEmpty)
        }
    }

    func testDuplicateSemanticGroupsFailClosedRegardlessOfRank() {
        let snapshot = recoverySnapshot(pressure: .critical)
        let context = recoveryContext(anchorAt: snapshot.sampledAt)
        let valid = recoveryOrdering(context: context)
        var groups = valid.groups
        groups.insert(groups[1], at: 2)
        groups = groups.enumerated().map { index, group in
            AppEmergencyRecoveryGroup(
                rank: UInt16(index),
                lane: group.lane,
                ruleID: group.ruleID,
                ruleRevision: group.ruleRevision,
                category: group.category,
                unavailableRootCount: group.unavailableRootCount,
                sources: group.sources
            )
        }
        let duplicate = AppEmergencyRecoveryOrdering(
            policyRevision: 1,
            context: context,
            observedRootCount: 1,
            candidateEvaluatedRootCount: 1,
            unavailableRootCount: 0,
            groups: groups
        )

        XCTAssertFalse(duplicate.hasValidPresentationShape)
        let presentation = ExplorerEmergencyRecoveryPresentation.make(
            volumeState: .loaded(snapshot),
            targetedState: .completed(
                recoveryBatch(context: context, ordering: duplicate)
            )
        )
        XCTAssertEqual(presentation.evidenceState, .unavailable)
        XCTAssertTrue(presentation.cards.isEmpty)
    }

    func testMenuBarMirrorIsBoundedAndPreservesCoreOrder() {
        let snapshot = recoverySnapshot(pressure: .critical)
        let context = recoveryContext(anchorAt: snapshot.sampledAt)
        let base = recoveryOrdering(context: context)
        var groups = base.groups
        groups.insert(
            AppEmergencyRecoveryGroup(
                rank: 1,
                lane: .staleSafeRegenerable,
                ruleID: "developer.python.pip_cache",
                ruleRevision: 1,
                category: .applicationCache,
                unavailableRootCount: 0,
                sources: [
                    AppEmergencyRecoverySource(
                        rootOrdinal: 0,
                        scanID: "scan:targeted:known-user-cache:two",
                        observedAt: context.capacityAnchorAt,
                        candidateCount: 1,
                        blockedCandidateCount: 1,
                        permissionIssueCount: nil
                    ),
                ]
            ),
            at: 1
        )
        groups = groups.enumerated().map { index, group in
            AppEmergencyRecoveryGroup(
                rank: UInt16(index),
                lane: group.lane,
                ruleID: group.ruleID,
                ruleRevision: group.ruleRevision,
                category: group.category,
                unavailableRootCount: group.unavailableRootCount,
                sources: group.sources
            )
        }
        let ordering = AppEmergencyRecoveryOrdering(
            policyRevision: 1,
            context: context,
            observedRootCount: 1,
            candidateEvaluatedRootCount: 1,
            unavailableRootCount: 0,
            groups: groups
        )
        let presentation = ExplorerEmergencyRecoveryPresentation.make(
            volumeState: .loaded(snapshot),
            targetedState: .completed(
                recoveryBatch(context: context, ordering: ordering)
            )
        )

        XCTAssertEqual(presentation.cards.count, 4)
        XCTAssertEqual(presentation.menuBarCards.count, 3)
        XCTAssertEqual(
            presentation.menuBarCards.map(\.id),
            presentation.cards.prefix(3).map(\.id)
        )
    }
}

private func recoverySnapshot(
    pressure: DiskPressureLevel,
    sampledAt: Date = Date(timeIntervalSince1970: 10000)
) -> VolumeCapacitySnapshot {
    VolumeCapacitySnapshot(
        stableVolumeID: "volume:macos:aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee",
        displayName: "Macintosh HD",
        filesystem: "apfs",
        isInternal: true,
        isRemovable: false,
        totalBytes: 1_000_000_000,
        filesystemAvailableBytes: 40_000_000,
        importantAvailableBytes: 45_000_000,
        effectiveAvailableBytes: 45_000_000,
        availabilityBasis: .importantUsage,
        pressure: pressure,
        criticalBoundaryBytes: 50_000_000,
        warningBoundaryBytes: 100_000_000,
        historyDisposition: .stored,
        sampledAt: sampledAt
    )
}

private func recoveryContext(anchorAt: Date) -> TargetedReclaimScanContext {
    TargetedReclaimScanContext(
        stableVolumeID: "volume:macos:aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee",
        capacityAnchorAt: anchorAt,
        pressure: .critical,
        pressureEpisodeStartedAt: anchorAt.addingTimeInterval(-600),
        lowPressureSequenceStartedAt: anchorAt.addingTimeInterval(-1200),
        policyRevision: 3,
        rootsRevision: 4,
        knownRootsPolicyRevision: 1,
        knownUserLibraryCachesIncluded: true,
        rootCatalogDigestSHA256: Data(repeating: 7, count: 32),
        rootCount: 1
    )
}

private func recoveryOrdering(
    context: TargetedReclaimScanContext
) -> AppEmergencyRecoveryOrdering {
    let scanID = "scan:targeted:known-user-cache:one"
    let observedAt = context.capacityAnchorAt.addingTimeInterval(-30)
    let candidateSource = AppEmergencyRecoverySource(
        rootOrdinal: 0,
        scanID: scanID,
        observedAt: observedAt,
        candidateCount: 2,
        blockedCandidateCount: 2,
        permissionIssueCount: nil
    )
    let navigationSource = AppEmergencyRecoverySource(
        rootOrdinal: 0,
        scanID: scanID,
        observedAt: observedAt,
        candidateCount: nil,
        blockedCandidateCount: nil,
        permissionIssueCount: nil
    )
    let permissionSource = AppEmergencyRecoverySource(
        rootOrdinal: 0,
        scanID: scanID,
        observedAt: observedAt,
        candidateCount: nil,
        blockedCandidateCount: nil,
        permissionIssueCount: 3
    )
    return AppEmergencyRecoveryOrdering(
        policyRevision: 1,
        context: context,
        observedRootCount: 1,
        candidateEvaluatedRootCount: 1,
        unavailableRootCount: 0,
        groups: [
            AppEmergencyRecoveryGroup(
                rank: 0,
                lane: .staleSafeRegenerable,
                ruleID: "developer.homebrew.cache",
                ruleRevision: 1,
                category: .applicationCache,
                unavailableRootCount: 0,
                sources: [candidateSource]
            ),
            AppEmergencyRecoveryGroup(
                rank: 1,
                lane: .guidedExploration,
                ruleID: nil,
                ruleRevision: nil,
                category: nil,
                unavailableRootCount: 0,
                sources: [navigationSource]
            ),
            AppEmergencyRecoveryGroup(
                rank: 2,
                lane: .permissionGap,
                ruleID: nil,
                ruleRevision: nil,
                category: nil,
                unavailableRootCount: 0,
                sources: [permissionSource]
            ),
        ]
    )
}

private func recoveryBatch(
    context: TargetedReclaimScanContext,
    ordering: AppEmergencyRecoveryOrdering
) -> TargetedReclaimScanBatch {
    TargetedReclaimScanBatch(
        context: context,
        completed: [],
        failed: [],
        emergencyRecovery: ordering
    )
}
