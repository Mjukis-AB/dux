import Foundation

enum ExplorerAccessibility {
    static let root = "explorer"
    static let sidebar = "explorer-sidebar"
    static let overviewDestination = "explorer-destination-overview"
    static let snapshotDestination = "explorer-destination-snapshot"
    static let recommendationsDestination = "explorer-destination-recommendations"
    static let recommendations = "explorer-recommendations"
    static let emergencyRecoveryHero = "explorer-emergency-recovery-hero"
    static let emergencyRecoveryStatus = "explorer-emergency-recovery-status"
    static let emergencyRecoveryCards = "explorer-emergency-recovery-cards"
    static let emergencyRecoveryFreshness = "explorer-emergency-recovery-freshness"
    static let emergencyRecoveryLimitations = "explorer-emergency-recovery-limitations"
    static let targetedReclaimScan = "explorer-targeted-reclaim-scan"
    static let targetedReclaimScanStatus = "explorer-targeted-reclaim-scan-status"
    static let targetedReclaimScanProgress = "explorer-targeted-reclaim-scan-progress"
    static let targetedReclaimScanCancel = "explorer-targeted-reclaim-scan-cancel"
    static let targetedReclaimScanRetry = "explorer-targeted-reclaim-scan-retry"
    static let cleanupHistoryDestination = "explorer-destination-cleanup-history"
    static let cleanupHistory = "explorer-cleanup-history"
    static let cleanupHistoryStatus = "explorer-cleanup-history-status"
    static let cleanupHistoryChart = "explorer-cleanup-history-chart"
    static let cleanupHistoryLoadMore = "explorer-cleanup-history-load-more"
    static let cleanupHistoryDetail = "explorer-cleanup-history-detail"
    static let cleanupHistoryDetailStatus = "explorer-cleanup-history-detail-status"
    static let cleanupHistoryDetailBack = "explorer-cleanup-history-detail-back"
    static let cleanupHistoryDetailRetry = "explorer-cleanup-history-detail-retry"
    static let cleanupHistoryDetailSummary = "explorer-cleanup-history-detail-summary"
    static let cleanupHistoryItemChart = "explorer-cleanup-history-item-chart"
    static let cleanupHistoryPathChart = "explorer-cleanup-history-path-chart"
    static let cleanupHistoryWarnings = "explorer-cleanup-history-warnings"
    static let cleanupHistoryRuleOutcomesStatus =
        "explorer-cleanup-history-rule-outcomes-status"
    static let cleanupHistoryRuleOutcomesRetry =
        "explorer-cleanup-history-rule-outcomes-retry"
    static let cleanupHistoryStorageThieves =
        "explorer-cleanup-history-storage-thieves"
    static let cleanupHistoryStorageThievesStatus =
        "explorer-cleanup-history-storage-thieves-status"
    static let cleanupHistoryStorageThievesChart =
        "explorer-cleanup-history-storage-thieves-chart"
    static let cleanupHistoryStorageThievesRetry =
        "explorer-cleanup-history-storage-thieves-retry"
    static let settingsDestination = "explorer-destination-settings"
    static let settingsShortcut = "explorer-settings-shortcut"
    static let capacityCard = "explorer-capacity-card"
    static let capacityBar = "explorer-capacity-bar"
    static let available = "explorer-capacity-available"
    static let used = "explorer-capacity-used"
    static let total = "explorer-capacity-total"
    static let pressure = "explorer-pressure"
    static let freshness = "explorer-capacity-freshness"
    static let capacityHistoryCard = "explorer-capacity-history-card"
    static let capacityHistoryStatus = "explorer-capacity-history-status"
    static let capacityHistoryChart = "explorer-capacity-history-chart"
    static let pressureEpisodeTimeline = "explorer-pressure-episode-timeline"
    static let pressureEpisodeList = "explorer-pressure-episode-list"
    static let scanCard = "explorer-scan-card"
    static let scanStatus = "explorer-scan-status"
    static let coverage = "explorer-coverage"
    static let refreshCapacity = "explorer-refresh-capacity"
    static let scanNow = "explorer-scan-now"
    static let cancelScan = "explorer-cancel-scan"
    static let snapshotBrowser = "explorer-snapshot-browser"
    static let snapshotBreadcrumbs = "explorer-snapshot-breadcrumbs"
    static let snapshotBack = "explorer-snapshot-back"
    static let snapshotSort = "explorer-snapshot-sort"
    static let snapshotTable = "explorer-snapshot-table"
    static let snapshotTreemap = "explorer-snapshot-treemap"
    static let snapshotTreemapOther = "explorer-snapshot-treemap-other"
    static let snapshotCategoryLegend = "explorer-snapshot-category-legend"
    static let snapshotCategoryColumn = "explorer-snapshot-category-column"
    static let snapshotInspectorCategory = "explorer-snapshot-inspector-category"
    static let snapshotInspector = "explorer-snapshot-inspector"
    static let snapshotPageStatus = "explorer-snapshot-page-status"
    static let snapshotPreviousPage = "explorer-snapshot-previous-page"
    static let snapshotNextPage = "explorer-snapshot-next-page"
    static let snapshotReload = "explorer-snapshot-reload"
    static let snapshotSubtreeRescan = "explorer-snapshot-subtree-rescan"
    static let snapshotSubtreeScanStatus = "explorer-snapshot-subtree-scan-status"
    static let snapshotSubtreeScanCancel = "explorer-snapshot-subtree-scan-cancel"
    static let snapshotSubtreeScanNotice = "explorer-snapshot-subtree-scan-notice"
    static let snapshotHistory = "explorer-snapshot-history"
    static let snapshotHistoryStatus = "explorer-snapshot-history-status"
    static let snapshotError = "explorer-snapshot-error"
    static let snapshotContentMode = "explorer-snapshot-content-mode"
    static let snapshotChanges = "explorer-snapshot-changes"
    static let snapshotChangesStatus = "explorer-snapshot-changes-status"
    static let snapshotChangesDisclosure = "explorer-snapshot-changes-disclosure"
    static let snapshotChangesTimeline = "explorer-snapshot-changes-timeline"
    static let snapshotChangesSummary = "explorer-snapshot-changes-summary"
    static let snapshotChangesBreadcrumbs = "explorer-snapshot-changes-breadcrumbs"
    static let snapshotChangesBack = "explorer-snapshot-changes-back"
    static let snapshotChangesSort = "explorer-snapshot-changes-sort"
    static let snapshotChangesTable = "explorer-snapshot-changes-table"
    static let snapshotChangesTreemap = "explorer-snapshot-changes-treemap"
    static let snapshotChangesOtherGrowth = "explorer-snapshot-changes-other-growth"
    static let snapshotChangesOtherShrinkage = "explorer-snapshot-changes-other-shrinkage"
    static let snapshotChangesLegend = "explorer-snapshot-changes-legend"
    static let snapshotChangesInspector = "explorer-snapshot-changes-inspector"
    static let snapshotChangesPageStatus = "explorer-snapshot-changes-page-status"
    static let snapshotChangesPreviousPage = "explorer-snapshot-changes-previous-page"
    static let snapshotChangesNextPage = "explorer-snapshot-changes-next-page"
    static let snapshotCandidates = "explorer-snapshot-candidates"
    static let snapshotCandidateGroups = "explorer-snapshot-candidate-groups"
    static let snapshotCandidateTable = "explorer-snapshot-candidate-table"
    static let snapshotCandidatePageStatus = "explorer-snapshot-candidate-page-status"
    static let snapshotCandidatePreviousPage =
        "explorer-snapshot-candidate-previous-page"
    static let snapshotCandidateNextPage = "explorer-snapshot-candidate-next-page"
    static let snapshotCandidateStatus = "explorer-snapshot-candidate-status"
    static let snapshotCandidateInspector = "explorer-snapshot-candidate-inspector"
    static let snapshotCandidateDetailStatus = "explorer-snapshot-candidate-detail-status"
    static let snapshotCandidatePaths = "explorer-snapshot-candidate-paths"
    static let snapshotCandidatePathPrevious = "explorer-snapshot-candidate-path-previous"
    static let snapshotCandidatePathNext = "explorer-snapshot-candidate-path-next"
    static let snapshotCandidateEvidence = "explorer-snapshot-candidate-evidence"
    static let snapshotCandidateEvidencePrevious =
        "explorer-snapshot-candidate-evidence-previous"
    static let snapshotCandidateEvidenceNext = "explorer-snapshot-candidate-evidence-next"
    static let snapshotCandidatePlanReview = "explorer-snapshot-candidate-plan-review"
    static let snapshotCandidatePlanReviewPrepare =
        "explorer-snapshot-candidate-plan-review-prepare"
    static let snapshotCandidatePlanReviewStatus =
        "explorer-snapshot-candidate-plan-review-status"
    static let snapshotCandidatePlanReviewSummary =
        "explorer-snapshot-candidate-plan-review-summary"
    static let snapshotCandidatePlanReviewWarnings =
        "explorer-snapshot-candidate-plan-review-warnings"
    static let snapshotCandidatePlanReviewRefresh =
        "explorer-snapshot-candidate-plan-review-refresh"
    static let snapshotCandidatePlanReviewClose =
        "explorer-snapshot-candidate-plan-review-close"
    static let snapshotCandidateDryRunStart =
        "explorer-snapshot-candidate-dry-run-start"
    static let snapshotCandidateDryRunStatus =
        "explorer-snapshot-candidate-dry-run-status"
    static let snapshotCandidateDryRunCancel =
        "explorer-snapshot-candidate-dry-run-cancel"
    static let snapshotCandidateDryRunDismiss =
        "explorer-snapshot-candidate-dry-run-dismiss"
    static let snapshotCandidateCleanupPrepare =
        "explorer-snapshot-candidate-cleanup-prepare"
    static let snapshotCandidateCleanupConfirm =
        "explorer-snapshot-candidate-cleanup-confirm"
    static let snapshotCandidateCleanupConfirmation =
        "explorer-snapshot-candidate-cleanup-confirmation"
    static let snapshotCandidateCleanupStatus =
        "explorer-snapshot-candidate-cleanup-status"
    static let snapshotCandidateCleanupCancel =
        "explorer-snapshot-candidate-cleanup-cancel"
    static let snapshotCandidateCleanupDismiss =
        "explorer-snapshot-candidate-cleanup-dismiss"
    static let snapshotCandidateCleanupSettings =
        "explorer-snapshot-candidate-cleanup-settings"
    static let snapshotCandidateCleanupHistory =
        "explorer-snapshot-candidate-cleanup-history"
    static let snapshotLargeFileThreshold = "explorer-snapshot-large-file-threshold"
    static let snapshotLargeFileAge = "explorer-snapshot-large-file-age"
    static let snapshotLargeFileTable = "explorer-snapshot-large-file-table"
    static let snapshotLargeFileStatus = "explorer-snapshot-large-file-status"
    static let snapshotCoverageView = "explorer-snapshot-coverage-view"
    static let snapshotCoverageStatus = "explorer-snapshot-coverage-status"
    static let snapshotCoverageBar = "explorer-snapshot-coverage-bar"
    static let snapshotCoverageIssueList = "explorer-snapshot-coverage-issue-list"
    static let snapshotRevealInFinder = "explorer-snapshot-reveal-in-finder"
    static let snapshotCopyPath = "explorer-snapshot-copy-path"
    static let snapshotQuickLook = "explorer-snapshot-quick-look"
    static let snapshotMoveToTrash = "explorer-snapshot-move-to-trash"
    static let snapshotTrashStatus = "explorer-snapshot-trash-status"
    static let snapshotLiveActionStatus = "explorer-snapshot-live-action-status"
    static let snapshotICloudLocalCopyReview =
        "explorer-snapshot-icloud-local-copy-review"
    static let snapshotICloudLocalCopyCheck =
        "explorer-snapshot-icloud-local-copy-check"
    static let snapshotICloudObservationView =
        "explorer-snapshot-icloud-observation-view"
    static let snapshotICloudObservationSourceStatus =
        "explorer-snapshot-icloud-observation-source-status"
    static let snapshotICloudObservationTable =
        "explorer-snapshot-icloud-observation-table"
    static let snapshotICloudObservationSummary =
        "explorer-snapshot-icloud-observation-summary"
    static let snapshotICloudObservationCheck =
        "explorer-snapshot-icloud-observation-check"
    static let snapshotICloudObservationStop =
        "explorer-snapshot-icloud-observation-stop"
    static let snapshotICloudObservationBatchStatus =
        "explorer-snapshot-icloud-observation-batch-status"
    static let snapshotICloudObservationDisclosure =
        "explorer-snapshot-icloud-observation-disclosure"
    static let snapshotICloudObservationInspector =
        "explorer-snapshot-icloud-observation-inspector"
    static let snapshotAIExplain = "explorer-snapshot-ai-explain"
    static let snapshotAIDisclosure = "explorer-snapshot-ai-disclosure"
    static let snapshotAIMetadata = "explorer-snapshot-ai-metadata"
    static let snapshotAISend = "explorer-snapshot-ai-send"
    static let snapshotAICancel = "explorer-snapshot-ai-cancel"
    static let snapshotAIProgress = "explorer-snapshot-ai-progress"
    static let snapshotAIResult = "explorer-snapshot-ai-result"
    static let snapshotAIFailure = "explorer-snapshot-ai-failure"
    static let snapshotAILegend = "explorer-snapshot-ai-legend"

