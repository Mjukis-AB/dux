@testable import DUX
import XCTest

@MainActor
final class CLIInstallationModelTests: XCTestCase {
    func testLoadPublishesAuthoritativeStatusWithoutPreparingMutation() async {
        let status = cliStatus(.absent)
        let service = CLIInstallerServiceSpy(status: status)
        let model = CLIInstallationModel(service: service)

        await model.loadStatus()

        XCTAssertEqual(model.state.status, status)
        XCTAssertNil(model.state.activity)
        XCTAssertNil(model.state.failure)
        let calls = await service.calls()
        XCTAssertEqual(calls.load, 1)
        XCTAssertEqual(calls.prepare, [])
        XCTAssertEqual(calls.perform, 0)
    }

    func testInstallRequiresSeparatePreparationAndConfirmation() async {
        let initial = cliStatus(.absent)
        let installed = cliStatus(
            .managed(
                CLIManagedInstallation(
                    version: CLIVersion("0.5.0")!,
                    sha256: initial.bundled.sha256,
                    relation: .current
                )
            )
        )
        let service = CLIInstallerServiceSpy(status: initial)
        await service.configureUpdate(
            CLIInstallationUpdate(status: installed, changed: true)
        )
        let model = CLIInstallationModel(service: service)
        await model.loadStatus()

        await model.prepare(.install)

        XCTAssertEqual(model.confirmation?.action, .install)
        var calls = await service.calls()
        XCTAssertEqual(calls.perform, 0)

        await model.confirmPreparedAction()

        XCTAssertNil(model.confirmation)
        XCTAssertEqual(model.state.status, installed)
        XCTAssertFalse(model.state.requiresAuthoritativeReload)
        calls = await service.calls()
        XCTAssertEqual(calls.perform, 1)
    }

    func testCancelDiscardsPreparedCapabilityWithoutMutation() async {
        let service = CLIInstallerServiceSpy(status: cliStatus(.absent))
        let model = CLIInstallationModel(service: service)
        await model.loadStatus()
        await model.prepare(.install)
        let prepared = model.confirmation

        await model.cancelPreparedAction()

        XCTAssertNil(model.confirmation)
        let calls = await service.calls()
        XCTAssertEqual(calls.perform, 0)
        XCTAssertEqual(calls.discarded, prepared.map { [$0.token] } ?? [])
    }

    func testUnavailableActionNeverReachesService() async {
        let status = cliStatus(.unmanaged)
        let service = CLIInstallerServiceSpy(status: status)
        let model = CLIInstallationModel(service: service)
        await model.loadStatus()

        await model.prepare(.install)
        await model.prepare(.uninstall)

        XCTAssertNil(model.confirmation)
        let calls = await service.calls()
        XCTAssertEqual(calls.prepare, [])
    }

    func testOutcomeUnknownClearsPriorObservationAndRequiresReload() async {
        let initial = cliStatus(.absent)
        let service = CLIInstallerServiceSpy(status: initial)
        await service.configurePerformError(.outcomeUnknown)
        let model = CLIInstallationModel(service: service)
        await model.loadStatus()
        await model.prepare(.install)

        await model.confirmPreparedAction()

        XCTAssertNil(model.state.status)
        XCTAssertTrue(model.state.requiresAuthoritativeReload)
        XCTAssertEqual(model.state.failure, .service(.outcomeUnknown))

        await service.configurePerformError(nil)
        await model.loadStatus(force: true)
        XCTAssertEqual(model.state.status, initial)
        XCTAssertFalse(model.state.requiresAuthoritativeReload)
    }

    func testUnexpectedMutationFailureClearsPriorObservationAndRequiresReload() async {
        let initial = cliStatus(.absent)
        let service = CLIInstallerServiceSpy(status: initial)
        await service.configureUnexpectedPerformError(true)
        let model = CLIInstallationModel(service: service)
        await model.loadStatus()
        await model.prepare(.install)

        await model.confirmPreparedAction()

        XCTAssertNil(model.state.status)
        XCTAssertTrue(model.state.requiresAuthoritativeReload)
        XCTAssertEqual(model.state.failure, .unexpected)
    }

