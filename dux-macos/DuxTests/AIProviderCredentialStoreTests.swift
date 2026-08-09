import Foundation
import LocalAuthentication
import Security
@testable import DUX
import XCTest

final class AIProviderCredentialStoreTests: XCTestCase {
    func testAdapterAccountsAreClosedToTheTwoReservedIdentifiers() {
        XCTAssertEqual(
            AIProviderCredentialAccount.allCases,
            [.anthropicMessagesV1, .openAIResponsesV1]
        )
        XCTAssertEqual(
            AIProviderCredentialAccount.anthropicMessagesV1.rawValue,
            "anthropic-messages-v1"
        )
        XCTAssertEqual(
            AIProviderCredentialAccount.openAIResponsesV1.rawValue,
            "openai-responses-v1"
        )
    }

    func testReadUsesExactTupleAndReturnsOnlyValidatedCredential() async throws {
        let secret = "sk-test_READ-123!"
        let keychain = ScriptedAIProviderKeychain(
            reads: [readResult(errSecSuccess, secret)]
        )
        let store = AIProviderCredentialStore(keychain: keychain)

        let credential = try await store.readForSingleRequest(
            for: .anthropicMessagesV1
        )

        XCTAssertEqual(credential, try AIProviderCredential(secret))
        let operations = keychain.recordedOperations()
        guard case let .copy(query)? = operations.first else {
            return XCTFail("expected one copy operation")
        }
        XCTAssertEqual(operations.count, 1)
        assertExactReadQuery(query, account: .anthropicMessagesV1)
    }

    func testPresenceMapsAbsentLockedUnavailableAndUnexpectedFailure() async {
        let keychain = ScriptedAIProviderKeychain(
            reads: [
                readResult(errSecItemNotFound),
                readResult(errSecInteractionNotAllowed),
                readResult(errSecNotAvailable),
                readResult(errSecParam),
                readResult(errSecSuccess, "present-key"),
            ]
        )
        let store = AIProviderCredentialStore(keychain: keychain)

        let absent = await store.presence(for: .openAIResponsesV1)
        let locked = await store.presence(for: .openAIResponsesV1)
        let unavailable = await store.presence(for: .openAIResponsesV1)
        let failed = await store.presence(for: .openAIResponsesV1)
        let present = await store.presence(for: .openAIResponsesV1)
        XCTAssertEqual(absent, .absent)
        XCTAssertEqual(locked, .failed(.locked))
        XCTAssertEqual(unavailable, .failed(.unavailable))
        XCTAssertEqual(failed, .failed(.failed))
        XCTAssertEqual(present, .present)

        for operation in keychain.recordedOperations() {
            guard case let .copy(query) = operation else {
                return XCTFail("presence must only read")
            }
            assertExactReadQuery(query, account: .openAIResponsesV1)
        }
    }

    func testReadRejectsMissingOrMalformedStoredDataWithoutReflectingIt() async {
        let malformed = "stored secret with spaces"
        let keychain = ScriptedAIProviderKeychain(
            reads: [
                AIProviderKeychainReadResult(status: errSecSuccess, data: nil),
                readResult(errSecSuccess, malformed),
            ]
        )
        let store = AIProviderCredentialStore(keychain: keychain)

        await assertStoreError(.corruptCredential) {
            _ = try await store.readForSingleRequest(for: .anthropicMessagesV1)
        }
        await assertStoreError(.corruptCredential) {
            _ = try await store.readForSingleRequest(for: .anthropicMessagesV1)
        }

        let error = AIProviderCredentialStoreError.corruptCredential
        for rendered in [
            error.localizedDescription,
            String(describing: error),
            String(reflecting: error),
        ] {
            XCTAssertFalse(rendered.contains(malformed))
        }
    }

    func testReplaceCreatesWithExactTupleDeviceOnlyAccessibilityAndValue() async throws {
        let secret = "create-key_123"
        let keychain = ScriptedAIProviderKeychain(adds: [errSecSuccess])
        let store = AIProviderCredentialStore(keychain: keychain)

        try await store.replace(secret, for: .openAIResponsesV1)

        let operations = keychain.recordedOperations()
        guard case let .add(attributes)? = operations.first else {
            return XCTFail("expected one add operation")
        }
        XCTAssertEqual(operations.count, 1)
        assertExactAddQuery(
            attributes,
            account: .openAIResponsesV1,
            secret: secret
        )
    }

