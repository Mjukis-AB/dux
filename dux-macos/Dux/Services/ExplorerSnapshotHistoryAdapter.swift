import Foundation

/// The only conversion boundary between generated history records and
/// app-owned Explorer models.
enum ExplorerSnapshotHistoryAdapter {
    private static let recordVersion: UInt32 = 1
    private static let maximumUnixMilliseconds: Int64 = 253_402_300_799_999

    static func map(
        _ page: RecentScanHistoryPage
    ) throws -> ExplorerSnapshotHistoryPage {
        guard page.recordVersion == recordVersion else {
            throw ExplorerSnapshotHistoryError.invalidResponse
        }
        let scans = try page.scans.map(mapItem)
        for pair in zip(scans, scans.dropFirst()) {
            let ordered = pair.0.startedAt > pair.1.startedAt
                || (pair.0.startedAt == pair.1.startedAt && pair.0.scanID < pair.1.scanID)
            guard ordered else {
                throw ExplorerSnapshotHistoryError.invalidResponse
            }
        }
        return ExplorerSnapshotHistoryPage(scans: scans, hasMore: page.hasMore)
    }

    private static func mapItem(
        _ raw: HistoricalScanSummary
    ) throws -> ExplorerHistoricalScan {
        guard
            raw.recordVersion == recordVersion,
            validScanID(raw.scanId),
            validUnixMilliseconds(raw.startedAtUnixMs),
            raw.completedAtUnixMs.map(validUnixMilliseconds) ?? true,
            raw.completedAtUnixMs.map({ $0 >= raw.startedAtUnixMs }) ?? true,
            validCoverage(raw.coverage)
        else {
            throw ExplorerSnapshotHistoryError.invalidResponse
        }

        let status: ExplorerHistoricalScanStatus
        switch raw.status {
        case .queued:
            status = .queued
        case .running:
            status = .running
        case .succeeded:
            status = .succeeded
        case .failed:
            status = .failed
        case .cancelled:
            status = .cancelled
        case .interrupted:
            status = .interrupted
        }
        let isSucceeded = status == .succeeded
        let isActive = status == .queued || status == .running
        guard
            (raw.counts != nil) == isSucceeded,
            !raw.snapshotRecorded || isSucceeded,
            isActive ? raw.completedAtUnixMs == nil : raw.completedAtUnixMs != nil
        else {
            throw ExplorerSnapshotHistoryError.invalidResponse
        }

        return ExplorerHistoricalScan(
            scanID: raw.scanId,
            startedAt: date(raw.startedAtUnixMs),
            completedAt: raw.completedAtUnixMs.map(date),
            status: status,
            counts: raw.counts.map {
                ExplorerHistoricalScanCounts(
                    directoryCount: $0.directoryCount,
                    fileCount: $0.fileCount,
                    logicalBytes: $0.logicalBytes,
                    allocatedBytes: $0.allocatedBytes
                )
            },
            coverage: mapCoverage(raw.coverage.status),
            coveragePermille: raw.coverage.measuredPermille,
            issueCount: raw.coverage.issueOccurrenceCount,
            snapshotRecorded: raw.snapshotRecorded
        )
    }

    static func validScanID(_ value: String) -> Bool {
        value.hasPrefix("scan:")
            && value.utf8.count > 5
            && value.utf8.count <= 128
            && value.unicodeScalars.allSatisfy { scalar in
                scalar.isASCII
                    && (CharacterSet.alphanumerics.contains(scalar)
                        || "._-:".unicodeScalars.contains(scalar))
            }
    }

    private static func validUnixMilliseconds(_ value: Int64) -> Bool {
        (0 ... maximumUnixMilliseconds).contains(value)
    }

    private static func date(_ unixMilliseconds: Int64) -> Date {
        Date(timeIntervalSince1970: Double(unixMilliseconds) / 1000)
    }

    private static func validCoverage(_ coverage: ScanCoverageSummary) -> Bool {
        guard
            coverage.recordVersion == recordVersion,
            coverage.issueRecordCount <= 256,
            coverage.issueOccurrenceCount >= coverage.issueRecordCount,
            coverage.measuredPermille.map({ $0 <= 1000 }) ?? true
        else {
            return false
        }
        switch coverage.status {
        case .unknown:
            return coverage.measuredPermille == nil
                && coverage.issueRecordCount == 0
                && coverage.issueOccurrenceCount == 0
        case .complete:
            return coverage.measuredPermille == 1000
                && coverage.issueRecordCount == 0
                && coverage.issueOccurrenceCount == 0
        case .limitedAccess, .partial:
            return coverage.issueRecordCount > 0 && coverage.measuredPermille != 1000
        }
    }

    private static func mapCoverage(_ coverage: ScanCoverageStatus) -> AppScanCoverage {
        switch coverage {
        case .unknown: .unknown
        case .complete: .complete
        case .limitedAccess: .limitedAccess
        case .partial: .partial
        }
    }
}

/// Converts generated cleanup-history records into bounded app observations.
/// Exact items remain path-free and omit evidence payloads, candidate IDs,
/// execution fences, and every mutation capability.
enum CleanupHistoryAdapter {
    private static let recordVersion: UInt32 = 1
    private static let maximumUnixMilliseconds: Int64 = 253_402_300_799_999
    private static let maximumPageSize = 64
    private static let maximumSessionItems: UInt16 = 64
    private static let maximumSessionPaths: UInt16 = 256
    private static let maximumSessionEvidence: UInt16 = 512
    private static let maximumSessionWarnings = 5

    static func map(
        _ page: CleanupHistoryPage
    ) throws -> CleanupHistoryPageModel {
        guard page.recordVersion == recordVersion,
              page.records.count <= maximumPageSize
        else {
            throw CleanupHistoryServiceError.invalidResponse
        }

        let records = try page.records.map(mapSummary)
        guard Set(records.map(\.sessionID)).count == records.count else {
            throw CleanupHistoryServiceError.invalidResponse
        }
        for pair in zip(records, records.dropFirst()) {
            let ordered = pair.0.startedAt > pair.1.startedAt
                || (pair.0.startedAt == pair.1.startedAt && pair.0.sessionID < pair.1.sessionID)
            guard ordered else {
                throw CleanupHistoryServiceError.invalidResponse
            }
        }

        let nextCursor = try page.nextCursor.map(mapCursor)
        if let nextCursor, let last = records.last {
            guard nextCursor.startedAt == last.startedAt,
                  nextCursor.sessionID == last.sessionID
            else {
                throw CleanupHistoryServiceError.invalidResponse
            }
        } else if nextCursor != nil {
            throw CleanupHistoryServiceError.invalidResponse
        }
        return CleanupHistoryPageModel(records: records, nextCursor: nextCursor)
    }

    static func mapSession(
        _ raw: CleanupSessionHistory,
        requestedSessionID: String
    ) throws -> CleanupHistorySessionDetailModel {
        guard
            raw.recordVersion == recordVersion,
            validStableToken(requestedSessionID),
            raw.summary.sessionId == requestedSessionID,
            raw.items.count <= Int(maximumSessionItems),
            raw.warnings.count <= maximumSessionWarnings,
            Set(raw.warnings).count == raw.warnings.count
        else {
            throw CleanupHistoryServiceError.invalidResponse
        }

        let summary = try mapSummary(raw.summary)
        var pathTotal = 0
        var evidenceTotal = 0
        var estimatedBytesTotal: UInt64 = 0
        var itemStatusCounts = MutableCleanupStatusCounts()
        let items = try raw.items.enumerated().map { index, item in
            guard item.ordinal == UInt16(index) else {
                throw CleanupHistoryServiceError.invalidResponse
            }
            pathTotal += Int(item.pathCount)
            evidenceTotal += Int(item.evidenceCount)
            let (nextEstimatedBytesTotal, overflowed) =
                estimatedBytesTotal.addingReportingOverflow(item.estimatedBytes)
            guard !overflowed else {
                throw CleanupHistoryServiceError.invalidResponse
            }
            estimatedBytesTotal = nextEstimatedBytesTotal
            itemStatusCounts.add(item.status)
            return try mapItem(item, format: raw.summary.format)
        }

        guard
            items.count == Int(summary.itemTotal),
            pathTotal == Int(summary.pathTotal),
            evidenceTotal == Int(summary.evidenceTotal),
            itemStatusCounts.matches(raw.summary.itemStatusCounts),
            raw.summary.format != .complete
            || estimatedBytesTotal == summary.estimatedBytes,
            raw.warnings == expectedWarnings(
                format: raw.summary.format,
                mode: raw.summary.mode,
                items: raw.items
            )
        else {
            throw CleanupHistoryServiceError.invalidResponse
        }

        return CleanupHistorySessionDetailModel(
            summary: summary,
            items: items,
            warnings: raw.warnings.map(map)
        )
    }

