import Foundation

struct DiskPressurePolicyConfiguration: Equatable, Sendable {
    static let bytesPerGiB: UInt64 = 1 << 30
    static let defaults = Self(
        criticalAvailableBytes: 10 * bytesPerGiB,
        criticalAvailableBasisPoints: 500,
        warningAvailableBytes: 30 * bytesPerGiB,
        warningAvailableBasisPoints: 1_000,
        recoveryBytes: 2 * bytesPerGiB,
        recoveryBasisPoints: 100
    )

    let criticalAvailableBytes: UInt64
    let criticalAvailableBasisPoints: UInt16
    let warningAvailableBytes: UInt64
    let warningAvailableBasisPoints: UInt16
    let recoveryBytes: UInt64
    let recoveryBasisPoints: UInt16
}

enum DiskPressurePolicySource: Equatable, Sendable {
    case `default`
    case stored
}

struct DiskPressurePolicy: Equatable, Sendable {
    let source: DiskPressurePolicySource
    let revision: UInt64
    let configuration: DiskPressurePolicyConfiguration
    let updatedAtUnixMilliseconds: Int64?
}

struct DiskPressurePolicyUpdateResult: Equatable, Sendable {
    let policy: DiskPressurePolicy
    let changed: Bool
}

enum DiskPressurePolicyServiceError: Error, Equatable, Sendable {
    case closed
    case invalidRecordVersion
    case thresholdBytesZero
    case thresholdBasisPointsOutOfRange
    case warningBytesBelowCritical
    case warningBasisPointsBelowCritical
    case warningThresholdMatchesCritical
    case recoveryBytesZero
    case recoveryBasisPointsOutOfRange
    case revisionExhausted
    case invalidClock
    case incompatibleSchema
    case retryable
    case unsafeStorage
    case corruptData
    case unavailable
    case outcomeUnknown
    case internalState
    case invalidResponse
}

enum DiskPressurePolicyField: Equatable, Sendable {
    case criticalGiB
    case criticalPercent
    case warningGiB
    case warningPercent
    case recoveryGiB
    case recoveryPercent
}

enum DiskPressurePolicyDraftError: Error, Equatable, Sendable {
    case invalidNumber(DiskPressurePolicyField)
    case zero(DiskPressurePolicyField)
    case outOfRange(DiskPressurePolicyField)
}

struct DiskPressurePolicyDraft: Equatable, Sendable {
    var criticalGiB: String
    var criticalPercent: String
    var warningGiB: String
    var warningPercent: String
    var recoveryGiB: String
    var recoveryPercent: String

    static let defaults = Self(configuration: .defaults)

    init(configuration: DiskPressurePolicyConfiguration) {
        criticalGiB = ExactPolicyDecimal.formatGiB(configuration.criticalAvailableBytes)
        criticalPercent = ExactPolicyDecimal.formatPercent(
            configuration.criticalAvailableBasisPoints
        )
        warningGiB = ExactPolicyDecimal.formatGiB(configuration.warningAvailableBytes)
        warningPercent = ExactPolicyDecimal.formatPercent(
            configuration.warningAvailableBasisPoints
        )
        recoveryGiB = ExactPolicyDecimal.formatGiB(configuration.recoveryBytes)
        recoveryPercent = ExactPolicyDecimal.formatPercent(configuration.recoveryBasisPoints)
    }

    init(
        criticalGiB: String,
        criticalPercent: String,
        warningGiB: String,
        warningPercent: String,
        recoveryGiB: String,
        recoveryPercent: String
    ) {
        self.criticalGiB = criticalGiB
        self.criticalPercent = criticalPercent
        self.warningGiB = warningGiB
        self.warningPercent = warningPercent
        self.recoveryGiB = recoveryGiB
        self.recoveryPercent = recoveryPercent
    }

