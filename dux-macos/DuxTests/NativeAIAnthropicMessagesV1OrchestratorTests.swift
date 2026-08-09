@testable import DUX
import Foundation
import XCTest

final class NativeAIAnthropicMessagesV1OrchestratorTests: XCTestCase {
    func testValidRealAdapterEnvelopeReturnsExactCoreProjectionOnce() async throws {
        let rig = try OrchestratorRig()
        let run = rig.start()
        defer { withExtendedLifetime(run) {} }
        let transaction = try await rig.onlyTransaction()

        XCTAssertEqual(transaction.resumeCount, 1)
        XCTAssertEqual(transaction.observation.url.absoluteString, "https://api.anthropic.com/v1/messages")
        XCTAssertEqual(transaction.observation.method, "POST")
        XCTAssertEqual(transaction.observation.headers["x-api-key"], rig.secret)
        XCTAssertFalse(transaction.observation.body.contains(Data(rig.secret.utf8)))

        try transaction.succeed(body: anthropicEnvelope(inner: rig.innerJSON))
        let settled = await eventually { rig.recorder.count == 1 }
        XCTAssertTrue(settled)
        try transaction.succeed(body: anthropicEnvelope(inner: rig.innerJSON))

        guard case let .success(result)? = rig.recorder.first else {
            return XCTFail("expected exact validated result")
        }
        XCTAssertEqual(result.provider, .trusted)
        XCTAssertEqual(result.validated, rig.validatedResult)
        XCTAssertEqual(rig.attempt.validatedBodies, [rig.innerJSON])
        XCTAssertEqual(rig.attempt.validateCount, 1)
        XCTAssertEqual(rig.attempt.releaseCount, 1)
        XCTAssertEqual(rig.network.transactionCount, 1)
        XCTAssertEqual(transaction.cancelCount, 0)
        XCTAssertEqual(transaction.invalidateCount, 1)
    }

    func testLockedAndCorruptCredentialErrorsAreBoundedAndCreateNoTask() async throws {
        for error in [
            AIProviderCredentialStoreError.locked,
            AIProviderCredentialStoreError.corruptCredential,
        ] {
            let rig = try OrchestratorRig(credentialFailure: error)
            let run = rig.start()
            let settled = await eventually { rig.recorder.count == 1 }
            XCTAssertTrue(settled)
            XCTAssertEqual(rig.recorder.failures, [.credentialUnavailable])
            XCTAssertEqual(rig.network.transactionCount, 0)
            XCTAssertEqual(rig.attempt.releaseCount, 1)
            withExtendedLifetime(run) {}
        }
    }

    func testMissingCredentialReleasesAttemptWithoutCreatingTask() async throws {
        let rig = try OrchestratorRig(hasCredential: false)
        let run = rig.start()
        defer { withExtendedLifetime(run) {} }

        let settled = await eventually { rig.recorder.count == 1 }
        XCTAssertTrue(settled)
        XCTAssertEqual(rig.recorder.failures, [.missingCredential])
        XCTAssertEqual(rig.attempt.releaseCount, 1)
        XCTAssertEqual(rig.network.transactionCount, 0)
    }

    func testInvalidDigestStopsBeforeKeychainAndNetwork() async throws {
        let rig = try OrchestratorRig(digest: String(repeating: "A", count: 64))
        let run = rig.start()
        defer { withExtendedLifetime(run) {} }

        let settled = await eventually { rig.recorder.count == 1 }
        XCTAssertTrue(settled)
        XCTAssertEqual(rig.recorder.failures, [.invalidCoreBinding])
        let readCount = await rig.credentials.readCount
        XCTAssertEqual(readCount, 0)
        XCTAssertEqual(rig.network.transactionCount, 0)
        XCTAssertEqual(rig.attempt.releaseCount, 1)
    }

