import Foundation
import XCTest

@testable import DUX

final class DiskPressureNotificationTests: XCTestCase {
    private let now = Date(timeIntervalSince1970: 2_000_000_000)

    func testOnlyNewWarningAndCriticalEpisodesProduceCandidates() {
        let cases: [(DiskPressureLevel?, DiskPressureLevel, DiskPressureNotificationUrgency?)] = [
            (nil, .warning, .warning),
            (nil, .critical, .critical),
            (.healthy, .warning, .warning),
            (.healthy, .critical, .critical),
            (.warning, .critical, .critical),
            (.warning, .warning, nil),
            (.critical, .critical, nil),
            (.critical, .warning, nil),
            (.warning, .healthy, nil),
            (.critical, .healthy, nil),
            (.healthy, .healthy, nil),
            (.healthy, .unknown, nil),
        ]

        for (previous, current, expected) in cases {
            let candidate = DiskPressureNotificationGate.candidate(
                for: snapshot(previous: previous, current: current),
                lastAcceptedAtForUrgency: nil
            )
            XCTAssertEqual(
                candidate?.payload.urgency,
                expected,
                "unexpected gate result for \(String(describing: previous)) -> \(current)"
            )
        }
    }

    func testOnlyNewlyStoredDurableObservationCanNotify() {
        let ineligible: [VolumeCapacityHistoryDisposition?] = [
            nil,
            .existingExact,
            .suppressedByHourlyCadence,
            .notStoredMissingOrdinaryAvailability,
            .notStoredMissingStableIdentity,
            .notStoredIncompleteMetadata,
        ]

        for disposition in ineligible {
            XCTAssertNil(
                DiskPressureNotificationGate.candidate(
                    for: snapshot(
                        previous: .healthy,
                        current: .warning,
                        disposition: disposition
                    ),
                    lastAcceptedAtForUrgency: nil
                )
            )
        }
    }

    func testCooldownIsInclusiveAtExactlyTwentyFourHours() {
        let snapshot = snapshot(previous: .healthy, current: .warning)
        XCTAssertNil(
            DiskPressureNotificationGate.candidate(
                for: snapshot,
                lastAcceptedAtForUrgency: now.addingTimeInterval(
                    -DiskPressureNotificationGate.cooldown + 1
                )
            )
        )
        XCTAssertNotNil(
            DiskPressureNotificationGate.candidate(
                for: snapshot,
                lastAcceptedAtForUrgency: now.addingTimeInterval(
                    -DiskPressureNotificationGate.cooldown
                )
            )
        )
    }

    func testFutureCooldownTimestampFailsClosed() {
        XCTAssertNil(
            DiskPressureNotificationGate.candidate(
                for: snapshot(previous: .warning, current: .critical),
                lastAcceptedAtForUrgency: now.addingTimeInterval(60)
            )
        )
    }

    func testCriticalUsesItsOwnUrgencyCooldown() {
        let critical = snapshot(previous: .warning, current: .critical)
        XCTAssertNotNil(
            DiskPressureNotificationGate.candidate(
                for: critical,
                lastAcceptedAtForUrgency: nil
            )
        )
        XCTAssertNil(
            DiskPressureNotificationGate.candidate(
                for: critical,
                lastAcceptedAtForUrgency: now.addingTimeInterval(-60)
            )
        )
    }

    func testCandidateContainsOnlyTruthfulPathFreeCapacityFacts() throws {
        let candidate = try XCTUnwrap(
            DiskPressureNotificationGate.candidate(
                for: snapshot(previous: .healthy, current: .warning),
                lastAcceptedAtForUrgency: nil
            )
        )
        XCTAssertEqual(candidate.availableBytes, 7 * 1_024 * 1_024 * 1_024)
        XCTAssertEqual(candidate.displayName, "Startup")
        XCTAssertEqual(candidate.sampledAt, now)
        XCTAssertEqual(candidate.payload.stableVolumeID, "volume:macos:test")
    }