    static func mapRuleOutcomes(
        _ raw: RuleOutcomeBatch,
        requestedSessionID: String,
        detail: CleanupHistorySessionDetailModel
    ) throws -> CleanupHistoryRuleOutcomeBatchModel {
        guard
            raw.recordVersion == recordVersion,
            validStableToken(requestedSessionID),
            raw.sessionId == requestedSessionID,
            detail.summary.sessionID == requestedSessionID,
            raw.outcomes.count == detail.items.count,
            raw.outcomes.count <= Int(maximumSessionItems)
        else {
            throw CleanupHistoryServiceError.invalidResponse
        }

        let outcomes = try zip(raw.outcomes, detail.items).enumerated().map {
            index, pair in
            let (outcome, item) = pair
            guard
                outcome.recordVersion == recordVersion,
                outcome.itemOrdinal == UInt16(index),
                outcome.itemOrdinal == item.ordinal,
                validStableToken(outcome.ruleId),
                outcome.ruleId == item.ruleID,
                outcome.ruleRevision > 0,
                outcome.ruleRevision == item.ruleRevision
            else {
                throw CleanupHistoryServiceError.invalidResponse
            }
            return try CleanupHistoryRuleOutcomeModel(
                itemOrdinal: outcome.itemOrdinal,
                ruleID: outcome.ruleId,
                ruleRevision: outcome.ruleRevision,
                state: mapRuleOutcomeState(outcome.state)
            )
        }
        return CleanupHistoryRuleOutcomeBatchModel(
            sessionID: raw.sessionId,
            outcomes: outcomes
        )
    }

    static func mapStorageThiefRanking(
        _ raw: StorageThiefRanking
    ) throws -> CleanupHistoryStorageThiefRankingModel {
        let maximumSourceSessions: UInt16 = 32
        let maximumGroups = 12
        guard
            raw.recordVersion == recordVersion,
            raw.permanentSafeSessionCount <= maximumSourceSessions,
            raw.manualCleanupSessionCount <= raw.permanentSafeSessionCount,
            raw.groups.count <= maximumGroups,
            raw.groups.count <= Int(raw.rankedRuleCount),
            raw.groups.count == min(Int(raw.rankedRuleCount), maximumGroups),
            !raw.hasOlderPermanentSafeSessions
                || raw.permanentSafeSessionCount == maximumSourceSessions
        else {
            throw CleanupHistoryServiceError.invalidResponse
        }

        var ruleIDs = Set<String>()
        let groups = try raw.groups.enumerated().map { index, group in
            let expectedRate = try storageThiefDailyRate(group)
            guard
                group.recordVersion == recordVersion,
                group.rank == UInt16(index + 1),
                validStableToken(group.ruleId),
                ruleIDs.insert(group.ruleId).inserted,
                group.latestRuleRevision > 0,
                group.observedRevisionCount > 0,
                group.successfulCleanupCount > 0,
                group.successfulManualCleanupCount <= group.successfulCleanupCount,
                group.observedRegrowthCycleCount > 0,
                group.manualRegrowthCycleCount <= group.observedRegrowthCycleCount,
                group.totalObservedRegrownBytes > 0,
                group.totalRegrowthDurationNanoseconds < 1_000_000_000,
                group.totalRegrowthDurationSeconds > 0
                    || group.totalRegrowthDurationNanoseconds > 0,
                group.bytesRegrownPerDay == expectedRate.value,
                group.rateCapped == expectedRate.capped,
                validUnixMilliseconds(group.latestCleanupAtUnixMs),
                validUnixMilliseconds(group.latestRegrowthAtUnixMs),
                !group.automationHistoryThresholdMet
                    || (group.successfulManualCleanupCount >= 2
                        && group.manualRegrowthCycleCount > 0)
            else {
                throw CleanupHistoryServiceError.invalidResponse
            }
            return CleanupHistoryStorageThiefGroupModel(
                rank: group.rank,
                ruleID: group.ruleId,
                latestRuleRevision: group.latestRuleRevision,
                observedRevisionCount: group.observedRevisionCount,
                successfulCleanupCount: group.successfulCleanupCount,
                successfulManualCleanupCount: group.successfulManualCleanupCount,
                observedRegrowthCycleCount: group.observedRegrowthCycleCount,
                manualRegrowthCycleCount: group.manualRegrowthCycleCount,
                totalObservedRegrownBytes: group.totalObservedRegrownBytes,
                totalRegrowthDurationSeconds: group.totalRegrowthDurationSeconds,
                totalRegrowthDurationNanoseconds: group.totalRegrowthDurationNanoseconds,
                bytesRegrownPerDay: group.bytesRegrownPerDay,
                rateCapped: group.rateCapped,
                latestCleanupAt: date(group.latestCleanupAtUnixMs),
                latestRegrowthAt: date(group.latestRegrowthAtUnixMs),
                automationHistoryThresholdMet: group.automationHistoryThresholdMet
            )
        }
        guard storageThiefGroupsAreOrdered(raw.groups) else {
            throw CleanupHistoryServiceError.invalidResponse
        }
        return CleanupHistoryStorageThiefRankingModel(
            permanentSafeSessionCount: raw.permanentSafeSessionCount,
            manualCleanupSessionCount: raw.manualCleanupSessionCount,
            rankedRuleCount: raw.rankedRuleCount,
            hasOlderPermanentSafeSessions: raw.hasOlderPermanentSafeSessions,
            groups: groups
        )
    }

    private struct StorageThiefUInt128: Equatable {
        let high: UInt64
        let low: UInt64
    }

    private struct StorageThiefUInt192: Equatable {
        let high: UInt64
        let middle: UInt64
        let low: UInt64
    }

    private static func storageThiefDailyRate(
        _ group: StorageThiefGroup
    ) throws -> (value: UInt64, capped: Bool) {
        guard let duration = storageThiefDuration(group) else {
            throw CleanupHistoryServiceError.invalidResponse
        }
        let dayNanoseconds: UInt64 = 86_400_000_000_000
        guard
            let numerator = storageThiefProduct(
                StorageThiefUInt128(
                    high: 0,
                    low: group.totalObservedRegrownBytes
                ),
                by: dayNanoseconds
            )
        else {
            throw CleanupHistoryServiceError.invalidResponse
        }

        // duration * (UInt64.max + 1) is an exact 64-bit shift.
        let firstUnrepresentable = StorageThiefUInt192(
            high: duration.high,
            middle: duration.low,
            low: 0
        )
        if !storageThiefLessThan(numerator, firstUnrepresentable) {
            return (UInt64.max, true)
        }

        var lower: UInt64 = 0
        var upper = UInt64.max
        while lower < upper {
            let distance = upper - lower
            let midpoint = lower + distance / 2 + distance % 2
            guard let product = storageThiefProduct(duration, by: midpoint) else {
                throw CleanupHistoryServiceError.invalidResponse
            }
            if storageThiefLessThan(numerator, product) {
                upper = midpoint - 1
            } else {
                lower = midpoint
            }
        }
        return (lower, false)
    }

    private static func storageThiefGroupsAreOrdered(
        _ groups: [StorageThiefGroup]
    ) -> Bool {
        zip(groups, groups.dropFirst()).allSatisfy { left, right in
            guard
                let leftDuration = storageThiefDuration(left),
                let rightDuration = storageThiefDuration(right),
                let leftRate = storageThiefProduct(
                    rightDuration,
                    by: left.totalObservedRegrownBytes
                ),
                let rightRate = storageThiefProduct(
                    leftDuration,
                    by: right.totalObservedRegrownBytes
                )
            else {
                return false
            }
            if leftRate != rightRate {
                return storageThiefLessThan(rightRate, leftRate)
            }
            if left.successfulCleanupCount != right.successfulCleanupCount {
                return left.successfulCleanupCount > right.successfulCleanupCount
            }
            if left.observedRegrowthCycleCount != right.observedRegrowthCycleCount {
                return left.observedRegrowthCycleCount
                    > right.observedRegrowthCycleCount
            }
            if left.latestRegrowthAtUnixMs != right.latestRegrowthAtUnixMs {
                return left.latestRegrowthAtUnixMs > right.latestRegrowthAtUnixMs
            }
            return left.ruleId < right.ruleId
        }
    }