    func testEarlierCoreDeadlineStopsBeforeKeychainAndRequestConstruction() async throws {
        let originalDeadline = UInt64(1000) + NativeAIRemoteLimits.deadlineNanoseconds
        let rig = try OrchestratorRig(
            attemptDeadlineNanoseconds: originalDeadline - 1
        )
        let run = rig.start()
        defer { withExtendedLifetime(run) {} }

        let settled = await eventually { rig.recorder.count == 1 }
        XCTAssertTrue(settled)
        XCTAssertEqual(rig.recorder.failures, [.invalidCoreBinding])
        let readCount = await rig.credentials.readCount
        XCTAssertEqual(readCount, 0)
        XCTAssertEqual(rig.network.transactionCount, 0)
        XCTAssertEqual(rig.attempt.releaseCount, 1)
    }

    func testMediaToolEnvelopeAndCoreProjectionRejectionAreBounded() async throws {
        for failure in ExtractionFixture.allCases {
            let rig = try OrchestratorRig()
            let run = rig.start()
            let transaction = try await rig.onlyTransaction()
            switch failure {
            case .media:
                try transaction.succeed(
                    body: anthropicEnvelope(inner: rig.innerJSON),
                    mediaType: "application/json; charset=utf-8"
                )
            case .tool:
                transaction.succeed(body: toolEnvelope())
            }
            let settled = await eventually { rig.recorder.count == 1 }
            XCTAssertTrue(settled)
            XCTAssertEqual(rig.recorder.failures, [.extractionRejected])
            XCTAssertEqual(rig.attempt.validateCount, 0)
            XCTAssertEqual(rig.network.transactionCount, 1)
            withExtendedLifetime(run) {}
        }

        let wrongProjection = try OrchestratorRig(projectionDigest: String(repeating: "b", count: 64))
        let run = wrongProjection.start()
        let transaction = try await wrongProjection.onlyTransaction()
        try transaction.succeed(body: anthropicEnvelope(inner: wrongProjection.innerJSON))
        let settled = await eventually { wrongProjection.recorder.count == 1 }
        XCTAssertTrue(settled)
        XCTAssertEqual(wrongProjection.recorder.failures, [.coreRejected])
        XCTAssertEqual(wrongProjection.attempt.validateCount, 1)
        withExtendedLifetime(run) {}
    }

    func testStatusRedirectChallengeAndResponseNPlusOneNeverRetry() async throws {
        let actions: [(OrchestratorNetworkTransactionFake) -> Void] = [
            { $0.rejectStatus() },
            { $0.redirect() },
            { $0.rejectAuthentication() },
            { $0.oversizedResponse() },
        ]
        let expected: [NativeAIAnthropicMessagesV1OrchestratorFailure] = [
            .status, .redirect, .authentication, .responseTooLarge,
        ]
        for (index, action) in actions.enumerated() {
            let rig = try OrchestratorRig()
            let run = rig.start()
            let transaction = try await rig.onlyTransaction()
            action(transaction)
            let settled = await eventually { rig.recorder.count == 1 }
            XCTAssertTrue(settled)
            XCTAssertEqual(rig.recorder.failures, [expected[index]])
            XCTAssertEqual(rig.network.transactionCount, 1)
            XCTAssertEqual(transaction.cancelCount, 1)
            XCTAssertEqual(transaction.invalidateCount, 1)
            withExtendedLifetime(run) {}
        }
    }