    static let allIdentifiers = [
        root,
        sidebar,
        overviewDestination,
        snapshotDestination,
        recommendationsDestination,
        recommendations,
        emergencyRecoveryHero,
        emergencyRecoveryStatus,
        emergencyRecoveryCards,
        emergencyRecoveryFreshness,
        emergencyRecoveryLimitations,
        targetedReclaimScan,
        targetedReclaimScanStatus,
        targetedReclaimScanProgress,
        targetedReclaimScanCancel,
        targetedReclaimScanRetry,
        cleanupHistoryDestination,
        cleanupHistory,
        cleanupHistoryStatus,
        cleanupHistoryChart,
        cleanupHistoryLoadMore,
        cleanupHistoryDetail,
        cleanupHistoryDetailStatus,
        cleanupHistoryDetailBack,
        cleanupHistoryDetailRetry,
        cleanupHistoryDetailSummary,
        cleanupHistoryItemChart,
        cleanupHistoryPathChart,
        cleanupHistoryWarnings,
        cleanupHistoryRuleOutcomesStatus,
        cleanupHistoryRuleOutcomesRetry,
        cleanupHistoryStorageThieves,
        cleanupHistoryStorageThievesStatus,
        cleanupHistoryStorageThievesChart,
        cleanupHistoryStorageThievesRetry,
        settingsDestination,
        settingsShortcut,
        capacityCard,
        capacityBar,
        available,
        used,
        total,
        pressure,
        freshness,
        capacityHistoryCard,
        capacityHistoryStatus,
        capacityHistoryChart,
        pressureEpisodeTimeline,
        pressureEpisodeList,
        scanCard,
        scanStatus,
        coverage,
        refreshCapacity,
        scanNow,
        cancelScan,
        snapshotBrowser,
        snapshotBreadcrumbs,
        snapshotBack,
        snapshotSort,
        snapshotTable,
        snapshotTreemap,
        snapshotTreemapOther,
        snapshotCategoryLegend,
        snapshotCategoryColumn,
        snapshotInspectorCategory,
        snapshotInspector,
        snapshotPageStatus,
        snapshotPreviousPage,
        snapshotNextPage,
        snapshotReload,
        snapshotSubtreeRescan,
        snapshotSubtreeScanStatus,
        snapshotSubtreeScanCancel,
        snapshotSubtreeScanNotice,
        snapshotHistory,
        snapshotHistoryStatus,
        snapshotError,
        snapshotContentMode,
        snapshotChanges,
        snapshotChangesStatus,
        snapshotChangesDisclosure,
        snapshotChangesTimeline,
        snapshotChangesSummary,
        snapshotChangesBreadcrumbs,
        snapshotChangesBack,
        snapshotChangesSort,
        snapshotChangesTable,
        snapshotChangesTreemap,
        snapshotChangesOtherGrowth,
        snapshotChangesOtherShrinkage,
        snapshotChangesLegend,
        snapshotChangesInspector,
        snapshotChangesPageStatus,
        snapshotChangesPreviousPage,
        snapshotChangesNextPage,
        snapshotCandidates,
        snapshotCandidateGroups,
        snapshotCandidateTable,
        snapshotCandidatePageStatus,
        snapshotCandidatePreviousPage,
        snapshotCandidateNextPage,
        snapshotCandidateStatus,
        snapshotCandidateInspector,
        snapshotCandidateDetailStatus,
        snapshotCandidatePaths,
        snapshotCandidatePathPrevious,
        snapshotCandidatePathNext,
        snapshotCandidateEvidence,
        snapshotCandidateEvidencePrevious,
        snapshotCandidateEvidenceNext,
        snapshotCandidatePlanReview,
        snapshotCandidatePlanReviewPrepare,
        snapshotCandidatePlanReviewStatus,
        snapshotCandidatePlanReviewSummary,
        snapshotCandidatePlanReviewWarnings,
        snapshotCandidatePlanReviewRefresh,
        snapshotCandidatePlanReviewClose,
        snapshotCandidateDryRunStart,
        snapshotCandidateDryRunStatus,
        snapshotCandidateDryRunCancel,
        snapshotCandidateDryRunDismiss,
        snapshotCandidateCleanupPrepare,
        snapshotCandidateCleanupConfirm,
        snapshotCandidateCleanupConfirmation,
        snapshotCandidateCleanupStatus,
        snapshotCandidateCleanupCancel,
        snapshotCandidateCleanupDismiss,
        snapshotCandidateCleanupSettings,
        snapshotCandidateCleanupHistory,
        snapshotLargeFileThreshold,
        snapshotLargeFileAge,
        snapshotLargeFileTable,
        snapshotLargeFileStatus,
        snapshotCoverageView,
        snapshotCoverageStatus,
        snapshotCoverageBar,
        snapshotCoverageIssueList,
        snapshotRevealInFinder,
        snapshotCopyPath,
        snapshotQuickLook,
        snapshotMoveToTrash,
        snapshotTrashStatus,
        snapshotLiveActionStatus,
        snapshotICloudLocalCopyReview,
        snapshotICloudLocalCopyCheck,
        snapshotICloudObservationView,
        snapshotICloudObservationSourceStatus,
        snapshotICloudObservationTable,
        snapshotICloudObservationSummary,
        snapshotICloudObservationCheck,
        snapshotICloudObservationStop,
        snapshotICloudObservationBatchStatus,
        snapshotICloudObservationDisclosure,
        snapshotICloudObservationInspector,
        snapshotAIExplain,
        snapshotAIDisclosure,
        snapshotAIMetadata,
        snapshotAISend,
        snapshotAICancel,
        snapshotAIProgress,
        snapshotAIResult,
        snapshotAIFailure,
        snapshotAILegend,
    ]

