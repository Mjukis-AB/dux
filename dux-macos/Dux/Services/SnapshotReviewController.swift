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
/// state. The current Explorer shell has no selected snapshot, so production
/// starts with an empty set; later paged navigation attaches here by scan ID.
actor DuxSnapshotReviewController {
    private struct LeaseEntry: Sendable {
        let generation: UUID
        let lease: any DuxSnapshotReviewLease
    }

    private let service: any DuxSnapshotReviewServing
    private let clock: any DuxSnapshotReviewRenewalClock

    private var leases: [String: LeaseEntry] = [:]
    private var pendingAcquisitions: [String: UUID] = [:]
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
        if leases[scanID] != nil {
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
            leases[scanID] = LeaseEntry(generation: generation, lease: lease)
            startRenewalLoopIfNeeded()
        } else {
            // Actor reentrancy can complete two acquisitions for the same
            // scan. Keep the first installed generation and release the stale
            // lease explicitly.
            await lease.release()
        }
    }

    func release(scanID: String) async {
        pendingAcquisitions.removeValue(forKey: scanID)
        guard let entry = leases.removeValue(forKey: scanID) else {
            return
        }
        await entry.lease.release()
        stopRenewalLoopIfEmpty()
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
