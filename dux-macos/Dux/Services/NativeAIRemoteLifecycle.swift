import Dispatch
import Foundation

// This one-shot lifecycle has no application caller. The exact provider adapter
// seals the request, while the prepared validator owns extraction and core
// validation. Callers cannot provide a generic transport payload.

enum NativeAIRemoteLimits {
    static let requestBytes = 384 * 1_024
    static let responseBytes = 64 * 1_024
    static let responseMediaTypeBytes = 256
    static let deadlineNanoseconds: UInt64 = 60 * 1_000_000_000
}

enum NativeAIRemotePreparationFailure: Error, Equatable, Sendable {
    case previewUnavailable
    case invalidCoreBinding
    case missingCredential
    case credentialUnavailable
    case requestRejected
}

enum NativeAIRemoteValidationFailure: Error, Equatable, Sendable {
    case extraction
    case coreRejected
}

enum NativeAIRemoteFailure: Error, Equatable, Sendable {
    case requestTooLarge
    case preparation(NativeAIRemotePreparationFailure)
    case deadlineExceeded
    case cancelled
    case redirect
    case authentication
    case invalidResponse
    case status
    case responseTooLarge
    case network
    case validation(NativeAIRemoteValidationFailure)
}

struct NativeAIRemoteResponseHead: Sendable {
    let statusCode: Int
    let declaredContentLength: Int64?
    let mediaType: String?
}

enum NativeAIRemoteValidationReceipt: Equatable, Sendable {
    case anthropicMessagesV1(NativeAIAnthropicMessagesV1CoreValidatedResult)
}

struct NativeAIRemoteAcceptedResponse: Sendable {
    let deliveredByteCount: Int
    let mediaType: String?
    let receipt: NativeAIRemoteValidationReceipt
}

struct NativeAIRemoteSealedRequest: Sendable, CustomStringConvertible,
    CustomDebugStringConvertible, CustomReflectable
{
    private let request: URLRequest
    private let encodedBody: Data

    init(request: URLRequest, encodedBody: Data) {
        precondition(request.httpBody == encodedBody)
        self.request = request
        self.encodedBody = encodedBody
    }

    var bodyByteCount: Int { encodedBody.count }

    func withRequestForSingleTransaction<Result>(
        _ body: (URLRequest, Data) throws -> Result
    ) rethrows -> Result {
        try body(request, encodedBody)
    }

    var description: String { "NativeAIRemoteSealedRequest(redacted)" }
    var debugDescription: String { description }
    var customMirror: Mirror {
        Mirror(
            self,
            children: EmptyCollection<(label: String?, value: Any)>(),
            displayStyle: .struct
        )
    }
}

protocol NativeAIRemoteScheduledAction: Sendable {
    func cancel()
}

protocol NativeAIRemoteClock: Sendable {
    func nowNanoseconds() -> UInt64
    func schedule(
        at deadlineNanoseconds: UInt64,
        _ action: @escaping @Sendable () -> Void
    ) -> any NativeAIRemoteScheduledAction
}

protocol NativeAIRemoteCancellableOperation: Sendable {
    func cancel()
}

protocol NativeAIRemoteValidator: Sendable {
    func validate(
        body: Data,
        head: NativeAIRemoteResponseHead,
        completion: @escaping @Sendable (
            Result<NativeAIRemoteValidationReceipt, NativeAIRemoteValidationFailure>
        ) -> Void
    ) -> any NativeAIRemoteCancellableOperation
}

final class NativeAIRemotePreparedRequest: @unchecked Sendable,
    CustomStringConvertible, CustomDebugStringConvertible, CustomReflectable
{
    let sealedRequest: NativeAIRemoteSealedRequest
    let validator: any NativeAIRemoteValidator

    private let lock = NSLock()
    private var releaseAction: (@Sendable () -> Void)?

    init(
        sealedRequest: NativeAIRemoteSealedRequest,
        validator: any NativeAIRemoteValidator,
        release: @escaping @Sendable () -> Void
    ) {
        self.sealedRequest = sealedRequest
        self.validator = validator
        self.releaseAction = release
    }

    func release() {
        let action = lock.withLock { () -> (@Sendable () -> Void)? in
            defer { releaseAction = nil }
            return releaseAction
        }
        action?()
    }

    deinit { release() }

    var description: String { "NativeAIRemotePreparedRequest(redacted)" }
    var debugDescription: String { description }
    var customMirror: Mirror {
        Mirror(
            self,
            children: EmptyCollection<(label: String?, value: Any)>(),
            displayStyle: .class
        )
    }
}

protocol NativeAIRemotePreparer: Sendable {
    func prepare(
        deadlineNanoseconds: UInt64,
        completion: @escaping @Sendable (
            Result<NativeAIRemotePreparedRequest, NativeAIRemotePreparationFailure>
        ) -> Void
    ) -> any NativeAIRemoteCancellableOperation
}

