import Foundation
import XCTest
@testable import DUX

final class DirectCargoEnrollmentModelTests: XCTestCase {
    func testURLSelectionPreservesNativeBytesAndRejectsUnsafeShapes() {
        let selected = DirectCargoExecutableSelection(
            fileURL: URL(fileURLWithPath: "/opt/toolchain/bin/cargo")
        )
        XCTAssertEqual(selected?.encodedPathBytes, Data("/opt/toolchain/bin/cargo".utf8))
        XCTAssertEqual(selected?.displayPath, "/opt/toolchain/bin/cargo")

        XCTAssertFalse(valid(Data("relative/cargo".utf8)))
        XCTAssertFalse(valid(Data("/opt/toolchain/bin/rustc".utf8)))
        XCTAssertFalse(valid(Data("/opt//bin/cargo".utf8)))
        XCTAssertFalse(valid(Data("/opt/../bin/cargo".utf8)))
        XCTAssertFalse(valid(Data("/opt/\u{7f}/cargo".utf8)))
        XCTAssertFalse(valid(Data([0x2F, 0xFF, 0x2F] + Array("cargo".utf8))))
        XCTAssertFalse(
            valid(Data(repeating: UInt8(ascii: "a"), count: 32 * 1_024 + 1))
        )
    }

    @MainActor
    func testSettingsAccessibilityAndFailureMessagesStayStable() {
        let identifiers = DirectCargoEnrollmentAccessibility.allControlIdentifiers
        XCTAssertEqual(Set(identifiers).count, identifiers.count)
        XCTAssertFalse(
            DuxSettingsView.message(
                for: DirectCargoEnrollmentFailure.enrollmentConfirmationRequired
            ).isEmpty
        )
        XCTAssertFalse(
            DuxSettingsView.message(
                for: DirectCargoEnrollmentFailure.enrollmentPreviewChanged
            ).isEmpty
        )
        XCTAssertFalse(
            DuxSettingsView.message(
                for: DirectCargoEnrollmentFailure.service(.outcomeUnknown)
            ).isEmpty
        )
        XCTAssertNotEqual(
            DuxSettingsView.message(
                for: DirectCargoEnrollmentFailure.service(.outcomeUnknown)
            ),
            DuxSettingsView.message(
                for: DirectCargoEnrollmentFailure.service(.outcomeUnknown),
                authoritativeReloadRequired: false
            )
        )
        XCTAssertFalse(
            DuxSettingsView.message(
                for: DirectCargoEnrollmentFailure.service(.invalidCodeSignature)
            ).isEmpty
        )
    }

    private func valid(_ bytes: Data) -> Bool {
        DirectCargoExecutableSelection.isValidUnixCargoPath(bytes)
    }
}

final class DirectCargoEnrollmentServiceTests: XCTestCase {
    func testInspectionUsesExactBytesOffMainAndReleaseIsExplicit() async throws {
        let selection = cargoSelection("/opt/toolchain/bin/cargo")
        let preview = FakeDirectCargoPreviewSession(info: previewInfo(selection))
        let engine = FakeDirectCargoEngine(
            status: notEnrolledStatus(),
            preview: preview
        )
        let service = EngineService(engine: engine)

        let lease = try await service.inspectDirectCargoExecutable(selection)

        XCTAssertEqual(lease.preview.executable, selection)
        XCTAssertEqual(engine.inspectionRequest?.executablePathBytes, selection.encodedPathBytes)
        XCTAssertEqual(engine.inspectionRequest?.pathEncoding, .unixBytes)
        XCTAssertFalse(engine.inspectionCalledOnMain)
        XCTAssertEqual(preview.releaseCount, 0)
        await lease.release()
        XCTAssertEqual(preview.releaseCount, 1)
    }

    func testMismatchedInspectionResponseIsRejectedAndReleased() async {
        let selected = cargoSelection("/opt/selected/bin/cargo")
        let returned = cargoSelection("/opt/different/bin/cargo")
        let preview = FakeDirectCargoPreviewSession(info: previewInfo(returned))
        let service = EngineService(
            engine: FakeDirectCargoEngine(
                status: notEnrolledStatus(),
                preview: preview
            )
        )

        await assertServiceError(.invalidResponse) {
            _ = try await service.inspectDirectCargoExecutable(selected)
        }
        XCTAssertEqual(preview.releaseCount, 1)
    }

