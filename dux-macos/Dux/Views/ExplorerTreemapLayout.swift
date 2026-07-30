import Foundation

enum ExplorerTreemapVisualID: Hashable, Sendable {
    case node(UInt64)
    case other
    case otherGrowth
    case otherShrinkage
}

struct ExplorerTreemapLayoutItem: Equatable, Sendable {
    let id: ExplorerTreemapVisualID
    let logicalBytes: UInt64
}

struct ExplorerTreemapLayoutRectangle: Equatable, Sendable {
    let id: ExplorerTreemapVisualID
    let rect: CGRect
}

/// A deterministic binary treemap. Every split follows the longer edge and
/// keeps the existing logical-size order, so identical input stays visually
/// stable across redraws without inventing category meaning.
enum ExplorerTreemapLayout {
    static func rectangles(
        for items: [ExplorerTreemapLayoutItem],
        in bounds: CGRect
    ) -> [ExplorerTreemapLayoutRectangle] {
        guard
            bounds.width.isFinite,
            bounds.height.isFinite,
            bounds.width > 0,
            bounds.height > 0
        else {
            return []
        }
        let positive = items.filter { $0.logicalBytes > 0 }
        guard !positive.isEmpty else {
            return []
        }
        var result: [ExplorerTreemapLayoutRectangle] = []
        result.reserveCapacity(positive.count)
        append(positive[...], in: bounds, to: &result)
        return result
    }

    private static func append(
        _ items: ArraySlice<ExplorerTreemapLayoutItem>,
        in bounds: CGRect,
        to result: inout [ExplorerTreemapLayoutRectangle]
    ) {
        guard let first = items.first else {
            return
        }
        if items.count == 1 {
            result.append(ExplorerTreemapLayoutRectangle(id: first.id, rect: bounds))
            return
        }

        let total = items.reduce(UInt64(0)) { partial, item in
            partial.addingReportingOverflow(item.logicalBytes).overflow
                ? UInt64.max
                : partial + item.logicalBytes
        }
        guard total > 0 else {
            return
        }

        let target = Double(total) / 2
        var leadingTotal: UInt64 = 0
        var selectedLeadingTotal: UInt64 = 0
        var splitIndex = items.startIndex
        var bestDistance = Double.greatestFiniteMagnitude
        for index in items.indices.dropLast() {
            let candidate = leadingTotal.addingReportingOverflow(items[index].logicalBytes)
            leadingTotal = candidate.overflow ? UInt64.max : candidate.partialValue
            let distance = abs(Double(leadingTotal) - target)
            if distance <= bestDistance {
                bestDistance = distance
                splitIndex = items.index(after: index)
                selectedLeadingTotal = leadingTotal
            } else {
                break
            }
        }

        let fraction = min(max(CGFloat(Double(selectedLeadingTotal) / Double(total)), 0), 1)
        let leadingBounds: CGRect
        let trailingBounds: CGRect
        if bounds.width >= bounds.height {
            let split = bounds.minX + bounds.width * fraction
            leadingBounds = CGRect(
                x: bounds.minX,
                y: bounds.minY,
                width: split - bounds.minX,
                height: bounds.height
            )
            trailingBounds = CGRect(
                x: split,
                y: bounds.minY,
                width: bounds.maxX - split,
                height: bounds.height
            )
        } else {
            let split = bounds.minY + bounds.height * fraction
            leadingBounds = CGRect(
                x: bounds.minX,
                y: bounds.minY,
                width: bounds.width,
                height: split - bounds.minY
            )
            trailingBounds = CGRect(
                x: bounds.minX,
                y: split,
                width: bounds.width,
                height: bounds.maxY - split
            )
        }
        append(items[..<splitIndex], in: leadingBounds, to: &result)
        append(items[splitIndex...], in: trailingBounds, to: &result)
    }
}
