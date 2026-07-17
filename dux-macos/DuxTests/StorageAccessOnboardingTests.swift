import Foundation
import XCTest
@testable import DUX

@MainActor
final class StorageAccessOnboardingTests: XCTestCase {
    private let en = Locale(identifier: "en_US")

    func testIntroductionStoreDefaultsRoundTripsAndPreservesUnknownValues() throws {
        let suiteName = "dux-storage-introduction-tests-\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suiteName))
        defer { defaults.removePersistentDomain(forName: suiteName) }
        let store = UserDefaultsStorageAccessIntroductionPreferenceStore(defaults: defaults)

        XCTAssertFalse(store.loadAcknowledged())
        for value in ["2|acknowledged", "1|future", "broken"] {
            defaults.set(value, forKey: UserDefaultsStorageAccessIntroductionPreferenceStore.key)
            XCTAssertFalse(store.loadAcknowledged())
            XCTAssertEqual(
                defaults.string(
                    forKey: UserDefaultsStorageAccessIntroductionPreferenceStore.key
                ),
                value
            )
        }
        defaults.set(7, forKey: UserDefaultsStorageAccessIntroductionPreferenceStore.key)
        XCTAssertFalse(store.loadAcknowledged())
        XCTAssertEqual(
            defaults.object(
                forKey: UserDefaultsStorageAccessIntroductionPreferenceStore.key
            ) as? Int,
            7
        )

        store.saveAcknowledged()
        XCTAssertTrue(store.loadAcknowledged())
        let domain = defaults.persistentDomain(forName: suiteName)
        XCTAssertEqual(domain?.count, 1)
    }

    func testEvidenceRequiresExactBoundedCountsAndCanonicalTime() {
        XCTAssertNotNil(evidence(readable: 1, unreadable: 1, unobserved: 1))
        XCTAssertNil(
            StorageAccessEvidence(
                readableLocationCount: 1,
                unreadableLocationCount: 1,
                unobservedLocationCount: 0,
                observedAt: Date(timeIntervalSince1970: 1)
            )
        )
        XCTAssertNil(
            StorageAccessEvidence(
                readableLocationCount: 3,
                unreadableLocationCount: 0,
                unobservedLocationCount: 0,
                observedAt: Date(timeIntervalSince1970: -1)
            )
        )
    }

    func testBroaderGuidanceAppearsOnlyAfterMeasuredIncompleteCoverage() {
        XCTAssertFalse(presentation(scanState: .idle).showsBroaderAnalysisAction)
        XCTAssertFalse(
            presentation(scanState: scanState(coverage: .complete))
                .showsBroaderAnalysisAction
        )

        for coverage in [
            AppScanCoverage.limitedAccess,
            .partial,
            .unknown,
        ] {
            let value = presentation(scanState: scanState(coverage: coverage))
            XCTAssertTrue(value.showsBroaderAnalysisAction)
            XCTAssertFalse(value.showsGuidance)
            XCTAssertFalse(value.showsSystemSettingsAction)
        }
    }

    func testRequestedGuidanceNeverClaimsFullDiskAccessStatus() throws {
        let states: [StorageAccessProbeState] = [
            .idle,
            .checking(previous: nil),
            .observed(evidence(readable: 3, unreadable: 0, unobserved: 0)),
            .observed(evidence(readable: 1, unreadable: 1, unobserved: 1)),
            .failed(previous: nil),
            .failed(previous: evidence(readable: 2, unreadable: 1, unobserved: 0)),
        ]

        for state in states {
            let value = presentation(
                scanState: scanState(coverage: .partial),
                requested: true,
                probeState: state
            )
            XCTAssertTrue(value.showsGuidance)
            XCTAssertTrue(value.showsSystemSettingsAction)
            XCTAssertFalse(value.showsBroaderAnalysisAction)
            let detail = try XCTUnwrap(value.statusDetail)
            XCTAssertFalse(detail.contains("enabled"), detail)
            XCTAssertFalse(detail.contains("granted"), detail)
        }

        let observed = presentation(
            scanState: scanState(coverage: .partial),
            requested: true,
            probeState: .observed(evidence(readable: 3, unreadable: 0, unobserved: 0))
        )
        XCTAssertTrue(
            try XCTUnwrap(observed.statusDetail).contains("does not expose an authoritative")
        )
    }

