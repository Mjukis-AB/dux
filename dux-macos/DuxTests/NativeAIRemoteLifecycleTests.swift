@testable import DUX
import Foundation
import XCTest

private final class NativeAITestResultRecorder: @unchecked Sendable {
    private let lock = NSLock()
    private var valuesStorage: [NativeAIRemoteLifecycleTestResult] = []

    func record(_ value: NativeAIRemoteLifecycleTestResult) {
        lock.withLock {
            valuesStorage.append(value)
        }
    }

    var values: [NativeAIRemoteLifecycleTestResult] {
        lock.withLock { valuesStorage }
    }
}

private final class NativeAIFakeScheduledAction: @unchecked Sendable,
    NativeAIRemoteLifecycleTestScheduledAction
{
    private weak var clock: NativeAIFakeClock?
    private let identifier: UInt64

    init(clock: NativeAIFakeClock, identifier: UInt64) {
        self.clock = clock
        self.identifier = identifier
    }

    func cancel() {
        clock?.cancel(identifier: identifier)
    }
}

private final class NativeAIFakeClock: @unchecked Sendable,
    NativeAIRemoteLifecycleTestClock
{
    private struct Scheduled {
        let identifier: UInt64
        let deadline: UInt64
        let action: @Sendable () -> Void
        var cancelled: Bool
    }

    private let lock = NSLock()
    private var nowStorage: UInt64
    private var nextIdentifier: UInt64 = 1
    private var scheduledStorage: [Scheduled] = []

    init(nowNanoseconds: UInt64 = 1_000) {
        nowStorage = nowNanoseconds
    }

    func nowNanoseconds() -> UInt64 {
        lock.withLock { nowStorage }
    }

    func schedule(
        at deadlineNanoseconds: UInt64,
        _ action: @escaping @Sendable () -> Void
    ) -> any NativeAIRemoteLifecycleTestScheduledAction {
        let identifier = lock.withLock { () -> UInt64 in
            let identifier = nextIdentifier
            nextIdentifier &+= 1
            scheduledStorage.append(
                Scheduled(
                    identifier: identifier,
                    deadline: deadlineNanoseconds,
                    action: action,
                    cancelled: false
                )
            )
            return identifier
        }
        return NativeAIFakeScheduledAction(clock: self, identifier: identifier)
    }

    func advance(by nanoseconds: UInt64) {
        let target = lock.withLock { () -> UInt64 in
            let (target, overflow) = nowStorage.addingReportingOverflow(nanoseconds)
            precondition(!overflow)
            return target
        }
        advance(to: target)
    }

    func advance(to target: UInt64) {
        while true {
            let action = lock.withLock { () -> (@Sendable () -> Void)? in
                precondition(target >= nowStorage)
                guard let index = scheduledStorage.indices
                    .filter({ !scheduledStorage[$0].cancelled })
                    .min(by: {
                        scheduledStorage[$0].deadline < scheduledStorage[$1].deadline
                    }), scheduledStorage[index].deadline <= target
                else {
                    nowStorage = target
                    return nil
                }
                nowStorage = scheduledStorage[index].deadline
                let action = scheduledStorage[index].action
                scheduledStorage[index].cancelled = true
                return action
            }
            guard let action else { return }
            action()
        }
    }

    func cancel(identifier: UInt64) {
        lock.withLock {
            guard let index = scheduledStorage.firstIndex(where: {
                $0.identifier == identifier
            }) else { return }
            scheduledStorage[index].cancelled = true
        }
    }

    var deadlines: [UInt64] {
        lock.withLock { scheduledStorage.map(\.deadline) }
    }
}