    static func snapshotTreemapCell(nodeID: UInt64) -> String {
        "explorer-snapshot-treemap-cell-\(nodeID)"
    }

    static func snapshotChangesTreemapCell(nodeID: UInt64) -> String {
        "explorer-snapshot-changes-treemap-cell-\(nodeID)"
    }

    static func snapshotAIGroupBadge(groupID: Int, nodeID: UInt64) -> String {
        "explorer-snapshot-ai-group-\(groupID)-node-\(nodeID)"
    }

    static func snapshotCategoryLegend(category: ExplorerStorageCategory) -> String {
        "explorer-snapshot-category-legend-\(category.presentation.palette.rawValue)"
    }

    static func cleanupHistoryRow(sessionID: String) -> String {
        "explorer-cleanup-history-row-\(sessionID)"
    }

    static func cleanupHistoryRowChart(sessionID: String) -> String {
        "\(cleanupHistoryChart)-\(sessionID)"
    }

    static func cleanupHistoryItem(ordinal: UInt16) -> String {
        "explorer-cleanup-history-item-\(ordinal)"
    }

    static func cleanupHistoryRuleOutcome(ordinal: UInt16) -> String {
        "explorer-cleanup-history-rule-outcome-\(ordinal)"
    }

    static func cleanupHistoryStorageThiefRow(ruleID: String) -> String {
        "explorer-cleanup-history-storage-thief-\(ruleID)"
    }