    func testProbeServiceSamplesOnlyThreeFixedLocationsOffMain() async throws {
        let recorder = StorageAccessLocationRecorder()
        let home = URL(filePath: "/test-home", directoryHint: .isDirectory)
        let service = StorageAccessProbeService(
            homeDirectory: home,
            observe: { location in
                recorder.record(location)
                return switch location.lastPathComponent {
                case "Mail": .readable
                case "Messages": .unreadable
                default: .unobserved
                }
            },
            now: { Date(timeIntervalSince1970: 10) }
        )

        let result = try await service.probe()

        XCTAssertEqual(result, evidence(readable: 1, unreadable: 1, unobserved: 1))
        XCTAssertEqual(
            Set(recorder.paths),
            [
                "/test-home/Library/Mail",
                "/test-home/Library/Messages",
                "/test-home/Library/Safari",
            ]
        )
        XCTAssertEqual(recorder.mainThreadObservations, 0)
    }

    func testAppModelPersistsIntroductionOnceWithoutProbing() async {
        let introductionStore = StorageAccessIntroductionStoreSpy(acknowledged: false)
        let probe = StorageAccessSequenceProbe(results: [])
        let model = AppModel(
            storageAccessIntroductionPreferenceStore: introductionStore,
            storageAccessProbe: probe
        )

        XCTAssertTrue(model.showsStorageAccessIntroduction)
        await model.refreshStorageAccessEvidenceAfterActivation()
        model.armStorageAccessSettingsReturnProbe()
        model.acknowledgeStorageAccessIntroduction()
        model.acknowledgeStorageAccessIntroduction()

        XCTAssertFalse(model.showsStorageAccessIntroduction)
        XCTAssertEqual(introductionStore.saveCount, 1)
        var probeCount = await probe.probeCount
        XCTAssertEqual(probeCount, 0)
    }

    func testExplicitRequestAndArmedActivationAreTheOnlyProbeTriggers() async {
        let first = evidence(readable: 1, unreadable: 2, unobserved: 0)
        let second = evidence(readable: 2, unreadable: 1, unobserved: 0)
        let probe = StorageAccessSequenceProbe(results: [.success(first), .success(second)])
        let model = AppModel(storageAccessProbe: probe)

        await model.refreshStorageAccessEvidenceAfterActivation()
        var probeCount = await probe.probeCount
        XCTAssertEqual(probeCount, 0)

        await model.requestBroaderStorageAnalysis()
        XCTAssertTrue(model.broaderStorageAnalysisRequested)
        XCTAssertEqual(model.storageAccessProbeState, .observed(first))
        probeCount = await probe.probeCount
        XCTAssertEqual(probeCount, 1)

        await model.refreshStorageAccessEvidenceAfterActivation()
        probeCount = await probe.probeCount
        XCTAssertEqual(probeCount, 1)

        model.armStorageAccessSettingsReturnProbe()
        await model.refreshStorageAccessEvidenceAfterActivation()
        XCTAssertEqual(model.storageAccessProbeState, .observed(second))
        probeCount = await probe.probeCount
        XCTAssertEqual(probeCount, 2)

        await model.refreshStorageAccessEvidenceAfterActivation()
        probeCount = await probe.probeCount
        XCTAssertEqual(probeCount, 2)
    }

    func testDuplicateProbeRequestsCoalesceAndSurviveCallerCancellation() async {
        let expected = evidence(readable: 2, unreadable: 0, unobserved: 1)
        let probe = ControllableStorageAccessProbe()
        let model = AppModel(storageAccessProbe: probe)

        let first = Task { @MainActor in
            await model.requestBroaderStorageAnalysis()
        }
        await probe.waitForProbeCount(1)
        let second = Task { @MainActor in
            await model.requestBroaderStorageAnalysis()
        }
        first.cancel()
        await Task.yield()
        var probeCount = await probe.probeCount
        XCTAssertEqual(probeCount, 1)

        await probe.succeed(with: expected)
        await first.value
        await second.value

        XCTAssertEqual(model.storageAccessProbeState, .observed(expected))
        probeCount = await probe.probeCount
        XCTAssertEqual(probeCount, 1)
    }

    func testFailedRecheckRetainsLastObservedEvidence() async {
        let first = evidence(readable: 1, unreadable: 1, unobserved: 1)
        let probe = StorageAccessSequenceProbe(
            results: [.success(first), .failure(StorageAccessTestError.failed)]
        )
        let model = AppModel(storageAccessProbe: probe)

        await model.requestBroaderStorageAnalysis()
        await model.refreshStorageAccessEvidence()

        XCTAssertEqual(model.storageAccessProbeState, .failed(previous: first))
    }

    func testInvalidationRejectsLateProbePublication() async {
        let late = evidence(readable: 3, unreadable: 0, unobserved: 0)
        let probe = ControllableStorageAccessProbe()
        let model = AppModel(storageAccessProbe: probe)
        let request = Task { @MainActor in
            await model.requestBroaderStorageAnalysis()
        }
        await probe.waitForProbeCount(1)

        model.invalidateStorageAccessProbeOperations()
        XCTAssertEqual(model.storageAccessProbeState, .idle)
        await probe.succeed(with: late)
        await request.value

        XCTAssertEqual(model.storageAccessProbeState, .idle)
        await model.requestBroaderStorageAnalysis()
        let probeCount = await probe.probeCount
        XCTAssertEqual(probeCount, 1)
    }