    func testCancellationDuringPreviewKeychainNetworkAndValidationSettlesOnce() async throws {
        let previewGate = AsyncTestGate()
        let previewRig = try OrchestratorRig(previewGate: previewGate)
        let previewRun = previewRig.start()
        let previewStarted = await eventually { await previewRig.preview.consumeCount == 1 }
        XCTAssertTrue(previewStarted)
        previewRun?.cancel()
        await previewGate.open()
        let previewReleased = await eventually { previewRig.attempt.releaseCount == 1 }
        XCTAssertTrue(previewReleased)
        XCTAssertEqual(previewRig.recorder.failures, [.cancelled])
        XCTAssertEqual(previewRig.network.transactionCount, 0)

        let keychainGate = AsyncTestGate()
        let keychainRig = try OrchestratorRig(credentialGate: keychainGate)
        let keychainRun = keychainRig.start()
        let keychainStarted = await eventually {
            await keychainRig.credentials.readCount == 1
        }
        XCTAssertTrue(keychainStarted)
        keychainRun?.cancel()
        await keychainGate.open()
        let keychainReleased = await eventually { keychainRig.attempt.releaseCount == 1 }
        XCTAssertTrue(keychainReleased)
        XCTAssertEqual(keychainRig.recorder.failures, [.cancelled])
        XCTAssertEqual(keychainRig.network.transactionCount, 0)

        let networkRig = try OrchestratorRig()
        let networkRun = networkRig.start()
        let networkTransaction = try await networkRig.onlyTransaction()
        networkRun?.cancel()
        networkTransaction.networkFailure()
        XCTAssertEqual(networkRig.recorder.failures, [.cancelled])
        XCTAssertEqual(networkTransaction.cancelCount, 1)
        XCTAssertEqual(networkTransaction.invalidateCount, 1)

        let validationGate = AsyncTestGate()
        let validationRig = try OrchestratorRig(validationGate: validationGate)
        let validationRun = validationRig.start()
        let validationTransaction = try await validationRig.onlyTransaction()
        try validationTransaction.succeed(
            body: anthropicEnvelope(inner: validationRig.innerJSON)
        )
        let validationStarted = await eventually {
            validationRig.attempt.validateCount == 1
        }
        XCTAssertTrue(validationStarted)
        validationRun?.cancel()
        await validationGate.open()
        let validationReleased = await eventually {
            validationRig.attempt.releaseCount == 1
        }
        XCTAssertTrue(validationReleased)
        XCTAssertEqual(validationRig.recorder.failures, [.cancelled])
        XCTAssertEqual(validationRig.recorder.count, 1)
    }

    func testDeadlineDuringLateKeychainCreatesNoTaskAndReleasesAttempt() async throws {
        let gate = AsyncTestGate()
        let rig = try OrchestratorRig(credentialGate: gate)
        let run = rig.start()
        defer { withExtendedLifetime(run) {} }
        let keychainStarted = await eventually { await rig.credentials.readCount == 1 }
        XCTAssertTrue(keychainStarted)

        rig.clock.advance(by: NativeAIRemoteLimits.deadlineNanoseconds)
        await gate.open()

        let released = await eventually { rig.attempt.releaseCount == 1 }
        XCTAssertTrue(released)
        XCTAssertEqual(rig.recorder.failures, [.deadlineExceeded])
        XCTAssertEqual(rig.network.transactionCount, 0)
    }

    func testSecondStartIsRejectedWithoutSecondConsume() async throws {
        let rig = try OrchestratorRig()
        let first = rig.start()
        let second = rig.orchestrator.start { [recorder = rig.secondRecorder] in
            recorder.record($0)
        }

        XCTAssertNil(second)
        XCTAssertEqual(rig.secondRecorder.failures, [.cancelled])
        let consumed = await eventually { await rig.preview.consumeCount == 1 }
        XCTAssertTrue(consumed)
        let consumeCount = await rig.preview.consumeCount
        XCTAssertEqual(consumeCount, 1)
        first?.cancel()
    }

    func testCredentialRequestResultAndErrorsStayRedacted() async throws {
        let rig = try OrchestratorRig()
        let run = rig.start()
        let transaction = try await rig.onlyTransaction()
        try transaction.succeed(body: anthropicEnvelope(inner: rig.innerJSON))
        let settled = await eventually { rig.recorder.count == 1 }
        XCTAssertTrue(settled)

        let rendered = [
            String(describing: rig.validatedResult),
            String(reflecting: rig.validatedResult),
            String(describing: rig.recorder.first as Any),
            NativeAIAnthropicMessagesV1OrchestratorFailure.network.description,
        ]
        for value in rendered {
            XCTAssertFalse(value.contains(rig.secret))
            XCTAssertFalse(value.contains(rig.innerSentinel))
        }
        withExtendedLifetime(run) {}
    }