    func configuration(
        decimalSeparator: String? = Locale.current.decimalSeparator
    ) throws -> DiskPressurePolicyConfiguration {
        let criticalBytes = try Self.bytes(
            criticalGiB,
            field: .criticalGiB,
            decimalSeparator: decimalSeparator
        )
        let criticalBasisPoints = try Self.basisPoints(
            criticalPercent,
            field: .criticalPercent,
            decimalSeparator: decimalSeparator
        )
        let warningBytes = try Self.bytes(
            warningGiB,
            field: .warningGiB,
            decimalSeparator: decimalSeparator
        )
        let warningBasisPoints = try Self.basisPoints(
            warningPercent,
            field: .warningPercent,
            decimalSeparator: decimalSeparator
        )
        let recoveryBytes = try Self.bytes(
            recoveryGiB,
            field: .recoveryGiB,
            decimalSeparator: decimalSeparator
        )
        let recoveryBasisPoints = try Self.basisPoints(
            recoveryPercent,
            field: .recoveryPercent,
            decimalSeparator: decimalSeparator
        )
        return DiskPressurePolicyConfiguration(
            criticalAvailableBytes: criticalBytes,
            criticalAvailableBasisPoints: criticalBasisPoints,
            warningAvailableBytes: warningBytes,
            warningAvailableBasisPoints: warningBasisPoints,
            recoveryBytes: recoveryBytes,
            recoveryBasisPoints: recoveryBasisPoints
        )
    }

    private static func bytes(
        _ value: String,
        field: DiskPressurePolicyField,
        decimalSeparator: String?
    ) throws -> UInt64 {
        guard let bytes = ExactPolicyDecimal.parseGiB(
            value,
            decimalSeparator: decimalSeparator
        ) else {
            throw DiskPressurePolicyDraftError.invalidNumber(field)
        }
        guard bytes > 0 else {
            throw DiskPressurePolicyDraftError.zero(field)
        }
        return bytes
    }

    private static func basisPoints(
        _ value: String,
        field: DiskPressurePolicyField,
        decimalSeparator: String?
    ) throws -> UInt16 {
        guard let basisPoints = ExactPolicyDecimal.parsePercent(
            value,
            decimalSeparator: decimalSeparator
        ) else {
            throw DiskPressurePolicyDraftError.invalidNumber(field)
        }
        guard basisPoints > 0 else {
            throw DiskPressurePolicyDraftError.zero(field)
        }
        guard basisPoints <= 10_000 else {
            throw DiskPressurePolicyDraftError.outOfRange(field)
        }
        return basisPoints
    }
}

enum DiskPressurePolicyState: Equatable {
    case idle
    case loading
    case ready
    case saving
    case resetting
    case failed(DiskPressurePolicyFailure)

    var isBusy: Bool {
        switch self {
        case .loading, .saving, .resetting: true
        case .idle, .ready, .failed: false
        }
    }
}

enum DiskPressurePolicyFailure: Equatable {
    case draft(DiskPressurePolicyDraftError)
    case service(DiskPressurePolicyServiceError)
    case unexpected
}

enum ExactPolicyDecimal {
    static func formatGiB(_ bytes: UInt64) -> String {
        let divisor = DiskPressurePolicyConfiguration.bytesPerGiB
        let whole = bytes / divisor
        var remainder = bytes % divisor
        guard remainder != 0 else {
            return String(whole)
        }

        var fraction = ""
        repeat {
            remainder *= 10
            fraction.append(String(remainder / divisor))
            remainder %= divisor
        } while remainder != 0
        return "\(whole).\(fraction)"
    }

    static func parseGiB(
        _ text: String,
        decimalSeparator: String? = Locale.current.decimalSeparator
    ) -> UInt64? {
        guard let number = components(text, decimalSeparator: decimalSeparator) else {
            return nil
        }
        guard let whole = UInt64(number.whole) else {
            return nil
        }
        let (wholeBytes, wholeOverflow) = whole.multipliedReportingOverflow(
            by: DiskPressurePolicyConfiguration.bytesPerGiB
        )
        guard !wholeOverflow else {
            return nil
        }
        guard !number.fraction.isEmpty else {
            return wholeBytes
        }

        var fraction = number.fraction
        while fraction.last == "0" {
            fraction.removeLast()
        }
        guard !fraction.isEmpty else {
            return wholeBytes
        }
        guard fraction.count <= 30 else {
            return nil
        }

        var quotient = fraction
        for _ in 0 ..< fraction.count {
            guard let division = divideDecimalDigits(quotient, by: 5), division.remainder == 0 else {
                return nil
            }
            quotient = division.quotient
        }
        guard let reduced = UInt64(quotient) else {
            return nil
        }
        let fractionalBytes = reduced << UInt64(30 - fraction.count)
        let (result, overflow) = wholeBytes.addingReportingOverflow(fractionalBytes)
        return overflow ? nil : result
    }

