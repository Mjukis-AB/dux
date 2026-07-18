import Foundation

/// Display-only historical candidate information. These values are scoped to
/// a retained review and never become cleanup or planning inputs.
enum ExplorerCandidatePathEncoding: Equatable, Sendable {
    case utf8
    case utf16LittleEndian
}

enum ExplorerCandidateCategory: Equatable, Hashable, Sendable {
    case developerArtifact
    case applicationCache
    case browserCache
    case logAndDiagnostic
    case installerAndDownload
    case deviceAndSimulatorData
    case cloudFile
    case largeReviewItem
    case protectedSystemData
    case unknownStorage
}

enum ExplorerCandidateSafety: Equatable, Sendable {
    case safeRegenerable
    case safeEvictable
    case reviewRequired
    case informational
    case protected
}

enum ExplorerCandidateAction: Equatable, Sendable {
    case removeKnownRegenerableContents
    case evictLocalCopy
    case moveToTrash
    case revealOnly
    case noAction
}

enum ExplorerCandidateStatus: Equatable, Sendable {
    case discovered
    case selected
    case dismissed
    case stale
    case planned
    case completed
    case failed
    case unavailable
}

enum ExplorerCandidateEvidenceKind: Equatable, Sendable {
    case matchedPath
    case requiredMarker
    case forbiddenMarkerAbsent
    case bundleIdentifier
    case minimumAge
    case minimumSize
    case inactiveProcess
    case cloudUploadComplete
}

enum ExplorerCandidateBlockReason: Equatable, Sendable {
    case missingOrIncompleteEvidence
    case missingModificationTime
    case partialScanCoverage
    case recentActivity
    case belowMinimumBytes
    case activeUse
    case accessDenied
    case protectedPath
    case protectedDescendant
    case symlinkBoundary
    case volumeBoundary
    case changedSinceScan
    case unsupportedPlatform
    case cloudUploadUnconfirmed
}

struct ExplorerCandidateObservedPath: Equatable, Sendable {
    let encoding: ExplorerCandidatePathEncoding
    let encodedBytes: Data
    let display: String
}

struct ExplorerCandidateSummary: Equatable, Identifiable, Sendable {
    var id: String { candidateID }

    let candidateID: String
    let ruleID: String
    let ruleRevision: UInt32
    let category: ExplorerCandidateCategory
    let estimatedBytes: UInt64
    let newestMtime: ExplorerSnapshotTimestamp?
    let safety: ExplorerCandidateSafety
    let action: ExplorerCandidateAction
    let ruleScheduleEligible: Bool
    let pathCount: UInt16
    let evidenceKinds: [ExplorerCandidateEvidenceKind]
    let blockers: [ExplorerCandidateBlockReason]
    let createdAt: ExplorerSnapshotTimestamp
    let status: ExplorerCandidateStatus
}

struct ExplorerCandidatePathPage: Equatable, Sendable {
    let scanID: String
    let candidate: ExplorerCandidateSummary
    let cursor: UInt16
    let nextCursor: UInt16?
    let totalPaths: UInt16
    let paths: [ExplorerCandidateObservedPath]
}

struct ExplorerCandidateEvidence: Equatable, Sendable {
    let ordinal: UInt16
    let kind: ExplorerCandidateEvidenceKind
    let path: ExplorerCandidateObservedPath?
    let identifier: String?
    let newestMtime: ExplorerSnapshotTimestamp?
    let minimumAgeSeconds: UInt64?
    let minimumAgeNanoseconds: UInt32?
    let observedBytes: UInt64?
    let minimumBytes: UInt64?
}

struct ExplorerCandidateEvidencePage: Equatable, Sendable {
    let scanID: String
    let candidate: ExplorerCandidateSummary
    let cursor: UInt16
    let nextCursor: UInt16?
    let totalEvidence: UInt16
    let evidence: [ExplorerCandidateEvidence]
}

enum ExplorerCandidateDetailError: Error, Equatable, Sendable {
    case reviewExpired
    case reviewNotAcquired
    case invalidLimit
    case invalidRequest
    case evaluationUnavailable
    case candidateNotFound
    case budgetExceeded
    case unavailable
    case invalidResponse
}

/// The only conversion boundary between generated candidate records and
/// app-owned Explorer models.
enum ExplorerCandidateDetailAdapter {
    static let maximumPageLimit: UInt16 = 64
    private static let recordVersion: UInt32 = 1
    private static let maximumPathBytes = 16 * 1_024 * 1_024
    private static let maximumIdentifierBytes = 4_096
    private static let maximumCandidateCount: UInt16 = 4_096
    private static let maximumUnixSeconds: UInt64 = 253_402_300_799

