import XCTest
@testable import DUX

final class PermanentCleanupPolicyServiceTests: XCTestCase {
    func testRealPolicyRoundTripIsTypedAndExecutedOffMainActor() async throws {
        let root = FileManager.default.temporaryDirectory
            .appending(path: "dux-permanent-cleanup-service-\(UUID().uuidString)", directoryHint: .isDirectory)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: false)
        defer {
            // DUX-DESTRUCTIVE: allow=test-swift-storage-roots-fixture-remove -- remove only this UUID-named temporary root
            try? FileManager.default.removeItem(at: root)
        }

        let engine = try DuxEngine(
            storage: EngineStorageRoots(
                dataRoot: root.appending(path: "data", directoryHint: .isDirectory).path,
                cacheRoot: root.appending(path: "cache", directoryHint: .isDirectory).path
            )
        )
        let service = EngineService(engine: engine)

        let initial = try await service.loadPermanentCleanupPolicy()
        XCTAssertTrue(initial.enabled)
        XCTAssertEqual(initial.source, .default)
        XCTAssertEqual(initial.revision, 0)
        XCTAssertNil(initial.updatedAtUnixMilliseconds)

        let disabled = try await service.setPermanentCleanupEnabled(false)
        XCTAssertFalse(disabled.policy.enabled)
        XCTAssertEqual(disabled.policy.source, .stored)
        XCTAssertTrue(disabled.changed)

        let unchanged = try await service.setPermanentCleanupEnabled(false)
        XCTAssertFalse(unchanged.changed)
        XCTAssertEqual(unchanged.policy, disabled.policy)

        let reset = try await service.resetPermanentCleanup()
        XCTAssertTrue(reset.policy.enabled)
        XCTAssertEqual(reset.policy.source, .default)
        XCTAssertTrue(reset.changed)

        let closed = await service.close()
        XCTAssertTrue(closed)
        do {
            _ = try await service.loadPermanentCleanupPolicy()
            XCTFail("Expected a closed policy service")
        } catch let error as PermanentCleanupPolicyServiceError {
            XCTAssertEqual(error, .closed)
        }
    }

    func testMalformedDefaultStateIsRejectedFailClosed() async {
        let service = EngineService(engine: InvalidPermanentCleanupPolicyEngine())

        do {
            _ = try await service.loadPermanentCleanupPolicy()
            XCTFail("Expected malformed permanent-cleanup response rejection")
        } catch let error as PermanentCleanupPolicyServiceError {
            XCTAssertEqual(error, .invalidResponse)
        } catch {
            XCTFail("Unexpected error: \(error)")
        }
    }
}

private final class InvalidPermanentCleanupPolicyEngine: DuxEngine, @unchecked Sendable {
    required init(unsafeFromHandle handle: UInt64) {
        super.init(unsafeFromHandle: handle)
    }

    init() {
        super.init(noHandle: NoHandle())
    }

    override func getPermanentCleanupPolicy() throws -> PermanentCleanupPolicyStatus {
        PermanentCleanupPolicyStatus(
            recordVersion: 1,
            enabled: false,
            source: .default,
            revision: 0,
            updatedAtUnixMs: nil
        )
    }
}