    func testTrustedProjectionRejectsDuplicateCollectionsAndMoreThan128MappedNodes() throws {
        let rig = try OrchestratorRig()
        let duplicateCollections = NativeAIAnthropicMessagesV1CoreValidatedResult(
            recordVersion: 1,
            inputSchemaVersion: 1,
            outputSchemaVersion: 1,
            privacyPolicyRevision: 1,
            providerBindingRevision: 1,
            binding: .trusted,
            inputDigestSHA256: rig.attempt.inputDigestSHA256,
            sourceScanID: rig.attempt.sourceScanID,
            selectedRootNodeID: rig.attempt.selectedRootNodeID,
            summary: "Validated.",
            labels: ["Duplicate", "Duplicate"],
            groups: [],
            questions: ["Duplicate?", "Duplicate?"],
            uncertainties: ["Duplicate.", "Duplicate."],
            researchSuggestions: ["Duplicate.", "Duplicate."]
        )
        XCTAssertFalse(duplicateCollections.isTrustedProjection(of: rig.attempt))

        let tooManyMappedNodes = NativeAIAnthropicMessagesV1CoreValidatedResult(
            recordVersion: 1,
            inputSchemaVersion: 1,
            outputSchemaVersion: 1,
            privacyPolicyRevision: 1,
            providerBindingRevision: 1,
            binding: .trusted,
            inputDigestSHA256: rig.attempt.inputDigestSHA256,
            sourceScanID: rig.attempt.sourceScanID,
            selectedRootNodeID: rig.attempt.selectedRootNodeID,
            summary: "Validated.",
            labels: [],
            groups: [
                .init(
                    recordVersion: 1,
                    title: "First",
                    snapshotNodeIDs: Array(1 ... 128),
                    reason: "First group."
                ),
                .init(
                    recordVersion: 1,
                    title: "Second",
                    snapshotNodeIDs: [129],
                    reason: "Second group."
                ),
            ],
            questions: [],
            uncertainties: [],
            researchSuggestions: []
        )
        XCTAssertFalse(tooManyMappedNodes.isTrustedProjection(of: rig.attempt))
    }
}

private enum ExtractionFixture: CaseIterable { case media, tool }
private enum OrchestratorFakeError: Error { case failed }

private actor AsyncTestGate {
    private var isOpen = false
    private var waiters: [CheckedContinuation<Void, Never>] = []

    func wait() async {
        if isOpen { return }
        await withCheckedContinuation { waiters.append($0) }
    }

    func open() {
        isOpen = true
        let values = waiters
        waiters.removeAll()
        values.forEach { $0.resume() }
    }
}

private actor OrchestratorPreviewFake: NativeAIAnthropicMessagesV1PreviewConsuming {
    let attempt: OrchestratorAttemptFake
    let gate: AsyncTestGate?
    private(set) var consumeCount = 0

    init(attempt: OrchestratorAttemptFake, gate: AsyncTestGate?) {
        self.attempt = attempt
        self.gate = gate
    }

    func consumeAnthropicMessagesV1PreviewOnce(
        deadlineNanoseconds _: UInt64,
        deadlineObservation _: NativeAIAnthropicMessagesV1DeadlineObservation
    ) async throws -> any NativeAIAnthropicMessagesV1Attempt {
        consumeCount += 1
        if consumeCount != 1 { throw OrchestratorFakeError.failed }
        await gate?.wait()
        return attempt
    }
}

private actor OrchestratorCredentialReaderFake: AIProviderCredentialRequestReading {
    let credential: AIProviderCredential?
    let failure: AIProviderCredentialStoreError?
    let gate: AsyncTestGate?
    private(set) var readCount = 0

    init(
        credential: AIProviderCredential?,
        failure: AIProviderCredentialStoreError?,
        gate: AsyncTestGate?
    ) {
        self.credential = credential
        self.failure = failure
        self.gate = gate
    }

    func readForSingleRequest(
        for account: AIProviderCredentialAccount
    ) async throws -> AIProviderCredential? {
        precondition(account == .anthropicMessagesV1)
        readCount += 1
        await gate?.wait()
        if let failure { throw failure }
        return credential
    }
}

