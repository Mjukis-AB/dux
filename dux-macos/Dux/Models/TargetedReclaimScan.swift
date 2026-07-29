import Foundation

/// Pressure evidence accepted by Rust for one focused discovery batch.
///
/// This is an observation only. It cannot select a cleanup mode, approve a
/// plan, or authorize a filesystem effect.
enum TargetedReclaimPressure: Equatable, Sendable {
    case warning
    case critical
}

/// The durable facts that make repeated capacity samples and process restarts
/// converge on the same focused discovery batch.
struct TargetedReclaimScanContext: Equatable, Sendable {
    let stableVolumeID: String
    /// Exact accepted capacity observation used for path-free final
    /// revalidation. It is intentionally not part of durable scan reuse
    /// identity, but prevents proof collisions between later samples.
    let capacityAnchorAt: Date
    let pressure: TargetedReclaimPressure
    /// Entry time for the current Warning or Critical episode. Escalating
    /// from Warning to Critical creates a new identity and forces independent
    /// durable revalidation.
    let pressureEpisodeStartedAt: Date
    /// Start of the contiguous low-space sequence, retained for truthful
    /// presentation but never used to reuse an older pressure level's scan.
    let lowPressureSequenceStartedAt: Date
    let policyRevision: UInt64
    let rootsRevision: UInt64
    let knownRootsPolicyRevision: UInt32
    let knownUserLibraryCachesIncluded: Bool
    let rootCatalogDigestSHA256: Data
    let rootCount: UInt16

    var identity: TargetedReclaimScanIdentity {
        TargetedReclaimScanIdentity(
            stableVolumeID: stableVolumeID,
            pressure: pressure,
            pressureEpisodeStartedAt: pressureEpisodeStartedAt,
            policyRevision: policyRevision,
            rootsRevision: rootsRevision,
            knownRootsPolicyRevision: knownRootsPolicyRevision,
            knownUserLibraryCachesIncluded: knownUserLibraryCachesIncluded,
            rootCatalogDigestSHA256: rootCatalogDigestSHA256,
            rootCount: rootCount
        )
    }
}

struct TargetedReclaimScanIdentity: Equatable, Sendable {
    let stableVolumeID: String
    let pressure: TargetedReclaimPressure
    let pressureEpisodeStartedAt: Date
    let policyRevision: UInt64
    let rootsRevision: UInt64
    let knownRootsPolicyRevision: UInt32
    let knownUserLibraryCachesIncluded: Bool
    let rootCatalogDigestSHA256: Data
    let rootCount: UInt16
}

enum TargetedReclaimScanRootKind: Equatable, Sendable {
    case knownUserLibraryCaches
    case configuredProject
}

struct TargetedReclaimScanRoot: Equatable, Sendable, Identifiable {
    var id: UInt16 { ordinal }

    let ordinal: UInt16
    let kind: TargetedReclaimScanRootKind
    let path: ProjectDiscoveryRoot

    var displayName: String {
        switch kind {
        case .knownUserLibraryCaches:
            "User caches"
        case .configuredProject:
            "Project folder"
        }
    }
}

enum TargetedReclaimScanResultSource: Equatable, Sendable {
    /// Rust reused a complete durable scan from this continuous low-space
    /// period instead of repeating filesystem traversal.
    case currentDurable
    /// The focused runner observed this scan through its terminal task.
    case focusedRun
    /// The focused runner joined an already-active exact-root scan.
    case joinedActive
}

struct TargetedReclaimRootResult: Equatable, Sendable, Identifiable {
    var id: UInt16 { ordinal }

    let ordinal: UInt16
    let root: TargetedReclaimScanRoot
    let source: TargetedReclaimScanResultSource
    let result: HomeScanTaskResult
}

/// Core-owned §13.3 recovery order. Raw values are presentation-independent
/// policy positions used only to validate that Swift did not receive a
/// reordered projection.
enum AppEmergencyRecoveryLane: UInt16, Equatable, Hashable, Sendable {
    case evictableCloud = 1
    case staleSafeRegenerable = 2
    case trashInformation = 3
    case reviewableInstallerArchive = 4
    case largeFile = 5
    case guidedExploration = 6
    case permissionGap = 7
}