private final class NativeAIPreparationFake: @unchecked Sendable,
    NativeAIRemoteLifecycleTestPreparation
{
    private let lock = NSLock()
    private let immediateResult: Bool?
    private var completions: [@Sendable (Bool) -> Void] = []
    private var callCountStorage = 0

    init(immediateResult: Bool?) {
        self.immediateResult = immediateResult
    }

    func prepare(_ completion: @escaping @Sendable (Bool) -> Void) {
        let immediateResult = lock.withLock { () -> Bool? in
            callCountStorage += 1
            guard let result = self.immediateResult else {
                completions.append(completion)
                return nil
            }
            return result
        }
        if let immediateResult {
            completion(immediateResult)
        }
    }

    func complete(_ succeeded: Bool, index: Int = 0) {
        let completion = lock.withLock { completions[index] }
        completion(succeeded)
    }

    var callCount: Int {
        lock.withLock { callCountStorage }
    }
}

private final class NativeAIValidationFake: @unchecked Sendable,
    NativeAIRemoteLifecycleTestValidation
{
    private let lock = NSLock()
    private let immediateResult: Bool?
    private var completions: [@Sendable (Bool) -> Void] = []
    private var observationsStorage: [NativeAIRemoteLifecycleTestValidationObservation] = []

    init(immediateResult: Bool?) {
        self.immediateResult = immediateResult
    }

    func validate(
        _ observation: NativeAIRemoteLifecycleTestValidationObservation,
        completion: @escaping @Sendable (Bool) -> Void
    ) {
        let immediateResult = lock.withLock { () -> Bool? in
            observationsStorage.append(observation)
            guard let result = self.immediateResult else {
                completions.append(completion)
                return nil
            }
            return result
        }
        if let immediateResult {
            completion(immediateResult)
        }
    }

    func complete(_ succeeded: Bool, index: Int = 0) {
        let completion = lock.withLock { completions[index] }
        completion(succeeded)
    }

    var observations: [NativeAIRemoteLifecycleTestValidationObservation] {
        lock.withLock { observationsStorage }
    }
}

private final class NativeAINetworkTransactionFake: @unchecked Sendable,
    NativeAIRemoteLifecycleTestNetworkTransaction
{
    private let lock = NSLock()
    let events: NativeAIRemoteLifecycleTestNetworkEvents
    private var resumeCountStorage = 0
    private var cancelCountStorage = 0
    private var invalidateCountStorage = 0

    init(events: NativeAIRemoteLifecycleTestNetworkEvents) {
        self.events = events
    }

    func resume() {
        lock.withLock { resumeCountStorage += 1 }
    }

    func cancel() {
        lock.withLock { cancelCountStorage += 1 }
    }

    func invalidate() {
        lock.withLock { invalidateCountStorage += 1 }
    }

    var resumeCount: Int { lock.withLock { resumeCountStorage } }
    var cancelCount: Int { lock.withLock { cancelCountStorage } }
    var invalidateCount: Int { lock.withLock { invalidateCountStorage } }
}

private final class NativeAINetworkFactoryFake: @unchecked Sendable,
    NativeAIRemoteLifecycleTestNetworkFactory
{
    private let lock = NSLock()
    private var requestsStorage: [NativeAIRemoteLifecycleTestRequestObservation] = []
    private var transactionsStorage: [NativeAINetworkTransactionFake] = []

    func makeTransaction(
        request: NativeAIRemoteLifecycleTestRequestObservation,
        events: NativeAIRemoteLifecycleTestNetworkEvents
    ) -> any NativeAIRemoteLifecycleTestNetworkTransaction {
        let transaction = NativeAINetworkTransactionFake(events: events)
        lock.withLock {
            requestsStorage.append(request)
            transactionsStorage.append(transaction)
        }
        return transaction
    }

    var requests: [NativeAIRemoteLifecycleTestRequestObservation] {
        lock.withLock { requestsStorage }
    }

    var transactions: [NativeAINetworkTransactionFake] {
        lock.withLock { transactionsStorage }
    }

    var transactionCount: Int {
        lock.withLock { transactionsStorage.count }
    }

    var onlyTransaction: NativeAINetworkTransactionFake {
        lock.withLock {
            precondition(transactionsStorage.count == 1)
            return transactionsStorage[0]
        }
    }

    func releaseTransactions() {
        lock.withLock {
            transactionsStorage.removeAll()
        }
    }
}