private final class OrchestratorAttemptFake: @unchecked Sendable,
    NativeAIAnthropicMessagesV1Attempt
{
    let binding: NativeAIAnthropicMessagesV1Binding
    let recordVersion: UInt32 = 1
    let inputSchemaVersion: UInt64 = 1
    let outputSchemaVersion: UInt64 = 1
    let privacyPolicyRevision: UInt64 = 1
    let providerBindingRevision: UInt64 = 1
    let canonicalMetadataJSON: Data
    let inputDigestSHA256: String
    let sourceScanID = "scan:orchestrator-test"
    let selectedRootNodeID: UInt64 = 42
    let deadlineNanoseconds: UInt64
    let validatedResult: NativeAIAnthropicMessagesV1CoreValidatedResult
    let validationGate: AsyncTestGate?

    private let lock = NSLock()
    private var validateCountStorage = 0
    private var releaseCountStorage = 0
    private var validatedBodiesStorage: [Data] = []

    init(
        binding: NativeAIAnthropicMessagesV1Binding = .trusted,
        canonicalMetadataJSON: Data,
        inputDigestSHA256: String,
        deadlineNanoseconds: UInt64,
        validatedResult: NativeAIAnthropicMessagesV1CoreValidatedResult,
        validationGate: AsyncTestGate?
    ) {
        self.binding = binding
        self.canonicalMetadataJSON = canonicalMetadataJSON
        self.inputDigestSHA256 = inputDigestSHA256
        self.deadlineNanoseconds = deadlineNanoseconds
        self.validatedResult = validatedResult
        self.validationGate = validationGate
    }

    func validateOnce(
        extractedInnerJSON: Data
    ) async throws -> NativeAIAnthropicMessagesV1CoreValidatedResult {
        let accepted = lock.withLock { () -> Bool in
            validateCountStorage += 1
            validatedBodiesStorage.append(extractedInnerJSON)
            return validateCountStorage == 1
        }
        guard accepted else { throw OrchestratorFakeError.failed }
        await validationGate?.wait()
        return validatedResult
    }

    func release() { lock.withLock { releaseCountStorage += 1 } }
    var validateCount: Int { lock.withLock { validateCountStorage } }
    var releaseCount: Int { lock.withLock { releaseCountStorage } }
    var validatedBodies: [Data] { lock.withLock { validatedBodiesStorage } }
}

private final class OrchestratorRecorder: @unchecked Sendable {
    typealias Value = Result<
        NativeAIAnthropicMessagesV1Result,
        NativeAIAnthropicMessagesV1OrchestratorFailure
    >
    private let lock = NSLock()
    private var values: [Value] = []

    func record(_ value: Value) { lock.withLock { values.append(value) } }
    var count: Int { lock.withLock { values.count } }
    var first: Value? { lock.withLock { values.first } }
    var failures: [NativeAIAnthropicMessagesV1OrchestratorFailure] {
        lock.withLock { values.compactMap { try? $0.getFailure() } }
    }
}

private extension Result {
    func getFailure() throws -> Failure {
        switch self {
        case .success: throw OrchestratorFakeError.failed
        case let .failure(error): error
        }
    }
}

private final class OrchestratorScheduledActionFake: @unchecked Sendable,
    NativeAIRemoteScheduledAction
{
    private let lock = NSLock()
    private var action: (@Sendable () -> Void)?

    init(action: @escaping @Sendable () -> Void) { self.action = action }
    func cancel() { lock.withLock { action = nil } }
    func fire() {
        let value = lock.withLock { () -> (@Sendable () -> Void)? in
            defer { action = nil }
            return action
        }
        value?()
    }
}

private final class OrchestratorClockFake: @unchecked Sendable, NativeAIRemoteClock {
    private let lock = NSLock()
    private var now: UInt64
    private var scheduled: [(UInt64, OrchestratorScheduledActionFake)] = []

    init(now: UInt64) { self.now = now }
    func nowNanoseconds() -> UInt64 { lock.withLock { now } }
    func schedule(
        at deadlineNanoseconds: UInt64,
        _ action: @escaping @Sendable () -> Void
    ) -> any NativeAIRemoteScheduledAction {
        let value = OrchestratorScheduledActionFake(action: action)
        lock.withLock { scheduled.append((deadlineNanoseconds, value)) }
        return value
    }