struct NativeAIRemoteNetworkSink: Sendable {
    let response: @Sendable (NativeAIRemoteResponseHead) -> Bool
    let data: @Sendable (Data, Int) -> Void
    let completed: @Sendable (Bool) -> Void
    let redirected: @Sendable () -> Void
    let rejectedAuthentication: @Sendable () -> Void
}

protocol NativeAIRemoteNetworkTransaction: Sendable {
    func resume()
    func cancel()
    func invalidate()
}

protocol NativeAIRemoteNetworkFactory: Sendable {
    func makeTransaction(
        request: NativeAIRemoteSealedRequest,
        sink: NativeAIRemoteNetworkSink
    ) -> any NativeAIRemoteNetworkTransaction
}

private final class NativeAIRemoteGuardedTransaction: @unchecked Sendable {
    // `raw` is Sendable; the lock serializes the resume/cancel/invalidate fence.
    private let lock = NSLock()
    private let raw: any NativeAIRemoteNetworkTransaction
    private var resumed = false
    private var cancelled = false
    private var invalidated = false

    init(_ raw: any NativeAIRemoteNetworkTransaction) {
        self.raw = raw
    }

    func resume() {
        let shouldResume = lock.withLock {
            guard !resumed, !cancelled, !invalidated else { return false }
            resumed = true
            return true
        }
        if shouldResume {
            raw.resume()
        }
    }

    func cancel() {
        let shouldCancel = lock.withLock {
            guard !cancelled else { return false }
            cancelled = true
            return true
        }
        if shouldCancel {
            raw.cancel()
        }
    }

    func invalidate() {
        let shouldInvalidate = lock.withLock {
            guard !invalidated else { return false }
            invalidated = true
            return true
        }
        if shouldInvalidate {
            raw.invalidate()
        }
    }
}

final class NativeAIRemoteLifecycleKernel: @unchecked Sendable {
    // All mutable lifecycle state is protected by `lock`; callbacks can arrive
    // concurrently from Foundation, cancellation, and the injected seams.
    private enum Phase {
        case idle
        case preparing
        case receiving
        case validating
        case terminal
    }

    private struct State {
        var phase: Phase = .idle
        var generation: UInt64 = 0
        var deadlineNanoseconds: UInt64 = 0
        var scheduledAction: (any NativeAIRemoteScheduledAction)?
        var preparation: (any NativeAIRemoteCancellableOperation)?
        var transaction: NativeAIRemoteGuardedTransaction?
        var validation: (any NativeAIRemoteCancellableOperation)?
        var preparedRequest: NativeAIRemotePreparedRequest?
        var responseHead: NativeAIRemoteResponseHead?
        var responseBody = Data()
        var deliveredByteCount = 0
        var completion: (@Sendable (Result<NativeAIRemoteAcceptedResponse, NativeAIRemoteFailure>) -> Void)?
    }

    private struct TerminalActions {
        let scheduledAction: (any NativeAIRemoteScheduledAction)?
        let preparation: (any NativeAIRemoteCancellableOperation)?
        let transaction: NativeAIRemoteGuardedTransaction?
        let validation: (any NativeAIRemoteCancellableOperation)?
        let preparedRequest: NativeAIRemotePreparedRequest?
        let completion: @Sendable (Result<NativeAIRemoteAcceptedResponse, NativeAIRemoteFailure>) -> Void
    }

    private let lock = NSLock()
    private let clock: any NativeAIRemoteClock
    private let preparer: any NativeAIRemotePreparer
    private let networkFactory: any NativeAIRemoteNetworkFactory
    private var state: State

    init(
        clock: any NativeAIRemoteClock,
        preparer: any NativeAIRemotePreparer,
        networkFactory: any NativeAIRemoteNetworkFactory,
        completion: @escaping @Sendable (
            Result<NativeAIRemoteAcceptedResponse, NativeAIRemoteFailure>
        ) -> Void
    ) {
        self.clock = clock
        self.preparer = preparer
        self.networkFactory = networkFactory
        self.state = State()
        self.state.completion = completion
    }

    deinit {
        let teardown = lock.withLock { () -> (
            (any NativeAIRemoteScheduledAction)?,
            (any NativeAIRemoteCancellableOperation)?,
            NativeAIRemoteGuardedTransaction?,
            (any NativeAIRemoteCancellableOperation)?,
            NativeAIRemotePreparedRequest?
        ) in
            let values = (
                state.scheduledAction,
                state.preparation,
                state.transaction,
                state.validation,
                state.preparedRequest
            )
            state.scheduledAction = nil
            state.preparation = nil
            state.transaction = nil
            state.validation = nil
            state.preparedRequest = nil
            state.completion = nil
            state.phase = .terminal
            return values
        }
        teardown.0?.cancel()
        teardown.1?.cancel()
        teardown.2?.cancel()
        teardown.2?.invalidate()
        teardown.3?.cancel()
        teardown.4?.release()
    }

