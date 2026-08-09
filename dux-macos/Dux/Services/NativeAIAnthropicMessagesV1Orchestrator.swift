import Foundation

/// Fixed identity trusted by both the Rust attempt and the reviewed adapter.
/// A mismatched attempt is released before any credential read or network work.
struct NativeAIAnthropicMessagesV1Binding: Equatable, Sendable {
    let account: AIProviderCredentialAccount
    let adapterID: String
    let adapterRevision: Int
    let endpoint: URL
    let model: String
    let requestMediaType: String
    let responseMediaType: String

    private init(
        account: AIProviderCredentialAccount,
        adapterID: String,
        adapterRevision: Int,
        endpoint: URL,
        model: String,
        requestMediaType: String,
        responseMediaType: String
    ) {
        self.account = account
        self.adapterID = adapterID
        self.adapterRevision = adapterRevision
        self.endpoint = endpoint
        self.model = model
        self.requestMediaType = requestMediaType
        self.responseMediaType = responseMediaType
    }

    static let trusted = NativeAIAnthropicMessagesV1Binding(
        account: .anthropicMessagesV1,
        adapterID: AnthropicMessagesV1Constants.adapterID,
        adapterRevision: AnthropicMessagesV1Constants.adapterRevision,
        endpoint: AnthropicMessagesV1Constants.endpoint,
        model: AnthropicMessagesV1Constants.model,
        requestMediaType: AnthropicMessagesV1Constants.mediaType,
        responseMediaType: AnthropicMessagesV1Constants.mediaType
    )
}

/// An opaque consumer implemented later by the reviewed FFI boundary. It has
/// no generic provider or payload selector.
protocol NativeAIAnthropicMessagesV1PreviewConsuming: Sendable {
    func consumeAnthropicMessagesV1PreviewOnce(
        deadlineNanoseconds: UInt64,
        deadlineObservation: NativeAIAnthropicMessagesV1DeadlineObservation
    ) async throws -> any NativeAIAnthropicMessagesV1Attempt
}

/// A paired wall/monotonic observation taken before the FFI preview consume.
/// EngineService uses it only to translate the core-owned wall expiry into the
/// lifecycle's monotonic domain without allowing either deadline to extend the
/// other.
struct NativeAIAnthropicMessagesV1DeadlineObservation: Equatable, Sendable {
    let monotonicNanoseconds: UInt64
    let unixMilliseconds: Int64
}

/// The exact, already privacy-reviewed core attempt. The returned value is an
/// inert receipt: no decoded provider data or action authority crosses back.
protocol NativeAIAnthropicMessagesV1Attempt: Sendable {
    var binding: NativeAIAnthropicMessagesV1Binding { get }
    var recordVersion: UInt32 { get }
    var inputSchemaVersion: UInt64 { get }
    var outputSchemaVersion: UInt64 { get }
    var privacyPolicyRevision: UInt64 { get }
    var providerBindingRevision: UInt64 { get }
    var canonicalMetadataJSON: Data { get }
    var inputDigestSHA256: String { get }
    var sourceScanID: String { get }
    var selectedRootNodeID: UInt64 { get }
    var deadlineNanoseconds: UInt64 { get }

    func validateOnce(
        extractedInnerJSON: Data
    ) async throws -> NativeAIAnthropicMessagesV1CoreValidatedResult
    func release()
}

struct NativeAIAnthropicMessagesV1ValidatedGroup: Equatable, Sendable,
    CustomStringConvertible, CustomDebugStringConvertible, CustomReflectable
{
    let recordVersion: UInt32
    let title: String
    let snapshotNodeIDs: [UInt64]
    let reason: String

    var description: String {
        "NativeAIAnthropicMessagesV1ValidatedGroup(nodeCount: \(snapshotNodeIDs.count))"
    }

    var debugDescription: String { description }
    var customMirror: Mirror {
        Mirror(
            self,
            children: ["recordVersion": recordVersion, "nodeCount": snapshotNodeIDs.count],
            displayStyle: .struct
        )
    }
}