    private static func storageThiefDuration(
        _ group: StorageThiefGroup
    ) -> StorageThiefUInt128? {
        let product = group.totalRegrowthDurationSeconds.multipliedFullWidth(
            by: 1_000_000_000
        )
        let (low, carry) = product.low.addingReportingOverflow(
            UInt64(group.totalRegrowthDurationNanoseconds)
        )
        let (high, overflow) = product.high.addingReportingOverflow(
            carry ? 1 : 0
        )
        guard !overflow, high != 0 || low != 0 else {
            return nil
        }
        return StorageThiefUInt128(high: high, low: low)
    }

    private static func storageThiefProduct(
        _ value: StorageThiefUInt128,
        by multiplier: UInt64
    ) -> StorageThiefUInt192? {
        let lowProduct = value.low.multipliedFullWidth(by: multiplier)
        let highProduct = value.high.multipliedFullWidth(by: multiplier)
        let (middle, carry) = lowProduct.high.addingReportingOverflow(
            highProduct.low
        )
        let (high, overflow) = highProduct.high.addingReportingOverflow(
            carry ? 1 : 0
        )
        guard !overflow else {
            return nil
        }
        return StorageThiefUInt192(
            high: high,
            middle: middle,
            low: lowProduct.low
        )
    }

    private static func storageThiefLessThan(
        _ left: StorageThiefUInt192,
        _ right: StorageThiefUInt192
    ) -> Bool {
        if left.high != right.high {
            return left.high < right.high
        }
        if left.middle != right.middle {
            return left.middle < right.middle
        }
        return left.low < right.low
    }

    static func mapCursor(_ raw: CleanupHistoryCursor) throws -> CleanupHistoryCursorModel {
        guard raw.recordVersion == recordVersion,
              validStableToken(raw.sessionId),
              validUnixMilliseconds(raw.startedAtUnixMs)
        else {
            throw CleanupHistoryServiceError.invalidResponse
        }
        return CleanupHistoryCursorModel(
            startedAt: date(raw.startedAtUnixMs),
            sessionID: raw.sessionId
        )
    }

    static func mapSummary(
        _ raw: CleanupSessionSummary
    ) throws -> CleanupHistorySessionSummaryModel {
        guard raw.recordVersion == recordVersion,
              validStableToken(raw.sessionId),
              validStableToken(raw.planId),
              validUnixMilliseconds(raw.startedAtUnixMs),
              raw.completedAtUnixMs.map(validUnixMilliseconds) ?? true,
              raw.planCreatedAtUnixMs.map(validUnixMilliseconds) ?? true,
              raw.planExpiresAtUnixMs.map(validUnixMilliseconds) ?? true,
              raw.completedAtUnixMs.map({ $0 >= raw.startedAtUnixMs }) ?? true,
              raw.itemTotal <= maximumSessionItems,
              raw.pathTotal <= maximumSessionPaths,
              raw.evidenceTotal <= maximumSessionEvidence,
              validCounts(raw.itemStatusCounts, total: raw.itemTotal),
              validCounts(raw.pathStatusCounts, total: raw.pathTotal),
              raw.sourceScanId.map(validStableToken) ?? true
        else {
            throw CleanupHistoryServiceError.invalidResponse
        }

        if let expires = raw.planExpiresAtUnixMs {
            guard let created = raw.planCreatedAtUnixMs, expires >= created else {
                throw CleanupHistoryServiceError.invalidResponse
            }
        }

        switch raw.format {
        case .legacyIncomplete:
            guard raw.sourceScanId == nil,
                  raw.planCreatedAtUnixMs == nil,
                  raw.planExpiresAtUnixMs == nil,
                  raw.cancellationRequested == nil,
                  raw.evidenceTotal == 0,
                  raw.status != .recovering
            else {
                throw CleanupHistoryServiceError.invalidResponse
            }
        case .complete:
            guard raw.sourceScanId != nil,
                  let created = raw.planCreatedAtUnixMs,
                  let expires = raw.planExpiresAtUnixMs,
                  raw.cancellationRequested != nil,
                  created <= raw.startedAtUnixMs,
                  expires > raw.startedAtUnixMs,
                  isTerminal(raw.status) == (raw.completedAtUnixMs != nil),
                  isTerminal(raw.status) || raw.verifiedCapacityDeltaBytes == nil,
                  raw.status != .planned || raw.cancellationRequested == false
            else {
                throw CleanupHistoryServiceError.invalidResponse
            }
        }

        return CleanupHistorySessionSummaryModel(
            sessionID: raw.sessionId,
            planID: raw.planId,
            format: map(raw.format),
            sourceScanID: raw.sourceScanId,
            startedAt: date(raw.startedAtUnixMs),
            completedAt: raw.completedAtUnixMs.map(date),
            planCreatedAt: raw.planCreatedAtUnixMs.map(date),
            planExpiresAt: raw.planExpiresAtUnixMs.map(date),
            mode: map(raw.mode),
            trigger: map(raw.trigger),
            status: map(raw.status),
            estimatedBytes: raw.estimatedBytes,
            verifiedCapacityDeltaBytes: raw.verifiedCapacityDeltaBytes,
            cancellationRequested: raw.cancellationRequested,
            itemTotal: raw.itemTotal,
            pathTotal: raw.pathTotal,
            evidenceTotal: raw.evidenceTotal,
            itemStatusCounts: map(raw.itemStatusCounts),
            pathStatusCounts: map(raw.pathStatusCounts)
        )
    }

    private static func mapItem(
        _ raw: CleanupItemSummary,
        format: CleanupRecordFormat
    ) throws -> CleanupHistoryItemModel {
        guard
            raw.recordVersion == recordVersion,
            raw.ordinal < maximumSessionItems,
            validStableToken(raw.ruleId),
            raw.ruleRevision > 0,
            raw.newestMtimeUnixMs.map(validUnixMilliseconds) ?? true,
            raw.pathCount > 0,
            raw.pathCount <= maximumSessionPaths,
            raw.evidenceCount <= maximumSessionEvidence,
            raw.errorRecorded || raw.errorCategory == nil,
            raw.errorCategory.map(validStableToken) ?? true
        else {
            throw CleanupHistoryServiceError.invalidResponse
        }

        switch format {
        case .legacyIncomplete:
            guard
                raw.category == nil,
                raw.safety == nil,
                raw.action == nil,
                raw.ruleScheduleEligible == nil,
                raw.newestMtimeUnixMs == nil,
                raw.evidenceCount == 0,
                raw.errorCategory == nil
            else {
                throw CleanupHistoryServiceError.invalidResponse
            }
        case .complete:
            guard
                raw.category != nil,
                raw.safety != nil,
                raw.action != nil,
                raw.ruleScheduleEligible != nil
            else {
                throw CleanupHistoryServiceError.invalidResponse
            }
        }

        return CleanupHistoryItemModel(
            ordinal: raw.ordinal,
            ruleID: raw.ruleId,
            ruleRevision: raw.ruleRevision,
            category: raw.category.map(map),
            safety: raw.safety.map(map),
            action: raw.action.map(map),
            ruleScheduleEligible: raw.ruleScheduleEligible,
            newestModificationAt: raw.newestMtimeUnixMs.map(date),
            estimatedBytes: raw.estimatedBytes,
            status: map(raw.status),
            errorRecorded: raw.errorRecorded,
            errorCategory: raw.errorCategory,
            pathCount: raw.pathCount,
            evidenceCount: raw.evidenceCount
        )
    }

    private static func map(_ raw: CleanupRecordFormat) -> CleanupHistoryRecordFormat {
        switch raw {
        case .legacyIncomplete: .legacyIncomplete
        case .complete: .complete
        }
    }

    private static func map(_ raw: CleanupMode) -> CleanupHistoryMode {
        switch raw {
        case .dryRun: .dryRun
        case .trash: .trash
        case .permanentSafe: .permanentSafe
        case .evictLocalCopy: .evictLocalCopy
        }
    }