private final class NativeAITestRig {
    let clock: NativeAIFakeClock
    let preparation: NativeAIPreparationFake
    let validation: NativeAIValidationFake
    let network: NativeAINetworkFactoryFake
    let recorder = NativeAITestResultRecorder()
    let bodyByteCount: Int
    private(set) var run: NativeAIRemoteLifecycleTestRun?

    init(
        bodyByteCount: Int = 0,
        clockNow: UInt64 = 1_000,
        preparationResult: Bool? = true,
        validationResult: Bool? = true
    ) {
        self.bodyByteCount = bodyByteCount
        self.clock = NativeAIFakeClock(nowNanoseconds: clockNow)
        self.preparation = NativeAIPreparationFake(immediateResult: preparationResult)
        self.validation = NativeAIValidationFake(immediateResult: validationResult)
        self.network = NativeAINetworkFactoryFake()
    }

    @discardableResult
    func start() -> NativeAIRemoteLifecycleTestRun {
        let harness = NativeAIRemoteLifecycleTestHarness(
            fixture: NativeAIRemoteLifecycleTestRequestFixture(
                bodyByteCount: bodyByteCount
            ),
            clock: clock,
            preparation: preparation,
            validation: validation,
            network: network
        )
        let run = harness.start { [recorder] result in
            recorder.record(result)
        }
        self.run = run
        return run
    }

    func dropRun() {
        run = nil
    }

    func succeed(
        chunks: [Int] = [],
        declaredContentLength: Int64? = nil
    ) {
        let transaction = network.onlyTransaction
        XCTAssertTrue(
            transaction.events.receiveResponse(
                declaredContentLength: declaredContentLength
            )
        )
        for chunk in chunks {
            transaction.events.receiveDeliveredBytes(chunk)
        }
        transaction.events.complete()
    }
}

final class NativeAIRemoteLifecycleConfigurationTests: XCTestCase {
    func testProductionConfigurationIsEphemeralAndNonPersistent() {
        let configuration = NativeAIRemoteLifecycleTestHarness.productionConfiguration

        XCTAssertFalse(configuration.hasCookieStorage)
        XCTAssertFalse(configuration.hasURLCache)
        XCTAssertFalse(configuration.hasCredentialStorage)
        XCTAssertFalse(configuration.setsCookies)
        XCTAssertFalse(configuration.waitsForConnectivity)
        XCTAssertFalse(configuration.isDiscretionary)
        XCTAssertTrue(configuration.reloadsIgnoringLocalCache)
    }

    func testDriverCancelsRedirectAndDefaultsOnlyServerTrust() {
        let policy = NativeAIRemoteLifecycleTestHarness.productionDriverPolicy

        XCTAssertFalse(policy.followsRedirects)
        XCTAssertEqual(policy.serverTrustDisposition, .performDefaultHandling)
        XCTAssertEqual(policy.otherAuthenticationDisposition, .cancel)
    }

    func testFixedRequestHasNoCallerControlledNetworkIdentity() {
        let rig = NativeAITestRig(bodyByteCount: 17)
        rig.start()

        XCTAssertEqual(rig.network.requests.count, 1)
        let request = try! XCTUnwrap(rig.network.requests.first)
        XCTAssertEqual(request.bodyByteCount, 17)
        XCTAssertEqual(request.method, "POST")
        XCTAssertEqual(request.scheme, "https")
        XCTAssertEqual(request.host, "dux-native-ai.invalid")
        XCTAssertTrue(request.reloadsIgnoringLocalCache)
        XCTAssertFalse(request.handlesCookies)
    }
}

