import Foundation

/// The origin of the path-free global permanent-cleanup kill switch.
enum PermanentCleanupPolicyOrigin: Equatable, Sendable {
    case `default`
    case stored
}

/// A validated, path-free snapshot of the global permanent-cleanup switch.
/// This setting never selects a target or grants execution authority.
struct PermanentCleanupPolicy: Equatable, Sendable {
    let enabled: Bool
    let source: PermanentCleanupPolicyOrigin
    let revision: UInt64
    let updatedAtUnixMilliseconds: Int64?
}

struct PermanentCleanupPolicyUpdateResult: Equatable, Sendable {
    let policy: PermanentCleanupPolicy
    let changed: Bool
}

enum PermanentCleanupPolicyServiceError: Error, Equatable, Sendable {
    case closed
    case corruptData
    case unavailable
    case outcomeUnknown
    case revisionExhausted
    case invalidClock
    case incompatibleSchema
    case retryable
    case unsafeStorage
    case budgetExceeded
    case internalState
    case invalidResponse
}

enum PermanentCleanupPolicyState: Equatable {
    case idle
    case loading
    case ready
    case disabling
    case enabling
    case resetting
    case failed(PermanentCleanupPolicyFailure)

    var isBusy: Bool {
        switch self {
        case .loading, .disabling, .enabling, .resetting:
            true
        case .idle, .ready, .failed:
            false
        }
    }
}

enum PermanentCleanupPolicyFailure: Equatable {
    case confirmationRequired
    case service(PermanentCleanupPolicyServiceError)
    case unexpected
}

/// Lossless, display-only observation of one deny-only cleanup prefix.
/// Mutations send these exact bytes back to Rust; display text is never parsed
/// into a path or used as cleanup authority.
struct CleanupExclusionPathObservation: Equatable, Hashable, Identifiable, Sendable {
    enum Encoding: Equatable, Hashable, Sendable {
        case unixBytes
        case windowsUTF16LittleEndian
    }

    var id: Self { self }

    let encoding: Encoding
    let encodedBytes: Data

    init(encoding: Encoding, encodedBytes: Data) {
        self.encoding = encoding
        self.encodedBytes = encodedBytes
    }

    init?(fileURL: URL) {
        guard fileURL.isFileURL else {
            return nil
        }
        let bytes = fileURL.withUnsafeFileSystemRepresentation { representation -> Data? in
            guard let representation else {
                return nil
            }
            return Data(bytes: representation, count: strlen(representation))
        }
        guard let bytes, bytes.first == UInt8(ascii: "/"), !bytes.contains(0) else {
            return nil
        }
        self.init(encoding: .unixBytes, encodedBytes: bytes)
    }

    /// Exact UTF-8 paths remain readable. Non-UTF-8 bytes use an unambiguous
    /// escaped form so Settings never silently replaces or drops information.
    var displayText: String {
        switch encoding {
        case .unixBytes:
            if let exact = String(data: encodedBytes, encoding: .utf8) {
                return exact
            }
            return "unix-bytes:" + Self.escapedBytes(encodedBytes)
        case .windowsUTF16LittleEndian:
            let bytes = [UInt8](encodedBytes)
            guard bytes.count.isMultiple(of: 2) else {
                return "windows-utf16le:" + Self.escapedBytes(encodedBytes)
            }
            let units = stride(from: 0, to: bytes.count, by: 2).map {
                UInt16(bytes[$0]) | (UInt16(bytes[$0 + 1]) << 8)
            }
            let decoded = String(decoding: units, as: UTF16.self)
            if Array(decoded.utf16) == units {
                return decoded
            }
            return "windows-utf16le:" + Self.escapedBytes(encodedBytes)
        }
    }

    private static func escapedBytes(_ data: Data) -> String {
        data.map { byte in
            if (0x20 ... 0x7E).contains(byte), byte != UInt8(ascii: "\\") {
                return String(UnicodeScalar(byte))
            }
            if byte == UInt8(ascii: "\\") {
                return "\\\\"
            }
            return String(format: "\\x%02x", byte)
        }.joined()
    }
}

enum CleanupExclusionsOrigin: Equatable, Sendable {
    case `default`
    case stored
}

/// Bounded deny-only settings observation. It cannot select, approve, or
/// execute cleanup.
struct CleanupExclusionsPolicy: Equatable, Sendable {
    let paths: [CleanupExclusionPathObservation]
    let source: CleanupExclusionsOrigin
    let revision: UInt64
    let updatedAtUnixMilliseconds: Int64?
}

struct CleanupExclusionsUpdateResult: Equatable, Sendable {
    let exclusions: CleanupExclusionsPolicy
    let changed: Bool
}

enum CleanupExclusionsServiceError: Error, Equatable, Sendable {
    case closed
    case invalidRecordVersion
    case invalidPath
    case tooManyPaths
    case revisionExhausted
    case invalidClock
    case incompatibleSchema
    case retryable
    case unsafeStorage
    case budgetExceeded
    case corruptData
    case unavailable
    case outcomeUnknown
    case internalState
    case invalidResponse
}

enum CleanupExclusionsState: Equatable {
    case idle
    case loading
    case ready
    case adding
    case removing
    case resetting
    case failed(CleanupExclusionsFailure)

    var isBusy: Bool {
        switch self {
        case .loading, .adding, .removing, .resetting:
            true
        case .idle, .ready, .failed:
            false
        }
    }
}

enum CleanupExclusionsFailure: Equatable {
    case confirmationRequired
    case invalidSelection
    case service(CleanupExclusionsServiceError)
    case unexpected
}