struct NativeAIAnthropicMessagesV1CoreValidatedResult: Equatable, Sendable,
    CustomStringConvertible, CustomDebugStringConvertible, CustomReflectable
{
    let recordVersion: UInt32
    let inputSchemaVersion: UInt64
    let outputSchemaVersion: UInt64
    let privacyPolicyRevision: UInt64
    let providerBindingRevision: UInt64
    let binding: NativeAIAnthropicMessagesV1Binding
    let inputDigestSHA256: String
    let sourceScanID: String
    let selectedRootNodeID: UInt64
    let summary: String
    let labels: [String]
    let groups: [NativeAIAnthropicMessagesV1ValidatedGroup]
    let questions: [String]
    let uncertainties: [String]
    let researchSuggestions: [String]

    var description: String {
        "NativeAIAnthropicMessagesV1CoreValidatedResult(labelCount: \(labels.count), groupCount: \(groups.count), questionCount: \(questions.count), uncertaintyCount: \(uncertainties.count), researchSuggestionCount: \(researchSuggestions.count))"
    }

    var debugDescription: String { description }
    var customMirror: Mirror {
        Mirror(
            self,
            children: [
                "recordVersion": recordVersion,
                "labelCount": labels.count,
                "groupCount": groups.count,
                "questionCount": questions.count,
                "uncertaintyCount": uncertainties.count,
                "researchSuggestionCount": researchSuggestions.count,
            ],
            displayStyle: .struct
        )
    }

    func isTrustedProjection(
        of attempt: any NativeAIAnthropicMessagesV1Attempt
    ) -> Bool {
        guard recordVersion == attempt.recordVersion,
              inputSchemaVersion == attempt.inputSchemaVersion,
              outputSchemaVersion == attempt.outputSchemaVersion,
              privacyPolicyRevision == attempt.privacyPolicyRevision,
              providerBindingRevision == attempt.providerBindingRevision,
              binding == attempt.binding,
              inputDigestSHA256.utf8.count == 64,
              inputDigestSHA256.utf8.allSatisfy({
                  (0x30 ... 0x39).contains($0) || (0x61 ... 0x66).contains($0)
              }),
              inputDigestSHA256 == attempt.inputDigestSHA256,
              sourceScanID == attempt.sourceScanID,
              selectedRootNodeID == attempt.selectedRootNodeID,
              (1 ... 128).contains(sourceScanID.utf8.count),
              (1 ... 4096).contains(summary.utf8.count),
              labels.count <= 16,
              groups.count <= 32,
              questions.count <= 16,
              uncertainties.count <= 16,
              researchSuggestions.count <= 8,
              Self.validUniqueStrings(labels, maximumBytes: 64),
              Self.validUniqueStrings(questions, maximumBytes: 512),
              Self.validUniqueStrings(uncertainties, maximumBytes: 512),
              Self.validUniqueStrings(researchSuggestions, maximumBytes: 512)
        else { return false }

        var nodeIDs = Set<UInt64>()
        for group in groups {
            guard group.recordVersion == 1,
                  (1 ... 256).contains(group.title.utf8.count),
                  (1 ... 1024).contains(group.reason.utf8.count),
                  (1 ... 128).contains(group.snapshotNodeIDs.count),
                  group.snapshotNodeIDs.allSatisfy({ nodeIDs.insert($0).inserted })
            else { return false }
        }
        return nodeIDs.count <= 128
    }

    private static func validUniqueStrings(
        _ values: [String],
        maximumBytes: Int
    ) -> Bool {
        Set(values).count == values.count
            && values.allSatisfy { (1 ... maximumBytes).contains($0.utf8.count) }
    }
}

#if DEBUG
    extension NativeAIAnthropicMessagesV1CoreValidatedResult {
        static let testFixture = NativeAIAnthropicMessagesV1CoreValidatedResult(
            recordVersion: 1,
            inputSchemaVersion: 1,
            outputSchemaVersion: 1,
            privacyPolicyRevision: 1,
            providerBindingRevision: 1,
            binding: .trusted,
            inputDigestSHA256: String(repeating: "a", count: 64),
            sourceScanID: "scan:test",
            selectedRootNodeID: 1,
            summary: "Validated.",
            labels: [],
            groups: [],
            questions: [],
            uncertainties: [],
            researchSuggestions: []
        )
    }
#endif

struct NativeAIAnthropicMessagesV1Result: Equatable, Sendable,
    CustomStringConvertible, CustomDebugStringConvertible, CustomReflectable
{
    let provider: NativeAIAnthropicMessagesV1Binding
    let validated: NativeAIAnthropicMessagesV1CoreValidatedResult

    fileprivate init(validated: NativeAIAnthropicMessagesV1CoreValidatedResult) {
        provider = validated.binding
        self.validated = validated
    }

    var description: String { "NativeAIAnthropicMessagesV1Result(validated)" }
    var debugDescription: String { description }
    var customMirror: Mirror {
        Mirror(
            self,
            children: ["provider": provider.adapterID],
            displayStyle: .struct
        )
    }
}