    static func mapPaths(
        _ raw: CandidatePathPage,
        expectedScanID: String,
        expectedCandidateID: String,
        expectedCursor: UInt16,
        requestedLimit: UInt16
    ) throws -> ExplorerCandidatePathPage {
        guard
            raw.recordVersion == recordVersion,
            validScanID(raw.scanId),
            raw.scanId == expectedScanID,
            raw.candidate.candidateId == expectedCandidateID,
            raw.cursor == expectedCursor,
            (1 ... maximumPageLimit).contains(requestedLimit),
            raw.totalPaths >= raw.cursor,
            raw.totalPaths <= maximumCandidateCount,
            raw.paths.count <= Int(UInt16.max),
            UInt16(raw.paths.count) == min(requestedLimit, raw.totalPaths - raw.cursor),
            validNext(raw.nextCursor, cursor: raw.cursor, count: raw.paths.count, total: raw.totalPaths)
        else {
            throw ExplorerCandidateDetailError.invalidResponse
        }
        let candidate = try mapSummary(raw.candidate)
        guard candidate.pathCount == raw.totalPaths else {
            throw ExplorerCandidateDetailError.invalidResponse
        }
        let paths = try raw.paths.map(mapPath)
        return ExplorerCandidatePathPage(
            scanID: raw.scanId,
            candidate: candidate,
            cursor: raw.cursor,
            nextCursor: raw.nextCursor,
            totalPaths: raw.totalPaths,
            paths: paths
        )
    }

    static func mapEvidence(
        _ raw: CandidateEvidencePage,
        expectedScanID: String,
        expectedCandidateID: String,
        expectedCursor: UInt16,
        requestedLimit: UInt16
    ) throws -> ExplorerCandidateEvidencePage {
        guard
            raw.recordVersion == recordVersion,
            validScanID(raw.scanId),
            raw.scanId == expectedScanID,
            raw.candidate.candidateId == expectedCandidateID,
            raw.cursor == expectedCursor,
            (1 ... maximumPageLimit).contains(requestedLimit),
            raw.totalEvidence >= raw.cursor,
            raw.totalEvidence <= maximumCandidateCount,
            raw.evidence.count <= Int(UInt16.max),
            UInt16(raw.evidence.count) == min(requestedLimit, raw.totalEvidence - raw.cursor),
            validNext(raw.nextCursor, cursor: raw.cursor, count: raw.evidence.count, total: raw.totalEvidence)
        else {
            throw ExplorerCandidateDetailError.invalidResponse
        }
        let candidate = try mapSummary(raw.candidate)
        guard candidate.evidenceKinds.count == raw.totalEvidence else {
            throw ExplorerCandidateDetailError.invalidResponse
        }
        var expectedOrdinal = raw.cursor
        let evidence = try raw.evidence.map { item in
            guard item.ordinal == expectedOrdinal else {
                throw ExplorerCandidateDetailError.invalidResponse
            }
            expectedOrdinal = expectedOrdinal == UInt16.max ? UInt16.max : expectedOrdinal + 1
            return try mapEvidence(item)
        }
        return ExplorerCandidateEvidencePage(
            scanID: raw.scanId,
            candidate: candidate,
            cursor: raw.cursor,
            nextCursor: raw.nextCursor,
            totalEvidence: raw.totalEvidence,
            evidence: evidence
        )
    }

    private static func mapSummary(_ raw: CandidateSummary) throws -> ExplorerCandidateSummary {
        guard
            raw.recordVersion == recordVersion,
            validCandidateID(raw.candidateId),
            validRuleID(raw.ruleId),
            raw.ruleRevision > 0,
            raw.pathCount <= maximumCandidateCount,
            raw.evidenceKinds.count <= Int(maximumCandidateCount),
            raw.blockers.count <= Int(maximumCandidateCount),
            validTimestamp(raw.createdAt),
            raw.newestMtime.map(validTimestamp) ?? true
        else {
            throw ExplorerCandidateDetailError.invalidResponse
        }
        return ExplorerCandidateSummary(
            candidateID: raw.candidateId,
            ruleID: raw.ruleId,
            ruleRevision: raw.ruleRevision,
            category: map(raw.category),
            estimatedBytes: raw.estimatedBytes,
            newestMtime: raw.newestMtime.map(mapTimestamp),
            safety: map(raw.safety),
            action: map(raw.action),
            ruleScheduleEligible: raw.ruleScheduleEligible,
            pathCount: raw.pathCount,
            evidenceKinds: raw.evidenceKinds.map(map),
            blockers: raw.blockers.map(map),
            createdAt: mapTimestamp(raw.createdAt),
            status: map(raw.status)
        )
    }