    func testStatusMappingRejectsMalformedResponseShapes() async {
        let validIdentity = enrolledIdentity()
        let unsafeSignature = DirectCargoCodeSignature(
            recordVersion: 1,
            class: .adHoc,
            flags: 0x2,
            codeDirectoryHashes: [
                DirectCargoCodeDirectoryHash(
                    recordVersion: 1,
                    bytes: Data(repeating: 1, count: 20)
                ),
            ],
            signingIdentifier: "cargo\nspoof",
            teamIdentifier: nil,
            designatedRequirementSha256: nil
        )
        let invalid = [
            DirectCargoEnrollmentStatus(
                recordVersion: 2,
                revision: 0,
                state: .notEnrolled,
                identity: nil,
                updatedAtUnixMs: nil
            ),
            DirectCargoEnrollmentStatus(
                recordVersion: 1,
                revision: 0,
                state: .notEnrolled,
                identity: validIdentity,
                updatedAtUnixMs: nil
            ),
            DirectCargoEnrollmentStatus(
                recordVersion: 1,
                revision: 1,
                state: .revoked,
                identity: nil,
                updatedAtUnixMs: nil
            ),
            enrolledStatus(
                identity: DirectCargoEnrollmentIdentity(
                    recordVersion: 1,
                    executablePath: validIdentity.executablePath,
                    executableSha256: validIdentity.executableSha256,
                    versionSha256: validIdentity.versionSha256,
                    cargoMajor: 1,
                    cargoMinor: 97,
                    cargoPatch: 0,
                    codeSignature: validIdentity.codeSignature
                )
            ),
            enrolledStatus(
                identity: DirectCargoEnrollmentIdentity(
                    recordVersion: 1,
                    executablePath: validIdentity.executablePath,
                    executableSha256: validIdentity.executableSha256,
                    versionSha256: validIdentity.versionSha256,
                    cargoMajor: 1,
                    cargoMinor: 96,
                    cargoPatch: 0,
                    codeSignature: unsafeSignature
                )
            ),
        ]

        for response in invalid {
            let service = EngineService(
                engine: FakeDirectCargoEngine(
                    status: response,
                    preview: FakeDirectCargoPreviewSession(
                        info: previewInfo(cargoSelection("/opt/toolchain/bin/cargo"))
                    )
                )
            )
            await assertServiceError(.invalidResponse) {
                _ = try await service.loadDirectCargoEnrollmentStatus()
            }
        }
    }

    func testCommitAndRevokeRejectResponsesThatDoNotMatchTheirExactOperations() async throws {
        let selection = cargoSelection("/opt/toolchain/bin/cargo")
        let preview = FakeDirectCargoPreviewSession(info: previewInfo(selection))
        let mismatchedIdentity = DirectCargoEnrollmentIdentity(
            recordVersion: 1,
            executablePath: DirectCargoExecutablePath(
                encoding: .unixBytes,
                encodedBytes: Data("/opt/toolchain/bin/cargo".utf8)
            ),
            executableSha256: Data(repeating: 0x99, count: 32),
            versionSha256: Data(repeating: 0x22, count: 32),
            cargoMajor: 1,
            cargoMinor: 96,
            cargoPatch: 0,
            codeSignature: codeSignature()
        )
        let engine = FakeDirectCargoEngine(
            status: notEnrolledStatus(),
            preview: preview,
            commitUpdate: DirectCargoEnrollmentUpdate(
                recordVersion: 1,
                status: enrolledStatus(identity: mismatchedIdentity),
                changed: true
            ),
            revokeUpdate: DirectCargoEnrollmentUpdate(
                recordVersion: 1,
                status: notEnrolledStatus(),
                changed: false
            )
        )
        let service = EngineService(engine: engine)
        let lease = try await service.inspectDirectCargoExecutable(selection)

        await assertServiceError(.outcomeUnknown) {
            _ = try await service.enrollDirectCargo(lease)
        }
        XCTAssertFalse(engine.commitCalledOnMain)
        await assertServiceError(.outcomeUnknown) {
            _ = try await service.revokeDirectCargoEnrollment()
        }
        XCTAssertFalse(engine.revokeCalledOnMain)
    }

    func testLinkedStaticInspectionDoesNotRunUnsignedSelectedBytes() async throws {
        let root = URL(fileURLWithPath: "/private/tmp", isDirectory: true)
            .appending(
                path: "dux-cargo-static-inspection-\(UUID().uuidString)",
                directoryHint: .isDirectory
            )
        try FileManager.default.createDirectory(
            at: root,
            withIntermediateDirectories: false
        )
        defer {
            // DUX-DESTRUCTIVE: allow=test-swift-cargo-inspection-fixture-remove -- remove only this UUID-named Cargo inspection test root
            try? FileManager.default.removeItem(at: root)
        }
        let sentinel = root.appending(path: "executed", directoryHint: .notDirectory)
        let executable = root.appending(path: "cargo", directoryHint: .notDirectory)
        let script = "#!/bin/sh\n/usr/bin/touch '\(sentinel.path)'\n"
        XCTAssertTrue(
            FileManager.default.createFile(
                atPath: executable.path,
                contents: Data(script.utf8),
                attributes: [.posixPermissions: 0o755]
            )
        )
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
        guard let selection = DirectCargoExecutableSelection(fileURL: executable) else {
            return XCTFail("Expected a valid lossless selection")
        }

        await assertServiceError(.invalidCodeSignature) {
            _ = try await service.inspectDirectCargoExecutable(selection)
        }
        XCTAssertFalse(FileManager.default.fileExists(atPath: sentinel.path))
        _ = await service.close()
    }