enum NativeAIAnthropicMessagesV1OrchestratorFailure: String, Error, Equatable,
    Sendable, LocalizedError, CustomStringConvertible, CustomDebugStringConvertible
{
    case previewUnavailable
    case invalidCoreBinding
    case missingCredential
    case credentialUnavailable
    case requestRejected
    case deadlineExceeded
    case cancelled
    case redirect
    case authentication
    case invalidResponse
    case status
    case responseTooLarge
    case network
    case extractionRejected
    case coreRejected

    var errorDescription: String? { "Native AI request failed (\(rawValue))." }
    var description: String { errorDescription ?? "Native AI request failed." }
    var debugDescription: String { description }
}

final class NativeAIAnthropicMessagesV1Run: @unchecked Sendable {
    private let kernel: NativeAIRemoteLifecycleKernel

    fileprivate init(kernel: NativeAIRemoteLifecycleKernel) {
        self.kernel = kernel
    }

    func cancel() { kernel.cancel() }

    deinit { kernel.cancel() }
}

/// Production-compiled but deliberately has no application caller. The only
/// production initializer fixes Keychain, adapter, clock, and Foundation policy;
/// the preview consumer must later be supplied by the exact FFI bridge.
final class NativeAIAnthropicMessagesV1Orchestrator: @unchecked Sendable {
    private let lock = NSLock()
    private var started = false
    private let previewConsumer: any NativeAIAnthropicMessagesV1PreviewConsuming
    private let credentialReader: any AIProviderCredentialRequestReading
    private let adapter: AnthropicMessagesV1Adapter
    private let clock: any NativeAIRemoteClock
    private let networkFactory: any NativeAIRemoteNetworkFactory

    convenience init(previewConsumer: any NativeAIAnthropicMessagesV1PreviewConsuming) {
        self.init(
            previewConsumer: previewConsumer,
            credentialReader: AIProviderCredentialStore(),
            adapter: AnthropicMessagesV1Adapter(),
            clock: NativeAISystemClock(),
            networkFactory: NativeAIFoundationNetworkFactory()
        )
    }

    init(
        previewConsumer: any NativeAIAnthropicMessagesV1PreviewConsuming,
        credentialReader: any AIProviderCredentialRequestReading,
        adapter: AnthropicMessagesV1Adapter,
        clock: any NativeAIRemoteClock,
        networkFactory: any NativeAIRemoteNetworkFactory
    ) {
        self.previewConsumer = previewConsumer
        self.credentialReader = credentialReader
        self.adapter = adapter
        self.clock = clock
        self.networkFactory = networkFactory
    }

    @discardableResult
    func start(
        completion: @escaping @Sendable (
            Result<
                NativeAIAnthropicMessagesV1Result,
                NativeAIAnthropicMessagesV1OrchestratorFailure
            >
        ) -> Void
    ) -> NativeAIAnthropicMessagesV1Run? {
        let mayStart = lock.withLock { () -> Bool in
            guard !started else { return false }
            started = true
            return true
        }
        guard mayStart else {
            completion(.failure(.cancelled))
            return nil
        }
        let preparer = NativeAIAnthropicMessagesV1Preparer(
            previewConsumer: previewConsumer,
            credentialReader: credentialReader,
            adapter: adapter,
            clock: clock
        )
        let kernel = NativeAIRemoteLifecycleKernel(
            clock: clock,
            preparer: preparer,
            networkFactory: networkFactory
        ) { result in
            switch result {
            case let .success(response):
                guard case let .anthropicMessagesV1(validated) = response.receipt
                else { return completion(.failure(.coreRejected)) }
                completion(
                    .success(
                        NativeAIAnthropicMessagesV1Result(
                            validated: validated
                        )
                    )
                )
            case let .failure(error):
                completion(.failure(Self.map(error)))
            }
        }
        let run = NativeAIAnthropicMessagesV1Run(kernel: kernel)
        kernel.start()
        return run
    }