    private static func map(_ raw: CleanupTrigger) -> CleanupHistoryTrigger {
        switch raw {
        case .manual: .manual
        case .lowDisk: .lowDisk
        case .scheduled: .scheduled
        case .cli: .cli
        }
    }

    private static func map(_ raw: CleanupSessionStatus) -> CleanupHistorySessionStatus {
        switch raw {
        case .planned: .planned
        case .running: .running
        case .recovering: .recovering
        case .completed: .completed
        case .partiallyCompleted: .partiallyCompleted
        case .failed: .failed
        case .cancelled: .cancelled
        case .interrupted: .interrupted
        case .rejected: .rejected
        case .dryRun: .dryRun
        }
    }

    private static func map(_ raw: CleanupItemStatus) -> CleanupHistoryItemStatus {
        switch raw {
        case .planned: .planned
        case .validating: .validating
        case .dryRun: .dryRun
        case .effectStarted: .effectStarted
        case .trashed: .trashed
        case .removed: .removed
        case .evicted: .evicted
        case .skipped: .skipped
        case .rejected: .rejected
        case .failed: .failed
        case .changedSincePlan: .changedSincePlan
        case .interrupted: .interrupted
        case .unavailable: .unavailable
        case .outcomeUnknown: .outcomeUnknown
        }
    }

    private static func map(_ raw: CleanupWarning) -> CleanupHistoryWarning {
        switch raw {
        case .estimatedBytesUnverified: .estimatedBytesUnverified
        case .dryRunDoesNotMutate: .dryRunDoesNotMutate
        case .trashDoesNotFreeSpaceImmediately: .trashDoesNotFreeSpaceImmediately
        case .permanentRemovalCannotBeUndone: .permanentRemovalCannotBeUndone
        case .cloudEvictionRequiresNetworkToRedownload:
            .cloudEvictionRequiresNetworkToRedownload
        }
    }

    private static func mapRuleOutcomeState(
        _ raw: RuleOutcomeState
    ) throws -> CleanupHistoryRuleOutcomeState {
        switch raw {
        case let .notEligible(reason):
            return .notEligible(reason: map(reason))
        case let .awaitingComparableScan(cleanedAtUnixMs):
            guard validUnixMilliseconds(cleanedAtUnixMs) else {
                throw CleanupHistoryServiceError.invalidResponse
            }
            return .awaitingComparableScan(cleanedAt: date(cleanedAtUnixMs))
        case let .superseded(cleanedAtUnixMs, supersededAtUnixMs):
            guard
                validUnixMilliseconds(cleanedAtUnixMs),
                validUnixMilliseconds(supersededAtUnixMs),
                supersededAtUnixMs >= cleanedAtUnixMs
            else {
                throw CleanupHistoryServiceError.invalidResponse
            }
            return .superseded(
                cleanedAt: date(cleanedAtUnixMs),
                supersededAt: date(supersededAtUnixMs)
            )
        case let .laterSizeObserved(cleanedAtUnixMs, observedAtUnixMs, observedBytes):
            guard
                validUnixMilliseconds(cleanedAtUnixMs),
                validUnixMilliseconds(observedAtUnixMs),
                observedAtUnixMs > cleanedAtUnixMs,
                observedBytes > 0
            else {
                throw CleanupHistoryServiceError.invalidResponse
            }
            return .laterSizeObserved(
                cleanedAt: date(cleanedAtUnixMs),
                observedAt: date(observedAtUnixMs),
                observedBytes: observedBytes
            )
        case let .zeroBaselineObserved(cleanedAtUnixMs, observedAtUnixMs):
            guard
                validUnixMilliseconds(cleanedAtUnixMs),
                validUnixMilliseconds(observedAtUnixMs),
                observedAtUnixMs > cleanedAtUnixMs
            else {
                throw CleanupHistoryServiceError.invalidResponse
            }
            return .zeroBaselineObserved(
                cleanedAt: date(cleanedAtUnixMs),
                observedAt: date(observedAtUnixMs)
            )
        case let .regrown(
            cleanedAtUnixMs,
            zeroObservedAtUnixMs,
            observedAtUnixMs,
            observedBytes
        ):
            guard
                validUnixMilliseconds(cleanedAtUnixMs),
                validUnixMilliseconds(zeroObservedAtUnixMs),
                validUnixMilliseconds(observedAtUnixMs),
                zeroObservedAtUnixMs > cleanedAtUnixMs,
                observedAtUnixMs > zeroObservedAtUnixMs,
                observedBytes > 0
            else {
                throw CleanupHistoryServiceError.invalidResponse
            }
            return .regrown(
                cleanedAt: date(cleanedAtUnixMs),
                zeroObservedAt: date(zeroObservedAtUnixMs),
                observedAt: date(observedAtUnixMs),
                observedBytes: observedBytes
            )
        }
    }

    private static func map(
        _ raw: RuleOutcomeNotEligibleReason
    ) -> CleanupHistoryRuleOutcomeNotEligibleReason {
        switch raw {
        case .sourceCleanupIncomplete: .sourceCleanupIncomplete
        case .itemNotSuccessfulPermanentRegenerable:
            .itemNotSuccessfulPermanentRegenerable
        case .sourceScanNotComparable: .sourceScanNotComparable
        case .sourceEvaluationNotComparable: .sourceEvaluationNotComparable
        case .sourceEvaluationAfterPlan: .sourceEvaluationAfterPlan
        case .sourceCandidateMismatch: .sourceCandidateMismatch
        }
    }

    private static func map(_ raw: CandidateCategory) -> ExplorerCandidateCategory {
        switch raw {
        case .developerArtifact: .developerArtifact
        case .applicationCache: .applicationCache
        case .browserCache: .browserCache
        case .logAndDiagnostic: .logAndDiagnostic
        case .installerAndDownload: .installerAndDownload
        case .deviceAndSimulatorData: .deviceAndSimulatorData
        case .cloudFile: .cloudFile
        case .largeReviewItem: .largeReviewItem
        case .protectedSystemData: .protectedSystemData
        case .unknownStorage: .unknownStorage
        }
    }

    private static func map(_ raw: CandidateSafety) -> ExplorerCandidateSafety {
        switch raw {
        case .safeRegenerable: .safeRegenerable
        case .safeEvictable: .safeEvictable
        case .reviewRequired: .reviewRequired
        case .informational: .informational
        case .protected: .protected
        }
    }

    private static func map(_ raw: CandidateAction) -> ExplorerCandidateAction {
        switch raw {
        case .removeKnownRegenerableContents: .removeKnownRegenerableContents
        case .evictLocalCopy: .evictLocalCopy
        case .moveToTrash: .moveToTrash
        case .revealOnly: .revealOnly
        case .noAction: .noAction
        }
    }

    private static func map(_ raw: CleanupStatusCounts) -> CleanupHistoryStatusCounts {
        CleanupHistoryStatusCounts(
            planned: raw.planned,
            validating: raw.validating,
            dryRun: raw.dryRun,
            effectStarted: raw.effectStarted,
            trashed: raw.trashed,
            removed: raw.removed,
            evicted: raw.evicted,
            skipped: raw.skipped,
            rejected: raw.rejected,
            failed: raw.failed,
            changedSincePlan: raw.changedSincePlan,
            interrupted: raw.interrupted,
            unavailable: raw.unavailable,
            outcomeUnknown: raw.outcomeUnknown,
            total: raw.total
        )
    }

    static func validStableToken(_ value: String) -> Bool {
        value.utf8.count > 0
            && value.utf8.count <= 128
            && value.unicodeScalars.allSatisfy { scalar in
                scalar.isASCII
                    && (CharacterSet.alphanumerics.contains(scalar)
                        || "._-:".unicodeScalars.contains(scalar))
            }
    }

    private static func validCounts(
        _ counts: CleanupStatusCounts,
        total: UInt16
    ) -> Bool {
        let sum = Int(counts.planned)
            + Int(counts.validating)
            + Int(counts.dryRun)
            + Int(counts.effectStarted)
            + Int(counts.trashed)
            + Int(counts.removed)
            + Int(counts.evicted)
            + Int(counts.skipped)
            + Int(counts.rejected)
            + Int(counts.failed)
            + Int(counts.changedSincePlan)
            + Int(counts.interrupted)
            + Int(counts.unavailable)
            + Int(counts.outcomeUnknown)
        return counts.total == total && sum == Int(total)
    }