    func start() {
        let now = clock.nowNanoseconds()
        let (deadline, overflow) = now.addingReportingOverflow(
            NativeAIRemoteLimits.deadlineNanoseconds
        )
        let generation = lock.withLock { () -> UInt64? in
            guard state.phase == .idle else { return nil }
            state.generation &+= 1
            state.phase = .preparing
            state.deadlineNanoseconds = deadline
            return state.generation
        }
        guard let generation else { return }
        guard !overflow else {
            finish(.failure(.deadlineExceeded), generation: generation)
            return
        }

        let scheduledAction = clock.schedule(at: deadline) { [weak self] in
            self?.finish(.failure(.deadlineExceeded), generation: generation)
        }
        let keepSchedule = lock.withLock { () -> Bool in
            guard state.phase != .terminal, state.generation == generation else {
                return false
            }
            state.scheduledAction = scheduledAction
            return true
        }
        guard keepSchedule else {
            scheduledAction.cancel()
            return
        }

        guard enforceDeadline(generation: generation) else { return }
        let preparation = preparer.prepare(
            deadlineNanoseconds: deadline
        ) { [weak self] result in
            self?.preparationCompleted(result, generation: generation)
        }
        let attached = lock.withLock { () -> Bool in
            guard state.phase == .preparing, state.generation == generation else {
                return false
            }
            state.preparation = preparation
            return true
        }
        if !attached {
            preparation.cancel()
        }
    }

    func cancel() {
        finish(.failure(.cancelled), generation: nil)
    }

    private func preparationCompleted(
        _ result: Result<NativeAIRemotePreparedRequest, NativeAIRemotePreparationFailure>,
        generation: UInt64
    ) {
        guard case let .success(preparedRequest) = result else {
            if case let .failure(error) = result {
                finish(.failure(.preparation(error)), generation: generation)
            }
            return
        }
        guard enforceDeadline(generation: generation) else {
            preparedRequest.release()
            return
        }
        guard preparedRequest.sealedRequest.bodyByteCount <= NativeAIRemoteLimits.requestBytes else {
            preparedRequest.release()
            finish(.failure(.requestTooLarge), generation: generation)
            return
        }

        let sink = NativeAIRemoteNetworkSink(
            response: { [weak self] head in
                self?.receivedResponse(head, generation: generation) ?? false
            },
            data: { [weak self] data, accountedByteCount in
                self?.receivedData(
                    data,
                    accountedByteCount: accountedByteCount,
                    generation: generation
                )
            },
            completed: { [weak self] failed in
                self?.networkCompleted(failed: failed, generation: generation)
            },
            redirected: { [weak self] in
                self?.finish(.failure(.redirect), generation: generation)
            },
            rejectedAuthentication: { [weak self] in
                self?.finish(.failure(.authentication), generation: generation)
            }
        )
        let transaction = NativeAIRemoteGuardedTransaction(
            networkFactory.makeTransaction(
                request: preparedRequest.sealedRequest,
                sink: sink
            )
        )
        let attached = lock.withLock { () -> Bool in
            guard state.phase == .preparing, state.generation == generation else {
                return false
            }
            state.phase = .receiving
            state.preparation = nil
            state.transaction = transaction
            state.preparedRequest = preparedRequest
            return true
        }
        guard attached else {
            transaction.cancel()
            transaction.invalidate()
            preparedRequest.release()
            return
        }
        guard enforceDeadline(generation: generation) else { return }
        transaction.resume()
    }

    private func receivedResponse(
        _ head: NativeAIRemoteResponseHead,
        generation: UInt64
    ) -> Bool {
        guard enforceDeadline(generation: generation) else { return false }
        guard head.statusCode == 200 else {
            finish(.failure(.status), generation: generation)
            return false
        }
        if let declaredLength = head.declaredContentLength {
            guard declaredLength >= 0 else {
                finish(.failure(.invalidResponse), generation: generation)
                return false
            }
            guard declaredLength <= Int64(NativeAIRemoteLimits.responseBytes) else {
                finish(.failure(.responseTooLarge), generation: generation)
                return false
            }
        }
        if let mediaType = head.mediaType,
           mediaType.utf8.count > NativeAIRemoteLimits.responseMediaTypeBytes
        {
            finish(.failure(.invalidResponse), generation: generation)
            return false
        }
        return lock.withLock {
            guard state.phase == .receiving, state.generation == generation else {
                return false
            }
            guard state.responseHead == nil else {
                return false
            }
            state.responseHead = head
            return true
        }
    }