    func testInitialInspectionFailureCanBeRetried() async {
        let status = cliStatus(.absent)
        let service = CLIInstallerServiceSpy(status: status)
        await service.configureLoadError(.retryable)
        let model = CLIInstallationModel(service: service)

        await model.loadStatus()

        XCTAssertNil(model.state.status)
        XCTAssertEqual(model.state.failure, .service(.retryable))

        await service.configureLoadError(nil)
        await model.loadStatus(force: true)

        XCTAssertEqual(model.state.status, status)
        XCTAssertNil(model.state.failure)
    }

    func testCallerCancellationDoesNotAbandonConfirmedMutation() async {
        let initial = cliStatus(.absent)
        let installed = cliStatus(
            .managed(
                CLIManagedInstallation(
                    version: CLIVersion("0.5.0")!,
                    sha256: initial.bundled.sha256,
                    relation: .current
                )
            )
        )
        let gate = CLIInstallerPerformGate()
        let service = CLIInstallerServiceSpy(status: initial, performGate: gate)
        await service.configureUpdate(
            CLIInstallationUpdate(status: installed, changed: true)
        )
        let model = CLIInstallationModel(service: service)
        await model.loadStatus()
        await model.prepare(.install)

        let caller = Task { @MainActor in
            await model.confirmPreparedAction()
        }
        await gate.waitUntilStarted()
        caller.cancel()
        await gate.release()
        await caller.value

        XCTAssertEqual(model.state.status, installed)
        let calls = await service.calls()
        XCTAssertEqual(calls.perform, 1)
    }

    func testShutdownWaitsForConfirmedMutationThenClosesService() async {
        let initial = cliStatus(.absent)
        let gate = CLIInstallerPerformGate()
        let service = CLIInstallerServiceSpy(status: initial, performGate: gate)
        await service.configureUpdate(
            CLIInstallationUpdate(status: initial, changed: false)
        )
        let model = CLIInstallationModel(service: service)
        await model.loadStatus()
        await model.prepare(.install)
        let mutation = Task { @MainActor in
            await model.confirmPreparedAction()
        }
        await gate.waitUntilStarted()

        let shutdown = Task { @MainActor in
            await model.shutdown()
        }
        await Task.yield()
        var calls = await service.calls()
        XCTAssertEqual(calls.close, 0)

        await gate.release()
        await mutation.value
        await shutdown.value

        calls = await service.calls()
        XCTAssertEqual(calls.perform, 1)
        XCTAssertEqual(calls.close, 1)
    }

    func testPresentationCoversEveryDispositionAndAccessibilityIDsAreUnique() {
        let bundled = cliStatus(.absent).bundled
        let cases: [(CLIInstallationDisposition, String, CLIInstallationAction?)] = [
            (.absent, "Not installed", .install),
            (
                .managed(
                    CLIManagedInstallation(
                        version: CLIVersion("0.4.0")!,
                        sha256: Data(repeating: 1, count: 32),
                        relation: .older
                    )
                ),
                "Upgrade available",
                .upgrade
            ),
            (
                .managed(
                    CLIManagedInstallation(
                        version: CLIVersion("0.5.0")!,
                        sha256: bundled.sha256,
                        relation: .current
                    )
                ),
                "Up to date",
                .reinstall
            ),
            (
                .managed(
                    CLIManagedInstallation(
                        version: CLIVersion("0.5.0")!,
                        sha256: Data(repeating: 2, count: 32),
                        relation: .sameVersionDifferentBuild
                    )
                ),
                "Different build installed",
                .reinstall
            ),
            (
                .managed(
                    CLIManagedInstallation(
                        version: CLIVersion("0.6.0")!,
                        sha256: Data(repeating: 3, count: 32),
                        relation: .newer
                    )
                ),
                "Newer CLI installed",
                nil
            ),
            (.unmanaged, "Existing file not managed by DUX", nil),
            (.unsafe(.symbolicLink), "Unsafe destination", nil),
        ]

        for (disposition, title, action) in cases {
            let presentation = CLIInstallationPresentation.make(
                status: CLIInstallationStatus(
                    bundled: bundled,
                    disposition: disposition,
                    pathEnvironment: .missing
                ),
                locale: Locale(identifier: "en_US")
            )
            XCTAssertEqual(presentation.statusTitle, title)
            XCTAssertEqual(presentation.primaryAction, action)
            XCTAssertFalse(presentation.pathDetail.isEmpty)
        }

        XCTAssertEqual(
            Set(CLIInstallationAccessibility.allIdentifiers).count,
            CLIInstallationAccessibility.allIdentifiers.count
        )
    }
}