/// One exact, path-free observation behind a recovery card. A source can open
/// an existing read-only review; it cannot select a candidate or authorize an
/// effect.
struct AppEmergencyRecoverySource: Equatable, Sendable {
    let rootOrdinal: UInt16
    let scanID: String
    let observedAt: Date
    let candidateCount: UInt32?
    let blockedCandidateCount: UInt32?
    let permissionIssueCount: UInt64?
}

/// One core-ranked observation group. Optional fields are validated against
/// the lane before this reaches render state.
struct AppEmergencyRecoveryGroup: Equatable, Sendable, Identifiable {
    var id: String {
        [
            String(rank),
            String(lane.rawValue),
            ruleID ?? "-",
            ruleRevision.map(String.init) ?? "-",
        ].joined(separator: ":")
    }

    let rank: UInt16
    let lane: AppEmergencyRecoveryLane
    let ruleID: String?
    let ruleRevision: UInt32?
    let category: ExplorerCandidateCategory?
    let unavailableRootCount: UInt16
    let sources: [AppEmergencyRecoverySource]

    var candidateCount: UInt64? {
        guard lane == .staleSafeRegenerable else {
            return nil
        }
        return sources.reduce(into: 0) { total, source in
            total = total.saturatingAdding(UInt64(source.candidateCount ?? 0))
        }
    }

    var blockedCandidateCount: UInt64? {
        guard lane == .staleSafeRegenerable else {
            return nil
        }
        return sources.reduce(into: 0) { total, source in
            total = total.saturatingAdding(UInt64(source.blockedCandidateCount ?? 0))
        }
    }

    var permissionIssueCount: UInt64? {
        guard lane == .permissionGap else {
            return nil
        }
        return sources.reduce(into: 0) { total, source in
            total = total.saturatingAdding(source.permissionIssueCount ?? 0)
        }
    }

    var newestObservationAt: Date? {
        sources.map(\.observedAt).max()
    }
}

private struct AppEmergencyRecoveryGroupIdentity: Hashable {
    let lane: AppEmergencyRecoveryLane
    let ruleID: String?
    let ruleRevision: UInt32?
    let category: ExplorerCandidateCategory?
}

/// Immutable observation-only result of atomically finalizing one exact
/// Critical targeted pass. It contains no paths, candidate IDs, byte forecast,
/// plan, approval, schedule, or executor capability.
struct AppEmergencyRecoveryOrdering: Equatable, Sendable {
    static let supportedPolicyRevision: UInt32 = 1
    static let maximumGroupCount = 64

    let policyRevision: UInt32
    let context: TargetedReclaimScanContext
    let observedRootCount: UInt16
    let candidateEvaluatedRootCount: UInt16
    let unavailableRootCount: UInt16
    let groups: [AppEmergencyRecoveryGroup]