    private func receivedData(
        _ data: Data,
        accountedByteCount: Int,
        generation: UInt64
    ) {
        guard enforceDeadline(generation: generation) else { return }
        guard accountedByteCount >= 0 else {
            finish(.failure(.invalidResponse), generation: generation)
            return
        }

        let failure = lock.withLock { () -> NativeAIRemoteFailure? in
            guard state.phase == .receiving,
                  state.generation == generation,
                  state.responseHead != nil
            else {
                return nil
            }
            let (newCount, overflow) = state.deliveredByteCount.addingReportingOverflow(
                accountedByteCount
            )
            guard !overflow, newCount <= NativeAIRemoteLimits.responseBytes else {
                return .responseTooLarge
            }
            // Production always accounts exactly Data.count. The separate count
            // exists only so the DEBUG harness can exercise checked arithmetic
            // without allocating an unbounded buffer.
            guard accountedByteCount == data.count else {
                return .invalidResponse
            }
            state.deliveredByteCount = newCount
            state.responseBody.append(data)
            return nil
        }
        if let failure {
            finish(.failure(failure), generation: generation)
        }
    }

    private func networkCompleted(failed: Bool, generation: UInt64) {
        guard enforceDeadline(generation: generation) else { return }
        if failed {
            finish(.failure(.network), generation: generation)
            return
        }

        let validationInput = lock.withLock { () -> (
            Data,
            NativeAIRemoteResponseHead,
            any NativeAIRemoteValidator
        )? in
            guard state.phase == .receiving,
                  state.generation == generation,
                  let head = state.responseHead,
                  let validator = state.preparedRequest?.validator
            else {
                return nil
            }
            state.phase = .validating
            return (state.responseBody, head, validator)
        }
        guard let validationInput else {
            let missingResponse = lock.withLock {
                state.phase == .receiving && state.generation == generation
            }
            if missingResponse {
                finish(.failure(.invalidResponse), generation: generation)
            }
            return
        }
        guard enforceDeadline(generation: generation) else { return }

        let validation = validationInput.2.validate(
            body: validationInput.0,
            head: validationInput.1
        ) { [weak self] result in
            self?.validationCompleted(
                result,
                bodyByteCount: validationInput.0.count,
                mediaType: validationInput.1.mediaType,
                generation: generation
            )
        }
        let attached = lock.withLock { () -> Bool in
            guard state.phase == .validating, state.generation == generation else {
                return false
            }
            state.validation = validation
            return true
        }
        if !attached {
            validation.cancel()
        }
    }

    private func validationCompleted(
        _ result: Result<NativeAIRemoteValidationReceipt, NativeAIRemoteValidationFailure>,
        bodyByteCount: Int,
        mediaType: String?,
        generation: UInt64
    ) {
        guard enforceDeadline(generation: generation) else { return }
        switch result {
        case let .success(receipt):
            finish(
                .success(
                    NativeAIRemoteAcceptedResponse(
                        deliveredByteCount: bodyByteCount,
                        mediaType: mediaType,
                        receipt: receipt
                    )
                ),
                generation: generation
            )
        case let .failure(error):
            finish(.failure(.validation(error)), generation: generation)
        }
    }

    private func enforceDeadline(generation: UInt64) -> Bool {
        let now = clock.nowNanoseconds()
        let expired = lock.withLock {
            guard state.phase != .terminal, state.generation == generation else {
                return false
            }
            return now >= state.deadlineNanoseconds
        }
        if expired {
            finish(.failure(.deadlineExceeded), generation: generation)
            return false
        }
        return lock.withLock {
            state.phase != .terminal && state.generation == generation
        }
    }

    private func finish(
        _ result: Result<NativeAIRemoteAcceptedResponse, NativeAIRemoteFailure>,
        generation: UInt64?
    ) {
        let actions = lock.withLock { () -> TerminalActions? in
            guard state.phase != .terminal,
                  state.phase != .idle,
                  generation == nil || generation == state.generation,
                  let completion = state.completion
            else {
                return nil
            }
            state.phase = .terminal
            state.generation &+= 1
            state.completion = nil
            let terminalActions = TerminalActions(
                scheduledAction: state.scheduledAction,
                preparation: state.preparation,
                transaction: state.transaction,
                validation: state.validation,
                preparedRequest: state.preparedRequest,
                completion: completion
            )
            state.scheduledAction = nil
            state.preparation = nil
            state.transaction = nil
            state.validation = nil
            state.preparedRequest = nil
            state.responseBody.removeAll(keepingCapacity: false)
            state.responseHead = nil
            return terminalActions
        }
        guard let actions else { return }

        actions.scheduledAction?.cancel()
        actions.preparation?.cancel()
        if case .failure = result {
            actions.transaction?.cancel()
        }
        actions.transaction?.invalidate()
        actions.validation?.cancel()
        actions.preparedRequest?.release()
        actions.completion(result)
    }
}

private final class NativeAISystemScheduledAction: @unchecked Sendable {
    // DispatchWorkItem cancellation is serialized to make repeated teardown safe.
    private let lock = NSLock()
    private var workItem: DispatchWorkItem?

    init(workItem: DispatchWorkItem) {
        self.workItem = workItem
    }

    func cancel() {
        let item = lock.withLock { () -> DispatchWorkItem? in
            defer { workItem = nil }
            return workItem
        }
        item?.cancel()
    }
}

extension NativeAISystemScheduledAction: NativeAIRemoteScheduledAction {}

