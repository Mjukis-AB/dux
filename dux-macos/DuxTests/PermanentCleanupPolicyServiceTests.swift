import XCTest
@testable import DUX

final class PermanentCleanupPolicyServiceTests: XCTestCase {
    func testRealPolicyRoundTripIsTypedAndExecutedOffMainActor() async throws {
        let root = FileManager.default.temporaryDirectory
            .appending(path: "dux-permanent-cleanup-service-\(UUID().uuidString)", directoryHint: .isDirectory)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: false)
        defer {
            // DUX-DESTRUCTIVE: allow=test-swift-permanent-policy-fixture-remove -- remove only this UUID-named permanent-policy test root
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

        let initial = try await service.loadPermanentCleanupPolicy()
        XCTAssertFalse(initial.enabled)
        XCTAssertEqual(initial.source, .default)
        XCTAssertEqual(initial.revision, 0)
        XCTAssertNil(initial.updatedAtUnixMilliseconds)

        let enabled = try await service.setPermanentCleanupEnabled(true)
        XCTAssertTrue(enabled.policy.enabled)
        XCTAssertEqual(enabled.policy.source, .stored)
        XCTAssertTrue(enabled.changed)

        let unchanged = try await service.setPermanentCleanupEnabled(true)
        XCTAssertFalse(unchanged.changed)
        XCTAssertEqual(unchanged.policy, enabled.policy)

        let reset = try await service.resetPermanentCleanup()
        XCTAssertFalse(reset.policy.enabled)
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
        for revision: UInt64 in [0, 7] {
            let service = EngineService(
                engine: InvalidPermanentCleanupPolicyEngine(revision: revision)
            )

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
}

private final class InvalidPermanentCleanupPolicyEngine: DuxEngine, @unchecked Sendable {
    private let revision: UInt64

    required init(unsafeFromHandle handle: UInt64) {
        revision = 0
        super.init(unsafeFromHandle: handle)
    }

    init(revision: UInt64) {
        self.revision = revision
        super.init(noHandle: NoHandle())
    }

    override func getPermanentCleanupPolicy() throws -> PermanentCleanupPolicyStatus {
        PermanentCleanupPolicyStatus(
            recordVersion: 1,
            enabled: true,
            source: .default,
            revision: revision,
            updatedAtUnixMs: revision == 0 ? nil : 1
        )
    }
}