    private static func mapPath(_ raw: CandidateObservedPath) throws -> ExplorerCandidateObservedPath {
        guard
            raw.encodedBytes.count <= maximumPathBytes,
            raw.display.utf8.count <= maximumPathBytes,
            !raw.display.contains("\0")
        else {
            throw ExplorerCandidateDetailError.invalidResponse
        }
        return ExplorerCandidateObservedPath(
            encoding: raw.encoding == .utf8 ? .utf8 : .utf16LittleEndian,
            encodedBytes: raw.encodedBytes,
            display: raw.display
        )
    }

    private static func mapEvidence(_ raw: CandidateEvidenceRecord) throws -> ExplorerCandidateEvidence {
        guard raw.recordVersion == recordVersion else {
            throw ExplorerCandidateDetailError.invalidResponse
        }
        let pathKinds: Set<CandidateEvidenceKind> = [
            .matchedPath, .requiredMarker, .forbiddenMarkerAbsent, .cloudUploadComplete,
        ]
        var path: ExplorerCandidateObservedPath?
        var identifier: String?
        var newestMtime: ExplorerSnapshotTimestamp?
        var minimumAgeSeconds: UInt64?
        var minimumAgeNanoseconds: UInt32?
        var observedBytes: UInt64?
        var minimumBytes: UInt64?
        switch raw.kind {
        case .matchedPath, .requiredMarker, .forbiddenMarkerAbsent, .cloudUploadComplete:
            guard let rawPath = raw.path, let mapped = try? mapPath(rawPath),
                  raw.identifier == nil, raw.newestMtime == nil,
                  raw.minimumAgeSeconds == nil, raw.minimumAgeNanoseconds == nil,
                  raw.observedBytes == nil, raw.minimumBytes == nil
            else { throw ExplorerCandidateDetailError.invalidResponse }
            path = mapped
        case .bundleIdentifier:
            guard let rawPath = raw.path, let mapped = try? mapPath(rawPath),
                  let rawIdentifier = raw.identifier,
                  rawIdentifier.utf8.count <= maximumIdentifierBytes,
                  !rawIdentifier.isEmpty,
                  raw.newestMtime == nil, raw.minimumAgeSeconds == nil,
                  raw.minimumAgeNanoseconds == nil, raw.observedBytes == nil,
                  raw.minimumBytes == nil
            else { throw ExplorerCandidateDetailError.invalidResponse }
            path = mapped
            identifier = rawIdentifier
        case .minimumAge:
            guard raw.path == nil, raw.identifier == nil,
                  let timestamp = raw.newestMtime,
                  validTimestamp(timestamp), let seconds = raw.minimumAgeSeconds,
                  let nanoseconds = raw.minimumAgeNanoseconds, nanoseconds < 1_000_000_000,
                  raw.observedBytes == nil, raw.minimumBytes == nil
            else { throw ExplorerCandidateDetailError.invalidResponse }
            newestMtime = mapTimestamp(timestamp)
            minimumAgeSeconds = seconds
            minimumAgeNanoseconds = nanoseconds
        case .minimumSize:
            guard raw.path == nil, raw.identifier == nil, raw.newestMtime == nil,
                  raw.minimumAgeSeconds == nil, raw.minimumAgeNanoseconds == nil,
                  let observed = raw.observedBytes, let minimum = raw.minimumBytes,
                  observed >= minimum
            else { throw ExplorerCandidateDetailError.invalidResponse }
            observedBytes = observed
            minimumBytes = minimum
        case .inactiveProcess:
            guard raw.path == nil, let rawIdentifier = raw.identifier,
                  rawIdentifier.utf8.count <= maximumIdentifierBytes,
                  !rawIdentifier.isEmpty, raw.newestMtime == nil,
                  raw.minimumAgeSeconds == nil, raw.minimumAgeNanoseconds == nil,
                  raw.observedBytes == nil, raw.minimumBytes == nil
            else { throw ExplorerCandidateDetailError.invalidResponse }
            identifier = rawIdentifier
        }
        guard pathKinds.contains(raw.kind) == (path != nil) else {
            throw ExplorerCandidateDetailError.invalidResponse
        }
        return ExplorerCandidateEvidence(
            ordinal: raw.ordinal,
            kind: map(raw.kind),
            path: path,
            identifier: identifier,
            newestMtime: newestMtime,
            minimumAgeSeconds: minimumAgeSeconds,
            minimumAgeNanoseconds: minimumAgeNanoseconds,
            observedBytes: observedBytes,
            minimumBytes: minimumBytes
        )
    }

    private static func validNext(
        _ next: UInt16?,
        cursor: UInt16,
        count: Int,
        total: UInt16
    ) -> Bool {
        let end = UInt32(cursor) + UInt32(count)
        guard end <= UInt32(total) else { return false }
        return end < UInt32(total) ? next == UInt16(end) : next == nil
    }

    private static func mapTimestamp(_ value: SnapshotNodeTimestamp) -> ExplorerSnapshotTimestamp {
        ExplorerSnapshotTimestamp(
            secondsSinceUnixEpoch: value.secondsSinceUnixEpoch,
            nanoseconds: value.nanoseconds
        )
    }