    var hasValidPresentationShape: Bool {
        let permissionGroupCount = groups.filter({
            $0.lane == .permissionGap
        }).count
        guard
            policyRevision == Self.supportedPolicyRevision,
            context.pressure == .critical,
            UInt32(observedRootCount) + UInt32(unavailableRootCount)
                == UInt32(context.rootCount),
            candidateEvaluatedRootCount <= observedRootCount,
            groups.count <= Self.maximumGroupCount,
            groups.enumerated().allSatisfy({ index, group in
                group.rank == UInt16(index)
            }),
            zip(groups, groups.dropFirst()).allSatisfy({ pair in
                pair.0.lane.rawValue <= pair.1.lane.rawValue
            }),
            (
                unavailableRootCount > 0
                    ? permissionGroupCount == 1
                    : permissionGroupCount <= 1
            )
        else {
            return false
        }

        var identities = Set<AppEmergencyRecoveryGroupIdentity>()
        for group in groups {
            let identity = AppEmergencyRecoveryGroupIdentity(
                lane: group.lane,
                ruleID: group.ruleID,
                ruleRevision: group.ruleRevision,
                category: group.category
            )
            guard
                group.sources.count <= Int(context.rootCount),
                identities.insert(identity).inserted,
                validSources(group.sources)
            else {
                return false
            }
            switch group.lane {
            case .staleSafeRegenerable:
                guard
                    let ruleID = group.ruleID,
                    !ruleID.isEmpty,
                    ruleID.utf8.count <= 256,
                    group.ruleRevision.map({ $0 > 0 }) == true,
                    group.category != nil,
                    group.unavailableRootCount == 0,
                    !group.sources.isEmpty,
                    group.sources.allSatisfy({
                        guard
                            let candidates = $0.candidateCount,
                            let blocked = $0.blockedCandidateCount
                        else {
                            return false
                        }
                        return candidates > 0
                            && blocked <= candidates
                            && $0.permissionIssueCount == nil
                    })
                else {
                    return false
                }
            case .guidedExploration:
                guard
                    group.ruleID == nil,
                    group.ruleRevision == nil,
                    group.category == nil,
                    group.unavailableRootCount == 0,
                    !group.sources.isEmpty,
                    group.sources.allSatisfy({
                        $0.candidateCount == nil
                            && $0.blockedCandidateCount == nil
                            && $0.permissionIssueCount == nil
                    })
                else {
                    return false
                }
            case .permissionGap:
                guard
                    group.ruleID == nil,
                    group.ruleRevision == nil,
                    group.category == nil,
                    group.unavailableRootCount == unavailableRootCount,
                    !group.sources.isEmpty || group.unavailableRootCount > 0,
                    group.sources.allSatisfy({
                        $0.candidateCount == nil
                            && $0.blockedCandidateCount == nil
                            && ($0.permissionIssueCount ?? 0) > 0
                    })
                else {
                    return false
                }
            case .evictableCloud, .trashInformation, .reviewableInstallerArchive, .largeFile:
                // Policy revision 1 has no authoritative source for these
                // lanes. A future source must increment the policy revision.
                return false
            }
        }
        return true
    }

    private func validSources(
        _ sources: [AppEmergencyRecoverySource]
    ) -> Bool {
        var identities = Set<String>()
        var previous: AppEmergencyRecoverySource?
        let latestPlausibleObservation = Date().addingTimeInterval(5 * 60)
        for source in sources {
            guard
                source.rootOrdinal < context.rootCount,
                !source.scanID.isEmpty,
                source.scanID.utf8.count <= 4_096,
                !source.scanID.unicodeScalars.contains(where: {
                    CharacterSet.controlCharacters.contains($0)
                }),
                source.observedAt >= context.pressureEpisodeStartedAt,
                source.observedAt <= latestPlausibleObservation,
                identities.insert("\(source.rootOrdinal):\(source.scanID)").inserted
            else {
                return false
            }
            if let previous {
                guard
                    previous.rootOrdinal < source.rootOrdinal
                    || (
                        previous.rootOrdinal == source.rootOrdinal
                            && previous.scanID < source.scanID
                    )
                else {
                    return false
                }
            }
            previous = source
        }
        return true
    }
}

enum TargetedReclaimRootFailure: Equatable, Sendable {
    case invalidRoot
    case rootMissing
    case accessDenied
    case rootChanged
    case differentVolume
    case volumeUnproven
    case busy
    case queueFull
    case budgetExceeded
    case unattempted
    case storageUnavailable
    case cancelled
    case scan(HomeScanTaskFailure)
    case invalidResponse
    case unexpected
}

struct TargetedReclaimFailedRoot: Equatable, Sendable, Identifiable {
    var id: UInt16 { ordinal }

    let ordinal: UInt16
    let root: TargetedReclaimScanRoot?
    let failure: TargetedReclaimRootFailure
}

struct TargetedReclaimScanProgress: Equatable, Sendable {
    let context: TargetedReclaimScanContext
    let completed: [TargetedReclaimRootResult]
    let failed: [TargetedReclaimFailedRoot]
    let activeOrdinal: UInt16?
    let activeProgress: ScanProgressFacts?

    var finishedRootCount: UInt16 {
        UInt16(clamping: completed.count + failed.count)
    }
}

struct TargetedReclaimScanBatch: Equatable, Sendable {
    let context: TargetedReclaimScanContext
    let completed: [TargetedReclaimRootResult]
    let failed: [TargetedReclaimFailedRoot]
    let emergencyRecovery: AppEmergencyRecoveryOrdering?