    func testPayloadRoundTripsExactVersionedRecommendationsFields() throws {
        let payload = try XCTUnwrap(
            DiskPressureNotificationPayload(
                stableVolumeID: "volume:macos:test",
                urgency: .critical
            )
        )
        XCTAssertEqual(
            payload.fields,
            [
                "recordVersion": "1",
                "route": "recommendations",
                "stableVolumeID": "volume:macos:test",
                "urgency": "critical",
            ]
        )
        XCTAssertEqual(DiskPressureNotificationPayload(fields: payload.fields), payload)
        XCTAssertEqual(
            DiskPressureNotificationPayload(
                userInfo: payload.fields.reduce(into: [AnyHashable: Any]()) { result, entry in
                    result[entry.key] = entry.value
                }
            ),
            payload
        )
    }

    func testPayloadRejectsMalformedUnknownAndOverBoundFields() {
        XCTAssertNil(
            DiskPressureNotificationPayload(stableVolumeID: "", urgency: .warning)
        )
        XCTAssertNil(
            DiskPressureNotificationPayload(stableVolumeID: "bad\nvalue", urgency: .warning)
        )
        XCTAssertNil(
            DiskPressureNotificationPayload(
                stableVolumeID: String(repeating: "a", count: 257),
                urgency: .warning
            )
        )
        XCTAssertNil(
            DiskPressureNotificationPayload(
                userInfo: [
                    "recordVersion": "1",
                    "route": "recommendations",
                    "stableVolumeID": "volume:macos:test",
                    "urgency": "warning",
                    "unexpected": "field",
                ]
            )
        )
        XCTAssertNil(
            DiskPressureNotificationPayload(
                fields: [
                    "recordVersion": "2",
                    "route": "recommendations",
                    "stableVolumeID": "volume:macos:test",
                    "urgency": "warning",
                ]
            )
        )
        XCTAssertNil(
            DiskPressureNotificationPayload(
                fields: [
                    "recordVersion": "1",
                    "route": "cleanup",
                    "stableVolumeID": "volume:macos:test",
                    "urgency": "warning",
                ]
            )
        )
    }

    func testMissingOrMalformedStableIdentityFailsClosed() {
        XCTAssertNil(
            DiskPressureNotificationGate.candidate(
                for: snapshot(
                    previous: .healthy,
                    current: .warning,
                    stableVolumeID: nil
                ),
                lastAcceptedAtForUrgency: nil
            )
        )
        XCTAssertNil(
            DiskPressureNotificationGate.candidate(
                for: snapshot(
                    previous: .healthy,
                    current: .warning,
                    stableVolumeID: "bad\nvalue"
                ),
                lastAcceptedAtForUrgency: nil
            )
        )
    }

    @MainActor
    func testAppModelDeliversOneTransitionAndHonorsCooldown() async {
        let now = Date(timeIntervalSince1970: 2_000_000_000)
        let snapshot = VolumeCapacitySnapshot(
            stableVolumeID: "volume:macos:test",
            displayName: "Startup",
            filesystem: "apfs",
            isInternal: true,
            isRemovable: false,
            totalBytes: 100 * 1_024 * 1_024 * 1_024,
            filesystemAvailableBytes: 7 * 1_024 * 1_024 * 1_024,
            importantAvailableBytes: 8 * 1_024 * 1_024 * 1_024,
            effectiveAvailableBytes: 7 * 1_024 * 1_024 * 1_024,
            availabilityBasis: .filesystemAvailable,
            pressure: .warning,
            previousDurablePressure: .healthy,
            criticalBoundaryBytes: 8 * 1_024 * 1_024 * 1_024,
            warningBoundaryBytes: 20 * 1_024 * 1_024 * 1_024,
            historyDisposition: .stored,
            sampledAt: now
        )
        let service = RecordingPressureNotificationService()
        let defaults = UserDefaults(suiteName: "dux.notification-test.\(UUID().uuidString)")!
        let model = AppModel(
            engineService: RecordingPressureEngine(snapshot: snapshot),
            volumeMonitor: RecordingPressureVolumeMonitor(snapshot: snapshot),
            notificationService: service,
            diskPressureNotificationCooldownStore:
                UserDefaultsDiskPressureNotificationCooldownStore(defaults: defaults)
        )

        await model.refreshVolumeCapacity()
        await waitForDelivery(service, count: 1)
        await model.refreshVolumeCapacity()
        await Task.yield()

        let deliveries = await service.deliveries()
        XCTAssertEqual(deliveries.count, 1)
        XCTAssertEqual(deliveries.first?.userInfo["route"], "recommendations")
        XCTAssertEqual(deliveries.first?.userInfo["urgency"], "warning")
    }