final class NativeAIRemoteLifecycleRequestBoundTests: XCTestCase {
    func testRequestAt384KiBStartsExactlyOneTask() {
        let rig = NativeAITestRig(
            bodyByteCount: NativeAIRemoteLifecycleTestHarness.requestByteLimit
        )
        rig.start()

        XCTAssertEqual(rig.network.transactionCount, 1)
        XCTAssertEqual(rig.network.onlyTransaction.resumeCount, 1)
        XCTAssertEqual(
            rig.network.requests.first?.bodyByteCount,
            384 * 1_024
        )
    }

    func testRequestOver384KiBFailsBeforeCreatingSessionOrTask() {
        let rig = NativeAITestRig(
            bodyByteCount: NativeAIRemoteLifecycleTestHarness.requestByteLimit + 1
        )
        rig.start()

        XCTAssertEqual(rig.recorder.values, [.failure(.requestTooLarge)])
        XCTAssertEqual(rig.network.transactionCount, 0)
    }
}

final class NativeAIRemoteLifecycleResponseBoundTests: XCTestCase {
    func testExactly64KiBDeliveredAcrossMultipleChunksIsAccepted() {
        let rig = NativeAITestRig()
        rig.start()
        rig.succeed(
            chunks: [1, 31_999, 33_536],
            declaredContentLength: 64 * 1_024
        )

        XCTAssertEqual(
            rig.recorder.values,
            [
                .accepted(
                    deliveredByteCount: 64 * 1_024,
                    mediaType: "application/x-dux-future-adapter+json"
                )
            ]
        )
        XCTAssertEqual(rig.validation.observations.first?.deliveredByteCount, 64 * 1_024)
    }

    func test64KiBPlusOneAcrossMultipleChunksFailsIncrementally() {
        let rig = NativeAITestRig()
        rig.start()
        let transaction = rig.network.onlyTransaction
        XCTAssertTrue(transaction.events.receiveResponse())
        transaction.events.receiveDeliveredBytes(32_768)
        transaction.events.receiveDeliveredBytes(32_768)
        transaction.events.receiveDeliveredBytes(1)

        XCTAssertEqual(rig.recorder.values, [.failure(.responseTooLarge)])
        XCTAssertTrue(rig.validation.observations.isEmpty)
        XCTAssertEqual(transaction.cancelCount, 1)
        XCTAssertEqual(transaction.invalidateCount, 1)
    }

    func testMissingContentLengthStillEnforcesDeliveredLimit() {
        let rig = NativeAITestRig()
        rig.start()
        rig.succeed(chunks: [20_000, 20_000, 25_536])

        XCTAssertEqual(
            rig.recorder.values,
            [
                .accepted(
                    deliveredByteCount: 65_536,
                    mediaType: "application/x-dux-future-adapter+json"
                )
            ]
        )
    }

    func testFalseSmallContentLengthCannotBypassDeliveredLimit() {
        let rig = NativeAITestRig()
        rig.start()
        let transaction = rig.network.onlyTransaction
        XCTAssertTrue(
            transaction.events.receiveResponse(declaredContentLength: 1)
        )
        transaction.events.receiveDeliveredBytes(65_537)

        XCTAssertEqual(rig.recorder.values, [.failure(.responseTooLarge)])
    }

    func testOversizedDeclaredLengthCancelsBeforeBodyDelivery() {
        let rig = NativeAITestRig()
        rig.start()
        let transaction = rig.network.onlyTransaction

        XCTAssertFalse(
            transaction.events.receiveResponse(
                declaredContentLength: Int64(
                    NativeAIRemoteLifecycleTestHarness.responseByteLimit + 1
                )
            )
        )
        XCTAssertEqual(rig.recorder.values, [.failure(.responseTooLarge)])
        XCTAssertEqual(transaction.cancelCount, 1)
        XCTAssertEqual(transaction.invalidateCount, 1)
    }

    func testCompressedResponseIsBoundByDeliveredDecompressedChunks() {
        let rig = NativeAITestRig()
        rig.start()
        let transaction = rig.network.onlyTransaction
        XCTAssertTrue(
            transaction.events.receiveResponse(declaredContentLength: 1_024)
        )
        transaction.events.receiveDeliveredBytes(40_000)
        transaction.events.receiveDeliveredBytes(25_537)

        XCTAssertEqual(rig.recorder.values, [.failure(.responseTooLarge)])
    }