struct NativeAISystemClock: NativeAIRemoteClock {
    private let queue = DispatchQueue(label: "se.mjukis.dux.native-ai-deadline")

    func nowNanoseconds() -> UInt64 {
        DispatchTime.now().uptimeNanoseconds
    }

    func schedule(
        at deadlineNanoseconds: UInt64,
        _ action: @escaping @Sendable () -> Void
    ) -> any NativeAIRemoteScheduledAction {
        let item = DispatchWorkItem(block: action)
        queue.asyncAfter(
            deadline: DispatchTime(uptimeNanoseconds: deadlineNanoseconds),
            execute: item
        )
        return NativeAISystemScheduledAction(workItem: item)
    }
}

enum NativeAIFoundationDriverPolicy {
    static func configuration() -> URLSessionConfiguration {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.httpCookieStorage = nil
        configuration.urlCache = nil
        configuration.urlCredentialStorage = nil
        configuration.httpShouldSetCookies = false
        configuration.waitsForConnectivity = false
        configuration.isDiscretionary = false
        configuration.requestCachePolicy = .reloadIgnoringLocalCacheData
        return configuration
    }

    static func authenticationDisposition(
        method: String
    ) -> URLSession.AuthChallengeDisposition {
        if method == NSURLAuthenticationMethodServerTrust {
            return .performDefaultHandling
        }
        return .cancelAuthenticationChallenge
    }
}

private final class NativeAIFoundationNetworkTransaction: NSObject,
    NativeAIRemoteNetworkTransaction,
    URLSessionDataDelegate,
    @unchecked Sendable
{
    // URLSession may invoke delegate methods concurrently. Mutable callback
    // fences are lock-protected; URLSession and URLSessionTask are thread-safe.
    private let lock = NSLock()
    private let sink: NativeAIRemoteNetworkSink
    private let request: NativeAIRemoteSealedRequest
    private var session: URLSession?
    private var task: URLSessionDataTask?
    private var resumed = false
    private var cancelled = false
    private var invalidated = false
    private var redirectReported = false

    init(request: NativeAIRemoteSealedRequest, sink: NativeAIRemoteNetworkSink) {
        self.request = request
        self.sink = sink
        super.init()
    }

    func resume() {
        lock.withLock {
            guard !resumed, !cancelled, !invalidated else { return }
            let delegateQueue = OperationQueue()
            delegateQueue.maxConcurrentOperationCount = 1
            let session = URLSession(
                configuration: NativeAIFoundationDriverPolicy.configuration(),
                delegate: self,
                delegateQueue: delegateQueue
            )
            let task = request.withRequestForSingleTransaction { request, _ in
                session.dataTask(with: request)
            }
            self.session = session
            self.task = task
            resumed = true
            task.resume()
        }
    }

    func cancel() {
        let task = lock.withLock { () -> URLSessionDataTask? in
            guard !cancelled else { return nil }
            cancelled = true
            return self.task
        }
        task?.cancel()
    }

    func invalidate() {
        let session = lock.withLock { () -> URLSession? in
            guard !invalidated else { return nil }
            invalidated = true
            return self.session
        }
        session?.finishTasksAndInvalidate()
    }

    func urlSession(
        _: URLSession,
        task _: URLSessionTask,
        willPerformHTTPRedirection _: HTTPURLResponse,
        newRequest _: URLRequest,
        completionHandler: @escaping @Sendable (URLRequest?) -> Void
    ) {
        completionHandler(nil)
        let shouldReport = lock.withLock {
            guard !redirectReported else { return false }
            redirectReported = true
            return true
        }
        if shouldReport {
            sink.redirected()
        }
    }

    func urlSession(
        _: URLSession,
        task _: URLSessionTask,
        didReceive challenge: URLAuthenticationChallenge,
        completionHandler: @escaping @Sendable (
            URLSession.AuthChallengeDisposition,
            URLCredential?
        ) -> Void
    ) {
        handle(challenge, completionHandler: completionHandler)
    }

    func urlSession(
        _: URLSession,
        didReceive challenge: URLAuthenticationChallenge,
        completionHandler: @escaping @Sendable (
            URLSession.AuthChallengeDisposition,
            URLCredential?
        ) -> Void
    ) {
        handle(challenge, completionHandler: completionHandler)
    }

    func urlSession(
        _: URLSession,
        dataTask _: URLSessionDataTask,
        didReceive response: URLResponse,
        completionHandler: @escaping @Sendable (URLSession.ResponseDisposition) -> Void
    ) {
        guard let response = response as? HTTPURLResponse else {
            completionHandler(.cancel)
            sink.completed(true)
            return
        }
        let expectedLength = response.expectedContentLength
        let accepted = sink.response(
            NativeAIRemoteResponseHead(
                statusCode: response.statusCode,
                declaredContentLength: expectedLength == -1
                    ? nil
                    : expectedLength,
                // Preserve only the bounded Content-Type value. Exact matching
                // (including parameters) belongs to the future fixed adapter.
                mediaType: response.value(forHTTPHeaderField: "Content-Type")
            )
        )
        completionHandler(accepted ? .allow : .cancel)
    }

    func urlSession(
        _: URLSession,
        dataTask _: URLSessionDataTask,
        didReceive data: Data
    ) {
        sink.data(data, data.count)
    }

    func urlSession(
        _: URLSession,
        task _: URLSessionTask,
        didCompleteWithError error: (any Error)?
    ) {
        sink.completed(error != nil)
    }

    private func handle(
        _ challenge: URLAuthenticationChallenge,
        completionHandler: @escaping @Sendable (
            URLSession.AuthChallengeDisposition,
            URLCredential?
        ) -> Void
    ) {
        let disposition = NativeAIFoundationDriverPolicy.authenticationDisposition(
            method: challenge.protectionSpace.authenticationMethod
        )
        completionHandler(disposition, nil)
        if disposition == .cancelAuthenticationChallenge {
            sink.rejectedAuthentication()
        }
    }
}

