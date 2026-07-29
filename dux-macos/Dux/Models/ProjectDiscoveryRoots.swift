import Foundation

/// One lossless, display-only project root selected for read-only discovery.
///
/// Persisting this observation does not start a scan and never creates a
/// cleanup target, plan, approval, or filesystem-effect capability.
struct ProjectDiscoveryRoot: Equatable, Hashable, Identifiable, Sendable {
    enum Encoding: Equatable, Hashable, Sendable {
        case unixBytes
        case windowsUTF16LittleEndian
    }

    static let maximumCount = 16
    static let maximumEncodedBytes = 32 * 1_024

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
        guard let bytes, Self.hasValidUnixShape(bytes) else {
            return nil
        }
        self.init(encoding: .unixBytes, encodedBytes: bytes)
    }

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

    /// Project roots are canonicalized by the core. This local check prevents
    /// obvious duplicate or nested picker selections before making an FFI
    /// request; it is not an authority or a substitute for Rust validation.
    func overlaps(_ other: Self) -> Bool {
        guard encoding == other.encoding else {
            return false
        }
        switch encoding {
        case .unixBytes:
            return Self.unixPath(encodedBytes, contains: other.encodedBytes)
                || Self.unixPath(other.encodedBytes, contains: encodedBytes)
        case .windowsUTF16LittleEndian:
            return encodedBytes == other.encodedBytes
        }
    }

    static func hasValidUnixShape(_ data: Data) -> Bool {
        let bytes = [UInt8](data)
        guard
            !bytes.isEmpty,
            bytes.count <= maximumEncodedBytes,
            bytes.first == UInt8(ascii: "/"),
            bytes != [UInt8(ascii: "/")],
            bytes.last != UInt8(ascii: "/"),
            !bytes.contains(0)
        else {
            return false
        }
        return bytes.dropFirst()
            .split(separator: UInt8(ascii: "/"), omittingEmptySubsequences: false)
            .allSatisfy { component in
                !component.isEmpty
                    && component != [UInt8(ascii: ".")]
                    && component != [UInt8(ascii: "."), UInt8(ascii: ".")]
                    && !component.contains(where: {
                        $0 <= 0x1F || $0 == 0x7F
                    })
            }
    }

    private static func unixPath(_ ancestor: Data, contains descendant: Data) -> Bool {
        if ancestor == descendant {
            return true
        }
        guard descendant.count > ancestor.count,
              descendant.starts(with: ancestor) else {
            return false
        }
        return descendant[descendant.index(descendant.startIndex, offsetBy: ancestor.count)]
            == UInt8(ascii: "/")
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

enum ProjectDiscoveryRootsOrigin: Equatable, Sendable {
    case `default`
    case stored
}

struct ProjectDiscoveryRoots: Equatable, Sendable {
    let roots: [ProjectDiscoveryRoot]
    let source: ProjectDiscoveryRootsOrigin
    let revision: UInt64
    let updatedAtUnixMilliseconds: Int64?
}

struct ProjectDiscoveryRootsUpdateResult: Equatable, Sendable {
    let roots: ProjectDiscoveryRoots
    let changed: Bool
}

enum ProjectDiscoveryRootsServiceError: Error, Equatable, Sendable {
    case closed
    case invalidRecordVersion
    case invalidPath
    case tooManyPaths
    case overlappingPaths
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

enum ProjectDiscoveryRootsState: Equatable {
    case idle
    case loading
    case ready
    case adding
    case removing
    case resetting
    case failed(ProjectDiscoveryRootsFailure)

    var isBusy: Bool {
        switch self {
        case .loading, .adding, .removing, .resetting:
            true
        case .idle, .ready, .failed:
            false
        }
    }
}

enum ProjectDiscoveryRootsFailure: Equatable {
    case invalidSelection
    case overlappingSelection
    case service(ProjectDiscoveryRootsServiceError)
    case unexpected
}