    func testUnrepresentablyLargeChunkFailsClosedWithoutAllocatingIt() {
        let rig = NativeAITestRig()
        rig.start()
        let transaction = rig.network.onlyTransaction
        XCTAssertTrue(transaction.events.receiveResponse())
        transaction.events.receiveDeliveredBytes(Int.max)

        XCTAssertEqual(rig.recorder.values, [.failure(.responseTooLarge)])
    }
}

final class NativeAIRemoteLifecycleProtocolFailureTests: XCTestCase {
    func testOnlyStatus200IsAccepted() {
        for statusCode in [199, 201, 204, 299, 300, 401, 429, 500] {
            let rig = NativeAITestRig()
            rig.start()
            let transaction = rig.network.onlyTransaction

            XCTAssertFalse(
                transaction.events.receiveResponse(statusCode: statusCode),
                "status \(statusCode)"
            )
            XCTAssertEqual(rig.recorder.values, [.failure(.status)])
            XCTAssertEqual(rig.network.transactionCount, 1)
            XCTAssertEqual(transaction.resumeCount, 1)
            XCTAssertEqual(transaction.cancelCount, 1)
            XCTAssertEqual(transaction.invalidateCount, 1)
        }
    }

    func testCompletionWithoutHTTPResponseFailsClosed() {
        let rig = NativeAITestRig()
        rig.start()
        rig.network.onlyTransaction.events.complete()

        XCTAssertEqual(rig.recorder.values, [.failure(.invalidResponse)])
    }

    func testNetworkFailureIsRedactedAndNeverRetried() {
        let rig = NativeAITestRig()
        rig.start()
        let transaction = rig.network.onlyTransaction
        transaction.events.complete(networkFailed: true)

        XCTAssertEqual(rig.recorder.values, [.failure(.network)])
        XCTAssertEqual(rig.network.transactionCount, 1)
        XCTAssertEqual(transaction.cancelCount, 1)
        XCTAssertEqual(transaction.invalidateCount, 1)
    }

    func testValidationFailureCancelsAndNeverRetries() {
        let rig = NativeAITestRig(validationResult: false)
        rig.start()
        rig.succeed(chunks: [8])

        XCTAssertEqual(rig.recorder.values, [.failure(.validation)])
        XCTAssertEqual(rig.network.transactionCount, 1)
        XCTAssertEqual(rig.network.onlyTransaction.cancelCount, 1)
        XCTAssertEqual(rig.network.onlyTransaction.invalidateCount, 1)
    }

    func testPreparationFailureNeverCreatesNetworkState() {
        let rig = NativeAITestRig(preparationResult: false)
        rig.start()

        XCTAssertEqual(rig.recorder.values, [.failure(.preparation)])
        XCTAssertEqual(rig.network.transactionCount, 0)
    }

    func testNonServerTrustChallengeCancelsAndNeverRetries() {
        let rig = NativeAITestRig()
        rig.start()
        let transaction = rig.network.onlyTransaction
        transaction.events.receiveNonServerTrustAuthenticationChallenge()

        XCTAssertEqual(rig.recorder.values, [.failure(.authentication)])
        XCTAssertEqual(rig.network.transactionCount, 1)
        XCTAssertEqual(transaction.cancelCount, 1)
        XCTAssertEqual(transaction.invalidateCount, 1)
    }
}