    private static func validUnixMilliseconds(_ value: Int64) -> Bool {
        (0 ... maximumUnixMilliseconds).contains(value)
    }

    private static func date(_ unixMilliseconds: Int64) -> Date {
        Date(timeIntervalSince1970: Double(unixMilliseconds) / 1000)
    }

    private static func isTerminal(_ status: CleanupSessionStatus) -> Bool {
        switch status {
        case .planned, .running, .recovering: false
        case .completed, .partiallyCompleted, .failed, .cancelled, .interrupted, .rejected,
             .dryRun:
            true
        }
    }

    private static func expectedWarnings(
        format: CleanupRecordFormat,
        mode: CleanupMode,
        items: [CleanupItemSummary]
    ) -> [CleanupWarning] {
        guard format == .complete else {
            return []
        }
        var warnings: [CleanupWarning] = [.estimatedBytesUnverified]
        switch mode {
        case .dryRun:
            warnings.append(.dryRunDoesNotMutate)
        case .trash:
            warnings.append(.trashDoesNotFreeSpaceImmediately)
        case .permanentSafe:
            warnings.append(.permanentRemovalCannotBeUndone)
        case .evictLocalCopy:
            break
        }
        if items.contains(where: { $0.action == .evictLocalCopy }) {
            warnings.append(.cloudEvictionRequiresNetworkToRedownload)
        }
        return warnings
    }

    private struct MutableCleanupStatusCounts {
        var planned = 0
        var validating = 0
        var dryRun = 0
        var effectStarted = 0
        var trashed = 0
        var removed = 0
        var evicted = 0
        var skipped = 0
        var rejected = 0
        var failed = 0
        var changedSincePlan = 0
        var interrupted = 0
        var unavailable = 0
        var outcomeUnknown = 0

        mutating func add(_ status: CleanupItemStatus) {
            switch status {
            case .planned: planned += 1
            case .validating: validating += 1
            case .dryRun: dryRun += 1
            case .effectStarted: effectStarted += 1
            case .trashed: trashed += 1
            case .removed: removed += 1
            case .evicted: evicted += 1
            case .skipped: skipped += 1
            case .rejected: rejected += 1
            case .failed: failed += 1
            case .changedSincePlan: changedSincePlan += 1
            case .interrupted: interrupted += 1
            case .unavailable: unavailable += 1
            case .outcomeUnknown: outcomeUnknown += 1
            }
        }

        func matches(_ counts: CleanupStatusCounts) -> Bool {
            planned == Int(counts.planned)
                && validating == Int(counts.validating)
                && dryRun == Int(counts.dryRun)
                && effectStarted == Int(counts.effectStarted)
                && trashed == Int(counts.trashed)
                && removed == Int(counts.removed)
                && evicted == Int(counts.evicted)
                && skipped == Int(counts.skipped)
                && rejected == Int(counts.rejected)
                && failed == Int(counts.failed)
                && changedSincePlan == Int(counts.changedSincePlan)
                && interrupted == Int(counts.interrupted)
                && unavailable == Int(counts.unavailable)
                && outcomeUnknown == Int(counts.outcomeUnknown)
                && Int(counts.total) == planned + validating + dryRun + effectStarted + trashed
                + removed + evicted + skipped + rejected + failed + changedSincePlan
                + interrupted + unavailable + outcomeUnknown
        }
    }
}

/// The only conversion boundary between generated snapshot-node records and
/// app-owned Explorer navigation models.
enum ExplorerSnapshotNodeAdapter {
    private static let recordVersion: UInt32 = 1
    private static let nodeRecordVersion: UInt32 = 2
    private static let maximumPageLimit = 200
    static let maximumTreemapCells: UInt16 = 64

    static func mapRoot(_ raw: SnapshotNode) throws -> ExplorerSnapshotNode {
        let root = try mapNode(raw, maximumNameBytes: 65536)
        guard
            root.id == 0,
            root.parentID == nil,
            root.depth == 0,
            root.kind == .directory
        else {
            throw ExplorerSnapshotNodeError.invalidResponse
        }
        return root
    }

    static func mapPage(
        _ raw: SnapshotNodePage,
        expectedParentID: UInt64,
        expectedOffset: UInt64,
        requestedLimit: UInt16
    ) throws -> ExplorerSnapshotNodePage {
        let remainingChildren = raw.totalChildren >= raw.offset
            ? raw.totalChildren - raw.offset
            : 0
        let expectedNodeCount = min(UInt64(requestedLimit), remainingChildren)
        guard
            (1 ... maximumPageLimit).contains(Int(requestedLimit)),
            raw.recordVersion == recordVersion,
            raw.parentId == expectedParentID,
            raw.offset == expectedOffset,
            raw.offset <= raw.totalChildren,
            UInt64(raw.nodes.count) == expectedNodeCount,
            raw.hasMore == (raw.offset + UInt64(raw.nodes.count) < raw.totalChildren)
        else {
            throw ExplorerSnapshotNodeError.invalidResponse
        }
        let nodes = try raw.nodes.map { try mapNode($0, maximumNameBytes: 1024) }
        guard
            Set(nodes.map(\.id)).count == nodes.count,
            nodes.allSatisfy({ $0.parentID == expectedParentID && $0.depth > 0 })
        else {
            throw ExplorerSnapshotNodeError.invalidResponse
        }
        return ExplorerSnapshotNodePage(
            parentID: raw.parentId,
            offset: raw.offset,
            totalChildren: raw.totalChildren,
            hasMore: raw.hasMore,
            nodes: nodes
        )
    }

    static func mapTreemap(
        _ raw: SnapshotTreemap,
        expectedParentID: UInt64,
        requestedMaxCells: UInt16
    ) throws -> ExplorerSnapshotTreemap {
        guard
            (1 ... maximumTreemapCells).contains(requestedMaxCells),
            raw.recordVersion == recordVersion,
            raw.parentId == expectedParentID,
            raw.cells.count <= Int(requestedMaxCells),
            raw.totalChildren == UInt64(raw.cells.count) + raw.otherChildCount,
            raw.zeroLogicalChildCount <= raw.otherChildCount,
            raw.otherLogicalBytes > 0
            || raw.zeroLogicalChildCount == raw.otherChildCount,
            raw.otherChildCount > 0
            || (raw.otherLogicalBytes == 0 && raw.zeroLogicalChildCount == 0)
        else {
            throw ExplorerSnapshotTreemapError.invalidResponse
        }

        var representedLogicalBytes: UInt64 = 0
        var previousLogicalBytes = UInt64.max
        var cells: [ExplorerSnapshotTreemapCell] = []
        cells.reserveCapacity(raw.cells.count)
        for (index, rawCell) in raw.cells.enumerated() {
            let node = try mapNode(rawCell.node, maximumNameBytes: 1024)
            let (nextTotal, overflow) = representedLogicalBytes.addingReportingOverflow(
                node.logicalBytes
            )
            guard
                rawCell.recordVersion == recordVersion,
                rawCell.logicalRank == UInt64(index),
                node.parentID == expectedParentID,
                node.depth > 0,
                node.logicalBytes > 0,
                node.logicalBytes <= previousLogicalBytes,
                !overflow
            else {
                throw ExplorerSnapshotTreemapError.invalidResponse
            }
            representedLogicalBytes = nextTotal
            previousLogicalBytes = node.logicalBytes
            cells.append(ExplorerSnapshotTreemapCell(node: node, logicalRank: rawCell.logicalRank))
        }

        let (accountedLogicalBytes, overflow) = representedLogicalBytes.addingReportingOverflow(
            raw.otherLogicalBytes
        )
        guard
            !overflow,
            accountedLogicalBytes == raw.totalChildLogicalBytes,
            Set(cells.map(\.id)).count == cells.count,
            raw.totalChildLogicalBytes > 0 || cells.isEmpty,
            raw.totalChildLogicalBytes > 0 || raw.zeroLogicalChildCount == raw.totalChildren
        else {
            throw ExplorerSnapshotTreemapError.invalidResponse
        }

        return ExplorerSnapshotTreemap(
            parentID: raw.parentId,
            totalChildren: raw.totalChildren,
            totalChildLogicalBytes: raw.totalChildLogicalBytes,
            otherChildCount: raw.otherChildCount,
            otherLogicalBytes: raw.otherLogicalBytes,
            zeroLogicalChildCount: raw.zeroLogicalChildCount,
            cells: cells
        )
    }