    init(
        context: TargetedReclaimScanContext,
        completed: [TargetedReclaimRootResult],
        failed: [TargetedReclaimFailedRoot],
        emergencyRecovery: AppEmergencyRecoveryOrdering? = nil
    ) {
        self.context = context
        self.completed = completed
        self.failed = failed
        self.emergencyRecovery = emergencyRecovery
    }

    var candidateCount: UInt64 {
        completed.reduce(into: 0) { total, root in
            guard case let .succeeded(count) = root.result.candidateEvaluation else {
                return
            }
            total = total.saturatingAdding(UInt64(count))
        }
    }

    var successfulRootCount: UInt16 {
        UInt16(clamping: completed.filter(\.result.succeeded).count)
    }

    var isComplete: Bool {
        completed.count + failed.count == Int(context.rootCount)
    }
}

enum TargetedReclaimScanDeferral: Equatable, Sendable {
    case userScanActive
    case overlappingExternalScan
    case queueFull
    case storageBusy
}

enum TargetedReclaimScanFailure: Equatable, Sendable {
    case missingStableVolumeIdentity
    case invalidPressureEvidence
    case configuredRootsChanged
    case incompatibleSchema
    case unsafeStorage
    case budgetExceeded
    case unavailable
    case corruptData
    case invalidResponse
    case unexpected
}

enum TargetedReclaimScanState: Equatable, Sendable {
    case idle
    case checking(previous: TargetedReclaimScanBatch?)
    case noEligibleRoots
    case deferred(TargetedReclaimScanDeferral, previous: TargetedReclaimScanBatch?)
    case scanning(TargetedReclaimScanProgress)
    case completed(TargetedReclaimScanBatch)
    case cancelled(previous: TargetedReclaimScanBatch?)
    case failed(TargetedReclaimScanFailure, previous: TargetedReclaimScanBatch?)

    var batch: TargetedReclaimScanBatch? {
        switch self {
        case let .checking(previous),
             let .deferred(_, previous),
             let .cancelled(previous),
             let .failed(_, previous):
            previous
        case let .completed(batch):
            batch
        case let .scanning(progress):
            TargetedReclaimScanBatch(
                context: progress.context,
                completed: progress.completed,
                failed: progress.failed
            )
        case .idle, .noEligibleRoots:
            nil
        }
    }

    var isActive: Bool {
        switch self {
        case .checking, .scanning:
            true
        case .idle, .noEligibleRoots, .deferred, .completed, .cancelled, .failed:
            false
        }
    }
}

/// One authoritative core admission. A task is returned only after Rust has
/// proved the pressure anchor and selected a root from the current registry.
enum TargetedReclaimScanDisposition: Sendable {
    case pressureNotActive
    case noEligibleRoots
    case unavailable(TargetedReclaimRootFailure)
    case current(HomeScanTaskResult)
    case started(any HomeScanTask)
    /// Observe an exact targeted task already owned by the engine. This handle
    /// is non-owning: cancelling the local presentation must only detach.
    case observing(any HomeScanTask)

    var task: (any HomeScanTask)? {
        switch self {
        case let .started(task), let .observing(task):
            task
        case .pressureNotActive, .noEligibleRoots, .unavailable, .current:
            nil
        }
    }

    var ownedTask: (any HomeScanTask)? {
        guard case let .started(task) = self else {
            return nil
        }
        return task
    }
}

struct TargetedReclaimScanAdmission: Sendable {
    let context: TargetedReclaimScanContext?
    let ordinal: UInt16?
    let root: TargetedReclaimScanRoot?
    let disposition: TargetedReclaimScanDisposition
}

enum TargetedReclaimScanServiceError: Error, Equatable, Sendable {
    case closed
    case invalidRecordVersion
    case invalidVolumeIdentity
    case invalidAnchor
    case pressureChanged
    case invalidOrdinal
    case configuredRootsChanged
    case rootMissing
    case rootAccessDenied
    case rootNotDirectory
    case rootSymlink
    case rootChanged
    case rootUnavailable
    case queueFull
    case busy
    case readOnlyStore
    case incompatibleSchema
    case unsafeStorage
    case budgetExceeded
    case corruptData
    case storageUnavailable
    case outcomeUnknown
    case internalState
    case invalidResponse
}