    func advance(by delta: UInt64) {
        let due = lock.withLock { () -> [OrchestratorScheduledActionFake] in
            now += delta
            return scheduled.filter { $0.0 <= now }.map(\.1)
        }
        due.forEach { $0.fire() }
    }
}

private struct OrchestratorRequestObservation: Sendable {
    let url: URL
    let method: String
    let headers: [String: String]
    let body: Data
}

private final class OrchestratorNetworkTransactionFake: @unchecked Sendable,
    NativeAIRemoteNetworkTransaction
{
    let observation: OrchestratorRequestObservation
    private let sink: NativeAIRemoteNetworkSink
    private let lock = NSLock()
    private var resumeCountStorage = 0
    private var cancelCountStorage = 0
    private var invalidateCountStorage = 0

    init(request: NativeAIRemoteSealedRequest, sink: NativeAIRemoteNetworkSink) {
        self.sink = sink
        observation = request.withRequestForSingleTransaction { request, body in
            OrchestratorRequestObservation(
                url: request.url!,
                method: request.httpMethod ?? "",
                headers: Dictionary(
                    uniqueKeysWithValues: (request.allHTTPHeaderFields ?? [:]).map {
                        ($0.key.lowercased(), $0.value)
                    }
                ),
                body: body
            )
        }
    }

    func resume() { lock.withLock { resumeCountStorage += 1 } }
    func cancel() { lock.withLock { cancelCountStorage += 1 } }
    func invalidate() { lock.withLock { invalidateCountStorage += 1 } }

    func succeed(body: Data, mediaType: String = "application/json") {
        guard sink.response(
            NativeAIRemoteResponseHead(
                statusCode: 200,
                declaredContentLength: Int64(body.count),
                mediaType: mediaType
            )
        ) else { return }
        sink.data(body, body.count)
        sink.completed(false)
    }

    func rejectStatus() {
        _ = sink.response(.init(statusCode: 201, declaredContentLength: 0, mediaType: "application/json"))
    }

    func redirect() { sink.redirected() }
    func rejectAuthentication() { sink.rejectedAuthentication() }
    func networkFailure() { sink.completed(true) }
    func oversizedResponse() {
        _ = sink.response(
            .init(
                statusCode: 200,
                declaredContentLength: Int64(NativeAIRemoteLimits.responseBytes + 1),
                mediaType: "application/json"
            )
        )
    }

    var resumeCount: Int { lock.withLock { resumeCountStorage } }
    var cancelCount: Int { lock.withLock { cancelCountStorage } }
    var invalidateCount: Int { lock.withLock { invalidateCountStorage } }
}

private final class OrchestratorNetworkFactoryFake: @unchecked Sendable,
    NativeAIRemoteNetworkFactory
{
    private let lock = NSLock()
    private var transactionsStorage: [OrchestratorNetworkTransactionFake] = []
    func makeTransaction(
        request: NativeAIRemoteSealedRequest,
        sink: NativeAIRemoteNetworkSink
    ) -> any NativeAIRemoteNetworkTransaction {
        let value = OrchestratorNetworkTransactionFake(request: request, sink: sink)
        lock.withLock { transactionsStorage.append(value) }
        return value
    }

    var transactionCount: Int { lock.withLock { transactionsStorage.count } }
    var first: OrchestratorNetworkTransactionFake? { lock.withLock { transactionsStorage.first } }
}

private final class OrchestratorRig {
    let secret = "sk-orchestrator-redaction-sentinel"
    let innerSentinel = "provider-inner-redaction-sentinel"
    let innerJSON: Data
    let validatedResult: NativeAIAnthropicMessagesV1CoreValidatedResult
    let attempt: OrchestratorAttemptFake
    let preview: OrchestratorPreviewFake
    let credentials: OrchestratorCredentialReaderFake
    let clock: OrchestratorClockFake
    let network = OrchestratorNetworkFactoryFake()
    let recorder = OrchestratorRecorder()
    let secondRecorder = OrchestratorRecorder()
    let orchestrator: NativeAIAnthropicMessagesV1Orchestrator

