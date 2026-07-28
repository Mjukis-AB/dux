import Foundation

/// Lossless native-path input selected by the user. Display text is never
/// parsed back into a path or used as authority.
struct DirectCargoExecutableSelection: Equatable, Sendable {
    static let maximumPathBytes = 32 * 1_024

    let encodedPathBytes: Data

    init(encodedPathBytes: Data) {
        self.encodedPathBytes = encodedPathBytes
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
        guard
            let bytes,
            Self.isValidUnixCargoPath(bytes)
        else {
            return nil
        }
        self.init(encodedPathBytes: bytes)
    }

    var displayPath: String {
        if let exact = String(data: encodedPathBytes, encoding: .utf8) {
            return exact
        }
        return "unix-bytes:" + encodedPathBytes.map { byte in
            if (0x20 ... 0x7E).contains(byte), byte != UInt8(ascii: "\\") {
                return String(UnicodeScalar(byte))
            }
            if byte == UInt8(ascii: "\\") {
                return "\\\\"
            }
            return String(format: "\\x%02x", byte)
        }.joined()
    }

    static func isValidUnixCargoPath(_ bytes: Data) -> Bool {
        guard
            !bytes.isEmpty,
            bytes.count <= maximumPathBytes,
            bytes.first == UInt8(ascii: "/"),
            bytes.last != UInt8(ascii: "/"),
            !bytes.contains(0),
            let path = String(data: bytes, encoding: .utf8),
            !path.unicodeScalars.contains(where: CharacterSet.controlCharacters.contains)
        else {
            return false
        }
        let components = bytes.dropFirst().split(
            separator: UInt8(ascii: "/"),
            omittingEmptySubsequences: false
        )
        return components.allSatisfy { component in
            !component.isEmpty
                && !component.elementsEqual(".".utf8)
                && !component.elementsEqual("..".utf8)
        } && components.last?.elementsEqual("cargo".utf8) == true
    }
}

enum DirectCargoSignatureKind: Equatable, Sendable {
    case adHoc
    case cms
}

/// Bounded macOS static-code evidence. Ad-hoc signatures provide integrity,
/// not publisher identity; only the explicit enrollment action grants local
/// discovery trust to the exact bytes.
struct DirectCargoSignatureEvidence: Equatable, Sendable {
    let kind: DirectCargoSignatureKind
    let flags: UInt32
    let codeDirectoryHashes: [Data]
    let signingIdentifier: String
    let teamIdentifier: String?
    let designatedRequirementSHA256: Data?

    var kindLabel: String {
        switch kind {
        case .adHoc: "Ad-hoc (local integrity only)"
        case .cms: "CMS signed"
        }
    }

    static func hasSafeBoundedIdentifier(_ value: String, maximumUTF8Bytes: Int) -> Bool {
        !value.isEmpty
            && value.utf8.count <= maximumUTF8Bytes
            && !value.unicodeScalars.contains(where: CharacterSet.controlCharacters.contains)
    }
}

struct DirectCargoEnrollmentPreviewModel: Equatable, Sendable {
    let executable: DirectCargoExecutableSelection
    let executableSHA256: Data
    let signature: DirectCargoSignatureEvidence
}

/// Exact evidence captured when an enrollment confirmation is presented.
/// A confirmation can authorize only the same generation and preview that
/// produced the text the user reviewed.
struct DirectCargoEnrollmentConfirmation: Equatable, Sendable {
    let generation: UInt64
    let preview: DirectCargoEnrollmentPreviewModel
}

struct DirectCargoVersion: Equatable, Sendable {
    let major: UInt32
    let minor: UInt32
    let patch: UInt32

    var displayText: String {
        "\(major).\(minor).\(patch)"
    }
}

struct DirectCargoEnrollmentIdentityModel: Equatable, Sendable {
    let executable: DirectCargoExecutableSelection
    let executableSHA256: Data
    let versionSHA256: Data
    let version: DirectCargoVersion
    let signature: DirectCargoSignatureEvidence
}

enum DirectCargoEnrollmentDisposition: Equatable, Sendable {
    case notEnrolled
    case enrolled(DirectCargoEnrollmentIdentityModel)
    case revoked
}

/// Revisioned trust state. Enrollment permits deterministic Cargo-backed
/// discovery only and never grants cleanup approval or execution authority.
struct DirectCargoEnrollmentStatusModel: Equatable, Sendable {
    let revision: UInt64
    let disposition: DirectCargoEnrollmentDisposition
    let updatedAtUnixMilliseconds: Int64?
}

struct DirectCargoEnrollmentUpdateModel: Equatable, Sendable {
    let status: DirectCargoEnrollmentStatusModel
    let changed: Bool
}

protocol DuxDirectCargoEnrollmentPreviewLease: AnyObject, Sendable {
    var preview: DirectCargoEnrollmentPreviewModel { get }
    func release() async
}

enum DirectCargoEnrollmentServiceError: Error, Equatable, Sendable {
    case closed
    case unsupportedPlatform
    case invalidRecordVersion
    case invalidExecutablePath
    case executableNotRegular
    case changedDuringInspection
    case inspectionUnavailable
    case inspectionLimitExceeded
    case invalidResolutionEnvironment
    case invalidCargoVersion
    case invalidCodeSignature
    case previewUnavailable
    case wrongEngine
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

enum DirectCargoEnrollmentViewState: Equatable {
    case idle
    case loading
    case ready
    case inspecting
    case awaitingEnrollmentConfirmation
    case enrolling
    case revoking
    case failed(DirectCargoEnrollmentFailure)

    var isBusy: Bool {
        switch self {
        case .loading, .inspecting, .enrolling, .revoking:
            true
        case .idle, .ready, .awaitingEnrollmentConfirmation, .failed:
            false
        }
    }
}

enum DirectCargoEnrollmentFailure: Equatable {
    case invalidSelection
    case enrollmentConfirmationRequired
    case enrollmentPreviewChanged
    case revocationConfirmationRequired
    case service(DirectCargoEnrollmentServiceError)
    case unexpected
}

extension Data {
    var duxLowercaseHex: String {
        map { String(format: "%02x", $0) }.joined()
    }
}