private func cliStatus(
    _ disposition: CLIInstallationDisposition
) -> CLIInstallationStatus {
    CLIInstallationStatus(
        bundled: CLIBundledMetadata(
            version: CLIVersion("0.5.0")!,
            databaseSchemaVersion: 16,
            snapshotFormatVersion: 1,
            sha256: Data(repeating: 0xAB, count: 32)
        ),
        disposition: disposition,
        pathEnvironment: .included
    )
}

private actor CLIInstallerPerformGate {
    private var started = false
    private var released = false
    private var startWaiters: [CheckedContinuation<Void, Never>] = []
    private var releaseWaiters: [CheckedContinuation<Void, Never>] = []

    func waitUntilStarted() async {
        if started {
            return
        }
        await withCheckedContinuation { continuation in
            startWaiters.append(continuation)
        }
    }

    func beginAndWaitForRelease() async {
        started = true
        let waiters = startWaiters
        startWaiters.removeAll()
        for waiter in waiters {
            waiter.resume()
        }
        if released {
            return
        }
        await withCheckedContinuation { continuation in
            releaseWaiters.append(continuation)
        }
    }

    func release() {
        released = true
        let waiters = releaseWaiters
        releaseWaiters.removeAll()
        for waiter in waiters {
            waiter.resume()
        }
    }
}

private actor CLIInstallerServiceSpy: CLIInstallerServing {
    struct Calls: Sendable {
        var load = 0
        var prepare: [CLIInstallationAction] = []
        var perform = 0
        var discarded: [UUID] = []
        var close = 0
    }

    private var status: CLIInstallationStatus
    private var loadError: CLIInstallerServiceError?
    private var update: CLIInstallationUpdate?
    private var performError: CLIInstallerServiceError?
    private var unexpectedPerformError = false
    private let performGate: CLIInstallerPerformGate?
    private var recorded = Calls()
    private var prepared: CLIInstallationConfirmation?

    init(
        status: CLIInstallationStatus,
        performGate: CLIInstallerPerformGate? = nil
    ) {
        self.status = status
        self.performGate = performGate
    }

    func configureUpdate(_ update: CLIInstallationUpdate) {
        self.update = update
    }

    func configureLoadError(_ error: CLIInstallerServiceError?) {
        loadError = error
    }

    func configurePerformError(_ error: CLIInstallerServiceError?) {
        performError = error
    }

    func configureUnexpectedPerformError(_ enabled: Bool) {
        unexpectedPerformError = enabled
    }

    func calls() -> Calls {
        recorded
    }

    func loadStatus() throws -> CLIInstallationStatus {
        recorded.load += 1
        if let loadError {
            throw loadError
        }
        return status
    }

    func prepare(
        _ action: CLIInstallationAction
    ) throws -> CLIInstallationConfirmation {
        recorded.prepare.append(action)
        let installedVersion: CLIVersion?
        switch status.disposition {
        case let .managed(installation):
            installedVersion = installation.version
        case .absent, .unmanaged, .unsafe:
            installedVersion = nil
        }
        let confirmation = CLIInstallationConfirmation(
            token: UUID(),
            action: action,
            bundledVersion: status.bundled.version,
            installedVersion: installedVersion,
            destinationDisplayText: CLIInstallationStatus.destinationDisplayText
        )
        prepared = confirmation
        return confirmation
    }

    func perform(
        _ confirmation: CLIInstallationConfirmation
    ) async throws -> CLIInstallationUpdate {
        recorded.perform += 1
        guard prepared == confirmation else {
            throw CLIInstallerServiceError.confirmationUnavailable
        }
        if let performGate {
            await performGate.beginAndWaitForRelease()
        }
        if let performError {
            throw performError
        }
        if unexpectedPerformError {
            throw CLIInstallerUnexpectedTestError()
        }
        let update = update ?? CLIInstallationUpdate(status: status, changed: false)
        status = update.status
        prepared = nil
        return update
    }

    func discard(_ confirmation: CLIInstallationConfirmation) {
        recorded.discarded.append(confirmation.token)
        if prepared == confirmation {
            prepared = nil
        }
    }

    func close() {
        recorded.close += 1
        prepared = nil
    }
}

private struct CLIInstallerUnexpectedTestError: Error {}