    private func snapshot(
        previous: DiskPressureLevel?,
        current: DiskPressureLevel,
        disposition: VolumeCapacityHistoryDisposition? = .stored,
        stableVolumeID: String? = "volume:macos:test"
    ) -> VolumeCapacitySnapshot {
        VolumeCapacitySnapshot(
            stableVolumeID: stableVolumeID,
            displayName: "Startup",
            filesystem: "apfs",
            isInternal: true,
            isRemovable: false,
            totalBytes: 100 * 1_024 * 1_024 * 1_024,
            filesystemAvailableBytes: 7 * 1_024 * 1_024 * 1_024,
            importantAvailableBytes: 8 * 1_024 * 1_024 * 1_024,
            effectiveAvailableBytes: 7 * 1_024 * 1_024 * 1_024,
            availabilityBasis: .filesystemAvailable,
            pressure: current,
            previousDurablePressure: previous,
            criticalBoundaryBytes: 8 * 1_024 * 1_024 * 1_024,
            warningBoundaryBytes: 20 * 1_024 * 1_024 * 1_024,
            historyDisposition: disposition,
            sampledAt: now
        )
    }

    @MainActor
    private func waitForDelivery(
        _ service: RecordingPressureNotificationService,
        count: Int
    ) async {
        for _ in 0..<100 {
            if await service.deliveries().count >= count {
                return
            }
            await Task.yield()
        }
    }
}

private actor RecordingPressureNotificationService: NotificationServing {
    private var values: [DiskPressureNotificationDelivery] = []

    func authorizationStatus() -> NotificationAuthorizationStatus { .authorized }
    func requestAuthorization() async throws {}

    func deliverDiskPressure(_ delivery: DiskPressureNotificationDelivery) {
        values.append(delivery)
    }

    func deliveries() -> [DiskPressureNotificationDelivery] { values }
}

private actor RecordingPressureVolumeMonitor: VolumeMonitoring {
    let snapshot: VolumeCapacitySnapshot

    init(snapshot: VolumeCapacitySnapshot) {
        self.snapshot = snapshot
    }

    func sampleStartupVolume() async throws -> VolumeCapacitySnapshot { snapshot }
}

private actor RecordingPressureEngine: EngineServing {
    let snapshot: VolumeCapacitySnapshot

    init(snapshot: VolumeCapacitySnapshot) {
        self.snapshot = snapshot
    }

    func loadStatus() async throws -> EngineStatus {
        EngineStatus(libraryVersion: "test", ffiContractVersion: 20, executedOffMainThread: true)
    }

    func observeVolumeCapacity(
        _: VolumeCapacitySnapshot
    ) async throws -> VolumeCapacitySnapshot {
        snapshot
    }

    func loadDiskPressurePolicy() async throws -> DiskPressurePolicy {
        DiskPressurePolicy(
            source: .default,
            revision: 0,
            configuration: .defaults,
            updatedAtUnixMilliseconds: nil
        )
    }

    func setDiskPressurePolicy(
        _ configuration: DiskPressurePolicyConfiguration
    ) async throws -> DiskPressurePolicyUpdateResult {
        DiskPressurePolicyUpdateResult(
            policy: DiskPressurePolicy(
                source: .stored,
                revision: 1,
                configuration: configuration,
                updatedAtUnixMilliseconds: 1
            ),
            changed: true
        )
    }

    func resetDiskPressurePolicy() async throws -> DiskPressurePolicyUpdateResult {
        DiskPressurePolicyUpdateResult(
            policy: DiskPressurePolicy(
                source: .default,
                revision: 1,
                configuration: .defaults,
                updatedAtUnixMilliseconds: 1
            ),
            changed: true
        )
    }
}