    static func formatPercent(_ basisPoints: UInt16) -> String {
        let whole = basisPoints / 100
        let remainder = basisPoints % 100
        guard remainder != 0 else {
            return String(whole)
        }
        if remainder % 10 == 0 {
            return "\(whole).\(remainder / 10)"
        }
        return "\(whole).\(String(format: "%02d", remainder))"
    }

    static func parsePercent(
        _ text: String,
        decimalSeparator: String? = Locale.current.decimalSeparator
    ) -> UInt16? {
        guard let number = components(text, decimalSeparator: decimalSeparator) else {
            return nil
        }
        var fraction = number.fraction
        while fraction.last == "0" {
            fraction.removeLast()
        }
        guard fraction.count <= 2 else {
            return nil
        }
        guard let whole = UInt64(number.whole) else {
            return nil
        }
        let (wholeBasisPoints, multiplyOverflow) = whole.multipliedReportingOverflow(by: 100)
        guard !multiplyOverflow else {
            return nil
        }
        let fractionBasisPoints: UInt64
        switch fraction.count {
        case 0: fractionBasisPoints = 0
        case 1: fractionBasisPoints = UInt64(fraction)! * 10
        case 2: fractionBasisPoints = UInt64(fraction)!
        default: return nil
        }
        let (result, addOverflow) = wholeBasisPoints.addingReportingOverflow(fractionBasisPoints)
        guard !addOverflow, result <= UInt64(UInt16.max) else {
            return nil
        }
        return UInt16(result)
    }

    private static func components(
        _ text: String,
        decimalSeparator: String?
    ) -> (whole: String, fraction: String)? {
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else {
            return nil
        }
        var separators: Set<Character> = ["."]
        if let decimalSeparator, decimalSeparator.count == 1,
           let separator = decimalSeparator.first {
            separators.insert(separator)
        }
        var separatorIndex: String.Index?
        for index in trimmed.indices where separators.contains(trimmed[index]) {
            guard separatorIndex == nil else {
                return nil
            }
            separatorIndex = index
        }
        let whole: Substring
        let fraction: Substring
        if let separatorIndex {
            whole = trimmed[..<separatorIndex]
            fraction = trimmed[trimmed.index(after: separatorIndex)...]
            guard !fraction.isEmpty else {
                return nil
            }
        } else {
            whole = Substring(trimmed)
            fraction = ""
        }
        guard !whole.isEmpty, whole.allSatisfy(\.isASCIIWholeNumber),
              fraction.allSatisfy(\.isASCIIWholeNumber) else {
            return nil
        }
        return (String(whole), String(fraction))
    }

    private static func divideDecimalDigits(
        _ digits: String,
        by divisor: UInt8
    ) -> (quotient: String, remainder: UInt8)? {
        guard divisor > 0 else {
            return nil
        }
        var quotient = ""
        var remainder: UInt8 = 0
        for character in digits {
            guard let digit = character.wholeNumberValue, digit < 10 else {
                return nil
            }
            let value = UInt16(remainder) * 10 + UInt16(digit)
            let next = UInt8(value / UInt16(divisor))
            remainder = UInt8(value % UInt16(divisor))
            if !quotient.isEmpty || next != 0 {
                quotient.append(String(next))
            }
        }
        return (quotient.isEmpty ? "0" : quotient, remainder)
    }
}

private extension Character {
    var isASCIIWholeNumber: Bool {
        unicodeScalars.count == 1 && unicodeScalars.first.map { $0.value >= 48 && $0.value <= 57 } == true
    }
}