    static func targetedReclaimScanRoot(ordinal: UInt16) -> String {
        "explorer-targeted-reclaim-scan-root-\(ordinal)"
    }

    static func targetedReclaimScanReview(ordinal: UInt16) -> String {
        "explorer-targeted-reclaim-scan-review-\(ordinal)"
    }

    static func emergencyRecoveryCard(kind: String) -> String {
        "explorer-emergency-recovery-card-\(kind)"
    }

    static func emergencyRecoveryCardAction(kind: String) -> String {
        "explorer-emergency-recovery-card-action-\(kind)"
    }
}

enum ExplorerDestination: String, CaseIterable, Identifiable, Sendable {
    case overview
    case snapshot
    case recommendations
    case cleanupHistory
    case settings

    var id: Self { self }
}

enum ExplorerKeyboardShortcut {
    static let scanNow: Character = "r"
    static let cancelScan: Character = "."
    static let settings: Character = ","

    static let allKeys = [scanNow, cancelScan, settings]
}

enum CleanupHistoryCapacityOutcomeKind: Equatable, Sendable {
    case unknown
    case increased
    case unchanged
    case decreased
}

struct CleanupHistoryCapacityOutcomePresentation: Equatable, Sendable {
    let kind: CleanupHistoryCapacityOutcomeKind
    let value: String
    let detail: String
}