    private func assertServiceError(
        _ expected: DirectCargoEnrollmentServiceError,
        operation: () async throws -> Void,
        file: StaticString = #filePath,
        line: UInt = #line
    ) async {
        do {
            try await operation()
            XCTFail("Expected \(expected)", file: file, line: line)
        } catch let error as DirectCargoEnrollmentServiceError {
            XCTAssertEqual(error, expected, file: file, line: line)
        } catch {
            XCTFail("Unexpected error: \(error)", file: file, line: line)
        }
    }

    private func cargoSelection(_ path: String) -> DirectCargoExecutableSelection {
        DirectCargoExecutableSelection(encodedPathBytes: Data(path.utf8))
    }

    private func previewInfo(
        _ selection: DirectCargoExecutableSelection
    ) -> DirectCargoEnrollmentPreviewInfo {
        DirectCargoEnrollmentPreviewInfo(
            recordVersion: 1,
            executablePath: DirectCargoExecutablePath(
                encoding: .unixBytes,
                encodedBytes: selection.encodedPathBytes
            ),
            executableSha256: Data(repeating: 0x11, count: 32),
            codeSignature: codeSignature()
        )
    }

    private func codeSignature() -> DirectCargoCodeSignature {
        DirectCargoCodeSignature(
            recordVersion: 1,
            class: .adHoc,
            flags: 0x2,
            codeDirectoryHashes: [
                DirectCargoCodeDirectoryHash(
                    recordVersion: 1,
                    bytes: Data(repeating: 0x33, count: 20)
                ),
            ],
            signingIdentifier: "cargo-test",
            teamIdentifier: nil,
            designatedRequirementSha256: nil
        )
    }

    private func enrolledIdentity() -> DirectCargoEnrollmentIdentity {
        DirectCargoEnrollmentIdentity(
            recordVersion: 1,
            executablePath: DirectCargoExecutablePath(
                encoding: .unixBytes,
                encodedBytes: Data("/opt/toolchain/bin/cargo".utf8)
            ),
            executableSha256: Data(repeating: 0x11, count: 32),
            versionSha256: Data(repeating: 0x22, count: 32),
            cargoMajor: 1,
            cargoMinor: 96,
            cargoPatch: 0,
            codeSignature: codeSignature()
        )
    }

    private func notEnrolledStatus() -> DirectCargoEnrollmentStatus {
        DirectCargoEnrollmentStatus(
            recordVersion: 1,
            revision: 0,
            state: .notEnrolled,
            identity: nil,
            updatedAtUnixMs: nil
        )
    }

    private func enrolledStatus(
        identity: DirectCargoEnrollmentIdentity
    ) -> DirectCargoEnrollmentStatus {
        DirectCargoEnrollmentStatus(
            recordVersion: 1,
            revision: 1,
            state: .enrolled,
            identity: identity,
            updatedAtUnixMs: 1
        )
    }
}

@MainActor
final class DirectCargoEnrollmentAppModelTests: XCTestCase {
    func testInspectionRequiresExplicitEnrollmentAndRevocationConfirmation() async {
        let service = DirectCargoEnrollmentEngineSpy()
        let model = AppModel(engineService: service)
        let selection = cargoSelection("/opt/toolchain/bin/cargo")

        await model.loadDirectCargoEnrollmentStatus()
        XCTAssertEqual(model.directCargoEnrollmentStatus?.disposition, .notEnrolled)

        await model.inspectDirectCargoExecutable(selection)
        XCTAssertEqual(
            model.directCargoEnrollmentState,
            .awaitingEnrollmentConfirmation
        )
        XCTAssertEqual(model.directCargoEnrollmentPreview?.executable, selection)
        guard let confirmation = model.directCargoEnrollmentConfirmation else {
            return XCTFail("Expected exact enrollment confirmation evidence")
        }
        let inspectionCount = await service.inspectionRequestCount()
        XCTAssertEqual(inspectionCount, 1)

        await model.enrollInspectedDirectCargo()
        XCTAssertEqual(
            model.directCargoEnrollmentState,
            .failed(.enrollmentConfirmationRequired)
        )
        var enrollmentCount = await service.enrollmentRequestCount()
        XCTAssertEqual(enrollmentCount, 0)

        await model.enrollInspectedDirectCargo(confirmation: confirmation)
        XCTAssertEqual(model.directCargoEnrollmentState, .ready)
        guard case let .enrolled(identity)? =
            model.directCargoEnrollmentStatus?.disposition
        else {
            return XCTFail("Expected enrolled identity")
        }
        XCTAssertEqual(identity.executable, selection)
        XCTAssertNil(model.directCargoEnrollmentPreview)
        enrollmentCount = await service.enrollmentRequestCount()
        XCTAssertEqual(enrollmentCount, 1)
        let releasesAfterEnrollment = await service.previewReleaseCount()
        XCTAssertEqual(releasesAfterEnrollment, 1)

        await model.revokeDirectCargoEnrollment()
        XCTAssertEqual(
            model.directCargoEnrollmentState,
            .failed(.revocationConfirmationRequired)
        )
        var revokeCount = await service.revocationRequestCount()
        XCTAssertEqual(revokeCount, 0)

        await model.revokeDirectCargoEnrollment(confirmed: true)
        XCTAssertEqual(model.directCargoEnrollmentStatus?.disposition, .revoked)
        XCTAssertEqual(model.directCargoEnrollmentState, .ready)
        revokeCount = await service.revocationRequestCount()
        XCTAssertEqual(revokeCount, 1)
    }