enum TargetedReclaimScanTone: Equatable, Sendable {
    case neutral
    case active
    case success
    case warning
    case failure
}

struct TargetedReclaimScanRootPresentation: Equatable, Sendable, Identifiable {
    var id: UInt16 { ordinal }

    let ordinal: UInt16
    let scanID: String?
    let path: String
    let title: String
    let detail: String
    let tone: TargetedReclaimScanTone
}

struct TargetedReclaimScanPresentation: Equatable, Sendable {
    let title: String
    let detail: String
    let symbol: String
    let tone: TargetedReclaimScanTone
    let progressValue: Double?
    let progressLabel: String?
    let rows: [TargetedReclaimScanRootPresentation]
    let canCancel: Bool
    let canRetry: Bool
    let showsConfigureRoots: Bool

    static func make(_ state: TargetedReclaimScanState) -> Self {
        let rows = rootRows(state.batch)
        switch state {
        case .idle:
            return Self(
                title: "Focused storage scan is standing by",
                detail: "DUX checks known user caches and configured project folders read-only while the startup disk is in Warning or Critical pressure.",
                symbol: "scope",
                tone: .neutral,
                progressValue: nil,
                progressLabel: nil,
                rows: rows,
                canCancel: false,
                canRetry: false,
                showsConfigureRoots: false
            )
        case let .checking(previous):
            return Self(
                title: "Checking focused scan evidence",
                detail: "Verifying the current low-space period and focused storage locations.",
                symbol: "scope",
                tone: .active,
                progressValue: nil,
                progressLabel: nil,
                rows: rootRows(previous),
                canCancel: true,
                canRetry: false,
                showsConfigureRoots: false
            )
        case .noEligibleRoots:
            return Self(
                title: "No focused locations are available",
                detail: "Known user-cache discovery is unavailable on this system. Add project folders in Settings for read-only focused discovery.",
                symbol: "folder.badge.plus",
                tone: .warning,
                progressValue: nil,
                progressLabel: nil,
                rows: [],
                canCancel: false,
                canRetry: false,
                showsConfigureRoots: true
            )
        case let .deferred(reason, previous):
            return Self(
                title: "Focused scan is waiting",
                detail: deferralDetail(reason),
                symbol: "clock.arrow.circlepath",
                tone: .warning,
                progressValue: nil,
                progressLabel: nil,
                rows: rootRows(previous),
                canCancel: false,
                canRetry: true,
                showsConfigureRoots: false
            )
        case let .scanning(progress):
            let finished = progress.finishedRootCount
            let count = progress.context.rootCount
            return Self(
                title: progress.context.pressure == .critical
                    ? "Scanning focused locations during Critical pressure"
                    : "Scanning focused locations during Warning pressure",
                detail: "Recording bounded, read-only observations. Candidate counts are review aids, not guaranteed reclaimable space.",
                symbol: "magnifyingglass",
                tone: .active,
                progressValue: count == 0 ? nil : Double(finished) / Double(count),
                progressLabel: "\(finished) of \(count) locations finished",
                rows: rows,
                canCancel: true,
                canRetry: false,
                showsConfigureRoots: false
            )
        case let .completed(batch):
            let candidates = batch.candidateCount
            let candidateText = candidates == 1 ? "1 candidate" : "\(candidates) candidates"
            let failures = batch.failed.count
            let failureText = failures == 0
                ? ""
                : " \(failures) folder\(failures == 1 ? "" : "s") need attention."
            return Self(
                title: "Focused storage observations are current",
                detail: "Observed \(candidateText) across \(batch.successfulRootCount) scanned folder\(batch.successfulRootCount == 1 ? "" : "s"). These are not guaranteed reclaimable bytes.\(failureText)",
                symbol: failures == 0 ? "checkmark.circle" : "exclamationmark.circle",
                tone: failures == 0 ? .success : .warning,
                progressValue: 1,
                progressLabel: "All focused locations checked",
                rows: rows,
                canCancel: false,
                canRetry: failures > 0,
                showsConfigureRoots: false
            )
        case let .cancelled(previous):
            return Self(
                title: "Focused scan stopped",
                detail: "No cleanup was performed. Completed folder results remain available.",
                symbol: "stop.circle",
                tone: .neutral,
                progressValue: nil,
                progressLabel: nil,
                rows: rootRows(previous),
                canCancel: false,
                canRetry: true,
                showsConfigureRoots: false
            )
        case let .failed(failure, previous):
            return Self(
                title: "Focused scan could not continue",
                detail: failureDetail(failure),
                symbol: "exclamationmark.triangle",
                tone: .failure,
                progressValue: nil,
                progressLabel: nil,
                rows: rootRows(previous),
                canCancel: false,
                canRetry: true,
                showsConfigureRoots: false
            )
        }
    }

