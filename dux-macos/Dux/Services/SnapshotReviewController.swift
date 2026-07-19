import Foundation

protocol DuxSnapshotReviewRenewalClock: Sendable {
    func sleepForRenewalInterval() async throws
}

struct ContinuousDuxSnapshotReviewRenewalClock: DuxSnapshotReviewRenewalClock {
    func sleepForRenewalInterval() async throws {
        try await Task.sleep(for: .seconds(300))
    }
}

/// Owns every live Explorer review lease independently from SwiftUI render
/// state. Explorer can acquire the newest available snapshot without first
/// trusting a history hint; later paged navigation attaches here by scan ID.
actor DuxSnapshotReviewController {
    private struct LeaseEntry: Sendable {
        let generation: UUID
        let lease: any DuxSnapshotReviewLease
        var ownerCount: UInt64
    }

    private let service: any DuxSnapshotReviewServing
    private let clock: any DuxSnapshotReviewRenewalClock

    private var leases: [String: LeaseEntry] = [:]
    private var pendingAcquisitions: [String: UUID] = [:]
    private var pendingLatestAcquisition: UUID?
    private var renewalTask: Task<Void, Never>?
    private var renewalInProgress = false
    private var renewalRequested = false
    private var isShuttingDown = false

    init(
        service: any DuxSnapshotReviewServing,
        clock: any DuxSnapshotReviewRenewalClock = ContinuousDuxSnapshotReviewRenewalClock()
    ) {
        self.service = service
        self.clock = clock
    }

    func acquire(scanID: String) async throws {
        guard !isShuttingDown else {
            throw EngineServiceError.closed
        }
        if retainExistingLease(scanID: scanID) {
            return
        }

        let generation = UUID()
        pendingAcquisitions[scanID] = generation
        let lease: any DuxSnapshotReviewLease
        do {
            lease = try await service.acquireExplorerReview(scanID: scanID)
        } catch {
            if pendingAcquisitions[scanID] == generation {
                pendingAcquisitions.removeValue(forKey: scanID)
            }
            throw error
        }
        guard
            !isShuttingDown,
            pendingAcquisitions[scanID] == generation
        else {
            await lease.release()
            throw CancellationError()
        }
        pendingAcquisitions.removeValue(forKey: scanID)
        if leases[scanID] == nil {
            leases[scanID] = LeaseEntry(
                generation: generation,
                lease: lease,
                ownerCount: 1
            )
            startRenewalLoopIfNeeded()
        } else {
            // Actor reentrancy can complete two acquisitions for the same
            // scan. Keep the first installed generation and release the stale
            // lease explicitly.
            _ = retainExistingLease(scanID: scanID)
            await lease.release()
        }
    }

    /// Acquires and retains the newest complete, non-tombstoned snapshot. The
    /// core selects the exact snapshot and returns its scan ID with the lease,
    /// so this path does not make an authority decision from history metadata.
    @discardableResult
    func acquireLatest() async throws -> String {
        guard !isShuttingDown else {
            throw EngineServiceError.closed
        }

        let generation = UUID()
        pendingLatestAcquisition = generation
        let lease: any DuxSnapshotReviewLease
        do {
            lease = try await service.acquireLatestExplorerReview()
        } catch {
            if pendingLatestAcquisition == generation {
                pendingLatestAcquisition = nil
            }
            throw error
        }
        guard
            !isShuttingDown,
            pendingLatestAcquisition == generation
        else {
            await lease.release()
            throw CancellationError()
        }
        pendingLatestAcquisition = nil

        let scanID = lease.scanID
        if leases[scanID] == nil {
            leases[scanID] = LeaseEntry(
                generation: generation,
                lease: lease,
                ownerCount: 1
            )
            startRenewalLoopIfNeeded()
        } else {
            _ = retainExistingLease(scanID: scanID)
            await lease.release()
        }
        return scanID
    }

    func release(scanID: String) async {
        pendingAcquisitions.removeValue(forKey: scanID)
        guard var entry = leases[scanID] else {
            return
        }
        if entry.ownerCount > 1 {
            entry.ownerCount -= 1
            leases[scanID] = entry
            return
        }
        leases.removeValue(forKey: scanID)
        await entry.lease.release()
        stopRenewalLoopIfEmpty()
    }

    func rootNode(scanID: String) async throws -> ExplorerSnapshotNode {
        guard !isShuttingDown else {
            throw EngineServiceError.closed
        }
        guard let entry = leases[scanID] else {
            throw ExplorerSnapshotNodeError.reviewNotAcquired
        }
        let node: ExplorerSnapshotNode
        do {
            node = try await entry.lease.rootNode()
        } catch {
            await discardExpiredLeaseIfCurrent(error, scanID: scanID, entry: entry)
            throw error
        }
        guard leases[scanID]?.generation == entry.generation else {
            throw CancellationError()
        }
        return node
    }

    func candidateSummaries(
        scanID: String,
        cursor: UInt16,
        limit: UInt16
    ) async throws -> ExplorerCandidateSummaryPage {
        guard !isShuttingDown else {
            throw EngineServiceError.closed
        }
        guard let entry = leases[scanID] else {
            throw ExplorerCandidateDetailError.reviewNotAcquired
        }
        let page: ExplorerCandidateSummaryPage
        do {
            page = try await entry.lease.candidateSummaries(cursor: cursor, limit: limit)
        } catch {
            await discardExpiredLeaseIfCurrent(error, scanID: scanID, entry: entry)
            throw error
        }
        guard leases[scanID]?.generation == entry.generation else {
            throw CancellationError()
        }
        return page
    }

    func childNodes(
        scanID: String,
        parentID: UInt64,
        sort: ExplorerSnapshotNodeSort,
        offset: UInt64,
        limit: UInt16
    ) async throws -> ExplorerSnapshotNodePage {
        guard !isShuttingDown else {
            throw EngineServiceError.closed
        }
        guard let entry = leases[scanID] else {
            throw ExplorerSnapshotNodeError.reviewNotAcquired
        }
        let page: ExplorerSnapshotNodePage
        do {
            page = try await entry.lease.childNodes(
                parentID: parentID,
                sort: sort,
                offset: offset,
                limit: limit
            )
        } catch {
            await discardExpiredLeaseIfCurrent(error, scanID: scanID, entry: entry)
            throw error
        }
        guard leases[scanID]?.generation == entry.generation else {
            throw CancellationError()
        }
        return page
    }

    func treemap(
        scanID: String,
        parentID: UInt64,
        maxCells: UInt16
    ) async throws -> ExplorerSnapshotTreemap {
        guard !isShuttingDown else {
            throw EngineServiceError.closed
        }
        guard let entry = leases[scanID] else {
            throw ExplorerSnapshotTreemapError.reviewNotAcquired
        }
        let treemap: ExplorerSnapshotTreemap
        do {
            treemap = try await entry.lease.treemap(parentID: parentID, maxCells: maxCells)
        } catch {
            await discardExpiredLeaseIfCurrent(error, scanID: scanID, entry: entry)
            throw error
        }
        guard leases[scanID]?.generation == entry.generation else {
            throw CancellationError()
        }
        return treemap
    }

    func largeFiles(
        scanID: String,
        minimumLogicalBytes: UInt64,
        modifiedBefore: ExplorerSnapshotTimestamp?,
        maxResults: UInt16
    ) async throws -> ExplorerSnapshotLargeFilesPage {
        guard !isShuttingDown else {
            throw EngineServiceError.closed
        }
        guard let entry = leases[scanID] else {
            throw ExplorerSnapshotLargeFilesError.reviewNotAcquired
        }
        let page: ExplorerSnapshotLargeFilesPage
        do {
            page = try await entry.lease.largeFiles(
                minimumLogicalBytes: minimumLogicalBytes,
                modifiedBefore: modifiedBefore,
                maxResults: maxResults
            )
        } catch {
            await discardExpiredLeaseIfCurrent(error, scanID: scanID, entry: entry)
            throw error
        }
        guard leases[scanID]?.generation == entry.generation else {
            throw CancellationError()
        }
        return page
    }

    func candidatePaths(
        scanID: String,
        candidateID: String,
        cursor: UInt16,
        limit: UInt16
    ) async throws -> ExplorerCandidatePathPage {
        guard !isShuttingDown else {
            throw EngineServiceError.closed
        }
        guard let entry = leases[scanID] else {
            throw ExplorerCandidateDetailError.reviewNotAcquired
        }
        let page: ExplorerCandidatePathPage
        do {
            page = try await entry.lease.candidatePaths(
                candidateID: candidateID,
                cursor: cursor,
                limit: limit
            )
        } catch {
            await discardExpiredLeaseIfCurrent(error, scanID: scanID, entry: entry)
            throw error
        }
        guard leases[scanID]?.generation == entry.generation else {
            throw CancellationError()
        }
        return page
    }

    func candidateEvidence(
        scanID: String,
        candidateID: String,
        cursor: UInt16,
        limit: UInt16
    ) async throws -> ExplorerCandidateEvidencePage {
        guard !isShuttingDown else {
            throw EngineServiceError.closed
        }
        guard let entry = leases[scanID] else {
            throw ExplorerCandidateDetailError.reviewNotAcquired
        }
        let page: ExplorerCandidateEvidencePage
        do {
            page = try await entry.lease.candidateEvidence(
                candidateID: candidateID,
                cursor: cursor,
                limit: limit
            )
        } catch {
            await discardExpiredLeaseIfCurrent(error, scanID: scanID, entry: entry)
            throw error
        }
        guard leases[scanID]?.generation == entry.generation else {
            throw CancellationError()
        }
        return page
    }

    func resolveLiveItem(
        scanID: String,
        nodeID: UInt64,
        purpose: ExplorerSnapshotLivePathPurpose
    ) async throws -> ExplorerResolvedLiveItem {
        guard !isShuttingDown else {
            throw EngineServiceError.closed
        }
        guard let entry = leases[scanID] else {
            throw ExplorerSnapshotLivePathError.reviewNotAcquired
        }
        let item: ExplorerResolvedLiveItem
        do {
            item = try await entry.lease.resolveLiveItem(nodeID: nodeID, purpose: purpose)
        } catch {
            await discardExpiredLeaseIfCurrent(error, scanID: scanID, entry: entry)
            throw error
        }
        guard leases[scanID]?.generation == entry.generation else {
            throw CancellationError()
        }
        return item
    }

    func executeTrash(scanID: String, nodeID: UInt64) async throws -> TrashPlatformResult {
        guard !isShuttingDown else {
            throw ExplorerTrashError.closed
        }
        guard let entry = leases[scanID] else {
            throw ExplorerTrashError.reviewNotAcquired
        }
        let result: TrashPlatformResult
        do {
            result = try await entry.lease.executeTrash(nodeID: nodeID)
        } catch {
            await discardExpiredLeaseIfCurrent(error, scanID: scanID, entry: entry)
            throw error
        }
        guard leases[scanID]?.generation == entry.generation else {
            throw CancellationError()
        }
        return result
    }

    func startSubtreeScan(
        sourceScanID: String,
        nodeID: UInt64
    ) async throws -> HomeScanStartDisposition {
        guard !isShuttingDown else {
            throw HomeScanServiceError.closed
        }
        guard let entry = leases[sourceScanID] else {
            throw HomeScanServiceError.rootUnavailable
        }
        let start: HomeScanStartDisposition
        do {
            start = try await entry.lease.startSubtreeScan(nodeID: nodeID)
        } catch {
            await discardExpiredLeaseIfCurrent(error, scanID: sourceScanID, entry: entry)
            throw Self.subtreeScanServiceError(error)
        }
        guard
            !isShuttingDown,
            leases[sourceScanID]?.generation == entry.generation
        else {
            _ = try? await start.task.requestCancellation()
            throw CancellationError()
        }
        return start
    }

    private func discardExpiredLeaseIfCurrent(
        _ error: Error,
        scanID: String,
        entry: LeaseEntry
    ) async {
        guard
            isReviewExpired(error),
            leases[scanID]?.generation == entry.generation
        else {
            return
        }
        leases.removeValue(forKey: scanID)
        await entry.lease.release()
        stopRenewalLoopIfEmpty()
    }

    private func isReviewExpired(_ error: Error) -> Bool {
        error as? ExplorerSnapshotNodeError == .reviewExpired
            || error as? ExplorerSnapshotTreemapError == .reviewExpired
            || error as? ExplorerSnapshotLargeFilesError == .reviewExpired
            || error as? ExplorerSnapshotLivePathError == .reviewExpired
            || error as? ExplorerSnapshotSubtreeScanError == .reviewExpired
    }

    private static func subtreeScanServiceError(_ error: Error) -> Error {
        guard let error = error as? ExplorerSnapshotSubtreeScanError else {
            return error
        }
        return switch error {
        case .reviewExpired, .nodeNotFound, .nodeNotDirectory, .rootUnavailable:
            HomeScanServiceError.rootUnavailable
        case .busy:
            HomeScanServiceError.busy
        case .foreignReview, .invalidResponse:
            HomeScanServiceError.invalidResponse
        case .unavailable:
            HomeScanServiceError.persistenceUnavailable
        }
    }

    func renewNow() async {
        guard !isShuttingDown else {
            return
        }
        guard !renewalInProgress else {
            renewalRequested = true
            return
        }
        renewalInProgress = true
        let current = leases
        for (scanID, entry) in current {
            do {
                _ = try await entry.lease.renew()
            } catch {
                if leases[scanID]?.generation == entry.generation {
                    leases.removeValue(forKey: scanID)
                    // Renew failure or expiry must promptly close the retained
                    // file handle. Explicit release is best-effort; the core
                    // expiry remains the durable fallback.
                    await entry.lease.release()
                }
            }
        }
        renewalInProgress = false
        stopRenewalLoopIfEmpty()
        if renewalRequested, !isShuttingDown {
            renewalRequested = false
            await renewNow()
        }
    }

    func shutdown() async {
        guard !isShuttingDown else {
            return
        }
        isShuttingDown = true
        pendingAcquisitions.removeAll(keepingCapacity: false)
        pendingLatestAcquisition = nil
        renewalRequested = false
        renewalTask?.cancel()
        renewalTask = nil
        let current = leases.values.map(\.lease)
        leases.removeAll(keepingCapacity: false)
        for lease in current {
            await lease.release()
        }
    }

    func activeLeaseCount() -> Int {
        leases.count
    }

    func activeOwnerCount(scanID: String) -> UInt64 {
        leases[scanID]?.ownerCount ?? 0
    }

    private func retainExistingLease(scanID: String) -> Bool {
        guard var entry = leases[scanID], entry.ownerCount < UInt64.max else {
            return false
        }
        entry.ownerCount += 1
        leases[scanID] = entry
        return true
    }

    private func startRenewalLoopIfNeeded() {
        guard renewalTask == nil, !leases.isEmpty, !isShuttingDown else {
            return
        }
        let clock = self.clock
        renewalTask = Task { [weak self] in
            while !Task.isCancelled {
                do {
                    try await clock.sleepForRenewalInterval()
                } catch {
                    return
                }
                await self?.renewNow()
            }
        }
    }

    private func stopRenewalLoopIfEmpty() {
        guard leases.isEmpty else {
            return
        }
        renewalTask?.cancel()
        renewalTask = nil
    }
}

extension DuxSnapshotReviewController: DuxSnapshotSubtreeScanServing {}