    static func ffiSort(_ sort: ExplorerSnapshotNodeSort) -> SnapshotNodeSort {
        switch sort {
        case .nameAscending: .nameAscending
        case .logicalBytesDescending: .logicalBytesDescending
        case .allocatedBytesDescending: .allocatedBytesDescending
        case .modifiedNewest: .modifiedNewest
        }
    }

    static func mapNode(
        _ raw: SnapshotNode,
        maximumNameBytes: Int
    ) throws -> ExplorerSnapshotNode {
        guard
            raw.recordVersion == nodeRecordVersion,
            !raw.name.encodedBytes.isEmpty,
            raw.name.encodedBytes.count <= maximumNameBytes,
            raw.modifiedAt.map(validTimestamp) ?? true,
            raw.accessedAt.map(validTimestamp) ?? true,
            raw.kind == .directory || raw.childCount == 0
        else {
            throw ExplorerSnapshotNodeError.invalidResponse
        }
        let name = try mapName(raw.name, maximumNameBytes: maximumNameBytes)
        return ExplorerSnapshotNode(
            id: raw.id,
            parentID: raw.parentId,
            depth: raw.depth,
            kind: mapKind(raw.kind),
            category: mapCategory(raw.category),
            name: name,
            logicalBytes: raw.logicalBytes,
            allocatedBytes: raw.allocatedBytes,
            fileCount: raw.fileCount,
            childCount: raw.childCount,
            modifiedAt: raw.modifiedAt.map(mapTimestamp),
            accessedAt: raw.accessedAt.map(mapTimestamp),
            scanFlags: ExplorerSnapshotScanFlags(
                inaccessible: raw.scanFlags.inaccessible,
                timedOut: raw.scanFlags.timedOut,
                hardLinkDuplicate: raw.scanFlags.hardLinkDuplicate,
                mountBoundary: raw.scanFlags.mountBoundary
            )
        )
    }

    static func mapName(
        _ raw: SnapshotNodeName,
        maximumNameBytes: Int
    ) throws -> ExplorerSnapshotNodeName {
        guard
            !raw.encodedBytes.isEmpty,
            raw.encodedBytes.count <= maximumNameBytes
        else {
            throw ExplorerSnapshotNodeError.invalidResponse
        }
        let (encoding, decoded) = try decodeName(raw)
        guard decoded == raw.display else {
            throw ExplorerSnapshotNodeError.invalidResponse
        }
        return ExplorerSnapshotNodeName(
            encoding: encoding,
            encodedBytes: raw.encodedBytes,
            display: raw.display
        )
    }

    private static func decodeName(
        _ raw: SnapshotNodeName
    ) throws -> (ExplorerSnapshotNameEncoding, String) {
        switch raw.encoding {
        case .unixBytes:
            guard !raw.encodedBytes.contains(0) else {
                throw ExplorerSnapshotNodeError.invalidResponse
            }
            return (.unixBytes, String(decoding: raw.encodedBytes, as: UTF8.self))
        case .windowsUtf16LittleEndian:
            guard
                raw.encodedBytes.count.isMultiple(of: 2),
                !raw.encodedBytes.isEmpty
            else {
                throw ExplorerSnapshotNodeError.invalidResponse
            }
            let bytes = [UInt8](raw.encodedBytes)
            let units = stride(from: 0, to: bytes.count, by: 2).map {
                UInt16(bytes[$0]) | (UInt16(bytes[$0 + 1]) << 8)
            }
            guard !units.contains(0) else {
                throw ExplorerSnapshotNodeError.invalidResponse
            }
            return (.windowsUTF16LittleEndian, String(decoding: units, as: UTF16.self))
        }
    }

    static func mapKind(_ kind: SnapshotNodeKind) -> ExplorerSnapshotNodeKind {
        switch kind {
        case .directory: .directory
        case .file: .file
        case .symlink: .symlink
        case .other: .other
        case .error: .error
        }
    }

    static func mapCategory(
        _ category: SnapshotStorageCategory
    ) -> ExplorerStorageCategory {
        switch category {
        case .unclassified: .unclassified
        case .developerArtifact: .developerArtifact
        case .applicationCache: .applicationCache
        case .browserCache: .browserCache
        case .logAndDiagnostic: .logAndDiagnostic
        case .installerAndDownload: .installerAndDownload
        case .deviceAndSimulatorData: .deviceAndSimulatorData
        case .cloudFile: .cloudFile
        case .largeReviewItem: .largeReviewItem
        case .protectedSystemData: .protectedSystemData
        case .unknownStorage: .unknownStorage
        }
    }

    private static func validTimestamp(_ timestamp: SnapshotNodeTimestamp) -> Bool {
        timestamp.nanoseconds < 1_000_000_000
    }

    static func mapTimestamp(
        _ timestamp: SnapshotNodeTimestamp
    ) -> ExplorerSnapshotTimestamp {
        ExplorerSnapshotTimestamp(
            secondsSinceUnixEpoch: timestamp.secondsSinceUnixEpoch,
            nanoseconds: timestamp.nanoseconds
        )
    }
}

/// Strict conversion boundary for the path-free, bounded Large Files
/// projection. Invalid or internally inconsistent records are never rendered.
enum ExplorerSnapshotLargeFilesAdapter {
    static let maximumResults: UInt16 = 200
    private static let recordVersion: UInt32 = 1
    private static let maximumContextComponents = 8

    static func request(
        minimumLogicalBytes: UInt64,
        modifiedBefore: ExplorerSnapshotTimestamp?,
        maxResults: UInt16
    ) throws -> SnapshotLargeFileRequest {
        guard
            minimumLogicalBytes > 0,
            (1 ... maximumResults).contains(maxResults),
            modifiedBefore.map({ $0.nanoseconds < 1_000_000_000 }) ?? true
        else {
            throw ExplorerSnapshotLargeFilesError.invalidRequest
        }
        return SnapshotLargeFileRequest(
            recordVersion: recordVersion,
            minimumLogicalBytes: minimumLogicalBytes,
            modifiedBefore: modifiedBefore.map {
                SnapshotNodeTimestamp(
                    secondsSinceUnixEpoch: $0.secondsSinceUnixEpoch,
                    nanoseconds: $0.nanoseconds
                )
            },
            maxResults: maxResults
        )
    }

    static func map(
        _ raw: SnapshotLargeFilePage,
        minimumLogicalBytes: UInt64,
        modifiedBefore: ExplorerSnapshotTimestamp?,
        requestedMaxResults: UInt16
    ) throws -> ExplorerSnapshotLargeFilesPage {
        guard
            minimumLogicalBytes > 0,
            (1 ... maximumResults).contains(requestedMaxResults),
            raw.recordVersion == recordVersion,
            raw.files.count <= Int(requestedMaxResults),
            raw.totalMatchingFiles >= UInt64(raw.files.count),
            UInt64(raw.files.count)
            == min(UInt64(requestedMaxResults), raw.totalMatchingFiles),
            raw.hasMore == (raw.totalMatchingFiles > UInt64(raw.files.count))
        else {
            throw ExplorerSnapshotLargeFilesError.invalidResponse
        }

        var files: [ExplorerSnapshotLargeFile] = []
        files.reserveCapacity(raw.files.count)
        var returnedLogicalBytes: UInt64 = 0
        for rawFile in raw.files {
            let node = try ExplorerSnapshotNodeAdapter.mapNode(
                rawFile.node,
                maximumNameBytes: 1024
            )
            let parentContext = try rawFile.parentContext.map {
                try ExplorerSnapshotNodeAdapter.mapName($0, maximumNameBytes: 1024)
            }
            let parentDepth = Int(node.depth) - 1
            let timestampMatches = switch (modifiedBefore, node.modifiedAt) {
            case (nil, _): true
            case let (cutoff?, observed?): timestamp(observed, precedes: cutoff)
            case (_?, nil): false
            }
            let contextMatches = rawFile.contextTruncated
                ? parentContext.count == maximumContextComponents
                && parentDepth > maximumContextComponents
                : parentContext.count == parentDepth
            let (nextTotal, overflow) = returnedLogicalBytes.addingReportingOverflow(
                node.logicalBytes
            )
            guard
                rawFile.recordVersion == recordVersion,
                node.kind == .file,
                node.parentID != nil,
                node.depth > 0,
                node.fileCount == 1,
                node.childCount == 0,
                node.logicalBytes >= minimumLogicalBytes,
                timestampMatches,
                parentContext.count <= maximumContextComponents,
                contextMatches,
                !overflow
            else {
                throw ExplorerSnapshotLargeFilesError.invalidResponse
            }
            returnedLogicalBytes = nextTotal
            files.append(
                ExplorerSnapshotLargeFile(
                    node: node,
                    parentContext: parentContext,
                    contextTruncated: rawFile.contextTruncated
                )
            )
        }

        guard
            Set(files.map(\.id)).count == files.count,
            zip(files, files.dropFirst()).allSatisfy(orderedBefore),
            raw.totalMatchingLogicalBytes >= returnedLogicalBytes,
            raw.hasMore
            || (raw.totalMatchingFiles == UInt64(files.count)
                && raw.totalMatchingLogicalBytes == returnedLogicalBytes)
        else {
            throw ExplorerSnapshotLargeFilesError.invalidResponse
        }

        return ExplorerSnapshotLargeFilesPage(
            minimumLogicalBytes: minimumLogicalBytes,
            modifiedBefore: modifiedBefore,
            totalMatchingFiles: raw.totalMatchingFiles,
            totalMatchingLogicalBytes: raw.totalMatchingLogicalBytes,
            hasMore: raw.hasMore,
            files: files
        )
    }