final class NativeAIRemoteLifecycleRedirectTests: XCTestCase {
    func testFirstRedirectTerminatesAndEveryLateCallbackIsDiscarded() {
        let rig = NativeAITestRig()
        rig.start()
        let transaction = rig.network.onlyTransaction
        XCTAssertTrue(transaction.events.receiveResponse())

        transaction.events.receiveRedirect()
        transaction.events.receiveDeliveredBytes(1)
        transaction.events.complete()
        transaction.events.receiveRedirect()
        transaction.events.receiveNonServerTrustAuthenticationChallenge()

        XCTAssertEqual(rig.recorder.values, [.failure(.redirect)])
        XCTAssertEqual(rig.network.transactionCount, 1)
        XCTAssertEqual(transaction.resumeCount, 1)
        XCTAssertEqual(transaction.cancelCount, 1)
        XCTAssertEqual(transaction.invalidateCount, 1)
        XCTAssertTrue(rig.validation.observations.isEmpty)
    }

    func testRedirectAfterSuccessCannotReplaceTerminalResult() {
        let rig = NativeAITestRig()
        rig.start()
        rig.succeed(chunks: [1])
        rig.network.onlyTransaction.events.receiveRedirect()

        XCTAssertEqual(
            rig.recorder.values,
            [
                .accepted(
                    deliveredByteCount: 1,
                    mediaType: "application/x-dux-future-adapter+json"
                )
            ]
        )
        XCTAssertEqual(rig.network.onlyTransaction.cancelCount, 0)
        XCTAssertEqual(rig.network.onlyTransaction.invalidateCount, 1)
    }

    func testCancellationWinsSequentialRaceWithRedirect() {
        let rig = NativeAITestRig()
        let run = rig.start()
        let transaction = rig.network.onlyTransaction

        run.cancel()
        transaction.events.receiveRedirect()

        XCTAssertEqual(rig.recorder.values, [.failure(.cancelled)])
        XCTAssertEqual(transaction.cancelCount, 1)
        XCTAssertEqual(transaction.invalidateCount, 1)
    }
}

final class NativeAIRemoteLifecycleCancellationTests: XCTestCase {
    func testCancelDuringCancellationInsensitivePreparationBlocksLateNetworkStart() {
        let rig = NativeAITestRig(preparationResult: nil)
        let run = rig.start()

        run.cancel()
        rig.preparation.complete(true)

        XCTAssertEqual(rig.recorder.values, [.failure(.cancelled)])
        XCTAssertEqual(rig.network.transactionCount, 0)
    }

    func testCancelDuringRequestCancelsTaskAndInvalidatesSessionOnce() {
        let rig = NativeAITestRig()
        let run = rig.start()
        let transaction = rig.network.onlyTransaction

        run.cancel()
        run.cancel()
        transaction.events.complete(networkFailed: true)

        XCTAssertEqual(rig.recorder.values, [.failure(.cancelled)])
        XCTAssertEqual(transaction.resumeCount, 1)
        XCTAssertEqual(transaction.cancelCount, 1)
        XCTAssertEqual(transaction.invalidateCount, 1)
    }

    func testCancelDuringResponseDiscardsFollowingBodyAndCompletion() {
        let rig = NativeAITestRig()
        let run = rig.start()
        let transaction = rig.network.onlyTransaction
        XCTAssertTrue(transaction.events.receiveResponse())
        transaction.events.receiveDeliveredBytes(4)

        run.cancel()
        transaction.events.receiveDeliveredBytes(4)
        transaction.events.complete()

        XCTAssertEqual(rig.recorder.values, [.failure(.cancelled)])
        XCTAssertTrue(rig.validation.observations.isEmpty)
        XCTAssertEqual(transaction.cancelCount, 1)
        XCTAssertEqual(transaction.invalidateCount, 1)
    }

    func testCancelAfterSuccessIsNoOp() {
        let rig = NativeAITestRig()
        let run = rig.start()
        rig.succeed(chunks: [2])

        run.cancel()
        run.cancel()

        XCTAssertEqual(rig.recorder.values.count, 1)
        XCTAssertEqual(rig.network.onlyTransaction.cancelCount, 0)
        XCTAssertEqual(rig.network.onlyTransaction.invalidateCount, 1)
    }

