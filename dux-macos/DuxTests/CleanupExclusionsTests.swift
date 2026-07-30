import Foundation
import XCTest
@testable import DUX

final class CleanupExclusionsModelAndServiceTests: XCTestCase {
    func testNonUTF8UnixPathHasLosslessEscapedPresentation() {
        let bytes = Data([0x2F, 0x74, 0x6D, 0x70, 0x2F, 0xFF, 0x5C])
        let path = CleanupExclusionPathObservation(
            encoding: .unixBytes,
            encodedBytes: bytes
        )

        XCTAssertEqual(path.encodedBytes, bytes)
        XCTAssertEqual(path.displayText, "unix-bytes:/tmp/\\xff\\\\")
        XCTAssertEqual(path.id, path)
    }

    func testRealServiceRoundTripPreservesBytesAndCanonicalOrdering() async throws {
        let root = FileManager.default.temporaryDirectory.appending(
            path: "dux-cleanup-exclusions-service-\(UUID().uuidString)",
            directoryHint: .isDirectory
        )
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: false)
        defer {
            // DUX-DESTRUCTIVE: allow=test-swift-cleanup-exclusions-fixture-remove -- remove only this UUID-named cleanup-exclusions test root
            try? FileManager.default.removeItem(at: root)
        }
        let engine = try DuxEngine(
            storage: EngineStorageRoots(
                dataRoot: root.appending(path: "data", directoryHint: .isDirectory).path,
                cacheRoot: root
                    .appending(path: "cache", directoryHint: .isDirectory)
                    .appending(path: "Dux", directoryHint: .isDirectory)
                    .path
            )
        )
        let service = EngineService(engine: engine)
        let later = CleanupExclusionPathObservation(
            encoding: .unixBytes,
            encodedBytes: Data("/Users/test/z".utf8)
        )
        let earlier = CleanupExclusionPathObservation(
            encoding: .unixBytes,
            encodedBytes: Data("/Applications".utf8)
        )

        let initial = try await service.loadCleanupExclusions()
        XCTAssertEqual(initial.source, .default)
        XCTAssertEqual(initial.paths, [])
        XCTAssertEqual(initial.revision, 0)

        let update = try await service.setCleanupExclusions([later, earlier])
        XCTAssertTrue(update.changed)
        XCTAssertEqual(update.exclusions.paths, [earlier, later])
        XCTAssertEqual(update.exclusions.source, .stored)
        let reloaded = try await service.loadCleanupExclusions()
        XCTAssertEqual(reloaded, update.exclusions)

        let reset = try await service.resetCleanupExclusions()
        XCTAssertTrue(reset.changed)
        XCTAssertEqual(reset.exclusions.source, .default)
        XCTAssertTrue(reset.exclusions.paths.isEmpty)
        let closed = await service.close()
        XCTAssertTrue(closed)
    }

    @MainActor
    func testSettingsAccessibilityAndMessagesStayStable() {
        let identifiers = CleanupExclusionsAccessibility.allStaticControlIdentifiers
        XCTAssertEqual(Set(identifiers).count, identifiers.count)
        XCTAssertNotEqual(CleanupExclusionsAccessibility.remove(0), CleanupExclusionsAccessibility.remove(1))
        XCTAssertFalse(
            DuxSettingsView.message(
                for: CleanupExclusionsFailure.confirmationRequired
            ).isEmpty
        )
        XCTAssertFalse(
            DuxSettingsView.message(
                for: CleanupExclusionsFailure.service(.invalidResponse)
            ).isEmpty
        )
    }
}

@MainActor
final class CleanupExclusionsAppModelTests: XCTestCase {
    func testRemovalAndResetRequireExplicitConfirmation() async {
        let first = observed("/Applications/Example.app")
        let second = observed("/Users/test/Library/Caches")
        let service = CleanupExclusionsEngineSpy(paths: [first, second])
        let model = AppModel(engineService: service)
        await model.loadCleanupExclusions()

        await model.removeCleanupExclusion(first)
        XCTAssertEqual(model.cleanupExclusionsState, .failed(.confirmationRequired))
        let initialSetCount = await service.setRequestCount()
        XCTAssertEqual(initialSetCount, 0)
        XCTAssertEqual(model.cleanupExclusions?.paths, [first, second])

        await model.removeCleanupExclusion(first, confirmed: true)
        XCTAssertEqual(model.cleanupExclusions?.paths, [second])
        let removalSetCount = await service.setRequestCount()
        XCTAssertEqual(removalSetCount, 1)

        await model.resetCleanupExclusions()
        XCTAssertEqual(model.cleanupExclusionsState, .failed(.confirmationRequired))
        let initialResetCount = await service.resetRequestCount()
        XCTAssertEqual(initialResetCount, 0)

        await model.resetCleanupExclusions(confirmed: true)
        XCTAssertEqual(model.cleanupExclusions?.source, .default)
        XCTAssertEqual(model.cleanupExclusions?.paths, [])
        let finalResetCount = await service.resetRequestCount()
        XCTAssertEqual(finalResetCount, 1)
    }