struct NativeAIFoundationNetworkFactory: NativeAIRemoteNetworkFactory {
    func makeTransaction(
        request: NativeAIRemoteSealedRequest,
        sink: NativeAIRemoteNetworkSink
    ) -> any NativeAIRemoteNetworkTransaction {
        NativeAIFoundationNetworkTransaction(request: request, sink: sink)
    }
}

#if DEBUG
enum NativeAIRemoteLifecycleTestFailure: String, Error, Equatable, Sendable,
    CustomStringConvertible
{
    case requestTooLarge
    case preparation
    case deadlineExceeded
    case cancelled
    case redirect
    case authentication
    case invalidResponse
    case status
    case responseTooLarge
    case network
    case validation

    var description: String { rawValue }
}

enum NativeAIRemoteLifecycleTestResult: Equatable, Sendable {
    case accepted(deliveredByteCount: Int, mediaType: String?)
    case failure(NativeAIRemoteLifecycleTestFailure)
}

struct NativeAIRemoteLifecycleTestRequestFixture: Sendable {
    let bodyByteCount: Int

    init(bodyByteCount: Int) {
        precondition(bodyByteCount >= 0)
        self.bodyByteCount = bodyByteCount
    }
}

struct NativeAIRemoteLifecycleTestRequestObservation: Equatable, Sendable {
    let bodyByteCount: Int
    let method: String
    let scheme: String
    let host: String
    let reloadsIgnoringLocalCache: Bool
    let handlesCookies: Bool

    fileprivate init(_ request: NativeAIRemoteSealedRequest) {
        let observation = request.withRequestForSingleTransaction { urlRequest, body in
            (
                body.count,
                urlRequest.httpMethod ?? "",
                urlRequest.url?.scheme ?? "",
                urlRequest.url?.host ?? "",
                urlRequest.cachePolicy == .reloadIgnoringLocalCacheData,
                urlRequest.httpShouldHandleCookies
            )
        }
        bodyByteCount = observation.0
        method = observation.1
        scheme = observation.2
        host = observation.3
        reloadsIgnoringLocalCache = observation.4
        handlesCookies = observation.5
    }
}

struct NativeAIRemoteLifecycleTestValidationObservation: Equatable, Sendable {
    let deliveredByteCount: Int
    let mediaType: String?
}

protocol NativeAIRemoteLifecycleTestScheduledAction: Sendable {
    func cancel()
}

protocol NativeAIRemoteLifecycleTestClock: Sendable {
    func nowNanoseconds() -> UInt64
    func schedule(
        at deadlineNanoseconds: UInt64,
        _ action: @escaping @Sendable () -> Void
    ) -> any NativeAIRemoteLifecycleTestScheduledAction
}

protocol NativeAIRemoteLifecycleTestPreparation: Sendable {
    func prepare(_ completion: @escaping @Sendable (Bool) -> Void)
}

protocol NativeAIRemoteLifecycleTestValidation: Sendable {
    func validate(
        _ observation: NativeAIRemoteLifecycleTestValidationObservation,
        completion: @escaping @Sendable (Bool) -> Void
    )
}

protocol NativeAIRemoteLifecycleTestNetworkTransaction: Sendable {
    func resume()
    func cancel()
    func invalidate()
}

protocol NativeAIRemoteLifecycleTestNetworkFactory: Sendable {
    func makeTransaction(
        request: NativeAIRemoteLifecycleTestRequestObservation,
        events: NativeAIRemoteLifecycleTestNetworkEvents
    ) -> any NativeAIRemoteLifecycleTestNetworkTransaction
}

final class NativeAIRemoteLifecycleTestNetworkEvents: Sendable {
    private static let fixedMediaType = "application/x-dux-future-adapter+json"
    private let sink: NativeAIRemoteNetworkSink

    fileprivate init(sink: NativeAIRemoteNetworkSink) {
        self.sink = sink
    }