    func testDroppingRunSettlesOutstandingTaskAndReleasesRun() {
        let rig = NativeAITestRig()
        var run: NativeAIRemoteLifecycleTestRun? = rig.start()
        weak let weakRun = run
        let transaction = rig.network.onlyTransaction
        rig.dropRun()

        run = nil

        XCTAssertNil(weakRun)
        XCTAssertEqual(rig.recorder.values, [.failure(.cancelled)])
        XCTAssertEqual(transaction.cancelCount, 1)
        XCTAssertEqual(transaction.invalidateCount, 1)
    }
}

final class NativeAIRemoteLifecycleDeadlineTests: XCTestCase {
    func testSingleAbsoluteDeadlineStartsBeforePreparation() {
        let initialNow: UInt64 = 123_456
        let rig = NativeAITestRig(
            clockNow: initialNow,
            preparationResult: nil
        )
        rig.start()

        XCTAssertEqual(
            rig.clock.deadlines,
            [initialNow + NativeAIRemoteLifecycleTestHarness.deadlineNanoseconds]
        )
        XCTAssertEqual(rig.preparation.callCount, 1)
    }

    func testDeadlineDuringPreparationReturnsAndLatePreparationCannotStartNetwork() {
        let rig = NativeAITestRig(preparationResult: nil)
        rig.start()

        rig.clock.advance(
            by: NativeAIRemoteLifecycleTestHarness.deadlineNanoseconds
        )
        rig.preparation.complete(true)

        XCTAssertEqual(rig.recorder.values, [.failure(.deadlineExceeded)])
        XCTAssertEqual(rig.network.transactionCount, 0)
    }

    func testDeadlineDuringRequestCancelsAndInvalidatesExactlyOnce() {
        let rig = NativeAITestRig()
        rig.start()
        let transaction = rig.network.onlyTransaction

        rig.clock.advance(
            by: NativeAIRemoteLifecycleTestHarness.deadlineNanoseconds
        )
        transaction.events.complete(networkFailed: true)

        XCTAssertEqual(rig.recorder.values, [.failure(.deadlineExceeded)])
        XCTAssertEqual(transaction.cancelCount, 1)
        XCTAssertEqual(transaction.invalidateCount, 1)
    }

    func testDeadlineDuringResponseDiscardsLateChunks() {
        let rig = NativeAITestRig()
        rig.start()
        let transaction = rig.network.onlyTransaction
        XCTAssertTrue(transaction.events.receiveResponse())
        transaction.events.receiveDeliveredBytes(1)

        rig.clock.advance(
            by: NativeAIRemoteLifecycleTestHarness.deadlineNanoseconds
        )
        transaction.events.receiveDeliveredBytes(1)
        transaction.events.complete()

        XCTAssertEqual(rig.recorder.values, [.failure(.deadlineExceeded)])
        XCTAssertTrue(rig.validation.observations.isEmpty)
    }

    func testDeadlineDuringValidationDiscardsLateAcceptance() {
        let rig = NativeAITestRig(validationResult: nil)
        rig.start()
        rig.succeed(chunks: [4])
        XCTAssertEqual(rig.validation.observations.count, 1)

        rig.clock.advance(
            by: NativeAIRemoteLifecycleTestHarness.deadlineNanoseconds
        )
        rig.validation.complete(true)

        XCTAssertEqual(rig.recorder.values, [.failure(.deadlineExceeded)])
        XCTAssertEqual(rig.network.onlyTransaction.cancelCount, 1)
        XCTAssertEqual(rig.network.onlyTransaction.invalidateCount, 1)
    }

    func testCancellationSettlementCancelsDeadlineAndCannotEmitTimeout() {
        let rig = NativeAITestRig()
        let run = rig.start()
        rig.clock.advance(
            by: NativeAIRemoteLifecycleTestHarness.deadlineNanoseconds - 1
        )

        run.cancel()
        rig.clock.advance(by: 1)

        XCTAssertEqual(rig.recorder.values, [.failure(.cancelled)])
        XCTAssertEqual(rig.network.onlyTransaction.cancelCount, 1)
        XCTAssertEqual(rig.network.onlyTransaction.invalidateCount, 1)
    }

