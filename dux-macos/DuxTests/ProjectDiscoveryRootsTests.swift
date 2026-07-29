import Foundation
import XCTest
@testable import DUX

final class ProjectDiscoveryRootModelTests: XCTestCase {
    func testFileURLRetainsExactNormalizedUnixBytes() throws {
        let root = try XCTUnwrap(
            ProjectDiscoveryRoot(
                fileURL: URL(filePath: "/Users/example/Projects", directoryHint: .isDirectory)
            )
        )

        XCTAssertEqual(root.encoding, .unixBytes)
        XCTAssertEqual(root.encodedBytes, Data("/Users/example/Projects".utf8))
        XCTAssertEqual(root.displayText, "/Users/example/Projects")
    }

    @MainActor
    func testSettingsAccessibilityAndMessagesStayStable() {
        let identifiers = ProjectDiscoveryRootsAccessibility.allStaticControlIdentifiers
        XCTAssertEqual(Set(identifiers).count, identifiers.count)
        XCTAssertNotEqual(
            ProjectDiscoveryRootsAccessibility.remove(0),
            ProjectDiscoveryRootsAccessibility.remove(1)
        )
        XCTAssertFalse(
            DuxSettingsView.message(for: ProjectDiscoveryRootsFailure.overlappingSelection)
                .isEmpty
        )
        XCTAssertFalse(
            DuxSettingsView.message(
                for: ProjectDiscoveryRootsFailure.service(.invalidResponse)
            ).isEmpty
        )
    }

    func testRootRelativeAmbiguousAndTrailingSlashShapesReject() {
        for bytes in ["/", "relative", "/a/", "/a//b", "/a/./b", "/a/../b"] {
            XCTAssertFalse(
                ProjectDiscoveryRoot.hasValidUnixShape(Data(bytes.utf8)),
                "accepted \(bytes)"
            )
        }
    }

    func testComponentAwareOverlapDoesNotUseSubstringPrefixes() {
        let project = observed("/Users/example/project")
        XCTAssertTrue(project.overlaps(observed("/Users/example/project/subfolder")))
        XCTAssertTrue(project.overlaps(project))
        XCTAssertFalse(project.overlaps(observed("/Users/example/project-copy")))
        XCTAssertFalse(project.overlaps(observed("/Users/example/other")))
    }

    func testNonUTF8UnixPathHasLosslessEscapedPresentation() {
        let bytes = Data([0x2F, 0x74, 0x6D, 0x70, 0x2F, 0xFF, 0x5C])
        let root = ProjectDiscoveryRoot(encoding: .unixBytes, encodedBytes: bytes)

        XCTAssertEqual(root.encodedBytes, bytes)
        XCTAssertEqual(root.displayText, "unix-bytes:/tmp/\\xff\\\\")
        XCTAssertEqual(root.id, root)
    }

    private func observed(_ path: String) -> ProjectDiscoveryRoot {
        ProjectDiscoveryRoot(encoding: .unixBytes, encodedBytes: Data(path.utf8))
    }
}

final class ProjectDiscoveryRootsEngineServiceTests: XCTestCase {
    func testRealEngineRoundTripsCanonicalRootsNoopAndResetOffMainThread() async throws {
        let fixture = try ProjectDiscoveryRootsEngineFixture()
        let service = EngineService(engine: fixture.engine)
        let alpha = observed("/Users/example/Alpha")
        let zeta = observed("/Users/example/Zeta")

        let initial = try await service.loadProjectDiscoveryRoots()
        XCTAssertEqual(initial.roots, [])
        XCTAssertEqual(initial.source, .default)
        XCTAssertEqual(initial.revision, 0)
        XCTAssertNil(initial.updatedAtUnixMilliseconds)

        let stored = try await service.setProjectDiscoveryRoots([zeta, alpha])
        XCTAssertTrue(stored.changed)
        XCTAssertEqual(stored.roots.roots, [alpha, zeta])
        XCTAssertEqual(stored.roots.source, .stored)
        XCTAssertEqual(stored.roots.revision, 1)
        XCTAssertNotNil(stored.roots.updatedAtUnixMilliseconds)

        let noChange = try await service.setProjectDiscoveryRoots([alpha, zeta])
        XCTAssertFalse(noChange.changed)
        XCTAssertEqual(noChange.roots, stored.roots)

        let reset = try await service.resetProjectDiscoveryRoots()
        XCTAssertTrue(reset.changed)
        XCTAssertEqual(reset.roots.roots, [])
        XCTAssertEqual(reset.roots.source, .default)
        XCTAssertEqual(reset.roots.revision, 0)
        XCTAssertNil(reset.roots.updatedAtUnixMilliseconds)

        let nonUTF8 = ProjectDiscoveryRoot(
            encoding: .unixBytes,
            encodedBytes: Data([0x2F, 0x74, 0x6D, 0x70, 0x2F, 0x70, 0x72, 0x6F, 0x6A, 0x65,
                                0x63, 0x74, 0x2D, 0xFF])
        )
        let lossless = try await service.setProjectDiscoveryRoots([nonUTF8])
        XCTAssertEqual(lossless.roots.roots, [nonUTF8])
        let reloadedLossless = try await service.loadProjectDiscoveryRoots()
        XCTAssertEqual(reloadedLossless.roots, [nonUTF8])

        let closed = await service.close()
        XCTAssertTrue(closed)
        do {
            _ = try await service.loadProjectDiscoveryRoots()
            XCTFail("Expected a closed project-root registry")
        } catch let error as ProjectDiscoveryRootsServiceError {
            XCTAssertEqual(error, .closed)
        }
    }