    private static func map(
        _ error: NativeAIRemoteFailure
    ) -> NativeAIAnthropicMessagesV1OrchestratorFailure {
        switch error {
        case .requestTooLarge, .preparation(.requestRejected): .requestRejected
        case .preparation(.previewUnavailable): .previewUnavailable
        case .preparation(.invalidCoreBinding): .invalidCoreBinding
        case .preparation(.missingCredential): .missingCredential
        case .preparation(.credentialUnavailable): .credentialUnavailable
        case .deadlineExceeded: .deadlineExceeded
        case .cancelled: .cancelled
        case .redirect: .redirect
        case .authentication: .authentication
        case .invalidResponse: .invalidResponse
        case .status: .status
        case .responseTooLarge: .responseTooLarge
        case .network: .network
        case .validation(.extraction): .extractionRejected
        case .validation(.coreRejected): .coreRejected
        }
    }
}

private final class NativeAIAnthropicMessagesV1TaskOperation: @unchecked Sendable,
    NativeAIRemoteCancellableOperation
{
    private let lock = NSLock()
    private var task: Task<Void, Never>?
    private var cancelled = false

    var isCancelled: Bool { lock.withLock { cancelled } }

    func install(_ task: Task<Void, Never>) {
        let cancelNow = lock.withLock { () -> Bool in
            guard self.task == nil else { return true }
            self.task = task
            return cancelled
        }
        if cancelNow { task.cancel() }
    }

    func cancel() {
        let task = lock.withLock { () -> Task<Void, Never>? in
            guard !cancelled else { return nil }
            cancelled = true
            return self.task
        }
        task?.cancel()
    }
}

private final class NativeAIAnthropicMessagesV1AttemptOwner: @unchecked Sendable {
    private let lock = NSLock()
    private var attempt: (any NativeAIAnthropicMessagesV1Attempt)?
    private var validationStarted = false
    private var validationActive = false
    private var releaseRequested = false

    init(_ attempt: any NativeAIAnthropicMessagesV1Attempt) {
        self.attempt = attempt
    }

    func beginValidation() -> (any NativeAIAnthropicMessagesV1Attempt)? {
        lock.withLock {
            guard !validationStarted, !releaseRequested, let attempt else {
                return nil
            }
            validationStarted = true
            validationActive = true
            return attempt
        }
    }

    func endValidation() {
        let released = lock.withLock { () -> (any NativeAIAnthropicMessagesV1Attempt)? in
            guard validationActive else { return nil }
            validationActive = false
            guard releaseRequested else { return nil }
            defer { attempt = nil }
            return attempt
        }
        released?.release()
    }

    func releaseWhenIdle() {
        let released = lock.withLock { () -> (any NativeAIAnthropicMessagesV1Attempt)? in
            guard !releaseRequested else { return nil }
            releaseRequested = true
            guard !validationActive else { return nil }
            defer { attempt = nil }
            return attempt
        }
        released?.release()
    }

    deinit { releaseWhenIdle() }
}

private struct NativeAIAnthropicMessagesV1ResponseValidator: NativeAIRemoteValidator {
    let adapter: AnthropicMessagesV1Adapter
    let owner: NativeAIAnthropicMessagesV1AttemptOwner

    func validate(
        body: Data,
        head: NativeAIRemoteResponseHead,
        completion: @escaping @Sendable (
            Result<NativeAIRemoteValidationReceipt, NativeAIRemoteValidationFailure>
        ) -> Void
    ) -> any NativeAIRemoteCancellableOperation {
        let extracted: Data
        switch adapter.extractResponse(body: body, mediaType: head.mediaType) {
        case let .success(value):
            extracted = value
        case .failure:
            completion(.failure(.extraction))
            return NativeAIAnthropicMessagesV1TaskOperation()
        }

        guard let attempt = owner.beginValidation() else {
            completion(.failure(.coreRejected))
            return NativeAIAnthropicMessagesV1TaskOperation()
        }

        let operation = NativeAIAnthropicMessagesV1TaskOperation()
        let task = Task {
            let result: Result<NativeAIRemoteValidationReceipt, NativeAIRemoteValidationFailure>
            do {
                let validated = try await attempt.validateOnce(
                    extractedInnerJSON: extracted
                )
                result = validated.isTrustedProjection(of: attempt)
                    ? .success(.anthropicMessagesV1(validated))
                    : .failure(.coreRejected)
            } catch {
                result = .failure(.coreRejected)
            }
            owner.endValidation()
            if !operation.isCancelled, !Task.isCancelled {
                completion(result)
            }
        }
        operation.install(task)
        return operation
    }
}

private struct NativeAIAnthropicMessagesV1Preparer: NativeAIRemotePreparer {
    let previewConsumer: any NativeAIAnthropicMessagesV1PreviewConsuming
    let credentialReader: any AIProviderCredentialRequestReading
    let adapter: AnthropicMessagesV1Adapter
    let clock: any NativeAIRemoteClock