    func testAddPublishesCanonicalServiceResultWithoutLosingBytes() async {
        let first = observed("/z")
        let second = CleanupExclusionPathObservation(
            encoding: .unixBytes,
            encodedBytes: Data([0x2F, 0x61, 0xFF])
        )
        let service = CleanupExclusionsEngineSpy(paths: [first])
        let model = AppModel(engineService: service)
        await model.loadCleanupExclusions()

        await model.addCleanupExclusion(second)

        XCTAssertEqual(model.cleanupExclusions?.paths, [second, first])
        XCTAssertEqual(model.cleanupExclusions?.paths.first?.encodedBytes, second.encodedBytes)
    }

    func testInvalidationRejectsLateLoadPublication() async {
        let service = CleanupExclusionsEngineSpy(paths: [observed("/private")])
        await service.suspendNextLoad()
        let model = AppModel(engineService: service)

        let load = Task { @MainActor in
            await model.loadCleanupExclusions()
        }
        await service.waitForLoadRequest()
        model.invalidateCleanupExclusionsOperations()
        await service.completeSuspendedLoad()
        await load.value

        XCTAssertNil(model.cleanupExclusions)
        XCTAssertEqual(model.cleanupExclusionsState, .idle)
    }

    private func observed(_ path: String) -> CleanupExclusionPathObservation {
        CleanupExclusionPathObservation(encoding: .unixBytes, encodedBytes: Data(path.utf8))
    }
}

private actor CleanupExclusionsEngineSpy: EngineServing {
    private var exclusions: CleanupExclusionsPolicy
    private var setCount = 0
    private var resetCount = 0
    private var suspendLoad = false
    private var loadStarted = false
    private var loadWaiter: CheckedContinuation<Void, Never>?
    private var suspendedLoad: CheckedContinuation<CleanupExclusionsPolicy, Never>?

    init(paths: [CleanupExclusionPathObservation]) {
        exclusions = CleanupExclusionsPolicy(
            paths: paths,
            source: paths.isEmpty ? .default : .stored,
            revision: paths.isEmpty ? 0 : 1,
            updatedAtUnixMilliseconds: paths.isEmpty ? nil : 1
        )
    }

    func loadStatus() async throws -> EngineStatus {
        EngineStatus(libraryVersion: "test", ffiContractVersion: 25, executedOffMainThread: true)
    }

    func observeVolumeCapacity(
        _ snapshot: VolumeCapacitySnapshot
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
                revision: 0,
                configuration: .defaults,
                updatedAtUnixMilliseconds: nil
            ),
            changed: true
        )
    }

    func loadCleanupExclusions() async throws -> CleanupExclusionsPolicy {
        loadStarted = true
        loadWaiter?.resume()
        loadWaiter = nil
        if suspendLoad {
            suspendLoad = false
            return await withCheckedContinuation { continuation in
                suspendedLoad = continuation
            }
        }
        return exclusions
    }

    func setCleanupExclusions(
        _ paths: [CleanupExclusionPathObservation]
    ) async throws -> CleanupExclusionsUpdateResult {
        setCount += 1
        let sorted = paths.sorted {
            $0.encodedBytes.lexicographicallyPrecedes($1.encodedBytes)
        }
        exclusions = CleanupExclusionsPolicy(
            paths: sorted,
            source: .stored,
            revision: exclusions.revision + 1,
            updatedAtUnixMilliseconds: Int64(exclusions.revision + 1)
        )
        return CleanupExclusionsUpdateResult(exclusions: exclusions, changed: true)
    }

    func resetCleanupExclusions() async throws -> CleanupExclusionsUpdateResult {
        resetCount += 1
        exclusions = CleanupExclusionsPolicy(
            paths: [],
            source: .default,
            revision: 0,
            updatedAtUnixMilliseconds: nil
        )
        return CleanupExclusionsUpdateResult(exclusions: exclusions, changed: true)
    }

    func setRequestCount() -> Int { setCount }
    func resetRequestCount() -> Int { resetCount }

    func suspendNextLoad() {
        suspendLoad = true
    }

    func waitForLoadRequest() async {
        guard !loadStarted else {
            return
        }
        await withCheckedContinuation { continuation in
            loadWaiter = continuation
        }
    }

    func completeSuspendedLoad() {
        suspendedLoad?.resume(returning: exclusions)
        suspendedLoad = nil
    }
}