    @discardableResult
    func receiveResponse(
        statusCode: Int = 200,
        declaredContentLength: Int64? = nil
    ) -> Bool {
        sink.response(
            NativeAIRemoteResponseHead(
                statusCode: statusCode,
                declaredContentLength: declaredContentLength,
                mediaType: Self.fixedMediaType
            )
        )
    }

    func receiveDeliveredBytes(_ byteCount: Int) {
        precondition(byteCount >= 0)
        let data: Data
        if byteCount <= NativeAIRemoteLimits.responseBytes + 1 {
            data = Data(repeating: 0, count: byteCount)
        } else {
            data = Data()
        }
        sink.data(data, byteCount)
    }

    func complete(networkFailed: Bool = false) {
        sink.completed(networkFailed)
    }

    func receiveRedirect() {
        sink.redirected()
    }

    func receiveNonServerTrustAuthenticationChallenge() {
        sink.rejectedAuthentication()
    }
}

struct NativeAIRemoteLifecycleTestConfiguration: Equatable, Sendable {
    let hasCookieStorage: Bool
    let hasURLCache: Bool
    let hasCredentialStorage: Bool
    let setsCookies: Bool
    let waitsForConnectivity: Bool
    let isDiscretionary: Bool
    let reloadsIgnoringLocalCache: Bool
}

enum NativeAIRemoteLifecycleTestChallengeDisposition: Equatable, Sendable {
    case performDefaultHandling
    case cancel
}

struct NativeAIRemoteLifecycleTestDriverPolicy: Equatable, Sendable {
    let followsRedirects: Bool
    let serverTrustDisposition: NativeAIRemoteLifecycleTestChallengeDisposition
    let otherAuthenticationDisposition: NativeAIRemoteLifecycleTestChallengeDisposition
}

private struct NativeAITestScheduledActionBridge: NativeAIRemoteScheduledAction {
    let raw: any NativeAIRemoteLifecycleTestScheduledAction

    func cancel() {
        raw.cancel()
    }
}

private struct NativeAITestClockBridge: NativeAIRemoteClock {
    let raw: any NativeAIRemoteLifecycleTestClock

    func nowNanoseconds() -> UInt64 {
        raw.nowNanoseconds()
    }

    func schedule(
        at deadlineNanoseconds: UInt64,
        _ action: @escaping @Sendable () -> Void
    ) -> any NativeAIRemoteScheduledAction {
        NativeAITestScheduledActionBridge(
            raw: raw.schedule(at: deadlineNanoseconds, action)
        )
    }
}

private struct NativeAITestCancellableOperation: NativeAIRemoteCancellableOperation {
    func cancel() {}
}

private struct NativeAITestPreparerBridge: NativeAIRemotePreparer {
    let raw: any NativeAIRemoteLifecycleTestPreparation
    let bodyByteCount: Int
    let validator: any NativeAIRemoteValidator

    func prepare(
        deadlineNanoseconds _: UInt64,
        completion: @escaping @Sendable (
            Result<NativeAIRemotePreparedRequest, NativeAIRemotePreparationFailure>
        ) -> Void
    ) -> any NativeAIRemoteCancellableOperation {
        raw.prepare { succeeded in
            guard succeeded else {
                completion(.failure(.requestRejected))
                return
            }
            var request = URLRequest(
                url: URL(string: "https://dux-native-ai.invalid/")!,
                cachePolicy: .reloadIgnoringLocalCacheData
            )
            request.httpMethod = "POST"
            request.httpShouldHandleCookies = false
            let body = Data(repeating: 0, count: bodyByteCount)
            request.httpBody = body
            completion(
                .success(
                    NativeAIRemotePreparedRequest(
                        sealedRequest: NativeAIRemoteSealedRequest(
                            request: request,
                            encodedBody: body
                        ),
                        validator: validator,
                        release: {}
                    )
                )
            )
        }
        return NativeAITestCancellableOperation()
    }
}

private struct NativeAITestValidatorBridge: NativeAIRemoteValidator {
    let raw: any NativeAIRemoteLifecycleTestValidation

    func validate(
        body: Data,
        head: NativeAIRemoteResponseHead,
        completion: @escaping @Sendable (
            Result<NativeAIRemoteValidationReceipt, NativeAIRemoteValidationFailure>
        ) -> Void
    ) -> any NativeAIRemoteCancellableOperation {
        raw.validate(
            NativeAIRemoteLifecycleTestValidationObservation(
                deliveredByteCount: body.count,
                mediaType: head.mediaType
            )
        ) { accepted in
            completion(
                accepted
                    ? .success(.anthropicMessagesV1(.testFixture))
                    : .failure(.coreRejected)
            )
        }
        return NativeAITestCancellableOperation()
    }
}

private struct NativeAITestNetworkTransactionBridge: NativeAIRemoteNetworkTransaction {
    let raw: any NativeAIRemoteLifecycleTestNetworkTransaction

