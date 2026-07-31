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

    func testTerminalQuiescenceJoinsCancelDiscardBeforeServiceClose() async {
        let discardGate = CLIInstallerCallGate()
        let service = CLIInstallerServiceSpy(
            status: cliStatus(.absent),
            discardGate: discardGate
        )
        let model = CLIInstallationModel(service: service)
        await model.loadStatus()
        await model.prepare(.install)

        let cancellation = Task { @MainActor in
            await model.cancelPreparedAction()
        }
        await discardGate.waitUntilStarted()
        let quiescence = Task { @MainActor in
            await model.quiesceForTerminalRuntime()
        }
        await Task.yield()

        var calls = await service.calls()
        XCTAssertEqual(calls.close, 0)

        await discardGate.release()
        await cancellation.value
        _ = await quiescence.value

        calls = await service.calls()
        XCTAssertEqual(calls.discarded.count, 1)
        XCTAssertEqual(calls.close, 1)
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
        let gate = CLIInstallerCallGate()
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

    func testTerminalQuiescenceJoinsEveryConfirmedMutationBeforeServiceClose() async {
        let cases: [(CLIInstallationAction, CLIInstallationStatus)] = [
            (.install, cliStatus(.absent)),
            (.upgrade, cliManagedStatus(relation: .older)),
            (.reinstall, cliManagedStatus(relation: .current)),
            (.uninstall, cliManagedStatus(relation: .current)),
        ]

        for (action, initial) in cases {
            let performGate = CLIInstallerCallGate()
            let closeGate = CLIInstallerCallGate()
            let service = CLIInstallerServiceSpy(
                status: initial,
                performGate: performGate,
                closeGate: closeGate
            )
            await service.configureUpdate(
                CLIInstallationUpdate(status: initial, changed: false)
            )
            let model = CLIInstallationModel(service: service)
            await model.loadStatus()
            await model.prepare(action)
            let mutation = Task { @MainActor in
                await model.confirmPreparedAction()
            }
            await performGate.waitUntilStarted()

            let completion = CLIInstallerCompletionFlag()
            let quiescence = Task { @MainActor in
                let proof = await model.quiesceForTerminalRuntime()
                await completion.markComplete()
                return proof
            }
            await Task.yield()
            var calls = await service.calls()
            XCTAssertEqual(calls.close, 0, "\(action)")

            await performGate.release()
            await closeGate.waitUntilStarted()
            let completedBeforeClose = await completion.isComplete()
            XCTAssertFalse(completedBeforeClose, "\(action)")
            calls = await service.calls()
            XCTAssertEqual(calls.perform, 1, "\(action)")
            XCTAssertEqual(calls.close, 1, "\(action)")

            await closeGate.release()
            await mutation.value
            _ = await quiescence.value
            let completedAfterClose = await completion.isComplete()
            XCTAssertTrue(completedAfterClose, "\(action)")
        }
    }

    func testTerminalQuiescenceCoalescesCancelledAndConcurrentCallersThroughClose() async {
        let closeGate = CLIInstallerCallGate()
        let service = CLIInstallerServiceSpy(
            status: cliStatus(.absent),
            closeGate: closeGate
        )
        let model = CLIInstallationModel(service: service)
        let firstCompletion = CLIInstallerCompletionFlag()
        let secondCompletion = CLIInstallerCompletionFlag()

        let first = Task { @MainActor in
            let proof = await model.quiesceForTerminalRuntime()
            await firstCompletion.markComplete()
            return proof
        }
        await closeGate.waitUntilStarted()
        first.cancel()
        let second = Task { @MainActor in
            let proof = await model.quiesceForTerminalRuntime()
            await secondCompletion.markComplete()
            return proof
        }
        await Task.yield()

        let firstCompletedBeforeClose = await firstCompletion.isComplete()
        let secondCompletedBeforeClose = await secondCompletion.isComplete()
        var calls = await service.calls()
        XCTAssertFalse(firstCompletedBeforeClose)
        XCTAssertFalse(secondCompletedBeforeClose)
        XCTAssertEqual(calls.close, 1)

        await closeGate.release()
        _ = await first.value
        _ = await second.value
        await model.shutdown()

        let firstCompletedAfterClose = await firstCompletion.isComplete()
        let secondCompletedAfterClose = await secondCompletion.isComplete()
        calls = await service.calls()
        XCTAssertTrue(firstCompletedAfterClose)
        XCTAssertTrue(secondCompletedAfterClose)
        XCTAssertEqual(calls.close, 1)
    }

    func testTerminalQuiescenceWinningRejectsConfirmationAndDiscardsItOnce() async {
        let closeGate = CLIInstallerCallGate()
        let service = CLIInstallerServiceSpy(
            status: cliStatus(.absent),
            closeGate: closeGate
        )
        let model = CLIInstallationModel(service: service)
        await model.loadStatus()
        await model.prepare(.install)
        let token = try? XCTUnwrap(model.confirmation).token

        let quiescence = Task { @MainActor in
            await model.quiesceForTerminalRuntime()
        }
        await closeGate.waitUntilStarted()
        await model.confirmPreparedAction()

        var calls = await service.calls()
        XCTAssertEqual(calls.perform, 0)
        XCTAssertEqual(calls.discarded, token.map { [$0] } ?? [])
        XCTAssertNil(model.confirmation)

        await closeGate.release()
        _ = await quiescence.value
        calls = await service.calls()
        XCTAssertEqual(calls.discarded, token.map { [$0] } ?? [])
        XCTAssertEqual(calls.close, 1)
    }

    func testTerminalQuiescenceJoinsLatePreparationAndDiscardsCapabilityOnce() async {
        let prepareGate = CLIInstallerCallGate()
        let service = CLIInstallerServiceSpy(
            status: cliStatus(.absent),
            prepareGate: prepareGate
        )
        let model = CLIInstallationModel(service: service)
        await model.loadStatus()
        let preparation = Task { @MainActor in
            await model.prepare(.install)
        }
        await prepareGate.waitUntilStarted()

        let quiescence = Task { @MainActor in
            await model.quiesceForTerminalRuntime()
        }
        await Task.yield()
        var calls = await service.calls()
        XCTAssertEqual(calls.close, 0)

        await prepareGate.release()
        await preparation.value
        _ = await quiescence.value

        calls = await service.calls()
        XCTAssertEqual(calls.prepare, [.install])
        XCTAssertEqual(calls.perform, 0)
        XCTAssertEqual(calls.discarded.count, 1)
        XCTAssertEqual(calls.close, 1)
        XCTAssertNil(model.confirmation)
    }

    func testTerminalQuiescenceNeverRetriesConfirmedMutationFailures() async {
        enum FailureCase {
            case typed
            case outcomeUnknown
            case unexpected
        }

        for failure in [FailureCase.typed, .outcomeUnknown, .unexpected] {
            let performGate = CLIInstallerCallGate()
            let service = CLIInstallerServiceSpy(
                status: cliStatus(.absent),
                performGate: performGate
            )
            switch failure {
            case .typed:
                await service.configurePerformError(.retryable)
            case .outcomeUnknown:
                await service.configurePerformError(.outcomeUnknown)
            case .unexpected:
                await service.configureUnexpectedPerformError(true)
            }
            let model = CLIInstallationModel(service: service)
            await model.loadStatus()
            await model.prepare(.install)
            let mutation = Task { @MainActor in
                await model.confirmPreparedAction()
            }
            await performGate.waitUntilStarted()
            let quiescence = Task { @MainActor in
                await model.quiesceForTerminalRuntime()
            }

            await performGate.release()
            await mutation.value
            _ = await quiescence.value

            let calls = await service.calls()
            XCTAssertEqual(calls.perform, 1)
            XCTAssertEqual(calls.close, 1)
        }
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

private func cliManagedStatus(
    relation: CLIInstalledVersionRelation
) -> CLIInstallationStatus {
    cliStatus(
        .managed(
            CLIManagedInstallation(
                version: CLIVersion(relation == .older ? "0.4.0" : "0.5.0")!,
                sha256: Data(repeating: 0xCD, count: 32),
                relation: relation
            )
        )
    )
}

private actor CLIInstallerCompletionFlag {
    private var complete = false

    func markComplete() {
        complete = true
    }

    func isComplete() -> Bool {
        complete
    }
}

private actor CLIInstallerCallGate {
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
    private let prepareGate: CLIInstallerCallGate?
    private let performGate: CLIInstallerCallGate?
    private let discardGate: CLIInstallerCallGate?
    private let closeGate: CLIInstallerCallGate?
    private var recorded = Calls()
    private var prepared: CLIInstallationConfirmation?

    init(
        status: CLIInstallationStatus,
        prepareGate: CLIInstallerCallGate? = nil,
        performGate: CLIInstallerCallGate? = nil,
        discardGate: CLIInstallerCallGate? = nil,
        closeGate: CLIInstallerCallGate? = nil
    ) {
        self.status = status
        self.prepareGate = prepareGate
        self.performGate = performGate
        self.discardGate = discardGate
        self.closeGate = closeGate
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
    ) async throws -> CLIInstallationConfirmation {
        recorded.prepare.append(action)
        if let prepareGate {
            await prepareGate.beginAndWaitForRelease()
        }
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

    func discard(_ confirmation: CLIInstallationConfirmation) async {
        recorded.discarded.append(confirmation.token)
        if let discardGate {
            await discardGate.beginAndWaitForRelease()
        }
        if prepared == confirmation {
            prepared = nil
        }
    }

    func close() async {
        recorded.close += 1
        if let closeGate {
            await closeGate.beginAndWaitForRelease()
        }
        prepared = nil
    }
}

private struct CLIInstallerUnexpectedTestError: Error {}
