@testable import DUX
import XCTest

@MainActor
final class AIProviderSettingsModelTests: XCTestCase {
    func testAccessibilityIdentifiersAreUniqueAndDisjointFromExplorer() {
        let identifiers = AIProviderSettingsAccessibility.allControlIdentifiers
        XCTAssertEqual(Set(identifiers).count, identifiers.count)
        XCTAssertTrue(
            Set(identifiers).isDisjoint(with: ExplorerAccessibility.allIdentifiers)
        )
    }

    func testRefreshSaveAndDeleteUseOnlyAnthropicAccount() async {
        let store = AIProviderSettingsStoreSpy()
        let model = AIProviderSettingsModel(store: store)

        await model.refresh()
        XCTAssertEqual(model.presence, .absent)

        await model.save("sk-ant-test")
        XCTAssertEqual(model.presence, .present)
        let saved = await store.savedValues()
        XCTAssertEqual(saved.count, 1)
        XCTAssertEqual(saved.first?.account, .anthropicMessagesV1)
        XCTAssertEqual(saved.first?.value, "sk-ant-test")

        await model.delete()
        XCTAssertEqual(model.presence, .absent)
        let deleted = await store.deletedAccounts()
        XCTAssertEqual(deleted, [.anthropicMessagesV1])
    }

    func testFailedSavePublishesBoundedFailure() async {
        let store = AIProviderSettingsStoreSpy(replaceFailure: .invalidCredential)
        let model = AIProviderSettingsModel(store: store)
        await model.save(" secret ")

        XCTAssertEqual(model.presence, .failed(.invalidCredential))
        XCTAssertEqual(
            model.failureMessage,
            AIProviderCredentialStoreError.invalidCredential.errorDescription
        )
    }

    func testTerminalFenceJoinsAcceptedSaveAndRejectsLatePublication() async throws {
        let store = SuspendedAIProviderSettingsStore()
        let model = AIProviderSettingsModel(store: store)
        let save = Task { @MainActor in
            await model.save("terminal-secret")
        }
        for _ in 0 ..< 2000 {
            if await store.hasSuspendedReplace() { break }
            try await Task.sleep(for: .milliseconds(1))
        }
        let hasSuspendedReplace = await store.hasSuspendedReplace()
        XCTAssertTrue(hasSuspendedReplace)

        let drain = model.beginTerminalRuntimeQuiescence()
        XCTAssertEqual(model.operation, .saving)
        await store.resumeReplace()
        await save.value
        await drain.value

        XCTAssertEqual(model.operation, .idle)
        XCTAssertEqual(model.presence, .absent)
        await model.save("late-secret")
        let replacementCount = await store.replacementCount()
        XCTAssertEqual(replacementCount, 1)
    }
}

private actor SuspendedAIProviderSettingsStore: AIProviderCredentialSettingsStoring {
    private var replacements = 0
    private var replaceContinuation: CheckedContinuation<Void, Never>?

    func presence(
        for _: AIProviderCredentialAccount
    ) async -> AIProviderCredentialPresence {
        .absent
    }

    func replace(
        _: String,
        for _: AIProviderCredentialAccount
    ) async throws {
        replacements += 1
        await withCheckedContinuation { continuation in
            replaceContinuation = continuation
        }
    }

    func delete(for _: AIProviderCredentialAccount) async throws {}

    func hasSuspendedReplace() -> Bool { replaceContinuation != nil }

    func resumeReplace() {
        replaceContinuation?.resume()
        replaceContinuation = nil
    }

    func replacementCount() -> Int { replacements }
}

private actor AIProviderSettingsStoreSpy: AIProviderCredentialSettingsStoring {
    struct SavedValue: Equatable, Sendable {
        let value: String
        let account: AIProviderCredentialAccount
    }

    private var presenceValue = AIProviderCredentialPresence.absent
    private var saved: [SavedValue] = []
    private var deleted: [AIProviderCredentialAccount] = []
    private let replaceFailure: AIProviderCredentialStoreError?

    init(replaceFailure: AIProviderCredentialStoreError? = nil) {
        self.replaceFailure = replaceFailure
    }

    func presence(
        for _: AIProviderCredentialAccount
    ) async -> AIProviderCredentialPresence {
        presenceValue
    }

    func replace(
        _ value: String,
        for account: AIProviderCredentialAccount
    ) async throws {
        if let replaceFailure { throw replaceFailure }
        saved.append(SavedValue(value: value, account: account))
        presenceValue = .present
    }

    func delete(for account: AIProviderCredentialAccount) async throws {
        deleted.append(account)
        presenceValue = .absent
    }

    func savedValues() -> [SavedValue] { saved }
    func deletedAccounts() -> [AIProviderCredentialAccount] { deleted }
}
