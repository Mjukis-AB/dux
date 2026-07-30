import Foundation

/// Strict conversion boundary for the path-free v47 snapshot comparison.
/// Every relationship and aggregate is independently checked before SwiftUI
/// can render it.
enum ExplorerSnapshotDiffAdapter {
    static let maximumPageLimit: UInt16 = 200
    static let maximumTreemapCells: UInt16 = 64

    private static let recordVersion: UInt32 = 1
    private static let maximumUnixMilliseconds: Int64 = 253_402_300_799_999

    static func mapInfo(
        _ raw: SnapshotDiffInfo,
        expectedCurrentScanID: String
    ) throws -> ExplorerSnapshotDiffInfo {
        guard
            raw.recordVersion == recordVersion,
            !raw.released,
            raw.currentScanId == expectedCurrentScanID,
            raw.currentScanId != raw.baselineScanId,
            ExplorerSnapshotHistoryAdapter.validScanID(raw.currentScanId),
            ExplorerSnapshotHistoryAdapter.validScanID(raw.baselineScanId),
            validUnixMilliseconds(raw.currentStartedAtUnixMs),
            validUnixMilliseconds(raw.currentCompletedAtUnixMs),
            validUnixMilliseconds(raw.baselineStartedAtUnixMs),
            validUnixMilliseconds(raw.baselineCompletedAtUnixMs),
            raw.currentStartedAtUnixMs <= raw.currentCompletedAtUnixMs,
            raw.baselineStartedAtUnixMs <= raw.baselineCompletedAtUnixMs,
            currentSortsBeforeBaseline(raw),
            validCoverage(raw.currentCoverage),
            validCoverage(raw.baselineCoverage)
        else {
            throw ExplorerSnapshotDiffFailure.invalidResponse
        }
        return ExplorerSnapshotDiffInfo(
            currentScanID: raw.currentScanId,
            baselineScanID: raw.baselineScanId,
            currentStartedAt: date(raw.currentStartedAtUnixMs),
            currentCompletedAt: date(raw.currentCompletedAtUnixMs),
            baselineStartedAt: date(raw.baselineStartedAtUnixMs),
            baselineCompletedAt: date(raw.baselineCompletedAtUnixMs),
            currentCoverage: mapCoverage(raw.currentCoverage),
            baselineCoverage: mapCoverage(raw.baselineCoverage)
        )
    }

    static func mapRoot(_ raw: SnapshotDiffNode) throws -> ExplorerSnapshotDiffNode {
        let root = try mapNode(raw, maximumNameBytes: 65_536)
        guard
            root.id == 0,
            root.parentID == nil,
            root.depth == 0,
            root.kind == .directory,
            root.currentKind == .directory,
            root.baselineKind == .directory,
            root.change != .added,
            root.change != .removed,
            root.change != .replaced,
            root.canDescend
        else {
            throw ExplorerSnapshotDiffFailure.invalidResponse
        }
        return root
    }

    static func mapPage(
        _ raw: SnapshotDiffNodePage,
        expectedParentID: UInt64,
        expectedOffset: UInt64,
        requestedLimit: UInt16,
        sort: ExplorerSnapshotDiffSort
    ) throws -> ExplorerSnapshotDiffNodePage {
        let remaining = raw.totalChildren >= raw.offset
            ? raw.totalChildren - raw.offset
            : 0
        let expectedCount = min(UInt64(requestedLimit), remaining)
        let (end, overflow) = raw.offset.addingReportingOverflow(UInt64(raw.nodes.count))
        guard
            (1 ... maximumPageLimit).contains(requestedLimit),
            raw.recordVersion == recordVersion,
            raw.parentId == expectedParentID,
            raw.offset == expectedOffset,
            raw.offset <= raw.totalChildren,
            !overflow,
            UInt64(raw.nodes.count) == expectedCount,
            raw.hasMore == (end < raw.totalChildren),
            raw.unchangedChildCount <= raw.totalChildren,
            raw.replacedChildCount <= raw.totalChildren
        else {
            throw ExplorerSnapshotDiffFailure.invalidResponse
        }

        let nodes = try raw.nodes.map {
            try mapNode($0, maximumNameBytes: 1024)
        }
        guard
            Set(nodes.map(\.id)).count == nodes.count,
            nodes.allSatisfy({ $0.parentID == expectedParentID && $0.depth > 0 }),
            nodes.elementsEqual(nodes.sorted { ordered($0, before: $1, sort: sort) })
        else {
            throw ExplorerSnapshotDiffFailure.invalidResponse
        }

        if raw.offset == 0, UInt64(nodes.count) == raw.totalChildren {
            try validateCompletePageTotals(raw, nodes: nodes)
        }

        return ExplorerSnapshotDiffNodePage(
            parentID: raw.parentId,
            offset: raw.offset,
            totalChildren: raw.totalChildren,
            hasMore: raw.hasMore,
            totalGrowthBytes: raw.totalGrowthBytes,
            totalShrinkageBytes: raw.totalShrinkageBytes,
            unchangedChildCount: raw.unchangedChildCount,
            replacedChildCount: raw.replacedChildCount,
            nodes: nodes
        )
    }