struct CleanupHistoryWarningPresentation: Equatable, Sendable {
    let title: String
    let detail: String
    let symbol: String
}

struct CleanupHistoryRuleOutcomePresentation: Equatable, Sendable {
    let title: String
    let detail: String
    let symbol: String
}

struct CleanupHistoryOutcomeGroups: Equatable, Sendable {
    let changedOnDisk: UInt16
    let notChanged: UInt16
    let needsAttention: UInt16
    let unresolved: UInt16
    let total: UInt16

    var accessibilitySummary: String {
        "\(total) total; \(changedOnDisk) changed on disk; \(notChanged) not changed; "
            + "\(needsAttention) need attention; \(unresolved) unresolved"
    }
}

/// Copy and visual grouping for immutable cleanup-history observations. None
/// of these projections can be fed back into planning or execution.
enum CleanupHistoryPresentation {
    static func sessionStatusTitle(_ status: CleanupHistorySessionStatus) -> String {
        switch status {
        case .planned: "Planned"
        case .running: "Running"
        case .recovering: "Recovering"
        case .completed: "Completed"
        case .partiallyCompleted: "Partially completed"
        case .failed: "Failed"
        case .cancelled: "Cancelled"
        case .interrupted: "Interrupted"
        case .rejected: "Rejected"
        case .dryRun: "Dry run"
        }
    }

    static func sessionStatusSymbol(_ status: CleanupHistorySessionStatus) -> String {
        switch status {
        case .completed: "checkmark.circle.fill"
        case .partiallyCompleted, .failed, .cancelled, .interrupted:
            "exclamationmark.circle.fill"
        case .running, .recovering: "arrow.triangle.2.circlepath"
        case .rejected: "xmark.circle.fill"
        case .planned, .dryRun: "clock"
        }
    }

    static func modeTitle(_ mode: CleanupHistoryMode) -> String {
        switch mode {
        case .dryRun: "Dry run"
        case .trash: "Trash"
        case .permanentSafe: "Permanent-safe"
        case .evictLocalCopy: "Evict local copy"
        }
    }

    static func triggerTitle(_ trigger: CleanupHistoryTrigger) -> String {
        switch trigger {
        case .manual: "Manual"
        case .lowDisk: "Low disk"
        case .scheduled: "Scheduled"
        case .cli: "CLI"
        }
    }

    static func itemStatusTitle(_ status: CleanupHistoryItemStatus) -> String {
        switch status {
        case .planned: "Planned"
        case .validating: "Validating"
        case .dryRun: "Dry run"
        case .effectStarted: "Effect started"
        case .trashed: "Moved to Trash"
        case .removed: "Removed"
        case .evicted: "Local copy evicted"
        case .skipped: "Skipped"
        case .rejected: "Rejected"
        case .failed: "Failed"
        case .changedSincePlan: "Changed since plan"
        case .interrupted: "Interrupted"
        case .unavailable: "Unavailable"
        case .outcomeUnknown: "Outcome unknown"
        }
    }

    static func itemStatusSymbol(_ status: CleanupHistoryItemStatus) -> String {
        switch status {
        case .trashed, .removed, .evicted: "checkmark.circle.fill"
        case .skipped, .rejected, .changedSincePlan: "minus.circle.fill"
        case .failed, .interrupted, .unavailable, .outcomeUnknown:
            "exclamationmark.circle.fill"
        case .planned, .validating, .dryRun, .effectStarted: "clock"
        }
    }

    static func capacityOutcome(
        deltaBytes: Int64?
    ) -> CleanupHistoryCapacityOutcomePresentation {
        guard let deltaBytes else {
            return CleanupHistoryCapacityOutcomePresentation(
                kind: .unknown,
                value: "Not verified",
                detail: "No valid pre- and post-cleanup capacity pair was recorded. This is unknown, not zero."
            )
        }
        if deltaBytes == 0 {
            return CleanupHistoryCapacityOutcomePresentation(
                kind: .unchanged,
                value: "0 bytes",
                detail: "Verified available capacity did not change during the bounded measurement window."
            )
        }

        let formatted = ByteCountFormatter.string(
            fromByteCount: deltaBytes,
            countStyle: .file
        )
        if deltaBytes > 0 {
            return CleanupHistoryCapacityOutcomePresentation(
                kind: .increased,
                value: "+\(formatted)",
                detail: "Verified available capacity increased."
            )
        }
        return CleanupHistoryCapacityOutcomePresentation(
            kind: .decreased,
            value: formatted,
            detail: "Verified available capacity decreased."
        )
    }