    init(
        hasCredential: Bool = true,
        credentialFailure: AIProviderCredentialStoreError? = nil,
        digest: String = String(repeating: "a", count: 64),
        projectionDigest: String? = nil,
        previewGate: AsyncTestGate? = nil,
        credentialGate: AsyncTestGate? = nil,
        validationGate: AsyncTestGate? = nil,
        attemptDeadlineNanoseconds: UInt64? = nil
    ) throws {
        let clock = OrchestratorClockFake(now: 1000)
        self.clock = clock
        innerJSON = Data("{\"summary\":\"\(innerSentinel)\"}".utf8)
        validatedResult = NativeAIAnthropicMessagesV1CoreValidatedResult(
            recordVersion: 1,
            inputSchemaVersion: 1,
            outputSchemaVersion: 1,
            privacyPolicyRevision: 1,
            providerBindingRevision: 1,
            binding: .trusted,
            inputDigestSHA256: projectionDigest ?? digest,
            sourceScanID: "scan:orchestrator-test",
            selectedRootNodeID: 42,
            summary: "Safe storage metadata explanation.",
            labels: ["Cache"],
            groups: [
                .init(recordVersion: 1, title: "Generated", snapshotNodeIDs: [7], reason: "Metadata-only reason."),
            ],
            questions: ["Confirm ownership?"],
            uncertainties: ["Age is incomplete."],
            researchSuggestions: ["Review application documentation."]
        )
        let metadata = Data("{\"schema_version\":1,\"task\":\"explain_storage_cluster\",\"input_digest_sha256\":\"\(digest)\",\"metadata\":{\"label\":\"cache\"}}".utf8)
        attempt = OrchestratorAttemptFake(
            canonicalMetadataJSON: metadata,
            inputDigestSHA256: digest,
            deadlineNanoseconds: attemptDeadlineNanoseconds
                ?? 1000 + NativeAIRemoteLimits.deadlineNanoseconds,
            validatedResult: validatedResult,
            validationGate: validationGate
        )
        preview = OrchestratorPreviewFake(attempt: attempt, gate: previewGate)
        let selectedCredential = hasCredential
            ? try AIProviderCredential(secret)
            : nil
        credentials = OrchestratorCredentialReaderFake(
            credential: selectedCredential,
            failure: credentialFailure,
            gate: credentialGate
        )
        orchestrator = NativeAIAnthropicMessagesV1Orchestrator(
            previewConsumer: preview,
            credentialReader: credentials,
            adapter: AnthropicMessagesV1Adapter(),
            clock: clock,
            networkFactory: network
        )
    }

    func start() -> NativeAIAnthropicMessagesV1Run? {
        orchestrator.start { [recorder] in recorder.record($0) }
    }

    func onlyTransaction() async throws -> OrchestratorNetworkTransactionFake {
        guard await eventually({ self.network.first != nil }), let value = network.first
        else { throw OrchestratorFakeError.failed }
        return value
    }
}

private func anthropicEnvelope(inner: Data) throws -> Data {
    let text = String(decoding: inner, as: UTF8.self)
    return try JSONSerialization.data(withJSONObject: [
        "content": [["text": text, "type": "text"]],
        "id": "msg_orchestrator_test",
        "model": "claude-sonnet-4-6",
        "role": "assistant",
        "stop_reason": "end_turn",
        "stop_sequence": NSNull(),
        "type": "message",
        "usage": ["input_tokens": 1, "output_tokens": 1],
    ], options: [.sortedKeys])
}

private func toolEnvelope() -> Data {
    Data(#"{"content":[{"id":"tool_1","input":{},"name":"delete","type":"tool_use"}],"id":"msg_tool","model":"claude-sonnet-4-6","role":"assistant","stop_reason":"end_turn","stop_sequence":null,"type":"message","usage":{"input_tokens":1,"output_tokens":1}}"#.utf8)
}

private func eventually(
    _ predicate: @escaping () async -> Bool
) async -> Bool {
    for _ in 0 ..< 20000 {
        if await predicate() { return true }
        await Task.yield()
    }
    return false
}