    func resume() { raw.resume() }
    func cancel() { raw.cancel() }
    func invalidate() { raw.invalidate() }
}

private struct NativeAITestNetworkFactoryBridge: NativeAIRemoteNetworkFactory {
    let raw: any NativeAIRemoteLifecycleTestNetworkFactory

    func makeTransaction(
        request: NativeAIRemoteSealedRequest,
        sink: NativeAIRemoteNetworkSink
    ) -> any NativeAIRemoteNetworkTransaction {
        NativeAITestNetworkTransactionBridge(
            raw: raw.makeTransaction(
                request: NativeAIRemoteLifecycleTestRequestObservation(request),
                events: NativeAIRemoteLifecycleTestNetworkEvents(sink: sink)
            )
        )
    }
}

final class NativeAIRemoteLifecycleTestRun: @unchecked Sendable {
    // The kernel is internally lock-protected; this wrapper has no mutable state.
    private let kernel: NativeAIRemoteLifecycleKernel

    fileprivate init(kernel: NativeAIRemoteLifecycleKernel) {
        self.kernel = kernel
    }

    func cancel() {
        kernel.cancel()
    }

    deinit {
        kernel.cancel()
    }
}

struct NativeAIRemoteLifecycleTestHarness: Sendable {
    let fixture: NativeAIRemoteLifecycleTestRequestFixture
    let clock: any NativeAIRemoteLifecycleTestClock
    let preparation: any NativeAIRemoteLifecycleTestPreparation
    let validation: any NativeAIRemoteLifecycleTestValidation
    let network: any NativeAIRemoteLifecycleTestNetworkFactory

    static var requestByteLimit: Int { NativeAIRemoteLimits.requestBytes }
    static var responseByteLimit: Int { NativeAIRemoteLimits.responseBytes }
    static var deadlineNanoseconds: UInt64 { NativeAIRemoteLimits.deadlineNanoseconds }

    static var productionConfiguration: NativeAIRemoteLifecycleTestConfiguration {
        let configuration = NativeAIFoundationDriverPolicy.configuration()
        return NativeAIRemoteLifecycleTestConfiguration(
            hasCookieStorage: configuration.httpCookieStorage != nil,
            hasURLCache: configuration.urlCache != nil,
            hasCredentialStorage: configuration.urlCredentialStorage != nil,
            setsCookies: configuration.httpShouldSetCookies,
            waitsForConnectivity: configuration.waitsForConnectivity,
            isDiscretionary: configuration.isDiscretionary,
            reloadsIgnoringLocalCache:
                configuration.requestCachePolicy == .reloadIgnoringLocalCacheData
        )
    }

    static var productionDriverPolicy: NativeAIRemoteLifecycleTestDriverPolicy {
        let serverTrust = NativeAIFoundationDriverPolicy.authenticationDisposition(
            method: NSURLAuthenticationMethodServerTrust
        )
        let other = NativeAIFoundationDriverPolicy.authenticationDisposition(
            method: NSURLAuthenticationMethodHTTPBasic
        )
        return NativeAIRemoteLifecycleTestDriverPolicy(
            followsRedirects: false,
            serverTrustDisposition: serverTrust == .performDefaultHandling
                ? .performDefaultHandling
                : .cancel,
            otherAuthenticationDisposition: other == .performDefaultHandling
                ? .performDefaultHandling
                : .cancel
        )
    }

    @discardableResult
    func start(
        completion: @escaping @Sendable (NativeAIRemoteLifecycleTestResult) -> Void
    ) -> NativeAIRemoteLifecycleTestRun {
        let validator = NativeAITestValidatorBridge(raw: validation)
        let kernel = NativeAIRemoteLifecycleKernel(
            clock: NativeAITestClockBridge(raw: clock),
            preparer: NativeAITestPreparerBridge(
                raw: preparation,
                bodyByteCount: fixture.bodyByteCount,
                validator: validator
            ),
            networkFactory: NativeAITestNetworkFactoryBridge(raw: network)
        ) { result in
            switch result {
            case let .success(response):
                completion(
                    .accepted(
                        deliveredByteCount: response.deliveredByteCount,
                        mediaType: response.mediaType
                    )
                )
            case let .failure(error):
                completion(.failure(Self.map(error)))
            }
        }
        let run = NativeAIRemoteLifecycleTestRun(kernel: kernel)
        kernel.start()
        return run
    }

    private static func map(
        _ error: NativeAIRemoteFailure
    ) -> NativeAIRemoteLifecycleTestFailure {
        switch error {
        case .requestTooLarge: .requestTooLarge
        case .preparation: .preparation
        case .deadlineExceeded: .deadlineExceeded
        case .cancelled: .cancelled
        case .redirect: .redirect
        case .authentication: .authentication
        case .invalidResponse: .invalidResponse
        case .status: .status
        case .responseTooLarge: .responseTooLarge
        case .network: .network
        case .validation: .validation
        }
    }
}
#endif