    func testCompletionOneNanosecondBeforeDeadlineSucceeds() {
        let rig = NativeAITestRig()
        rig.start()
        rig.clock.advance(
            by: NativeAIRemoteLifecycleTestHarness.deadlineNanoseconds - 1
        )

        rig.succeed(chunks: [1])

        XCTAssertEqual(
            rig.recorder.values,
            [
                .accepted(
                    deliveredByteCount: 1,
                    mediaType: "application/x-dux-future-adapter+json"
                )
            ]
        )
    }

    func testClockNearOverflowFailsClosedBeforePreparation() {
        let rig = NativeAITestRig(
            clockNow: UInt64.max - 1,
            preparationResult: nil
        )
        rig.start()

        XCTAssertEqual(rig.recorder.values, [.failure(.deadlineExceeded)])
        XCTAssertEqual(rig.preparation.callCount, 0)
        XCTAssertEqual(rig.network.transactionCount, 0)
    }
}

final class NativeAIRemoteLifecycleTerminalFenceTests: XCTestCase {
    func testSuccessCreatesOneSessionTaskResumeAndInvalidateWithNoRetry() {
        let rig = NativeAITestRig()
        rig.start()
        rig.succeed(chunks: [1, 2, 3])
        let transaction = rig.network.onlyTransaction

        XCTAssertEqual(rig.network.transactionCount, 1)
        XCTAssertEqual(transaction.resumeCount, 1)
        XCTAssertEqual(transaction.cancelCount, 0)
        XCTAssertEqual(transaction.invalidateCount, 1)
        XCTAssertEqual(rig.recorder.values.count, 1)
    }

    func testTerminalKernelReleasesItsTransaction() {
        let rig = NativeAITestRig()
        rig.start()
        var transaction: NativeAINetworkTransactionFake? = rig.network.onlyTransaction
        weak let weakTransaction = transaction

        rig.succeed(chunks: [1])
        rig.network.releaseTransactions()
        transaction = nil

        XCTAssertNil(weakTransaction)
    }

    func testAllLateCallbackKindsAfterTimeoutAreDiscarded() {
        let rig = NativeAITestRig(validationResult: nil)
        rig.start()
        let transaction = rig.network.onlyTransaction

        rig.clock.advance(
            by: NativeAIRemoteLifecycleTestHarness.deadlineNanoseconds
        )
        _ = transaction.events.receiveResponse(statusCode: 200)
        transaction.events.receiveDeliveredBytes(1)
        transaction.events.receiveRedirect()
        transaction.events.receiveNonServerTrustAuthenticationChallenge()
        transaction.events.complete(networkFailed: true)

        XCTAssertEqual(rig.recorder.values, [.failure(.deadlineExceeded)])
        XCTAssertEqual(transaction.resumeCount, 1)
        XCTAssertEqual(transaction.cancelCount, 1)
        XCTAssertEqual(transaction.invalidateCount, 1)
        XCTAssertEqual(rig.network.transactionCount, 1)
    }

    func testEveryErrorIsAPathFreeFixedCategory() {
        let errors: [NativeAIRemoteLifecycleTestFailure] = [
            .requestTooLarge,
            .preparation,
            .deadlineExceeded,
            .cancelled,
            .redirect,
            .authentication,
            .invalidResponse,
            .status,
            .responseTooLarge,
            .network,
            .validation,
        ]
        let forbiddenFragments = [
            "/Users/",
            "file://",
            "https://",
            "Authorization",
            "secret",
            "provider prose",
            "underlying",
        ]

        XCTAssertEqual(Set(errors.map(\.description)).count, errors.count)
        for error in errors {
            for fragment in forbiddenFragments {
                XCTAssertFalse(error.description.localizedCaseInsensitiveContains(fragment))
            }
        }
    }
}