    private func observed(_ path: String) -> ProjectDiscoveryRoot {
        ProjectDiscoveryRoot(encoding: .unixBytes, encodedBytes: Data(path.utf8))
    }
}

@MainActor
final class ProjectDiscoveryRootsAppModelTests: XCTestCase {
    func testAddRejectsNestedRootWithoutCallingService() async {
        let configured = observed("/Users/example/Projects")
        let service = ProjectDiscoveryRootsEngineSpy(roots: [configured])
        let model = AppModel(engineService: service)
        await model.loadProjectDiscoveryRoots()

        await model.addProjectDiscoveryRoot(observed("/Users/example/Projects/DUX"))

        XCTAssertEqual(model.projectDiscoveryRootsState, .failed(.overlappingSelection))
        XCTAssertEqual(model.projectDiscoveryRoots?.roots, [configured])
        let setCount = await service.setRequestCount()
        XCTAssertEqual(setCount, 0)
    }

    func testAddRemoveAndResetPublishCanonicalServiceResults() async {
        let later = observed("/Users/example/Zeta")
        let earlier = observed("/Users/example/Alpha")
        let service = ProjectDiscoveryRootsEngineSpy(roots: [later])
        let model = AppModel(engineService: service)
        await model.loadProjectDiscoveryRoots()

        await model.addProjectDiscoveryRoot(earlier)
        XCTAssertEqual(model.projectDiscoveryRoots?.roots, [earlier, later])

        await model.removeProjectDiscoveryRoot(later)
        XCTAssertEqual(model.projectDiscoveryRoots?.roots, [earlier])

        await model.removeProjectDiscoveryRoot(earlier)
        XCTAssertEqual(model.projectDiscoveryRoots?.roots, [])
        XCTAssertEqual(model.projectDiscoveryRoots?.source, .stored)

        await model.resetProjectDiscoveryRoots()
        XCTAssertEqual(model.projectDiscoveryRoots?.roots, [])
        XCTAssertEqual(model.projectDiscoveryRoots?.source, .default)
        let setCount = await service.setRequestCount()
        let resetCount = await service.resetRequestCount()
        XCTAssertEqual(setCount, 3)
        XCTAssertEqual(resetCount, 1)
    }

    func testInvalidationRejectsLateLoadPublication() async {
        let service = ProjectDiscoveryRootsEngineSpy(
            roots: [observed("/Users/example/Projects")]
        )
        await service.suspendNextLoad()
        let model = AppModel(engineService: service)

        let load = Task { @MainActor in
            await model.loadProjectDiscoveryRoots()
        }
        await service.waitForLoadRequest()
        model.invalidateProjectDiscoveryRootsOperations()
        await service.completeSuspendedLoad()
        await load.value

        XCTAssertNil(model.projectDiscoveryRoots)
        XCTAssertEqual(model.projectDiscoveryRootsState, .idle)
    }

    func testAmbiguousMutationBlocksFurtherEditsUntilAuthoritativeReload() async {
        let alpha = observed("/Users/example/Alpha")
        let beta = observed("/Users/example/Beta")
        let gamma = observed("/Users/example/Gamma")
        let service = ProjectDiscoveryRootsEngineSpy(roots: [alpha])
        let model = AppModel(engineService: service)
        await model.loadProjectDiscoveryRoots()
        await service.commitNextSet(
            reporting: .outcomeUnknown,
            failAutomaticReload: true
        )

        await model.addProjectDiscoveryRoot(beta)

        XCTAssertEqual(
            model.projectDiscoveryRootsState,
            .failed(.service(.outcomeUnknown))
        )
        XCTAssertTrue(model.projectDiscoveryRootsRequiresAuthoritativeReload)
        XCTAssertEqual(model.projectDiscoveryRoots?.roots, [alpha])

        await model.addProjectDiscoveryRoot(gamma)
        let blockedSetCount = await service.setRequestCount()
        XCTAssertEqual(blockedSetCount, 1)

        await model.loadProjectDiscoveryRoots()
        XCTAssertFalse(model.projectDiscoveryRootsRequiresAuthoritativeReload)
        XCTAssertEqual(model.projectDiscoveryRootsState, .ready)
        XCTAssertEqual(model.projectDiscoveryRoots?.roots, [alpha, beta])
    }