    func testDiscardReleasesPreviewAndStatusReloadPreservesPendingReview() async {
        let service = DirectCargoEnrollmentEngineSpy()
        let model = AppModel(engineService: service)
        await model.loadDirectCargoEnrollmentStatus()
        await model.inspectDirectCargoExecutable(cargoSelection("/usr/local/bin/cargo"))

        await model.loadDirectCargoEnrollmentStatus()
        XCTAssertEqual(
            model.directCargoEnrollmentState,
            .awaitingEnrollmentConfirmation
        )
        XCTAssertNotNil(model.directCargoEnrollmentPreview)
        let loadCount = await service.statusRequestCount()
        XCTAssertEqual(loadCount, 1)

        await model.discardDirectCargoEnrollmentPreview()
        XCTAssertNil(model.directCargoEnrollmentPreview)
        XCTAssertEqual(model.directCargoEnrollmentState, .ready)
        let releaseCount = await service.previewReleaseCount()
        XCTAssertEqual(releaseCount, 1)
    }

    func testTerminalQuiescenceJoinsDiscardAfterPreviewSlotIsCleared() async {
        let service = DirectCargoEnrollmentEngineSpy()
        let model = AppModel(engineService: service)
        await model.inspectDirectCargoExecutable(
            cargoSelection("/usr/local/bin/cargo")
        )
        await service.suspendNextPreviewRelease()

        let discard = Task { @MainActor in
            await model.discardDirectCargoEnrollmentPreview()
        }
        await service.waitForPreviewRelease()
        XCTAssertNil(model.directCargoEnrollmentPreview)

        let completion = DirectCargoTerminalCompletionProbe()
        let terminal = Task { @MainActor in
            await model.quiesceForTerminalRuntime()
            await completion.finish()
        }
        await Task.yield()
        let completedBeforeRelease = await completion.count()
        XCTAssertEqual(completedBeforeRelease, 0)

        await service.completePreviewRelease()
        await discard.value
        await terminal.value
        let completedAfterRelease = await completion.count()
        XCTAssertEqual(completedAfterRelease, 1)
    }

    func testConfirmationRejectsPreviewReplacementAfterEvidenceWasDisplayed() async {
        let service = DirectCargoEnrollmentEngineSpy()
        let model = AppModel(engineService: service)
        let first = cargoSelection("/opt/first/bin/cargo")
        let second = cargoSelection("/opt/second/bin/cargo")

        await model.inspectDirectCargoExecutable(first)
        guard let displayedConfirmation = model.directCargoEnrollmentConfirmation else {
            return XCTFail("Expected confirmation for the displayed preview")
        }
        await model.inspectDirectCargoExecutable(second)

        await model.enrollInspectedDirectCargo(confirmation: displayedConfirmation)

        XCTAssertEqual(
            model.directCargoEnrollmentState,
            .failed(.enrollmentPreviewChanged)
        )
        XCTAssertEqual(model.directCargoEnrollmentPreview?.executable, second)
        XCTAssertNotEqual(
            model.directCargoEnrollmentConfirmation,
            displayedConfirmation
        )
        let enrollmentCount = await service.enrollmentRequestCount()
        XCTAssertEqual(enrollmentCount, 0)
        let releaseCount = await service.previewReleaseCount()
        XCTAssertEqual(releaseCount, 1)
    }

    func testInvalidationSuppressesLateInspectionAndReleasesItsPreview() async {
        let service = DirectCargoEnrollmentEngineSpy()
        await service.suspendNextInspection()
        let model = AppModel(engineService: service)

        let inspection = Task { @MainActor in
            await model.inspectDirectCargoExecutable(
                cargoSelection("/opt/rust/bin/cargo")
            )
        }
        await service.waitForInspectionRequest()
        model.invalidateDirectCargoEnrollmentOperations()
        await service.completeSuspendedInspection()
        await inspection.value

        XCTAssertNil(model.directCargoEnrollmentPreview)
        XCTAssertNil(model.directCargoEnrollmentStatus)
        XCTAssertEqual(model.directCargoEnrollmentState, .idle)
        let releaseCount = await service.previewReleaseCount()
        XCTAssertEqual(releaseCount, 1)
    }

    func testSettingsDismissalReleasesEventualSuspendedInspectionPreview() async {
        let service = DirectCargoEnrollmentEngineSpy()
        await service.suspendNextInspection()
        let model = AppModel(engineService: service)

        let inspection = Task { @MainActor in
            await model.inspectDirectCargoExecutable(
                cargoSelection("/opt/rust/bin/cargo")
            )
        }
        await service.waitForInspectionRequest()
        let dismissal = Task { @MainActor in
            await model.dismissDirectCargoEnrollmentPresentation()
        }
        await Task.yield()
        await service.completeSuspendedInspection()
        await dismissal.value
        await inspection.value

        XCTAssertNil(model.directCargoEnrollmentPreview)
        XCTAssertNil(model.directCargoEnrollmentConfirmation)
        XCTAssertEqual(model.directCargoEnrollmentState, .idle)
        let releaseCount = await service.previewReleaseCount()
        XCTAssertEqual(releaseCount, 1)
    }