    func prepare(
        deadlineNanoseconds: UInt64,
        completion: @escaping @Sendable (
            Result<NativeAIRemotePreparedRequest, NativeAIRemotePreparationFailure>
        ) -> Void
    ) -> any NativeAIRemoteCancellableOperation {
        let operation = NativeAIAnthropicMessagesV1TaskOperation()
        let task = Task {
            await Task.yield()
            guard !operation.isCancelled, !Task.isCancelled else { return }

            let attempt: any NativeAIAnthropicMessagesV1Attempt
            do {
                let observedNanoseconds = clock.nowNanoseconds()
                guard observedNanoseconds < deadlineNanoseconds,
                      let observedUnixMilliseconds = Self.nowUnixMilliseconds()
                else {
                    return
                }
                attempt = try await previewConsumer.consumeAnthropicMessagesV1PreviewOnce(
                    deadlineNanoseconds: deadlineNanoseconds,
                    deadlineObservation: NativeAIAnthropicMessagesV1DeadlineObservation(
                        monotonicNanoseconds: observedNanoseconds,
                        unixMilliseconds: observedUnixMilliseconds
                    )
                )
            } catch {
                if !operation.isCancelled, !Task.isCancelled {
                    completion(.failure(.previewUnavailable))
                }
                return
            }

            guard !operation.isCancelled,
                  !Task.isCancelled,
                  clock.nowNanoseconds() < attempt.deadlineNanoseconds
            else {
                attempt.release()
                return
            }
            guard attempt.binding == .trusted,
                  attempt.recordVersion == 1,
                  attempt.inputSchemaVersion == 1,
                  attempt.outputSchemaVersion == 1,
                  attempt.privacyPolicyRevision == 1,
                  attempt.providerBindingRevision == 1,
                  attempt.deadlineNanoseconds == deadlineNanoseconds,
                  attempt.inputDigestSHA256.utf8.count == 64,
                  attempt.inputDigestSHA256.utf8.allSatisfy({
                      (0x30 ... 0x39).contains($0) || (0x61 ... 0x66).contains($0)
                  }),
                  (1 ... 128).contains(attempt.sourceScanID.utf8.count),
                  !attempt.canonicalMetadataJSON.isEmpty
            else {
                attempt.release()
                completion(.failure(.invalidCoreBinding))
                return
            }

            let credential: AIProviderCredential?
            do {
                credential = try await credentialReader.readForSingleRequest(
                    for: .anthropicMessagesV1
                )
            } catch {
                attempt.release()
                if !operation.isCancelled, !Task.isCancelled {
                    completion(.failure(.credentialUnavailable))
                }
                return
            }

            guard !operation.isCancelled,
                  !Task.isCancelled,
                  clock.nowNanoseconds() < attempt.deadlineNanoseconds
            else {
                attempt.release()
                return
            }
            guard let credential else {
                attempt.release()
                completion(.failure(.missingCredential))
                return
            }

            let sealedRequest: NativeAIRemoteSealedRequest
            switch adapter.prepareRequest(
                canonicalMetadataJSON: attempt.canonicalMetadataJSON,
                credential: credential
            ) {
            case let .success(value):
                sealedRequest = value
            case .failure:
                attempt.release()
                completion(.failure(.requestRejected))
                return
            }

            guard !operation.isCancelled,
                  !Task.isCancelled,
                  clock.nowNanoseconds() < attempt.deadlineNanoseconds
            else {
                attempt.release()
                return
            }
            let owner = NativeAIAnthropicMessagesV1AttemptOwner(attempt)
            let prepared = NativeAIRemotePreparedRequest(
                sealedRequest: sealedRequest,
                validator: NativeAIAnthropicMessagesV1ResponseValidator(
                    adapter: adapter,
                    owner: owner
                ),
                release: { owner.releaseWhenIdle() }
            )
            guard !operation.isCancelled,
                  !Task.isCancelled,
                  clock.nowNanoseconds() < attempt.deadlineNanoseconds
            else {
                prepared.release()
                return
            }
            completion(.success(prepared))
        }
        operation.install(task)
        return operation
    }

    private static func nowUnixMilliseconds() -> Int64? {
        let milliseconds = Date().timeIntervalSince1970 * 1000
        guard milliseconds.isFinite,
              milliseconds >= 0,
              milliseconds <= Double(Int64.max)
        else { return nil }
        return Int64(milliseconds.rounded(.down))
    }
}