    private static func rootRows(
        _ batch: TargetedReclaimScanBatch?
    ) -> [TargetedReclaimScanRootPresentation] {
        guard let batch else {
            return []
        }
        let successes = batch.completed.map { root in
            let candidateText: String
            switch root.result.candidateEvaluation {
            case let .succeeded(candidateCount):
                candidateText = candidateCount == 1
                    ? "1 deterministic candidate"
                    : "\(candidateCount) deterministic candidates"
            case .notRun:
                candidateText = "candidate evaluation did not run"
            case .failed:
                candidateText = "candidate evaluation needs attention"
            }
            let sourceText = switch root.source {
            case .currentDurable:
                "reused durable scan"
            case .focusedRun:
                "new focused scan"
            case .joinedActive:
                "observed existing focused scan"
            }
            let observedAt = root.result.completedAt.formatted(
                date: .abbreviated,
                time: .shortened
            )
            let logical = byteCount(root.result.logicalBytes)
            let sizeText = root.result.allocatedBytes.map {
                "\(byteCount($0)) allocated; \(logical) logical"
            } ?? "\(logical) logical; allocated size unavailable"
            let coverageText = coverage(
                root.result.coverage,
                permille: root.result.coveragePermille
            )
            return TargetedReclaimScanRootPresentation(
                ordinal: root.ordinal,
                scanID: root.result.scanID,
                path: root.root.path.displayText,
                title: root.result.succeeded
                    ? "\(root.root.displayName) observed \(observedAt)"
                    : "\(root.root.displayName) scan incomplete",
                detail: "\(sourceText); \(sizeText); \(coverageText); \(root.result.issueCount) coverage issue\(root.result.issueCount == 1 ? "" : "s"); \(candidateText)",
                tone: root.result.succeeded ? .success : .warning
            )
        }
        let failures = batch.failed.map { root in
            TargetedReclaimScanRootPresentation(
                ordinal: root.ordinal,
                scanID: nil,
                path: root.root?.path.displayText ?? "Focused location \(root.ordinal + 1)",
                title: "Could not scan",
                detail: rootFailureDetail(root.failure),
                tone: .failure
            )
        }
        return (successes + failures).sorted { $0.ordinal < $1.ordinal }
    }

    private static func deferralDetail(_ reason: TargetedReclaimScanDeferral) -> String {
        switch reason {
        case .userScanActive:
            "A user-requested scan has priority. Focused discovery will retry afterward."
        case .overlappingExternalScan:
            "Another scan overlaps a configured folder. Focused discovery will retry later."
        case .queueFull:
            "The bounded engine queue is busy. Focused discovery will retry later."
        case .storageBusy:
            "Durable scan history is temporarily busy. No result was overwritten."
        }
    }

    private static func failureDetail(_ failure: TargetedReclaimScanFailure) -> String {
        switch failure {
        case .missingStableVolumeIdentity:
            "The startup volume has no stable identity, so DUX cannot bind this scan to the low-space observation."
        case .invalidPressureEvidence:
            "The Warning or Critical observation could not be proven from durable capacity history."
        case .configuredRootsChanged:
            "Saved project folders changed during the batch. Retry to use the newest registry."
        case .incompatibleSchema:
            "The local DUX database is incompatible with this build."
        case .unsafeStorage:
            "DUX refused storage that did not pass its safety checks."
        case .budgetExceeded:
            "The bounded history query exceeded its fixed resource budget."
        case .unavailable:
            "The scan engine or durable history is temporarily unavailable."
        case .corruptData:
            "Stored scan evidence failed validation and was not used."
        case .invalidResponse:
            "The storage engine returned an invalid focused-scan response."
        case .unexpected:
            "An unexpected error stopped focused discovery."
        }
    }