    func testPostCommitProjectionFailureAutomaticallyReloadsAuthoritativeRoots() async {
        let alpha = observed("/Users/example/Alpha")
        let beta = observed("/Users/example/Beta")
        let service = ProjectDiscoveryRootsEngineSpy(roots: [alpha])
        let model = AppModel(engineService: service)
        await model.loadProjectDiscoveryRoots()
        await service.commitNextSet(reporting: .invalidResponse)

        await model.addProjectDiscoveryRoot(beta)

        XCTAssertFalse(model.projectDiscoveryRootsRequiresAuthoritativeReload)
        XCTAssertEqual(model.projectDiscoveryRootsState, .ready)
        XCTAssertEqual(model.projectDiscoveryRoots?.roots, [alpha, beta])
        let setCount = await service.setRequestCount()
        XCTAssertEqual(setCount, 1)
    }

    private func observed(_ path: String) -> ProjectDiscoveryRoot {
        ProjectDiscoveryRoot(encoding: .unixBytes, encodedBytes: Data(path.utf8))
    }
}

private final class ProjectDiscoveryRootsEngineFixture {
    let engine: DuxEngine

    private let root: URL

    init() throws {
        root = FileManager.default.temporaryDirectory.appending(
            path: "dux-project-roots-tests-\(UUID().uuidString)",
            directoryHint: .isDirectory
        )
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: false)
        engine = try DuxEngine(
            storage: EngineStorageRoots(
                dataRoot: root.appending(path: "data", directoryHint: .isDirectory).path,
                cacheRoot: root.appending(path: "cache", directoryHint: .isDirectory).path
            )
        )
    }

    deinit {
        _ = engine.close()
        // DUX-DESTRUCTIVE: allow=test-project-roots-fixture-remove -- remove only this UUID-named temporary fixture
        try? FileManager.default.removeItem(at: root)
    }
}

private actor ProjectDiscoveryRootsEngineSpy: EngineServing {
    private var roots: ProjectDiscoveryRoots
    private var setCount = 0
    private var resetCount = 0
    private var suspendLoad = false
    private var loadStarted = false
    private var committedSetError: ProjectDiscoveryRootsServiceError?
    private var failLoadAfterUncertainSet = false
    private var nextLoadFails = false
    private var loadWaiter: CheckedContinuation<Void, Never>?
    private var suspendedLoad: CheckedContinuation<ProjectDiscoveryRoots, Never>?

    init(roots: [ProjectDiscoveryRoot]) {
        self.roots = ProjectDiscoveryRoots(
            roots: roots,
            source: roots.isEmpty ? .default : .stored,
            revision: roots.isEmpty ? 0 : 1,
            updatedAtUnixMilliseconds: roots.isEmpty ? nil : 1
        )
    }

    func loadStatus() async throws -> EngineStatus {
        EngineStatus(libraryVersion: "test", ffiContractVersion: 37, executedOffMainThread: true)
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

    func loadProjectDiscoveryRoots() async throws -> ProjectDiscoveryRoots {
        loadStarted = true
        loadWaiter?.resume()
        loadWaiter = nil
        if suspendLoad {
            suspendLoad = false
            return await withCheckedContinuation { continuation in
                suspendedLoad = continuation
            }
        }
        if nextLoadFails {
            nextLoadFails = false
            throw ProjectDiscoveryRootsServiceError.unavailable
        }
        return roots
    }

    func setProjectDiscoveryRoots(
        _ newRoots: [ProjectDiscoveryRoot]
    ) async throws -> ProjectDiscoveryRootsUpdateResult {
        setCount += 1
        let sorted = newRoots.sorted {
            $0.encodedBytes.lexicographicallyPrecedes($1.encodedBytes)
        }
        roots = ProjectDiscoveryRoots(
            roots: sorted,
            source: .stored,
            revision: roots.revision + 1,
            updatedAtUnixMilliseconds: Int64(roots.revision + 1)
        )
        if let error = committedSetError {
            committedSetError = nil
            nextLoadFails = failLoadAfterUncertainSet
            failLoadAfterUncertainSet = false
            throw error
        }
        return ProjectDiscoveryRootsUpdateResult(roots: roots, changed: true)
    }

    func resetProjectDiscoveryRoots() async throws -> ProjectDiscoveryRootsUpdateResult {
        resetCount += 1
        roots = ProjectDiscoveryRoots(
            roots: [],
            source: .default,
            revision: 0,
            updatedAtUnixMilliseconds: nil
        )
        return ProjectDiscoveryRootsUpdateResult(roots: roots, changed: true)
    }

    func setRequestCount() -> Int { setCount }
    func resetRequestCount() -> Int { resetCount }

    func commitNextSet(
        reporting error: ProjectDiscoveryRootsServiceError,
        failAutomaticReload: Bool = false
    ) {
        committedSetError = error
        failLoadAfterUncertainSet = failAutomaticReload
    }

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
        suspendedLoad?.resume(returning: roots)
        suspendedLoad = nil
    }
}