    func testOutcomeUnknownConsumesPreviewAndReloadsStatusWithoutRetry() async {
        let service = DirectCargoEnrollmentEngineSpy()
        let model = AppModel(engineService: service)
        let selection = cargoSelection("/opt/toolchain/bin/cargo")
        await model.loadDirectCargoEnrollmentStatus()
        await model.inspectDirectCargoExecutable(selection)
        guard let confirmation = model.directCargoEnrollmentConfirmation else {
            return XCTFail("Expected exact enrollment confirmation evidence")
        }
        await service.failNextEnrollmentWithOutcomeUnknown()

        await model.enrollInspectedDirectCargo(confirmation: confirmation)

        XCTAssertEqual(
            model.directCargoEnrollmentState,
            .failed(.service(.outcomeUnknown))
        )
        guard case .enrolled? = model.directCargoEnrollmentStatus?.disposition else {
            return XCTFail("Expected the one authoritative reload to publish enrollment")
        }
        XCTAssertFalse(model.directCargoEnrollmentNeedsStatusReload)
        XCTAssertNil(model.directCargoEnrollmentPreview)
        let enrollCount = await service.enrollmentRequestCount()
        XCTAssertEqual(enrollCount, 1)
        let statusCount = await service.statusRequestCount()
        XCTAssertEqual(statusCount, 2)
        let releaseCount = await service.previewReleaseCount()
        XCTAssertEqual(releaseCount, 1)
    }

    func testRevokeOutcomeUnknownBlocksRetryUntilAuthoritativeReloadSucceeds() async {
        let service = DirectCargoEnrollmentEngineSpy()
        let model = AppModel(engineService: service)
        await model.loadDirectCargoEnrollmentStatus()
        await model.inspectDirectCargoExecutable(
            cargoSelection("/opt/toolchain/bin/cargo")
        )
        guard let confirmation = model.directCargoEnrollmentConfirmation else {
            return XCTFail("Expected exact enrollment confirmation evidence")
        }
        await model.enrollInspectedDirectCargo(confirmation: confirmation)
        await service.failNextRevocationWithOutcomeUnknown()
        await service.failNextStatusLoads(1)

        await model.revokeDirectCargoEnrollment(confirmed: true)

        XCTAssertEqual(
            model.directCargoEnrollmentState,
            .failed(.service(.outcomeUnknown))
        )
        XCTAssertTrue(model.directCargoEnrollmentNeedsStatusReload)
        var revokeCount = await service.revocationRequestCount()
        XCTAssertEqual(revokeCount, 1)

        await model.revokeDirectCargoEnrollment(confirmed: true)
        revokeCount = await service.revocationRequestCount()
        XCTAssertEqual(revokeCount, 1)
        await model.inspectDirectCargoExecutable(
            cargoSelection("/opt/retry/bin/cargo")
        )
        let inspectionCount = await service.inspectionRequestCount()
        XCTAssertEqual(inspectionCount, 1)
        XCTAssertEqual(
            model.directCargoEnrollmentState,
            .failed(.service(.outcomeUnknown))
        )

        await model.loadDirectCargoEnrollmentStatus()
        XCTAssertEqual(model.directCargoEnrollmentStatus?.disposition, .revoked)
        XCTAssertEqual(model.directCargoEnrollmentState, .ready)
        XCTAssertFalse(model.directCargoEnrollmentNeedsStatusReload)
        let statusCount = await service.statusRequestCount()
        XCTAssertEqual(statusCount, 3)
    }

    func testReplacementReleasesPriorPreviewAndShutdownReleasesCurrentPreview() async {
        let service = DirectCargoEnrollmentEngineSpy()
        let model = AppModel(engineService: service)

        await model.inspectDirectCargoExecutable(cargoSelection("/opt/first/bin/cargo"))
        await model.inspectDirectCargoExecutable(cargoSelection("/opt/second/bin/cargo"))

        XCTAssertEqual(
            model.directCargoEnrollmentPreview?.executable,
            cargoSelection("/opt/second/bin/cargo")
        )
        var releaseCount = await service.previewReleaseCount()
        XCTAssertEqual(releaseCount, 1)

        await model.shutdownDirectCargoEnrollment()
        XCTAssertNil(model.directCargoEnrollmentPreview)
        XCTAssertEqual(model.directCargoEnrollmentState, .idle)
        releaseCount = await service.previewReleaseCount()
        XCTAssertEqual(releaseCount, 2)
    }