    private static func rootFailureDetail(_ failure: TargetedReclaimRootFailure) -> String {
        switch failure {
        case .invalidRoot:
            "The saved path is not a supported scan root."
        case .rootMissing:
            "The folder no longer exists."
        case .accessDenied:
            "macOS denied access to this folder."
        case .rootChanged:
            "The folder changed while DUX validated it."
        case .differentVolume:
            "This folder is on a different volume and cannot relieve pressure on the startup disk."
        case .volumeUnproven:
            "DUX could not prove that this folder belongs to the pressured startup volume."
        case .busy:
            "Another scan currently overlaps this folder."
        case .queueFull:
            "The bounded scan queue was full."
        case .budgetExceeded:
            "This focused pass reached its bounded observation budget."
        case .unattempted:
            "This folder was not attempted after the focused pass reached its budget."
        case .storageUnavailable:
            "Durable scan storage was unavailable."
        case .cancelled:
            "The scan was cancelled before it completed."
        case let .scan(failure):
            switch failure {
            case .rootChanged:
                "The folder changed while it was scanned."
            case .scanFailed:
                "Filesystem traversal failed."
            case .snapshotRejected:
                "The immutable snapshot failed validation."
            case .persistenceUnavailable:
                "Durable scan storage was unavailable."
            case .persistenceOutcomeUnknown:
                "The durable outcome could not be proven."
            case .internalFailure:
                "The scan engine reported an internal failure."
            }
        case .invalidResponse:
            "The scan task returned an invalid response."
        case .unexpected:
            "An unexpected error stopped this folder scan."
        }
    }

    private static func byteCount(_ bytes: UInt64) -> String {
        ByteCountFormatter.string(
            fromByteCount: Int64(min(bytes, UInt64(Int64.max))),
            countStyle: .file
        )
    }

    private static func coverage(
        _ coverage: AppScanCoverage,
        permille: UInt16?
    ) -> String {
        let label = switch coverage {
        case .complete: "complete coverage"
        case .limitedAccess: "limited-access coverage"
        case .partial: "partial coverage"
        case .unknown: "unknown coverage"
        }
        guard let permille else {
            return label
        }
        let fraction = Double(permille) / 1_000
        return "\(fraction.formatted(.percent.precision(.fractionLength(1)))) measured, \(label)"
    }
}

protocol DuxTargetedReclaimScanServing: Sendable {
    func startTargetedReclaimScan(
        stableVolumeID: String,
        anchorAt: Date,
        ordinal: UInt16,
        expectedRootsRevision: UInt64?,
        expectedRootCatalogDigestSHA256: Data?
    ) async throws -> TargetedReclaimScanAdmission
    func validateTargetedReclaimScan(
        _ context: TargetedReclaimScanContext
    ) async throws -> TargetedReclaimScanContext
    func finalizeEmergencyRecovery(
        _ context: TargetedReclaimScanContext
    ) async throws -> AppEmergencyRecoveryOrdering
}

extension DuxTargetedReclaimScanServing {
    func startTargetedReclaimScan(
        stableVolumeID _: String,
        anchorAt _: Date,
        ordinal _: UInt16,
        expectedRootsRevision _: UInt64?,
        expectedRootCatalogDigestSHA256 _: Data?
    ) async throws -> TargetedReclaimScanAdmission {
        throw TargetedReclaimScanServiceError.storageUnavailable
    }

    func validateTargetedReclaimScan(
        _: TargetedReclaimScanContext
    ) async throws -> TargetedReclaimScanContext {
        throw TargetedReclaimScanServiceError.storageUnavailable
    }

    func finalizeEmergencyRecovery(
        _: TargetedReclaimScanContext
    ) async throws -> AppEmergencyRecoveryOrdering {
        throw TargetedReclaimScanServiceError.storageUnavailable
    }
}

struct UnavailableTargetedReclaimScanService: DuxTargetedReclaimScanServing {}

private extension UInt64 {
    func saturatingAdding(_ other: UInt64) -> UInt64 {
        let (sum, overflow) = addingReportingOverflow(other)
        return overflow ? .max : sum
    }
}