    static func warning(
        _ warning: CleanupHistoryWarning
    ) -> CleanupHistoryWarningPresentation {
        switch warning {
        case .estimatedBytesUnverified:
            CleanupHistoryWarningPresentation(
                title: "Estimate is not measured capacity",
                detail: "The plan estimate may differ from the verified available-space change.",
                symbol: "ruler"
            )
        case .dryRunDoesNotMutate:
            CleanupHistoryWarningPresentation(
                title: "Dry run changed no files",
                detail: "This session observed what would happen without performing cleanup.",
                symbol: "eye"
            )
        case .trashDoesNotFreeSpaceImmediately:
            CleanupHistoryWarningPresentation(
                title: "Trash may still use space",
                detail: "Moving items to Trash does not guarantee immediate free-space recovery.",
                symbol: "trash"
            )
        case .permanentRemovalCannotBeUndone:
            CleanupHistoryWarningPresentation(
                title: "Permanent removal cannot be undone",
                detail: "This historical result is informational and cannot repeat the action.",
                symbol: "exclamationmark.triangle"
            )
        case .cloudEvictionRequiresNetworkToRedownload:
            CleanupHistoryWarningPresentation(
                title: "Cloud data may need downloading",
                detail: "An evicted local copy requires network access to download again.",
                symbol: "icloud.and.arrow.down"
            )
        }
    }

    static func ruleOutcome(
        _ state: CleanupHistoryRuleOutcomeState
    ) -> CleanupHistoryRuleOutcomePresentation {
        switch state {
        case let .notEligible(reason):
            return CleanupHistoryRuleOutcomePresentation(
                title: "Not comparable",
                detail: ruleOutcomeNotEligibleReason(reason),
                symbol: "slash.circle"
            )
        case .awaitingComparableScan:
            return CleanupHistoryRuleOutcomePresentation(
                title: "Waiting for a later comparable scan",
                detail: "No later complete matching observation is available yet.",
                symbol: "clock.arrow.circlepath"
            )
        case .superseded:
            return CleanupHistoryRuleOutcomePresentation(
                title: "Superseded by a later cleanup",
                detail: "Tracking ended before a terminal observation. This is not a regrowth result.",
                symbol: "arrow.triangle.branch"
            )
        case let .laterSizeObserved(_, _, observedBytes):
            return CleanupHistoryRuleOutcomePresentation(
                title: "Later size observed",
                detail: "\(StorageByteFormatter.string(from: observedBytes)) was observed without an explicit zero baseline, so this is not confirmed regrowth.",
                symbol: "ruler"
            )
        case .zeroBaselineObserved:
            return CleanupHistoryRuleOutcomePresentation(
                title: "Zero baseline observed",
                detail: "A comparable scan observed zero reclaimable bytes. No regrowth has been observed.",
                symbol: "0.circle"
            )
        case let .regrown(_, zeroObservedAt, observedAt, observedBytes):
            let elapsed = max(0, observedAt.timeIntervalSince(zeroObservedAt))
            return CleanupHistoryRuleOutcomePresentation(
                title: "Regrown",
                detail: "\(StorageByteFormatter.string(from: observedBytes)) was observed \(duration(elapsed)) after the zero baseline.",
                symbol: "chart.line.uptrend.xyaxis"
            )
        }
    }

    static func ruleOutcomeNotEligibleReason(
        _ reason: CleanupHistoryRuleOutcomeNotEligibleReason
    ) -> String {
        switch reason {
        case .sourceCleanupIncomplete:
            "The source cleanup did not finish in an eligible terminal state."
        case .itemNotSuccessfulPermanentRegenerable:
            "This item was not a successful permanent cleanup of regenerable data."
        case .sourceScanNotComparable:
            "The cleanup’s source scan lacks the exact comparable storage identity."
        case .sourceEvaluationNotComparable:
            "The cleanup’s source evaluation is not comparable under the current rules."
        case .sourceEvaluationAfterPlan:
            "The source evaluation completed after the cleanup plan was created."
        case .sourceCandidateMismatch:
            "The source candidate no longer exactly matches the recorded cleanup item."
        }
    }

    private static func duration(_ interval: TimeInterval) -> String {
        let seconds = Int(interval.rounded(.down))
        if seconds < 60 {
            return seconds == 1 ? "1 second" : "\(seconds) seconds"
        }
        let minutes = seconds / 60
        if minutes < 60 {
            return minutes == 1 ? "1 minute" : "\(minutes) minutes"
        }
        let hours = minutes / 60
        if hours < 24 {
            return hours == 1 ? "1 hour" : "\(hours) hours"
        }
        let days = hours / 24
        return days == 1 ? "1 day" : "\(days) days"
    }

    static func outcomeGroups(
        _ counts: CleanupHistoryStatusCounts
    ) -> CleanupHistoryOutcomeGroups {
        CleanupHistoryOutcomeGroups(
            changedOnDisk: sum(counts.trashed, counts.removed, counts.evicted),
            notChanged: sum(
                counts.dryRun,
                counts.skipped,
                counts.rejected,
                counts.changedSincePlan
            ),
            needsAttention: sum(
                counts.failed,
                counts.interrupted,
                counts.unavailable,
                counts.outcomeUnknown
            ),
            unresolved: sum(counts.planned, counts.validating, counts.effectStarted),
            total: counts.total
        )
    }

    private static func sum(_ values: UInt16...) -> UInt16 {
        UInt16(
            clamping: values.reduce(0) { total, value in
                total + Int(value)
            }
        )
    }
}

enum ExplorerCapacityStatus: Equatable, Sendable {
    case refreshing(message: String)
    case stale(message: String)
}

enum ExplorerCapacityBreakdown: Equatable, Sendable {
    case known(usedFraction: Double)
    case unavailable(message: String)
}

struct ExplorerCapacitySnapshotPresentation: Equatable, Sendable {
    let volumeName: String
    let pressure: DiskPressureLevel
    let pressureTitle: String
    let availableValue: String
    let usedValue: String
    let totalValue: String
    let availabilityBasis: String
    let freshness: String
    let breakdown: ExplorerCapacityBreakdown
    let accessibilitySummary: String
}