    func testConcurrentRuntimeShutdownSharesPipelineWhileConfirmedCommitIsSuspended() async {
        let service = DirectCargoEnrollmentEngineSpy()
        let model = AppModel(engineService: service)
        let maintenance = DirectCargoRuntimeMaintenanceSpy()
        let capacity = DirectCargoRuntimeCapacitySpy()
        let reviews = DirectCargoRuntimeReviewSpy()
        let scans = DirectCargoRuntimeScanSpy()
        let runtime = AppRuntime(
            model: model,
            engineService: service,
            scheduler: maintenance,
            capacityScheduler: capacity,
            reviews: reviews,
            scans: scans
        )
        await model.inspectDirectCargoExecutable(
            cargoSelection("/opt/toolchain/bin/cargo")
        )
        guard let confirmation = model.directCargoEnrollmentConfirmation else {
            return XCTFail("Expected exact enrollment confirmation evidence")
        }
        await service.suspendNextEnrollment()
        let enrollment = Task { @MainActor in
            await model.enrollInspectedDirectCargo(confirmation: confirmation)
        }
        await service.waitForEnrollmentRequest()

        let firstShutdown = Task { @MainActor in
            await runtime.shutdown()
        }
        for _ in 0 ..< 100 where model.directCargoEnrollmentPreview != nil {
            await Task.yield()
        }

        XCTAssertNil(model.directCargoEnrollmentPreview)
        XCTAssertNil(model.directCargoEnrollmentConfirmation)
        let closesBeforeCommitFinished = await service.engineCloseCount()
        XCTAssertEqual(closesBeforeCommitFinished, 0)

        let secondShutdown = Task { @MainActor in
            await runtime.shutdown()
        }
        await Task.yield()
        await service.completeSuspendedEnrollment()
        await enrollment.value
        await firstShutdown.value
        await secondShutdown.value

        let enrollmentCount = await service.enrollmentRequestCount()
        XCTAssertEqual(enrollmentCount, 1)
        let closeCount = await service.engineCloseCount()
        XCTAssertEqual(closeCount, 1)
        let maintenanceStops = await maintenance.stopCount()
        XCTAssertEqual(maintenanceStops, 1)
        let capacityStops = await capacity.stopCount()
        XCTAssertEqual(capacityStops, 1)
        let reviewShutdowns = await reviews.shutdownCount()
        XCTAssertEqual(reviewShutdowns, 1)
        XCTAssertEqual(scans.shutdownCount, 1)
        XCTAssertEqual(model.directCargoEnrollmentState, .idle)
    }

    private func cargoSelection(_ path: String) -> DirectCargoExecutableSelection {
        DirectCargoExecutableSelection(encodedPathBytes: Data(path.utf8))
    }
}

private final class FakeDirectCargoPreviewSession:
    DirectCargoEnrollmentPreviewSession, @unchecked Sendable
{
    private let returnedInfo: DirectCargoEnrollmentPreviewInfo
    private(set) var releaseCount = 0

    required init(unsafeFromHandle handle: UInt64) {
        fatalError("Unsupported test initializer: \(handle)")
    }

    init(info: DirectCargoEnrollmentPreviewInfo) {
        returnedInfo = info
        super.init(noHandle: NoHandle())
    }

    override func info() throws -> DirectCargoEnrollmentPreviewInfo {
        returnedInfo
    }

    override func release() throws -> DirectCargoEnrollmentPreviewReleaseOutcome {
        releaseCount += 1
        return releaseCount == 1 ? .released : .alreadyUnavailable
    }
}

private final class FakeDirectCargoEngine: DuxEngine, @unchecked Sendable {
    private let returnedStatus: DirectCargoEnrollmentStatus
    private let returnedPreview: DirectCargoEnrollmentPreviewSession
    private let returnedCommitUpdate: DirectCargoEnrollmentUpdate?
    private let returnedRevokeUpdate: DirectCargoEnrollmentUpdate?

    private(set) var inspectionRequest: DirectCargoEnrollmentInspectionRequest?
    private(set) var inspectionCalledOnMain = false
    private(set) var commitCalledOnMain = false
    private(set) var revokeCalledOnMain = false

    required init(unsafeFromHandle handle: UInt64) {
        fatalError("Unsupported test initializer: \(handle)")
    }

    init(
        status: DirectCargoEnrollmentStatus,
        preview: DirectCargoEnrollmentPreviewSession,
        commitUpdate: DirectCargoEnrollmentUpdate? = nil,
        revokeUpdate: DirectCargoEnrollmentUpdate? = nil
    ) {
        returnedStatus = status
        returnedPreview = preview
        returnedCommitUpdate = commitUpdate
        returnedRevokeUpdate = revokeUpdate
        super.init(noHandle: NoHandle())
    }

    override func directCargoEnrollmentStatus() throws -> DirectCargoEnrollmentStatus {
        returnedStatus
    }

    override func inspectDirectCargoEnrollment(
        request: DirectCargoEnrollmentInspectionRequest
    ) throws -> DirectCargoEnrollmentPreviewSession {
        inspectionCalledOnMain = Thread.isMainThread
        inspectionRequest = request
        return returnedPreview
    }

