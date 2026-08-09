import Foundation
import Observation

enum AIProviderSettingsOperation: Equatable {
    case idle
    case loading
    case saving
    case deleting
}

struct UnavailableAIProviderCredentialSettingsStore:
    AIProviderCredentialSettingsStoring
{
    func presence(
        for _: AIProviderCredentialAccount
    ) async -> AIProviderCredentialPresence {
        .absent
    }

    func replace(
        _: String,
        for _: AIProviderCredentialAccount
    ) async throws {
        throw AIProviderCredentialStoreError.unavailable
    }

    func delete(for _: AIProviderCredentialAccount) async throws {
        throw AIProviderCredentialStoreError.unavailable
    }
}

@MainActor
@Observable
final class AIProviderSettingsModel {
    let providerName: String
    let modelName: String
    private(set) var presence = AIProviderCredentialPresence.absent
    private(set) var operation = AIProviderSettingsOperation.idle
    private(set) var failureMessage: String?

    private let store: any AIProviderCredentialSettingsStoring
    private var generation: UInt64 = 0
    private var terminalQuiescenceStarted = false
    private var admittedOperationCount = 0
    private var terminalWaiters: [CheckedContinuation<Void, Never>] = []
    private var terminalQuiescenceTask: Task<Void, Never>?

    init(store: any AIProviderCredentialSettingsStoring) {
        self.store = store
        let disclosure = reviewedAnthropicMessagesV1Disclosure
        providerName = disclosure.providerName
        modelName = disclosure.model
    }

    var isBusy: Bool { operation != .idle }
    var hasSavedCredential: Bool { presence == .present }

    func refresh() async {
        guard !isBusy, beginOperation() else { return }
        defer { finishOperation() }
        generation &+= 1
        let operationGeneration = generation
        operation = .loading
        failureMessage = nil
        let presence = await store.presence(for: .anthropicMessagesV1)
        guard
            operationGeneration == generation,
            !terminalQuiescenceStarted,
            !Task.isCancelled
        else { return }
        self.presence = presence
        operation = .idle
        if case let .failed(error) = presence {
            failureMessage = error.errorDescription
        }
    }

    func save(_ value: String) async {
        guard !isBusy, beginOperation() else { return }
        defer { finishOperation() }
        generation &+= 1
        let operationGeneration = generation
        operation = .saving
        failureMessage = nil
        do {
            try await store.replace(value, for: .anthropicMessagesV1)
            guard
                operationGeneration == generation,
                !terminalQuiescenceStarted,
                !Task.isCancelled
            else { return }
            presence = .present
            operation = .idle
        } catch {
            guard
                operationGeneration == generation,
                !terminalQuiescenceStarted,
                !Task.isCancelled
            else { return }
            presence = .failed((error as? AIProviderCredentialStoreError) ?? .failed)
            failureMessage = (error as? LocalizedError)?.errorDescription
                ?? "The Anthropic API key could not be saved."
            operation = .idle
        }
    }

    func delete() async {
        guard !isBusy, beginOperation() else { return }
        defer { finishOperation() }
        generation &+= 1
        let operationGeneration = generation
        operation = .deleting
        failureMessage = nil
        do {
            try await store.delete(for: .anthropicMessagesV1)
            guard
                operationGeneration == generation,
                !terminalQuiescenceStarted,
                !Task.isCancelled
            else { return }
            presence = .absent
            operation = .idle
        } catch {
            guard
                operationGeneration == generation,
                !terminalQuiescenceStarted,
                !Task.isCancelled
            else { return }
            presence = .failed((error as? AIProviderCredentialStoreError) ?? .failed)
            failureMessage = (error as? LocalizedError)?.errorDescription
                ?? "The Anthropic API key could not be removed."
            operation = .idle
        }
    }

    /// Installs the terminal admission fence synchronously, then joins every
    /// Keychain operation accepted before that fence. Repeated callers share
    /// the same retained drain.
    func beginTerminalRuntimeQuiescence() -> Task<Void, Never> {
        if let terminalQuiescenceTask {
            return terminalQuiescenceTask
        }
        terminalQuiescenceStarted = true
        generation &+= 1
        let task = Task { @MainActor [weak self] in
            guard let self else { return }
            await waitForAdmittedOperations()
            operation = .idle
            failureMessage = nil
        }
        terminalQuiescenceTask = task
        return task
    }

    private func beginOperation() -> Bool {
        guard !terminalQuiescenceStarted else { return false }
        admittedOperationCount += 1
        return true
    }

    private func finishOperation() {
        precondition(admittedOperationCount > 0)
        admittedOperationCount -= 1
        guard admittedOperationCount == 0 else { return }
        let waiters = terminalWaiters
        terminalWaiters.removeAll(keepingCapacity: false)
        for waiter in waiters {
            waiter.resume()
        }
    }

    private func waitForAdmittedOperations() async {
        guard admittedOperationCount != 0 else { return }
        await withCheckedContinuation { continuation in
            terminalWaiters.append(continuation)
        }
    }
}