    static func mapTreemap(
        _ raw: SnapshotDiffTreemap,
        expectedParentID: UInt64,
        requestedMaxCells: UInt16
    ) throws -> ExplorerSnapshotDiffTreemap {
        let (representedAndOtherGrowth, growthCountOverflow) = UInt64(raw.cells.count)
            .addingReportingOverflow(raw.otherGrowthChildCount)
        let (changedCount, changedCountOverflow) = representedAndOtherGrowth
            .addingReportingOverflow(raw.otherShrinkageChildCount)
        let (totalCount, totalCountOverflow) = raw.changedChildCount
            .addingReportingOverflow(raw.unchangedChildCount)
        guard
            (1 ... maximumTreemapCells).contains(requestedMaxCells),
            raw.recordVersion == recordVersion,
            raw.parentId == expectedParentID,
            UInt64(raw.cells.count)
                == min(UInt64(requestedMaxCells), raw.changedChildCount),
            !growthCountOverflow,
            !changedCountOverflow,
            !totalCountOverflow,
            changedCount == raw.changedChildCount,
            totalCount == raw.totalChildren,
            raw.replacedChildCount <= raw.totalChildren,
            (raw.otherGrowthChildCount == 0) == (raw.otherGrowthBytes == 0),
            (raw.otherShrinkageChildCount == 0) == (raw.otherShrinkageBytes == 0)
        else {
            throw ExplorerSnapshotDiffFailure.invalidResponse
        }

        var cells: [ExplorerSnapshotDiffTreemapCell] = []
        var representedGrowth = UInt64(0)
        var representedShrinkage = UInt64(0)
        var previousMagnitude = UInt64.max
        var previousNode: ExplorerSnapshotDiffNode?
        cells.reserveCapacity(raw.cells.count)
        for (index, rawCell) in raw.cells.enumerated() {
            let node = try mapNode(rawCell.node, maximumNameBytes: 1024)
            guard
                rawCell.recordVersion == recordVersion,
                rawCell.magnitudeRank == UInt64(index),
                node.parentID == expectedParentID,
                node.depth > 0,
                node.logicalChange.magnitudeBytes > 0,
                node.logicalChange.magnitudeBytes <= previousMagnitude,
                node.logicalChange.direction != .unchanged
            else {
                throw ExplorerSnapshotDiffFailure.invalidResponse
            }
            if let previousNode,
               node.logicalChange.magnitudeBytes == previousMagnitude,
               !ordered(previousNode, before: node, sort: .magnitudeDescending)
            {
                throw ExplorerSnapshotDiffFailure.invalidResponse
            }
            previousMagnitude = node.logicalChange.magnitudeBytes
            previousNode = node
            switch node.logicalChange.direction {
            case .growth:
                representedGrowth = try checkedAdd(
                    representedGrowth,
                    node.logicalChange.magnitudeBytes
                )
            case .shrinkage:
                representedShrinkage = try checkedAdd(
                    representedShrinkage,
                    node.logicalChange.magnitudeBytes
                )
            case .unchanged:
                throw ExplorerSnapshotDiffFailure.invalidResponse
            }
            cells.append(
                ExplorerSnapshotDiffTreemapCell(
                    node: node,
                    magnitudeRank: rawCell.magnitudeRank
                )
            )
        }

        guard
            Set(cells.map(\.id)).count == cells.count,
            try checkedAdd(representedGrowth, raw.otherGrowthBytes) == raw.totalGrowthBytes,
            try checkedAdd(representedShrinkage, raw.otherShrinkageBytes)
                == raw.totalShrinkageBytes
        else {
            throw ExplorerSnapshotDiffFailure.invalidResponse
        }

        return ExplorerSnapshotDiffTreemap(
            parentID: raw.parentId,
            totalChildren: raw.totalChildren,
            changedChildCount: raw.changedChildCount,
            totalGrowthBytes: raw.totalGrowthBytes,
            totalShrinkageBytes: raw.totalShrinkageBytes,
            otherGrowthChildCount: raw.otherGrowthChildCount,
            otherGrowthBytes: raw.otherGrowthBytes,
            otherShrinkageChildCount: raw.otherShrinkageChildCount,
            otherShrinkageBytes: raw.otherShrinkageBytes,
            unchangedChildCount: raw.unchangedChildCount,
            replacedChildCount: raw.replacedChildCount,
            cells: cells
        )
    }