    func testReplaceUpdatesExactExistingItemAndItsAccessibility() async throws {
        let secret = "replacement-key_456"
        let keychain = ScriptedAIProviderKeychain(
            adds: [errSecDuplicateItem],
            updates: [errSecSuccess]
        )
        let store = AIProviderCredentialStore(keychain: keychain)

        try await store.replace(secret, for: .anthropicMessagesV1)

        let operations = keychain.recordedOperations()
        XCTAssertEqual(operations.count, 2)
        guard case let .add(addQuery) = operations[0],
              case let .update(query, attributes) = operations[1]
        else {
            return XCTFail("expected add then update")
        }
        assertExactAddQuery(
            addQuery,
            account: .anthropicMessagesV1,
            secret: secret
        )
        assertExactItemQuery(query, account: .anthropicMessagesV1)
        assertExactReplacementAttributes(attributes, secret: secret)
    }

    func testReplaceSettlesDisappearanceAndReappearanceRacesWithBoundedRetries() async throws {
        let disappears = ScriptedAIProviderKeychain(
            adds: [errSecDuplicateItem, errSecSuccess],
            updates: [errSecItemNotFound]
        )
        try await AIProviderCredentialStore(keychain: disappears).replace(
            "race-key-1",
            for: .openAIResponsesV1
        )
        XCTAssertEqual(disappears.recordedOperations().kinds, [.add, .update, .add])

        let reappears = ScriptedAIProviderKeychain(
            adds: [errSecDuplicateItem, errSecDuplicateItem],
            updates: [errSecItemNotFound, errSecSuccess]
        )
        try await AIProviderCredentialStore(keychain: reappears).replace(
            "race-key-2",
            for: .openAIResponsesV1
        )
        let operations = reappears.recordedOperations()
        XCTAssertEqual(operations.kinds, [.add, .update, .add, .update])
        for operation in operations {
            switch operation {
            case let .add(query):
                assertExactAddQuery(
                    query,
                    account: .openAIResponsesV1,
                    secret: "race-key-2"
                )
            case let .update(query, attributes):
                assertExactItemQuery(query, account: .openAIResponsesV1)
                assertExactReplacementAttributes(attributes, secret: "race-key-2")
            default:
                XCTFail("replace used an unexpected operation")
            }
        }
    }

    func testReplacementFailureMappingIsBoundedAndNeverContainsSecret() async {
        let secret = "never-print-this-key"
        let cases: [(OSStatus, AIProviderCredentialStoreError)] = [
            (errSecInteractionNotAllowed, .locked),
            (errSecNotAvailable, .unavailable),
            (errSecAuthFailed, .failed),
            (errSecParam, .failed),
        ]

        for (status, expected) in cases {
            let keychain = ScriptedAIProviderKeychain(adds: [status])
            let store = AIProviderCredentialStore(keychain: keychain)
            do {
                try await store.replace(secret, for: .anthropicMessagesV1)
                XCTFail("expected replacement failure")
            } catch let error as AIProviderCredentialStoreError {
                XCTAssertEqual(error, expected)
                for rendered in [
                    error.localizedDescription,
                    String(describing: error),
                    String(reflecting: error),
                ] {
                    XCTAssertFalse(rendered.contains(secret))
                }
            } catch {
                XCTFail("unexpected error type: \(type(of: error))")
            }
        }
    }

