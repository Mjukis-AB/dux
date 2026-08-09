import Foundation
import LocalAuthentication
import Security

/// Keychain account names are code-owned so callers cannot select an arbitrary
/// credential item. Network identity and authentication remain outside this
/// dormant storage boundary.
enum AIProviderCredentialAccount: String, CaseIterable, Sendable {
    case anthropicMessagesV1 = "anthropic-messages-v1"
    case openAIResponsesV1 = "openai-responses-v1"
}

struct AIProviderCredential: Equatable, Sendable, CustomStringConvertible,
    CustomDebugStringConvertible, CustomReflectable
{
    static let maximumUTF8ByteCount = 512

    fileprivate let utf8: Data

    init(_ value: String) throws {
        try self.init(validatingUTF8: Data(value.utf8))
    }

    fileprivate init(validatingUTF8 data: Data) throws {
        guard !data.isEmpty,
              data.count <= Self.maximumUTF8ByteCount,
              data.allSatisfy({ (0x21 ... 0x7E).contains($0) })
        else {
            throw AIProviderCredentialStoreError.invalidCredential
        }
        utf8 = data
    }

    var description: String { "<redacted-ai-provider-credential>" }
    var debugDescription: String { description }
    var customMirror: Mirror {
        Mirror(self, children: ["value": description])
    }
}

enum AIProviderCredentialPresence: Equatable, Sendable {
    case absent
    case present
    case failed(AIProviderCredentialStoreError)
}

enum AIProviderCredentialStoreError: Error, Equatable, Sendable,
    LocalizedError, CustomStringConvertible, CustomDebugStringConvertible
{
    case invalidCredential
    case corruptCredential
    case locked
    case unavailable
    case failed

    var errorDescription: String? {
        switch self {
        case .invalidCredential:
            "The provider credential is invalid."
        case .corruptCredential:
            "The stored provider credential is invalid."
        case .locked:
            "The provider credential is unavailable while the Keychain is locked."
        case .unavailable:
            "The provider credential store is unavailable."
        case .failed:
            "The provider credential operation failed."
        }
    }

    var description: String { errorDescription ?? "Provider credential failure." }
    var debugDescription: String { description }
}

struct AIProviderKeychainReadResult: Sendable {
    let status: OSStatus
    let data: Data?
}

/// Narrow injection seam around Security.framework. It deliberately accepts
/// only complete Keychain dictionaries; no service/account/access-group input
/// can enter through the client implementation.
protocol AIProviderKeychainClient: Sendable {
    func add(_ attributes: [CFString: Any]) -> OSStatus
    func copyMatching(_ query: [CFString: Any]) -> AIProviderKeychainReadResult
    func update(
        _ query: [CFString: Any],
        attributes: [CFString: Any]
    ) -> OSStatus
    func delete(_ query: [CFString: Any]) -> OSStatus
}

/// Settings can inspect state and make an explicit replacement or deletion,
/// but cannot recover secret bytes.
protocol AIProviderCredentialSettingsStoring: Sendable {
    func presence(
        for account: AIProviderCredentialAccount
    ) async -> AIProviderCredentialPresence
    func replace(
        _ value: String,
        for account: AIProviderCredentialAccount
    ) async throws
    func delete(for account: AIProviderCredentialAccount) async throws
}

/// A request adapter receives the narrow secret-reading capability without
/// gaining mutation authority.
protocol AIProviderCredentialRequestReading: Sendable {
    func readForSingleRequest(
        for account: AIProviderCredentialAccount
    ) async throws -> AIProviderCredential?
}

private struct SystemAIProviderKeychainClient: AIProviderKeychainClient {
    func add(_ attributes: [CFString: Any]) -> OSStatus {
        SecItemAdd(attributes as CFDictionary, nil)
    }

    func copyMatching(_ query: [CFString: Any]) -> AIProviderKeychainReadResult {
        var item: CFTypeRef?
        let status = SecItemCopyMatching(query as CFDictionary, &item)
        return AIProviderKeychainReadResult(status: status, data: item as? Data)
    }

    func update(
        _ query: [CFString: Any],
        attributes: [CFString: Any]
    ) -> OSStatus {
        SecItemUpdate(query as CFDictionary, attributes as CFDictionary)
    }

    func delete(_ query: [CFString: Any]) -> OSStatus {
        SecItemDelete(query as CFDictionary)
    }
}