    override func commitDirectCargoEnrollment(
        preview _: DirectCargoEnrollmentPreviewSession
    ) throws -> DirectCargoEnrollmentUpdate {
        commitCalledOnMain = Thread.isMainThread
        guard let returnedCommitUpdate else {
            throw DirectCargoEnrollmentError.InternalState
        }
        return returnedCommitUpdate
    }

    override func revokeDirectCargoEnrollment() throws -> DirectCargoEnrollmentUpdate {
        revokeCalledOnMain = Thread.isMainThread
        guard let returnedRevokeUpdate else {
            throw DirectCargoEnrollmentError.InternalState
        }
        return returnedRevokeUpdate
    }
}

private actor DirectCargoEnrollmentEngineSpy: EngineServing, DuxEngineClosing {
    private let tracker = DirectCargoPreviewReleaseTracker()
    private var status = DirectCargoEnrollmentStatusModel(
        revision: 0,
        disposition: .notEnrolled,
        updatedAtUnixMilliseconds: nil
    )
    private var statusCount = 0
    private var inspectCount = 0
    private var enrollCount = 0
    private var revokeCount = 0
    private var closeCount = 0
    private var suspendInspection = false
    private var suspendEnrollment = false
    private var nextEnrollmentIsOutcomeUnknown = false
    private var nextRevocationIsOutcomeUnknown = false
    private var statusLoadFailuresRemaining = 0
    private var inspectionStarted = false
    private var enrollmentStarted = false
    private var inspectionWaiter: CheckedContinuation<Void, Never>?
    private var enrollmentWaiter: CheckedContinuation<Void, Never>?
    private var suspendedInspection:
        CheckedContinuation<any DuxDirectCargoEnrollmentPreviewLease, Never>?
    private var suspendedEnrollment: CheckedContinuation<Void, Never>?

    func loadStatus() async throws -> EngineStatus {
        EngineStatus(libraryVersion: "test", ffiContractVersion: 27, executedOffMainThread: true)
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

    func loadDirectCargoEnrollmentStatus() async throws
        -> DirectCargoEnrollmentStatusModel
    {
        statusCount += 1
        if statusLoadFailuresRemaining > 0 {
            statusLoadFailuresRemaining -= 1
            throw DirectCargoEnrollmentServiceError.unavailable
        }
        return status
    }

    func inspectDirectCargoExecutable(
        _ selection: DirectCargoExecutableSelection
    ) async throws -> any DuxDirectCargoEnrollmentPreviewLease {
        inspectCount += 1
        inspectionStarted = true
        inspectionWaiter?.resume()
        inspectionWaiter = nil
        let preview = DirectCargoPreviewLease(
            preview: previewModel(selection),
            tracker: tracker
        )
        if suspendInspection {
            suspendInspection = false
            return await withCheckedContinuation { continuation in
                suspendedInspection = continuation
            }
        }
        return preview
    }

    func enrollDirectCargo(
        _ preview: any DuxDirectCargoEnrollmentPreviewLease
    ) async throws -> DirectCargoEnrollmentUpdateModel {
        enrollCount += 1
        enrollmentStarted = true
        enrollmentWaiter?.resume()
        enrollmentWaiter = nil
        if suspendEnrollment {
            suspendEnrollment = false
            await withCheckedContinuation { continuation in
                suspendedEnrollment = continuation
            }
        }
        let info = preview.preview
        status = DirectCargoEnrollmentStatusModel(
            revision: 1,
            disposition: .enrolled(
                DirectCargoEnrollmentIdentityModel(
                    executable: info.executable,
                    executableSHA256: info.executableSHA256,
                    versionSHA256: Data(repeating: 0x22, count: 32),
                    version: DirectCargoVersion(major: 1, minor: 96, patch: 0),
                    signature: info.signature
                )
            ),
            updatedAtUnixMilliseconds: 1
        )
        if nextEnrollmentIsOutcomeUnknown {
            nextEnrollmentIsOutcomeUnknown = false
            throw DirectCargoEnrollmentServiceError.outcomeUnknown
        }
        return DirectCargoEnrollmentUpdateModel(status: status, changed: true)
    }

    func revokeDirectCargoEnrollment() async throws -> DirectCargoEnrollmentUpdateModel {
        revokeCount += 1
        status = DirectCargoEnrollmentStatusModel(
            revision: status.revision + 1,
            disposition: .revoked,
            updatedAtUnixMilliseconds: 2
        )
        if nextRevocationIsOutcomeUnknown {
            nextRevocationIsOutcomeUnknown = false
            throw DirectCargoEnrollmentServiceError.outcomeUnknown
        }
        return DirectCargoEnrollmentUpdateModel(status: status, changed: true)
    }

    func close() async -> Bool {
        closeCount += 1
        return true
    }

    func statusRequestCount() -> Int { statusCount }
    func inspectionRequestCount() -> Int { inspectCount }
    func enrollmentRequestCount() -> Int { enrollCount }
    func revocationRequestCount() -> Int { revokeCount }
    func previewReleaseCount() async -> Int { await tracker.count() }
    func engineCloseCount() -> Int { closeCount }

    func suspendNextPreviewRelease() async {
        await tracker.suspendNextRelease()
    }

    func waitForPreviewRelease() async {
        await tracker.waitForRelease()
    }

    func completePreviewRelease() async {
        await tracker.completeRelease()
    }

    func suspendNextInspection() {
        suspendInspection = true
    }

    func failNextEnrollmentWithOutcomeUnknown() {
        nextEnrollmentIsOutcomeUnknown = true
    }

    func failNextRevocationWithOutcomeUnknown() {
        nextRevocationIsOutcomeUnknown = true
    }

    func failNextStatusLoads(_ count: Int) {
        statusLoadFailuresRemaining = count
    }

    func suspendNextEnrollment() {
        suspendEnrollment = true
    }

    func waitForInspectionRequest() async {
        guard !inspectionStarted else {
            return
        }
        await withCheckedContinuation { continuation in
            inspectionWaiter = continuation
        }
    }

    func completeSuspendedInspection() {
        suspendedInspection?.resume(
            returning: DirectCargoPreviewLease(
                preview: previewModel(
                    DirectCargoExecutableSelection(
                        encodedPathBytes: Data("/opt/rust/bin/cargo".utf8)
                    )
                ),
                tracker: tracker
            )
        )
        suspendedInspection = nil
    }

    func waitForEnrollmentRequest() async {
        guard !enrollmentStarted else {
            return
        }
        await withCheckedContinuation { continuation in
            enrollmentWaiter = continuation
        }
    }

    func completeSuspendedEnrollment() {
        suspendedEnrollment?.resume()
        suspendedEnrollment = nil
    }

    private func previewModel(
        _ selection: DirectCargoExecutableSelection
    ) -> DirectCargoEnrollmentPreviewModel {
        DirectCargoEnrollmentPreviewModel(
            executable: selection,
            executableSHA256: Data(repeating: 0x11, count: 32),
            signature: DirectCargoSignatureEvidence(
                kind: .adHoc,
                flags: 0x2,
                codeDirectoryHashes: [Data(repeating: 0x33, count: 20)],
                signingIdentifier: "cargo-test",
                teamIdentifier: nil,
                designatedRequirementSHA256: nil
            )
        )
    }
}

private final class DirectCargoPreviewLease:
    DuxDirectCargoEnrollmentPreviewLease, @unchecked Sendable
{
    let preview: DirectCargoEnrollmentPreviewModel
    private let tracker: DirectCargoPreviewReleaseTracker

    init(
        preview: DirectCargoEnrollmentPreviewModel,
        tracker: DirectCargoPreviewReleaseTracker
    ) {
        self.preview = preview
        self.tracker = tracker
    }

    func release() async {
        await tracker.recordRelease()
    }
}

private actor DirectCargoPreviewReleaseTracker {
    private var releases = 0
    private var shouldSuspendRelease = false
    private var releaseContinuation: CheckedContinuation<Void, Never>?
    private var releaseWaiters: [CheckedContinuation<Void, Never>] = []

    func recordRelease() async {
        releases += 1
        let waiters = releaseWaiters
        releaseWaiters.removeAll()
        for waiter in waiters {
            waiter.resume()
        }
        guard shouldSuspendRelease else {
            return
        }
        shouldSuspendRelease = false
        await withCheckedContinuation { continuation in
            releaseContinuation = continuation
        }
    }

    func count() -> Int {
        releases
    }

    func suspendNextRelease() {
        shouldSuspendRelease = true
    }

    func waitForRelease() async {
        guard releases == 0 else {
            return
        }
        await withCheckedContinuation { continuation in
            releaseWaiters.append(continuation)
        }
    }

    func completeRelease() {
        releaseContinuation?.resume()
        releaseContinuation = nil
    }
}

private actor DirectCargoTerminalCompletionProbe {
    private var completions = 0

    func finish() {
        completions += 1
    }

    func count() -> Int {
        completions
    }
}

private actor DirectCargoRuntimeMaintenanceSpy: DuxMaintenanceScheduling {
    private var stops = 0
    func start() async {}
    func signal(_: DuxMaintenanceTrigger) async {}
    func stop() async { stops += 1 }
    func quiesceForTerminalRuntime() async { await stop() }
    func stopCount() -> Int { stops }
}

private actor DirectCargoRuntimeCapacitySpy: DuxCapacityScheduling {
    private var stops = 0
    func start() async {}
    func signal(_: DuxCapacitySamplingTrigger) async {}
    func stop() async { stops += 1 }
    func quiesceForTerminalRuntime() async { await stop() }
    func stopCount() -> Int { stops }
}

private actor DirectCargoRuntimeReviewSpy: DuxReviewManaging {
    private var shutdowns = 0
    func renewNow() async {}
    func shutdown() async { shutdowns += 1 }
    func shutdownCount() -> Int { shutdowns }
}

@MainActor
private final class DirectCargoRuntimeScanSpy: DuxScanManaging {
    private(set) var shutdownCount = 0
    func shutdownTargetedReclaimScan() async {}
    func shutdownHomeScan() async { shutdownCount += 1 }
}