    func testCredentialValidationIsConservativeBoundedAndPrecedesKeychainAccess() async throws {
        let invalid = [
            "",
            "contains space",
            "contains\nnewline",
            "contains\ttab",
            "non-ascii-å",
            String(repeating: "x", count: AIProviderCredential.maximumUTF8ByteCount + 1),
        ]

        for value in invalid {
            let keychain = ScriptedAIProviderKeychain()
            let store = AIProviderCredentialStore(keychain: keychain)
            await assertStoreError(.invalidCredential) {
                try await store.replace(value, for: .openAIResponsesV1)
            }
            XCTAssertTrue(keychain.recordedOperations().isEmpty)
        }

        let boundedVisibleASCII = "!" + String(
            repeating: "x",
            count: AIProviderCredential.maximumUTF8ByteCount - 2
        ) + "~"
        let keychain = ScriptedAIProviderKeychain(adds: [errSecSuccess])
        try await AIProviderCredentialStore(keychain: keychain).replace(
            boundedVisibleASCII,
            for: .openAIResponsesV1
        )
        XCTAssertEqual(keychain.recordedOperations().count, 1)
    }

    func testCredentialDescriptionDebugDescriptionAndMirrorAreRedacted() throws {
        let secret = "sk-redaction-sentinel"
        let credential = try AIProviderCredential(secret)

        XCTAssertFalse(String(describing: credential).contains(secret))
        XCTAssertFalse(String(reflecting: credential).contains(secret))
        XCTAssertFalse(
            credential.customMirror.children.contains {
                String(describing: $0.value).contains(secret)
            }
        )
    }

    func testDeleteUsesExactTupleAndIsIdempotent() async throws {
        let keychain = ScriptedAIProviderKeychain(
            deletes: [errSecSuccess, errSecItemNotFound]
        )
        let store = AIProviderCredentialStore(keychain: keychain)

        try await store.delete(for: .anthropicMessagesV1)
        try await store.delete(for: .anthropicMessagesV1)

        let operations = keychain.recordedOperations()
        XCTAssertEqual(operations.count, 2)
        for operation in operations {
            guard case let .delete(query) = operation else {
                return XCTFail("delete used an unexpected operation")
            }
            assertExactItemQuery(query, account: .anthropicMessagesV1)
        }
    }

    func testDeleteMapsLockedAndOtherFailuresWithoutRawStatusDetails() async {
        for (status, expected) in [
            (errSecInteractionNotAllowed, AIProviderCredentialStoreError.locked),
            (errSecNotAvailable, .unavailable),
            (errSecAuthFailed, .failed),
            (errSecParam, .failed),
        ] {
            let store = AIProviderCredentialStore(
                keychain: ScriptedAIProviderKeychain(deletes: [status])
            )
            await assertStoreError(expected) {
                try await store.delete(for: .openAIResponsesV1)
            }
        }
    }

    func testWrongServiceAccountOrTupleFlagsCannotSatisfyRead() async throws {
        let keychain = TupleMatchingAIProviderKeychain(
            records: [
                .init(
                    itemClass: kSecClassGenericPassword as String,
                    dataProtection: true,
                    service: "wrong.service",
                    account: AIProviderCredentialAccount.anthropicMessagesV1.rawValue,
                    synchronizable: false,
                    data: Data("wrong-service-secret".utf8)
                ),
                .init(
                    itemClass: kSecClassGenericPassword as String,
                    dataProtection: true,
                    service: AIProviderCredentialStore.service,
                    account: AIProviderCredentialAccount.openAIResponsesV1.rawValue,
                    synchronizable: false,
                    data: Data("wrong-account-secret".utf8)
                ),
                .init(
                    itemClass: kSecClassGenericPassword as String,
                    dataProtection: true,
                    service: AIProviderCredentialStore.service,
                    account: AIProviderCredentialAccount.anthropicMessagesV1.rawValue,
                    synchronizable: true,
                    data: Data("sync-secret".utf8)
                ),
            ]
        )
        let store = AIProviderCredentialStore(keychain: keychain)

        let credential = try await store.readForSingleRequest(
            for: .anthropicMessagesV1
        )
        XCTAssertNil(credential)

        guard case let .copy(query)? = keychain.recordedOperations().first else {
            return XCTFail("expected a read query")
        }
        assertExactReadQuery(query, account: .anthropicMessagesV1)
    }
}

private enum RecordedKeychainOperation {
    case add([CFString: Any])
    case copy([CFString: Any])
    case update([CFString: Any], [CFString: Any])
    case delete([CFString: Any])