    private static func orderedBefore(
        _ pair: (ExplorerSnapshotLargeFile, ExplorerSnapshotLargeFile)
    ) -> Bool {
        let left = pair.0.node
        let right = pair.1.node
        if left.logicalBytes != right.logicalBytes {
            return left.logicalBytes > right.logicalBytes
        }
        let leftEncoding = encodingKey(left.name.encoding)
        let rightEncoding = encodingKey(right.name.encoding)
        if leftEncoding != rightEncoding {
            return leftEncoding < rightEncoding
        }
        if left.name.encodedBytes != right.name.encodedBytes {
            return left.name.encodedBytes.lexicographicallyPrecedes(right.name.encodedBytes)
        }
        return left.id < right.id
    }

    private static func encodingKey(_ encoding: ExplorerSnapshotNameEncoding) -> UInt8 {
        switch encoding {
        case .unixBytes: 0
        case .windowsUTF16LittleEndian: 1
        }
    }

    private static func timestamp(
        _ observed: ExplorerSnapshotTimestamp,
        precedes cutoff: ExplorerSnapshotTimestamp
    ) -> Bool {
        observed.secondsSinceUnixEpoch < cutoff.secondsSinceUnixEpoch
            || (observed.secondsSinceUnixEpoch == cutoff.secondsSinceUnixEpoch
                && observed.nanoseconds < cutoff.nanoseconds)
    }
}

/// Strict conversion boundary for an ephemeral, identity-checked current path.
/// Historical display names never enter this adapter as path input.
enum ExplorerSnapshotLivePathAdapter {
    private static let recordVersion: UInt32 = 1
    private static let maximumPathBytes = 32 * 1024

    static func request(
        nodeID: UInt64,
        purpose: ExplorerSnapshotLivePathPurpose
    ) -> SnapshotLiveTargetRequest {
        SnapshotLiveTargetRequest(
            recordVersion: recordVersion,
            nodeId: nodeID,
            purpose: ffiPurpose(purpose)
        )
    }

    static func map(
        _ raw: SnapshotLiveTarget,
        requestedNodeID: UInt64,
        requestedPurpose: ExplorerSnapshotLivePathPurpose
    ) throws -> ExplorerResolvedLiveItem {
        let expectedPurpose = ffiPurpose(requestedPurpose)
        let kind: ExplorerSnapshotNodeKind = switch raw.kind {
        case .directory: .directory
        case .file: .file
        }
        let bytes = [UInt8](raw.absolutePathBytes)
        guard
            raw.recordVersion == recordVersion,
            raw.nodeId == requestedNodeID,
            raw.purpose == expectedPurpose,
            raw.pathEncoding == .unixBytes,
            !bytes.isEmpty,
            bytes.count <= maximumPathBytes,
            bytes.first == UInt8(ascii: "/"),
            !bytes.contains(0),
            !bytes.contains(where: { $0 < 0x20 || $0 == 0x7F }),
            validAbsoluteUnixPath(bytes),
            raw.displayPath == String(decoding: bytes, as: UTF8.self),
            requestedPurpose != .quickLook || kind == .file
        else {
            throw ExplorerSnapshotLivePathError.invalidResponse
        }

        let exactText = String(data: Data(bytes), encoding: .utf8)
        guard raw.exactTextPath == exactText else {
            throw ExplorerSnapshotLivePathError.invalidResponse
        }
        guard let exactText else {
            // Swift Foundation's value-type URL rewrites invalid UTF-8 bytes.
            // Refuse a different current path rather than presenting it.
            throw ExplorerSnapshotLivePathError.unsupportedItem
        }
        guard !exactText.unicodeScalars.contains(where: {
            $0.properties.generalCategory == .control
        }) else {
            throw ExplorerSnapshotLivePathError.invalidResponse
        }
        let url = try url(
            fromValidatedUnixPathBytes: bytes,
            isDirectory: kind == .directory
        )
        return ExplorerResolvedLiveItem(
            nodeID: raw.nodeId,
            kind: kind,
            url: url,
            exactTextPath: exactText
        )
    }

    /// Convert one core-issued, current Unix path to a Foundation URL without
    /// allowing Foundation to normalize or substitute bytes. This is shared by
    /// read-only presentation and the future one-shot Trash callback.
    static func url(
        fromValidatedUnixPathBytes bytes: [UInt8],
        isDirectory: Bool
    ) throws -> URL {
        guard
            !bytes.isEmpty,
            bytes.count <= maximumPathBytes,
            bytes.first == UInt8(ascii: "/"),
            !bytes.contains(0),
            !bytes.contains(where: { $0 < 0x20 || $0 == 0x7F }),
            validAbsoluteUnixPath(bytes),
            let exactText = String(data: Data(bytes), encoding: .utf8),
            !exactText.unicodeScalars.contains(where: {
                $0.properties.generalCategory == .control
            })
        else {
            throw ExplorerSnapshotLivePathError.invalidResponse
        }
        var terminated = bytes
        terminated.append(0)
        let url = terminated.withUnsafeBufferPointer { buffer in
            buffer.baseAddress!.withMemoryRebound(to: CChar.self, capacity: buffer.count) {
                URL(
                    fileURLWithFileSystemRepresentation: $0,
                    isDirectory: isDirectory,
                    relativeTo: nil
                )
            }
        }
        let roundTrippedBytes = url.withUnsafeFileSystemRepresentation { pointer -> Data? in
            guard let pointer else {
                return nil
            }
            return Data(bytes: pointer, count: strlen(pointer))
        }
        guard url.isFileURL, roundTrippedBytes == Data(bytes) else {
            throw ExplorerSnapshotLivePathError.unsupportedItem
        }
        return url
    }

    private static func validAbsoluteUnixPath(_ bytes: [UInt8]) -> Bool {
        guard bytes.first == UInt8(ascii: "/") else {
            return false
        }
        if bytes.count == 1 {
            return true
        }
        guard bytes.last != UInt8(ascii: "/") else {
            return false
        }
        return bytes.dropFirst().split(separator: UInt8(ascii: "/"), omittingEmptySubsequences: false)
            .allSatisfy { component in
                !component.isEmpty
                    && component != [UInt8(ascii: ".")]
                    && component != [UInt8(ascii: "."), UInt8(ascii: ".")]
            }
    }

    private static func ffiPurpose(
        _ purpose: ExplorerSnapshotLivePathPurpose
    ) -> SnapshotLiveTargetPurpose {
        switch purpose {
        case .reveal: .reveal
        case .copyPath: .copyPath
        case .quickLook: .quickLook
        }
    }
}

/// Strict boundary for bounded, root-relative historical scan issues. Values
/// remain display observations and cannot be used as filesystem paths.
enum ExplorerScanCoverageDetailsAdapter {
    static let maximumPageLimit: UInt16 = 64
    private static let recordVersion: UInt32 = 1
    private static let maximumIssueRecords: UInt16 = 256
    private static let maximumLocationComponents = 8
    private static let maximumLocationComponentCharacters = 128