enum ExplorerCapacityPresentation: Equatable, Sendable {
    case loading(message: String)
    case snapshot(
        ExplorerCapacitySnapshotPresentation,
        status: ExplorerCapacityStatus?
    )
    case failed(title: String, detail: String)
}

struct ExplorerCoveragePresentation: Equatable, Sendable {
    let coverage: AppScanCoverage
    let title: String
    let detail: String
}

struct ExplorerActionPresentation: Equatable, Sendable {
    let refreshCapacityEnabled: Bool
    let showScanNow: Bool
    let scanNowEnabled: Bool
    let showCancelScan: Bool
    let cancelScanEnabled: Bool
}

struct ExplorerPresentation: Equatable, Sendable {
    let capacity: ExplorerCapacityPresentation
    let coverage: ExplorerCoveragePresentation
    let scan: ExplorerScanPresentation?
    let actions: ExplorerActionPresentation
}

extension ExplorerPresentation {
    static func make(
        volumeState: VolumeCapacityState,
        scanState: AppScanState,
        now: Date = .now,
        locale: Locale = .current
    ) -> Self {
        return Self(
            capacity: capacityPresentation(
                for: volumeState,
                now: now,
                locale: locale
            ),
            coverage: coveragePresentation(for: scanState, locale: locale),
            scan: ExplorerScanPresentation.make(
                scanState: scanState,
                now: now,
                locale: locale
            ),
            actions: actionPresentation(
                volumeState: volumeState,
                scanState: scanState
            )
        )
    }

    private static func capacityPresentation(
        for state: VolumeCapacityState,
        now: Date,
        locale: Locale
    ) -> ExplorerCapacityPresentation {
        switch state {
        case .idle, .loading:
            return .loading(
                message: String(localized: "Checking startup disk…", locale: locale)
            )
        case let .loaded(snapshot):
            return .snapshot(
                snapshotPresentation(snapshot, now: now, locale: locale),
                status: nil
            )
        case let .refreshing(snapshot):
            return .snapshot(
                snapshotPresentation(snapshot, now: now, locale: locale),
                status: .refreshing(
                    message: String(localized: "Updating capacity…", locale: locale)
                )
            )
        case let .stale(snapshot, failure):
            return .snapshot(
                snapshotPresentation(snapshot, now: now, locale: locale),
                status: .stale(message: staleMessage(for: failure, locale: locale))
            )
        case let .failed(failure):
            return .failed(
                title: String(localized: "Storage capacity unavailable", locale: locale),
                detail: capacityFailureDetail(for: failure, locale: locale)
            )
        }
    }

    private static func snapshotPresentation(
        _ snapshot: VolumeCapacitySnapshot,
        now: Date,
        locale: Locale
    ) -> ExplorerCapacitySnapshotPresentation {
        let volumeName = snapshot.displayName ?? String(
            localized: "Startup Disk",
            locale: locale
        )
        let availableValue = MenuBarCapacityFormatter.gib(
            snapshot.effectiveAvailableBytes,
            locale: locale
        )
        let totalValue = MenuBarCapacityFormatter.gib(snapshot.totalBytes, locale: locale)
        let usedValue = snapshot.usedBytes.map {
            MenuBarCapacityFormatter.gib($0, locale: locale)
        } ?? String(localized: "Unavailable", locale: locale)
        let availabilityBasis = switch snapshot.availabilityBasis {
        case .importantUsage:
            String(localized: "Available for important use", locale: locale)
        case .filesystemAvailable:
            String(localized: "Filesystem available", locale: locale)
        }
        let pressureTitle = pressureTitle(for: snapshot.pressure, locale: locale)
        let freshness = String(
            localized: "Updated \(relativeText(for: snapshot.sampledAt, now: now, locale: locale))",
            locale: locale
        )
        let breakdown: ExplorerCapacityBreakdown = if let usedFraction = snapshot.usedFraction {
            .known(usedFraction: min(max(usedFraction, 0), 1))
        } else {
            .unavailable(
                message: String(
                    localized: "Used-space breakdown is unavailable from this capacity sample.",
                    locale: locale
                )
            )
        }
        let availableSummary = String(
            localized: "\(availableValue) available",
            locale: locale
        )
        let summaryParts = [
            volumeName,
            pressureTitle,
            availableSummary,
            String(localized: "\(usedValue) used", locale: locale),
            String(localized: "\(totalValue) total", locale: locale),
            availabilityBasis,
            freshness,
        ]
        let summary = if snapshot.pressure == .critical {
            ([availableSummary] + summaryParts.filter { $0 != availableSummary })
                .joined(separator: ". ")
        } else {
            summaryParts.joined(separator: ". ")
        }
        return ExplorerCapacitySnapshotPresentation(
            volumeName: volumeName,
            pressure: snapshot.pressure,
            pressureTitle: pressureTitle,
            availableValue: availableValue,
            usedValue: usedValue,
            totalValue: totalValue,
            availabilityBasis: availabilityBasis,
            freshness: freshness,
            breakdown: breakdown,
            accessibilitySummary: summary
        )
    }