    enum Kind: Equatable {
        case add
        case copy
        case update
        case delete
    }

    var kind: Kind {
        switch self {
        case .add: .add
        case .copy: .copy
        case .update: .update
        case .delete: .delete
        }
    }
}

private extension Array where Element == RecordedKeychainOperation {
    var kinds: [RecordedKeychainOperation.Kind] { map(\.kind) }
}

private final class ScriptedAIProviderKeychain: AIProviderKeychainClient,
    @unchecked Sendable
{
    private let lock = NSLock()
    private var adds: [OSStatus]
    private var reads: [AIProviderKeychainReadResult]
    private var updates: [OSStatus]
    private var deletes: [OSStatus]
    private var operations: [RecordedKeychainOperation] = []

    init(
        adds: [OSStatus] = [],
        reads: [AIProviderKeychainReadResult] = [],
        updates: [OSStatus] = [],
        deletes: [OSStatus] = []
    ) {
        self.adds = adds
        self.reads = reads
        self.updates = updates
        self.deletes = deletes
    }

    func add(_ attributes: [CFString: Any]) -> OSStatus {
        lock.withLock {
            operations.append(.add(attributes))
            return adds.isEmpty ? errSecParam : adds.removeFirst()
        }
    }

    func copyMatching(_ query: [CFString: Any]) -> AIProviderKeychainReadResult {
        lock.withLock {
            operations.append(.copy(query))
            return reads.isEmpty ? readResult(errSecParam) : reads.removeFirst()
        }
    }

    func update(
        _ query: [CFString: Any],
        attributes: [CFString: Any]
    ) -> OSStatus {
        lock.withLock {
            operations.append(.update(query, attributes))
            return updates.isEmpty ? errSecParam : updates.removeFirst()
        }
    }

    func delete(_ query: [CFString: Any]) -> OSStatus {
        lock.withLock {
            operations.append(.delete(query))
            return deletes.isEmpty ? errSecParam : deletes.removeFirst()
        }
    }

    func recordedOperations() -> [RecordedKeychainOperation] {
        lock.withLock { operations }
    }
}

private final class TupleMatchingAIProviderKeychain: AIProviderKeychainClient,
    @unchecked Sendable
{
    struct Record: Sendable {
        let itemClass: String
        let dataProtection: Bool
        let service: String
        let account: String
        let synchronizable: Bool
        let data: Data
    }

    private let lock = NSLock()
    private let records: [Record]
    private var operations: [RecordedKeychainOperation] = []

    init(records: [Record]) {
        self.records = records
    }

    func add(_: [CFString: Any]) -> OSStatus { errSecParam }

    func copyMatching(_ query: [CFString: Any]) -> AIProviderKeychainReadResult {
        lock.withLock {
            operations.append(.copy(query))
            guard let match = records.first(where: { $0.matches(query) }) else {
                return readResult(errSecItemNotFound)
            }
            return AIProviderKeychainReadResult(
                status: errSecSuccess,
                data: match.data
            )
        }
    }

    func update(_: [CFString: Any], attributes _: [CFString: Any]) -> OSStatus {
        errSecParam
    }

    func delete(_: [CFString: Any]) -> OSStatus { errSecParam }

    func recordedOperations() -> [RecordedKeychainOperation] {
        lock.withLock { operations }
    }
}

private extension TupleMatchingAIProviderKeychain.Record {
    func matches(_ query: [CFString: Any]) -> Bool {
        (query[kSecClass] as? String) == itemClass
            && (query[kSecUseDataProtectionKeychain] as? Bool) == dataProtection
            && (query[kSecAttrService] as? String) == service
            && (query[kSecAttrAccount] as? String) == account
            && (query[kSecAttrSynchronizable] as? Bool) == synchronizable
    }
}

private func readResult(
    _ status: OSStatus,
    _ value: String? = nil
) -> AIProviderKeychainReadResult {
    AIProviderKeychainReadResult(
        status: status,
        data: value.map { Data($0.utf8) }
    )
}