    static func ffiSort(_ sort: ExplorerSnapshotDiffSort) -> SnapshotDiffNodeSort {
        switch sort {
        case .nameAscending: .nameAscending
        case .magnitudeDescending: .magnitudeDescending
        case .currentBytesDescending: .currentBytesDescending
        }
    }

    private static func mapNode(
        _ raw: SnapshotDiffNode,
        maximumNameBytes: Int
    ) throws -> ExplorerSnapshotDiffNode {
        let name = try ExplorerSnapshotNodeAdapter.mapName(
            raw.name,
            maximumNameBytes: maximumNameBytes
        )
        let logicalChange = mapValue(raw.logicalChange)
        guard
            raw.recordVersion == recordVersion,
            validPresence(raw),
            logicalChange == expectedValue(
                current: raw.currentLogicalBytes,
                baseline: raw.baselineLogicalBytes
            ),
            validChange(raw.change, value: logicalChange, raw: raw),
            validAllocatedChange(raw),
            validDescent(raw)
        else {
            throw ExplorerSnapshotDiffFailure.invalidResponse
        }

        return ExplorerSnapshotDiffNode(
            id: raw.id,
            parentID: raw.parentId,
            depth: raw.depth,
            name: name,
            kind: ExplorerSnapshotNodeAdapter.mapKind(raw.kind),
            currentKind: raw.currentKind.map(ExplorerSnapshotNodeAdapter.mapKind),
            baselineKind: raw.baselineKind.map(ExplorerSnapshotNodeAdapter.mapKind),
            category: ExplorerSnapshotNodeAdapter.mapCategory(raw.category),
            change: mapChange(raw.change),
            logicalChange: logicalChange,
            currentLogicalBytes: raw.currentLogicalBytes,
            baselineLogicalBytes: raw.baselineLogicalBytes,
            currentAllocatedBytes: raw.currentAllocatedBytes,
            baselineAllocatedBytes: raw.baselineAllocatedBytes,
            allocatedChange: raw.allocatedChange.map(mapValue),
            currentFileCount: raw.currentFileCount,
            baselineFileCount: raw.baselineFileCount,
            currentChildCount: raw.currentChildCount,
            baselineChildCount: raw.baselineChildCount,
            currentScanFlags: raw.currentScanFlags.map(mapFlags),
            baselineScanFlags: raw.baselineScanFlags.map(mapFlags),
            canDescend: raw.canDescend
        )
    }

    private static func validPresence(_ raw: SnapshotDiffNode) -> Bool {
        let currentPresent = raw.currentLogicalBytes != nil
        let baselinePresent = raw.baselineLogicalBytes != nil
        return (currentPresent || baselinePresent)
            && (raw.currentKind != nil) == currentPresent
            && (raw.baselineKind != nil) == baselinePresent
            && (raw.currentFileCount != nil) == currentPresent
            && (raw.currentChildCount != nil) == currentPresent
            && (raw.currentScanFlags != nil) == currentPresent
            && (raw.currentAllocatedBytes == nil || currentPresent)
            && (raw.baselineFileCount != nil) == baselinePresent
            && (raw.baselineChildCount != nil) == baselinePresent
            && (raw.baselineScanFlags != nil) == baselinePresent
            && (raw.baselineAllocatedBytes == nil || baselinePresent)
            && raw.kind == (raw.currentKind ?? raw.baselineKind)
            && raw.currentKind.map({ $0 == .directory || raw.currentChildCount == 0 }) ?? true
            && raw.baselineKind.map({ $0 == .directory || raw.baselineChildCount == 0 }) ?? true
    }