    private static func validScanID(_ value: String) -> Bool {
        value.hasPrefix("scan:") && value.utf8.count > 5 && value.utf8.count <= 128
            && value.unicodeScalars.allSatisfy { $0.isASCII && (CharacterSet.alphanumerics.contains($0) || "._-:".unicodeScalars.contains($0)) }
    }

    private static func validCandidateID(_ value: String) -> Bool {
        value.hasPrefix("candidate:") && value.utf8.count > 10 && value.utf8.count <= 128
            && value.unicodeScalars.allSatisfy { $0.isASCII && (CharacterSet.alphanumerics.contains($0) || "._-:".unicodeScalars.contains($0)) }
    }

    private static func validRuleID(_ value: String) -> Bool {
        guard !value.isEmpty, value.utf8.count <= 128 else { return false }
        return value.split(separator: ".").allSatisfy { part in
            let scalars = part.unicodeScalars
            guard let first = scalars.first, let last = scalars.last,
                  (first.value >= 97 && first.value <= 122)
                    || (first.value >= 48 && first.value <= 57),
                  (last.value >= 97 && last.value <= 122)
                    || (last.value >= 48 && last.value <= 57)
            else { return false }
            return scalars.allSatisfy {
                ( $0.value >= 97 && $0.value <= 122 )
                    || ( $0.value >= 48 && $0.value <= 57 )
                    || $0 == "_" || $0 == "-"
            }
        }
    }

    private static func validTimestamp(_ value: SnapshotNodeTimestamp) -> Bool {
        value.secondsSinceUnixEpoch <= maximumUnixSeconds && value.nanoseconds < 1_000_000_000
    }

    private static func map(_ value: CandidateCategory) -> ExplorerCandidateCategory {
        switch value {
        case .developerArtifact: .developerArtifact
        case .applicationCache: .applicationCache
        case .browserCache: .browserCache
        case .logAndDiagnostic: .logAndDiagnostic
        case .installerAndDownload: .installerAndDownload
        case .deviceAndSimulatorData: .deviceAndSimulatorData
        case .cloudFile: .cloudFile
        case .largeReviewItem: .largeReviewItem
        case .protectedSystemData: .protectedSystemData
        case .unknownStorage: .unknownStorage
        }
    }

    private static func map(_ value: CandidateSafety) -> ExplorerCandidateSafety {
        switch value {
        case .safeRegenerable: .safeRegenerable
        case .safeEvictable: .safeEvictable
        case .reviewRequired: .reviewRequired
        case .informational: .informational
        case .protected: .protected
        }
    }

    private static func map(_ value: CandidateAction) -> ExplorerCandidateAction {
        switch value {
        case .removeKnownRegenerableContents: .removeKnownRegenerableContents
        case .evictLocalCopy: .evictLocalCopy
        case .moveToTrash: .moveToTrash
        case .revealOnly: .revealOnly
        case .noAction: .noAction
        }
    }

    private static func map(_ value: CandidateStatus) -> ExplorerCandidateStatus {
        switch value {
        case .discovered: .discovered
        case .selected: .selected
        case .dismissed: .dismissed
        case .stale: .stale
        case .planned: .planned
        case .completed: .completed
        case .failed: .failed
        case .unavailable: .unavailable
        }
    }

    private static func map(_ value: CandidateEvidenceKind) -> ExplorerCandidateEvidenceKind {
        switch value {
        case .matchedPath: .matchedPath
        case .requiredMarker: .requiredMarker
        case .forbiddenMarkerAbsent: .forbiddenMarkerAbsent
        case .bundleIdentifier: .bundleIdentifier
        case .minimumAge: .minimumAge
        case .minimumSize: .minimumSize
        case .inactiveProcess: .inactiveProcess
        case .cloudUploadComplete: .cloudUploadComplete
        }
    }

    private static func map(_ value: CandidateBlockReason) -> ExplorerCandidateBlockReason {
        switch value {
        case .missingOrIncompleteEvidence: .missingOrIncompleteEvidence
        case .missingModificationTime: .missingModificationTime
        case .partialScanCoverage: .partialScanCoverage
        case .recentActivity: .recentActivity
        case .belowMinimumBytes: .belowMinimumBytes
        case .activeUse: .activeUse
        case .accessDenied: .accessDenied
        case .protectedPath: .protectedPath
        case .protectedDescendant: .protectedDescendant
        case .symlinkBoundary: .symlinkBoundary
        case .volumeBoundary: .volumeBoundary
        case .changedSinceScan: .changedSinceScan
        case .unsupportedPlatform: .unsupportedPlatform
        case .cloudUploadUnconfirmed: .cloudUploadUnconfirmed
        }
    }
}