private func assertExactItemQuery(
    _ query: [CFString: Any],
    account: AIProviderCredentialAccount,
    file: StaticString = #filePath,
    line: UInt = #line
) {
    XCTAssertEqual(
        keyNames(query),
        keyNames(AIProviderCredentialStore.itemQuery(for: account)),
        file: file,
        line: line
    )
    XCTAssertEqual(
        query[kSecClass] as? String,
        kSecClassGenericPassword as String,
        file: file,
        line: line
    )
    XCTAssertEqual(
        query[kSecUseDataProtectionKeychain] as? Bool,
        true,
        file: file,
        line: line
    )
    XCTAssertEqual(
        query[kSecAttrService] as? String,
        AIProviderCredentialStore.service,
        file: file,
        line: line
    )
    XCTAssertEqual(
        query[kSecAttrAccount] as? String,
        account.rawValue,
        file: file,
        line: line
    )
    XCTAssertEqual(
        query[kSecAttrSynchronizable] as? Bool,
        false,
        file: file,
        line: line
    )
    let authenticationContext = query[kSecUseAuthenticationContext] as? LAContext
    XCTAssertEqual(
        authenticationContext?.interactionNotAllowed,
        true,
        file: file,
        line: line
    )
    XCTAssertNil(query[kSecAttrAccessGroup], file: file, line: line)
}

private func assertExactReadQuery(
    _ query: [CFString: Any],
    account: AIProviderCredentialAccount,
    file: StaticString = #filePath,
    line: UInt = #line
) {
    var itemOnly = query
    XCTAssertEqual(itemOnly.removeValue(forKey: kSecReturnData) as? Bool, true)
    XCTAssertEqual(
        itemOnly.removeValue(forKey: kSecMatchLimit) as? String,
        kSecMatchLimitOne as String,
        file: file,
        line: line
    )
    assertExactItemQuery(itemOnly, account: account, file: file, line: line)
}

private func assertExactAddQuery(
    _ query: [CFString: Any],
    account: AIProviderCredentialAccount,
    secret: String,
    file: StaticString = #filePath,
    line: UInt = #line
) {
    var itemOnly = query
    XCTAssertEqual(
        itemOnly.removeValue(forKey: kSecAttrAccessible) as? String,
        kSecAttrAccessibleWhenUnlockedThisDeviceOnly as String,
        file: file,
        line: line
    )
    XCTAssertEqual(
        itemOnly.removeValue(forKey: kSecValueData) as? Data,
        Data(secret.utf8),
        file: file,
        line: line
    )
    assertExactItemQuery(itemOnly, account: account, file: file, line: line)
}

private func assertExactReplacementAttributes(
    _ attributes: [CFString: Any],
    secret: String,
    file: StaticString = #filePath,
    line: UInt = #line
) {
    XCTAssertEqual(
        keyNames(attributes),
        Set([
            kSecAttrAccessible as String,
            kSecValueData as String,
        ]),
        file: file,
        line: line
    )
    XCTAssertEqual(
        attributes[kSecAttrAccessible] as? String,
        kSecAttrAccessibleWhenUnlockedThisDeviceOnly as String,
        file: file,
        line: line
    )
    XCTAssertEqual(
        attributes[kSecValueData] as? Data,
        Data(secret.utf8),
        file: file,
        line: line
    )
    XCTAssertNil(attributes[kSecAttrAccessGroup], file: file, line: line)
    XCTAssertNil(attributes[kSecAttrSynchronizable], file: file, line: line)
}

private func keyNames(_ dictionary: [CFString: Any]) -> Set<String> {
    Set(dictionary.keys.map { $0 as String })
}

private func assertStoreError(
    _ expected: AIProviderCredentialStoreError,
    file: StaticString = #filePath,
    line: UInt = #line,
    operation: () async throws -> Void
) async {
    do {
        try await operation()
        XCTFail("expected credential store error", file: file, line: line)
    } catch let error as AIProviderCredentialStoreError {
        XCTAssertEqual(error, expected, file: file, line: line)
    } catch {
        XCTFail("unexpected error type: \(type(of: error))", file: file, line: line)
    }
}