    private static func validChange(
        _ change: SnapshotDiffChange,
        value: ExplorerSnapshotDiffValue,
        raw: SnapshotDiffNode
    ) -> Bool {
        let currentPresent = raw.currentLogicalBytes != nil
        let baselinePresent = raw.baselineLogicalBytes != nil
        switch change {
        case .added:
            return currentPresent && !baselinePresent
                && (value.direction == .growth || value.direction == .unchanged)
        case .removed:
            return !currentPresent && baselinePresent
                && (value.direction == .shrinkage || value.direction == .unchanged)
                && raw.category == .unclassified
        case .grew:
            return currentPresent && baselinePresent
                && raw.currentKind == raw.baselineKind
                && value.direction == .growth
        case .shrank:
            return currentPresent && baselinePresent
                && raw.currentKind == raw.baselineKind
                && value.direction == .shrinkage
        case .unchanged:
            return currentPresent && baselinePresent
                && raw.currentKind == raw.baselineKind
                && value.direction == .unchanged
        case .replaced:
            return currentPresent && baselinePresent
                && raw.currentKind != raw.baselineKind
        }
    }

    private static func validAllocatedChange(_ raw: SnapshotDiffNode) -> Bool {
        let currentPresent = raw.currentLogicalBytes != nil
        let baselinePresent = raw.baselineLogicalBytes != nil
        let expected: ExplorerSnapshotDiffValue?
        switch (currentPresent, baselinePresent) {
        case (true, true):
            if let current = raw.currentAllocatedBytes,
               let baseline = raw.baselineAllocatedBytes
            {
                expected = expectedValue(current: current, baseline: baseline)
            } else {
                expected = nil
            }
        case (true, false):
            expected = raw.currentAllocatedBytes.flatMap {
                expectedValue(current: $0, baseline: 0)
            }
        case (false, true):
            expected = raw.baselineAllocatedBytes.flatMap {
                expectedValue(current: 0, baseline: $0)
            }
        case (false, false):
            return false
        }
        return raw.allocatedChange.map(mapValue) == expected
    }

    private static func validDescent(_ raw: SnapshotDiffNode) -> Bool {
        raw.canDescend
            == (raw.currentKind == .directory || raw.baselineKind == .directory)
    }

    private static func expectedValue(
        current: UInt64?,
        baseline: UInt64?
    ) -> ExplorerSnapshotDiffValue? {
        guard current != nil || baseline != nil else {
            return nil
        }
        let current = current ?? 0
        let baseline = baseline ?? 0
        if current > baseline {
            return ExplorerSnapshotDiffValue(
                direction: .growth,
                magnitudeBytes: current - baseline
            )
        }
        if current < baseline {
            return ExplorerSnapshotDiffValue(
                direction: .shrinkage,
                magnitudeBytes: baseline - current
            )
        }
        return ExplorerSnapshotDiffValue(direction: .unchanged, magnitudeBytes: 0)
    }

    private static func mapValue(_ raw: SnapshotDiffValue) -> ExplorerSnapshotDiffValue {
        let direction: ExplorerSnapshotDiffDirection = switch raw.direction {
        case .growth: .growth
        case .shrinkage: .shrinkage
        case .unchanged: .unchanged
        }
        return ExplorerSnapshotDiffValue(
            direction: direction,
            magnitudeBytes: raw.magnitudeBytes
        )
    }

    private static func mapChange(_ raw: SnapshotDiffChange) -> ExplorerSnapshotDiffChange {
        switch raw {
        case .added: .added
        case .removed: .removed
        case .grew: .grew
        case .shrank: .shrank
        case .unchanged: .unchanged
        case .replaced: .replaced
        }
    }

    private static func mapFlags(_ raw: SnapshotNodeScanFlags) -> ExplorerSnapshotScanFlags {
        ExplorerSnapshotScanFlags(
            inaccessible: raw.inaccessible,
            timedOut: raw.timedOut,
            hardLinkDuplicate: raw.hardLinkDuplicate,
            mountBoundary: raw.mountBoundary
        )
    }

    private static func validateCompletePageTotals(
        _ raw: SnapshotDiffNodePage,
        nodes: [ExplorerSnapshotDiffNode]
    ) throws {
        var growth = UInt64(0)
        var shrinkage = UInt64(0)
        var unchanged = UInt64(0)
        var replaced = UInt64(0)
        for node in nodes {
            switch node.logicalChange.direction {
            case .growth:
                growth = try checkedAdd(growth, node.logicalChange.magnitudeBytes)
            case .shrinkage:
                shrinkage = try checkedAdd(shrinkage, node.logicalChange.magnitudeBytes)
            case .unchanged:
                unchanged = try checkedAdd(unchanged, 1)
            }
            if node.change == .replaced {
                replaced = try checkedAdd(replaced, 1)
            }
        }
        guard
            growth == raw.totalGrowthBytes,
            shrinkage == raw.totalShrinkageBytes,
            unchanged == raw.unchangedChildCount,
            replaced == raw.replacedChildCount
        else {
            throw ExplorerSnapshotDiffFailure.invalidResponse
        }
    }