    static func map(
        _ raw: ScanCoverageDetailsPage,
        requestedScanID: String,
        requestedOffset: UInt16,
        requestedLimit: UInt16
    ) throws -> ExplorerScanCoverageDetailsPage {
        guard
            ExplorerSnapshotHistoryAdapter.validScanID(requestedScanID),
            (1 ... maximumPageLimit).contains(requestedLimit),
            raw.recordVersion == recordVersion,
            raw.scanId == requestedScanID,
            raw.offset == requestedOffset,
            raw.totalIssueRecords <= maximumIssueRecords,
            raw.coverage.recordVersion == recordVersion,
            raw.coverage.issueRecordCount == UInt64(raw.totalIssueRecords),
            raw.coverage.issueOccurrenceCount == raw.totalIssueOccurrences,
            raw.coverage.measuredPermille.map({ $0 <= 1000 }) ?? true,
            Int(raw.offset) <= Int(raw.totalIssueRecords),
            raw.issues.count
            == min(Int(requestedLimit), Int(raw.totalIssueRecords) - Int(raw.offset)),
            raw.hasMore == (Int(raw.offset) + raw.issues.count < Int(raw.totalIssueRecords))
        else {
            throw ExplorerScanCoverageError.invalidResponse
        }

        var pageOccurrences: UInt64 = 0
        let issues = try raw.issues.enumerated().map { index, issue in
            let expectedOrdinal = Int(raw.offset) + index
            let (nextOccurrences, overflow) = pageOccurrences.addingReportingOverflow(
                UInt64(issue.occurrenceCount)
            )
            guard
                issue.recordVersion == recordVersion,
                issue.ordinal == UInt16(expectedOrdinal),
                issue.occurrenceCount > 0,
                !overflow,
                nextOccurrences <= raw.totalIssueOccurrences,
                issue.locationComponents.count <= maximumLocationComponents,
                issue.locationComponents.allSatisfy(validLocationComponent)
            else {
                throw ExplorerScanCoverageError.invalidResponse
            }
            pageOccurrences = nextOccurrences
            let kind = mapKind(issue.kind)
            let locationScope: ExplorerScanCoverageLocationScope
            switch issue.locationScope {
            case .global:
                guard
                    permitsGlobalScope(kind),
                    issue.locationComponents.isEmpty,
                    !issue.locationTruncated
                else {
                    throw ExplorerScanCoverageError.invalidResponse
                }
                locationScope = .global
            case .scanRoot:
                guard issue.locationComponents.isEmpty, !issue.locationTruncated else {
                    throw ExplorerScanCoverageError.invalidResponse
                }
                locationScope = .scanRoot
            case .descendant:
                guard
                    !issue.locationComponents.isEmpty
                else {
                    throw ExplorerScanCoverageError.invalidResponse
                }
                locationScope = .descendant
            }
            return ExplorerScanCoverageIssue(
                ordinal: issue.ordinal,
                kind: kind,
                occurrenceCount: issue.occurrenceCount,
                locationScope: locationScope,
                locationComponents: issue.locationComponents,
                locationTruncated: issue.locationTruncated
            )
        }
        let coverage: AppScanCoverage = switch raw.coverage.status {
        case .unknown: .unknown
        case .complete: .complete
        case .limitedAccess: .limitedAccess
        case .partial: .partial
        }
        return ExplorerScanCoverageDetailsPage(
            scanID: raw.scanId,
            coverage: coverage,
            measuredPermille: raw.coverage.measuredPermille,
            offset: raw.offset,
            totalIssueRecords: raw.totalIssueRecords,
            totalIssueOccurrences: raw.totalIssueOccurrences,
            hasMore: raw.hasMore,
            issues: issues
        )
    }

    static func assemble(
        _ pages: [ExplorerScanCoverageDetailsPage],
        requestedScanID: String
    ) throws -> ExplorerScanCoverageDetails {
        guard let first = pages.first else {
            throw ExplorerScanCoverageError.invalidResponse
        }
        var issues: [ExplorerScanCoverageIssue] = []
        var occurrences: UInt64 = 0
        for (index, page) in pages.enumerated() {
            guard
                page.scanID == requestedScanID,
                page.coverage == first.coverage,
                page.measuredPermille == first.measuredPermille,
                page.totalIssueRecords == first.totalIssueRecords,
                page.totalIssueOccurrences == first.totalIssueOccurrences,
                page.offset == UInt16(issues.count),
                page.hasMore == (index < pages.count - 1)
            else {
                throw ExplorerScanCoverageError.invalidResponse
            }
            for issue in page.issues {
                let (next, overflow) = occurrences.addingReportingOverflow(
                    UInt64(issue.occurrenceCount)
                )
                guard !overflow else {
                    throw ExplorerScanCoverageError.invalidResponse
                }
                occurrences = next
            }
            issues.append(contentsOf: page.issues)
        }
        guard
            issues.count == Int(first.totalIssueRecords),
            occurrences == first.totalIssueOccurrences,
            Set(issues.map(\.ordinal)).count == issues.count,
            canonicalKindOrder(issues),
            validCoverage(
                first.coverage,
                measuredPermille: first.measuredPermille,
                issues: issues
            )
        else {
            throw ExplorerScanCoverageError.invalidResponse
        }
        return ExplorerScanCoverageDetails(
            scanID: requestedScanID,
            coverage: first.coverage,
            measuredPermille: first.measuredPermille,
            totalIssueRecords: first.totalIssueRecords,
            totalIssueOccurrences: first.totalIssueOccurrences,
            issues: issues
        )
    }

    private static func validCoverage(
        _ coverage: AppScanCoverage,
        measuredPermille: UInt16?,
        issues: [ExplorerScanCoverageIssue]
    ) -> Bool {
        switch coverage {
        case .unknown:
            measuredPermille == nil && issues.isEmpty
        case .complete:
            measuredPermille == 1000 && issues.isEmpty
        case .limitedAccess:
            measuredPermille != 1000 && !issues.isEmpty
                && issues.allSatisfy { $0.kind == .permissionDenied }
        case .partial:
            measuredPermille != 1000 && !issues.isEmpty
                && issues.contains { $0.kind != .permissionDenied }
        }
    }

    private static func mapKind(
        _ kind: HistoricalScanIssueKind
    ) -> ExplorerScanCoverageIssueKind {
        switch kind {
        case .permissionDenied: .permissionDenied
        case .timedOut: .timedOut
        case .differentFilesystem: .differentFilesystem
        case .networkOrVirtualFilesystem: .networkOrVirtualFilesystem
        case .symlinkSkipped: .symlinkSkipped
        case .fileChangedDuringScan: .fileChangedDuringScan
        case .metadataError: .metadataError
        case .cancelled: .cancelled
        case .policyExcluded: .policyExcluded
        case .depthLimited: .depthLimited
        case .probePoolExhausted: .probePoolExhausted
        case .filesystemBoundaryUnknown: .filesystemBoundaryUnknown
        case .issueLimitReached: .issueLimitReached
        }
    }

    private static func kindRank(_ kind: ExplorerScanCoverageIssueKind) -> UInt8 {
        switch kind {
        case .permissionDenied: 0
        case .timedOut: 1
        case .differentFilesystem: 2
        case .networkOrVirtualFilesystem: 3
        case .symlinkSkipped: 4
        case .fileChangedDuringScan: 5
        case .metadataError: 6
        case .cancelled: 7
        case .policyExcluded: 8
        case .depthLimited: 9
        case .probePoolExhausted: 10
        case .filesystemBoundaryUnknown: 11
        case .issueLimitReached: 12
        }
    }

    private static func canonicalKindOrder(_ issues: [ExplorerScanCoverageIssue]) -> Bool {
        zip(issues, issues.dropFirst()).allSatisfy { pair in
            kindRank(pair.0.kind) <= kindRank(pair.1.kind)
        }
    }

    private static func permitsGlobalScope(_ kind: ExplorerScanCoverageIssueKind) -> Bool {
        switch kind {
        case .cancelled, .probePoolExhausted, .issueLimitReached:
            true
        default:
            false
        }
    }

    private static func validLocationComponent(_ component: String) -> Bool {
        !component.isEmpty
            && component.unicodeScalars.count <= maximumLocationComponentCharacters
            && component != "."
            && component != ".."
            && !component.contains("/")
            && !component.unicodeScalars.contains(where: { $0.value == 0 })
    }
}