    private static func coveragePresentation(
        for state: AppScanState,
        locale: Locale
    ) -> ExplorerCoveragePresentation {
        let summary: AppScanSummary? = switch state.phase {
        case let .succeeded(summary) where state.scope?.isHome != false: summary
        case .succeeded:
            state.lastSuccessful
        case .idle, .queued, .scanning, .finalizing, .evaluating,
             .cancellationRequested, .cancelled, .failed:
            state.lastSuccessful
        }
        guard let summary else {
            return ExplorerCoveragePresentation(
                coverage: .unknown,
                title: String(localized: "No Home scan loaded", locale: locale),
                detail: String(
                    localized: "Run a Home scan to load current aggregate coverage in this window without changing files.",
                    locale: locale
                )
            )
        }

        let issueCount = summary.progress.issueCount.formatted(.number.locale(locale))
        let reportedCoverage = summary.coveragePermille.map {
            (Double($0) / 1000).formatted(
                .percent.locale(locale).precision(.fractionLength(0 ... 1))
            )
        }
        func detail(_ base: String) -> String {
            guard let reportedCoverage else {
                return base
            }
            return [
                base,
                String(
                    localized: "Home coverage reported as \(reportedCoverage).",
                    locale: locale
                ),
            ].joined(separator: " ")
        }
        switch summary.coverage {
        case .complete:
            return ExplorerCoveragePresentation(
                coverage: .complete,
                title: String(localized: "Complete Home coverage", locale: locale),
                detail: detail(
                    String(
                        localized: "The latest confirmed Home scan completed with \(issueCount) access issues.",
                        locale: locale
                    )
                )
            )
        case .limitedAccess:
            return ExplorerCoveragePresentation(
                coverage: .limitedAccess,
                title: String(localized: "Limited Home access", locale: locale),
                detail: detail(
                    String(
                        localized: "The latest confirmed Home scan observed \(issueCount) access issues.",
                        locale: locale
                    )
                )
            )
        case .partial:
            return ExplorerCoveragePresentation(
                coverage: .partial,
                title: String(localized: "Partial Home coverage", locale: locale),
                detail: detail(
                    String(
                        localized: "The latest confirmed Home scan observed \(issueCount) access issues and did not cover every readable item.",
                        locale: locale
                    )
                )
            )
        case .unknown:
            return ExplorerCoveragePresentation(
                coverage: .unknown,
                title: String(localized: "Home coverage unavailable", locale: locale),
                detail: detail(
                    String(
                        localized: "The latest confirmed Home scan did not report a reliable coverage classification and observed \(issueCount) access issues.",
                        locale: locale
                    )
                )
            )
        }
    }

    private static func actionPresentation(
        volumeState: VolumeCapacityState,
        scanState: AppScanState
    ) -> ExplorerActionPresentation {
        let refreshEnabled = switch volumeState {
        case .idle, .loading, .refreshing: false
        case .loaded, .stale, .failed: true
        }
        switch scanState.phase {
        case .queued, .scanning, .finalizing, .evaluating:
            return ExplorerActionPresentation(
                refreshCapacityEnabled: refreshEnabled,
                showScanNow: false,
                scanNowEnabled: false,
                showCancelScan: true,
                cancelScanEnabled: true
            )
        case .cancellationRequested:
            return ExplorerActionPresentation(
                refreshCapacityEnabled: refreshEnabled,
                showScanNow: false,
                scanNowEnabled: false,
                showCancelScan: true,
                cancelScanEnabled: false
            )
        case .idle, .succeeded, .cancelled, .failed:
            return ExplorerActionPresentation(
                refreshCapacityEnabled: refreshEnabled,
                showScanNow: true,
                scanNowEnabled: true,
                showCancelScan: false,
                cancelScanEnabled: false
            )
        }
    }

    private static func relativeText(
        for date: Date,
        now: Date,
        locale: Locale
    ) -> String {
        guard now.timeIntervalSince(date) >= 60 else {
            return String(localized: "just now", locale: locale)
        }
        let formatter = RelativeDateTimeFormatter()
        formatter.locale = locale
        formatter.unitsStyle = .full
        return formatter.localizedString(for: date, relativeTo: now)
    }

    private static func staleMessage(
        for failure: VolumeCapacityFailure,
        locale: Locale
    ) -> String {
        switch failure {
        case .unavailable:
            String(
                localized: "The latest capacity check was unavailable. Showing the last confirmed sample.",
                locale: locale
            )
        case .invalidObservation:
            String(
                localized: "The latest capacity check was invalid. Showing the last confirmed sample.",
                locale: locale
            )
        case .engineUnavailable:
            String(
                localized: "The storage engine could not classify the latest capacity. Showing the last confirmed sample.",
                locale: locale
            )
        }
    }

    private static func pressureTitle(
        for pressure: DiskPressureLevel,
        locale: Locale
    ) -> String {
        switch pressure {
        case .healthy: String(localized: "Healthy", locale: locale)
        case .warning: String(localized: "Low space", locale: locale)
        case .critical: String(localized: "Critically low space", locale: locale)
        case .unknown: String(localized: "Pressure unknown", locale: locale)
        }
    }

    private static func capacityFailureDetail(
        for failure: VolumeCapacityFailure,
        locale: Locale
    ) -> String {
        switch failure {
        case .unavailable:
            String(
                localized: "macOS did not report startup-disk capacity.",
                locale: locale
            )
        case .invalidObservation:
            String(
                localized: "The startup-disk capacity response was invalid.",
                locale: locale
            )
        case .engineUnavailable:
            String(
                localized: "The storage engine is unavailable.",
                locale: locale
            )
        }
    }
}