    private static func ordered(
        _ left: ExplorerSnapshotDiffNode,
        before right: ExplorerSnapshotDiffNode,
        sort: ExplorerSnapshotDiffSort
    ) -> Bool {
        switch sort {
        case .nameAscending:
            return nameComesBefore(left, right)
        case .magnitudeDescending:
            if left.logicalChange.magnitudeBytes != right.logicalChange.magnitudeBytes {
                return left.logicalChange.magnitudeBytes > right.logicalChange.magnitudeBytes
            }
            return nameComesBefore(left, right)
        case .currentBytesDescending:
            let leftBytes = left.currentLogicalBytes ?? 0
            let rightBytes = right.currentLogicalBytes ?? 0
            if leftBytes != rightBytes {
                return leftBytes > rightBytes
            }
            return nameComesBefore(left, right)
        }
    }

    private static func nameComesBefore(
        _ left: ExplorerSnapshotDiffNode,
        _ right: ExplorerSnapshotDiffNode
    ) -> Bool {
        let leftRank = encodingRank(left.name.encoding)
        let rightRank = encodingRank(right.name.encoding)
        if leftRank != rightRank {
            return leftRank < rightRank
        }
        if left.name.encodedBytes != right.name.encodedBytes {
            return left.name.encodedBytes.lexicographicallyPrecedes(
                right.name.encodedBytes
            )
        }
        return left.id < right.id
    }

    private static func encodingRank(_ encoding: ExplorerSnapshotNameEncoding) -> UInt8 {
        switch encoding {
        case .unixBytes: 0
        case .windowsUTF16LittleEndian: 1
        }
    }

    private static func checkedAdd(_ left: UInt64, _ right: UInt64) throws -> UInt64 {
        let value = left.addingReportingOverflow(right)
        guard !value.overflow else {
            throw ExplorerSnapshotDiffFailure.invalidResponse
        }
        return value.partialValue
    }

    private static func validCoverage(_ raw: ScanCoverageSummary) -> Bool {
        guard
            raw.recordVersion == recordVersion,
            raw.issueRecordCount <= 256,
            raw.issueOccurrenceCount >= raw.issueRecordCount,
            raw.measuredPermille.map({ $0 <= 1000 }) ?? true
        else {
            return false
        }
        switch raw.status {
        case .unknown:
            return raw.measuredPermille == nil
                && raw.issueRecordCount == 0
                && raw.issueOccurrenceCount == 0
        case .complete:
            return raw.measuredPermille == 1000
                && raw.issueRecordCount == 0
                && raw.issueOccurrenceCount == 0
        case .limitedAccess, .partial:
            return raw.issueRecordCount > 0 && raw.measuredPermille != 1000
        }
    }

    private static func mapCoverage(
        _ raw: ScanCoverageSummary
    ) -> ExplorerSnapshotDiffCoverage {
        let status: AppScanCoverage = switch raw.status {
        case .unknown: .unknown
        case .complete: .complete
        case .limitedAccess: .limitedAccess
        case .partial: .partial
        }
        return ExplorerSnapshotDiffCoverage(
            status: status,
            measuredPermille: raw.measuredPermille,
            issueRecordCount: raw.issueRecordCount,
            issueOccurrenceCount: raw.issueOccurrenceCount
        )
    }

    private static func currentSortsBeforeBaseline(_ raw: SnapshotDiffInfo) -> Bool {
        if raw.currentCompletedAtUnixMs != raw.baselineCompletedAtUnixMs {
            return raw.currentCompletedAtUnixMs > raw.baselineCompletedAtUnixMs
        }
        if raw.currentStartedAtUnixMs != raw.baselineStartedAtUnixMs {
            return raw.currentStartedAtUnixMs > raw.baselineStartedAtUnixMs
        }
        return raw.currentScanId < raw.baselineScanId
    }

    private static func validUnixMilliseconds(_ value: Int64) -> Bool {
        (0 ... maximumUnixMilliseconds).contains(value)
    }

    private static func date(_ value: Int64) -> Date {
        Date(timeIntervalSince1970: Double(value) / 1000)
    }
}