    func testAccessibilityIdentifiersAreStableAndUnique() {
        XCTAssertEqual(
            StorageAccessAccessibility.introduction,
            "storage-access-introduction"
        )
        XCTAssertEqual(
            Set(StorageAccessAccessibility.allIdentifiers).count,
            StorageAccessAccessibility.allIdentifiers.count
        )
        XCTAssertTrue(
            Set(StorageAccessAccessibility.allIdentifiers)
                .isDisjoint(with: ExplorerAccessibility.allIdentifiers)
        )
    }

    private func presentation(
        scanState: AppScanState,
        requested: Bool = false,
        probeState: StorageAccessProbeState = .idle
    ) -> StorageAccessOnboardingPresentation {
        StorageAccessOnboardingPresentation.make(
            scanState: scanState,
            broaderAnalysisRequested: requested,
            probeState: probeState,
            locale: en
        )
    }

    private func scanState(coverage: AppScanCoverage) -> AppScanState {
        let summary = AppScanSummary(
            scanID: "scan-access",
            startedAt: Date(timeIntervalSince1970: 1),
            completedAt: Date(timeIntervalSince1970: 2),
            progress: ScanProgressFacts(
                files: 1,
                directories: 1,
                knownAllocatedBytes: 1,
                issueCount: coverage == .complete ? 0 : 1
            ),
            logicalBytes: 1,
            coverage: coverage,
            coveragePermille: coverage == .complete ? 1_000 : nil,
            snapshotAvailable: true
        )
        return AppScanState(phase: .succeeded(summary), lastSuccessful: summary)
    }

    private func evidence(
        readable: UInt8,
        unreadable: UInt8,
        unobserved: UInt8
    ) -> StorageAccessEvidence {
        try! XCTUnwrap(
            StorageAccessEvidence(
                readableLocationCount: readable,
                unreadableLocationCount: unreadable,
                unobservedLocationCount: unobserved,
                observedAt: Date(timeIntervalSince1970: 10)
            )
        )
    }
}

@MainActor
private final class StorageAccessIntroductionStoreSpy:
    StorageAccessIntroductionPreferenceStoring
{
    private let acknowledged: Bool
    private(set) var saveCount = 0

    init(acknowledged: Bool) {
        self.acknowledged = acknowledged
    }

    func loadAcknowledged() -> Bool { acknowledged }
    func saveAcknowledged() { saveCount += 1 }
}

private actor StorageAccessSequenceProbe: StorageAccessProbing {
    private var results: [Result<StorageAccessEvidence, Error>]
    private(set) var probeCount = 0

    init(results: [Result<StorageAccessEvidence, Error>]) {
        self.results = results
    }

    func probe() async throws -> StorageAccessEvidence {
        probeCount += 1
        guard !results.isEmpty else {
            throw StorageAccessTestError.failed
        }
        return try results.removeFirst().get()
    }
}

private actor ControllableStorageAccessProbe: StorageAccessProbing {
    private var continuation: CheckedContinuation<StorageAccessEvidence, Error>?
    private var waiters: [CheckedContinuation<Void, Never>] = []
    private(set) var probeCount = 0

    func probe() async throws -> StorageAccessEvidence {
        probeCount += 1
        for waiter in waiters {
            waiter.resume()
        }
        waiters.removeAll(keepingCapacity: false)
        return try await withCheckedThrowingContinuation { continuation in
            self.continuation = continuation
        }
    }

    func waitForProbeCount(_ expected: Int) async {
        guard probeCount < expected else {
            return
        }
        await withCheckedContinuation { continuation in
            waiters.append(continuation)
        }
    }

    func succeed(with evidence: StorageAccessEvidence) {
        continuation?.resume(returning: evidence)
        continuation = nil
    }
}

private final class StorageAccessLocationRecorder: @unchecked Sendable {
    private let lock = NSLock()
    private var recordedPaths: [String] = []
    private var recordedMainThreadObservations = 0

    var paths: [String] {
        lock.withLock { recordedPaths }
    }

    var mainThreadObservations: Int {
        lock.withLock { recordedMainThreadObservations }
    }

    func record(_ location: URL) {
        lock.withLock {
            recordedPaths.append(location.path)
            if Thread.isMainThread {
                recordedMainThreadObservations += 1
            }
        }
    }
}

private enum StorageAccessTestError: Error {
    case failed
}