actor AIProviderCredentialStore: AIProviderCredentialSettingsStoring,
    AIProviderCredentialRequestReading
{
    static let service = "se.mjukis.dux.ai-provider-key.v1"

    private let keychain: any AIProviderKeychainClient

    init() {
        keychain = SystemAIProviderKeychainClient()
    }

    init(keychain: any AIProviderKeychainClient) {
        self.keychain = keychain
    }

    func presence(
        for account: AIProviderCredentialAccount
    ) async -> AIProviderCredentialPresence {
        do {
            return try await readForSingleRequest(for: account) == nil
                ? .absent
                : .present
        } catch let error as AIProviderCredentialStoreError {
            return .failed(error)
        } catch {
            return .failed(.failed)
        }
    }

    func readForSingleRequest(
        for account: AIProviderCredentialAccount
    ) async throws -> AIProviderCredential? {
        let result = keychain.copyMatching(Self.readQuery(for: account))
        switch result.status {
        case errSecSuccess:
            guard let data = result.data else {
                throw AIProviderCredentialStoreError.corruptCredential
            }
            do {
                return try AIProviderCredential(validatingUTF8: data)
            } catch {
                throw AIProviderCredentialStoreError.corruptCredential
            }
        case errSecItemNotFound:
            return nil
        default:
            throw Self.map(result.status)
        }
    }

    func replace(
        _ value: String,
        for account: AIProviderCredentialAccount
    ) async throws {
        let credential = try AIProviderCredential(value)
        let addStatus = keychain.add(
            Self.addQuery(for: account, credential: credential)
        )

        switch addStatus {
        case errSecSuccess:
            return
        case errSecDuplicateItem:
            try replaceExisting(credential, for: account)
        default:
            throw Self.map(addStatus)
        }
    }

    func delete(for account: AIProviderCredentialAccount) async throws {
        let status = keychain.delete(Self.itemQuery(for: account))
        guard status == errSecSuccess || status == errSecItemNotFound else {
            throw Self.map(status)
        }
    }

    private func replaceExisting(
        _ credential: AIProviderCredential,
        for account: AIProviderCredentialAccount
    ) throws {
        let query = Self.itemQuery(for: account)
        let attributes = Self.replacementAttributes(credential)
        let updateStatus = keychain.update(query, attributes: attributes)

        switch updateStatus {
        case errSecSuccess:
            return
        case errSecItemNotFound:
            // The item can disappear after the duplicate result. One bounded
            // add/update retry settles either side of that race.
            let retryAddStatus = keychain.add(
                Self.addQuery(for: account, credential: credential)
            )
            switch retryAddStatus {
            case errSecSuccess:
                return
            case errSecDuplicateItem:
                let retryUpdateStatus = keychain.update(
                    query,
                    attributes: attributes
                )
                guard retryUpdateStatus == errSecSuccess else {
                    throw Self.map(retryUpdateStatus)
                }
            default:
                throw Self.map(retryAddStatus)
            }
        default:
            throw Self.map(updateStatus)
        }
    }

    nonisolated static func itemQuery(
        for account: AIProviderCredentialAccount
    ) -> [CFString: Any] {
        let authenticationContext = LAContext()
        authenticationContext.interactionNotAllowed = true
        return [
            kSecClass: kSecClassGenericPassword,
            kSecUseDataProtectionKeychain: true,
            kSecAttrService: service,
            kSecAttrAccount: account.rawValue,
            kSecAttrSynchronizable: false,
            kSecUseAuthenticationContext: authenticationContext,
        ]
    }

    nonisolated static func readQuery(
        for account: AIProviderCredentialAccount
    ) -> [CFString: Any] {
        var query = itemQuery(for: account)
        query[kSecReturnData] = true
        query[kSecMatchLimit] = kSecMatchLimitOne
        return query
    }

    nonisolated static func addQuery(
        for account: AIProviderCredentialAccount,
        credential: AIProviderCredential
    ) -> [CFString: Any] {
        var query = itemQuery(for: account)
        query[kSecAttrAccessible] = kSecAttrAccessibleWhenUnlockedThisDeviceOnly
        query[kSecValueData] = credential.utf8
        return query
    }

    nonisolated static func replacementAttributes(
        _ credential: AIProviderCredential
    ) -> [CFString: Any] {
        [
            kSecAttrAccessible: kSecAttrAccessibleWhenUnlockedThisDeviceOnly,
            kSecValueData: credential.utf8,
        ]
    }

    nonisolated static func map(_ status: OSStatus) -> AIProviderCredentialStoreError {
        switch status {
        case errSecInteractionNotAllowed:
            .locked
        case errSecNotAvailable:
            .unavailable
        default:
            .failed
        }
    }
}
