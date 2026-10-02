import Foundation

enum ExplorerDiskMapVisualID: Hashable, Sendable {
    case node(UInt64)
    case used
    case scanned
    case available
    case unmapped
    case other
}

struct ExplorerDiskMapLayoutItem: Equatable, Sendable {
    let id: ExplorerDiskMapVisualID
    let allocatedBytes: UInt64
}

struct ExplorerDiskMapLayoutCircle: Equatable, Sendable {
    let id: ExplorerDiskMapVisualID
    let center: CGPoint
    let radius: CGFloat
}

/// A deterministic, bounded tangent pack. Each new circle is placed against a
/// pair of existing circles at the position that keeps the enclosing footprint
/// most compact. A common final scale preserves exact area ratios.
enum ExplorerDiskMapLayout {
    private struct UnitCircle {
        let id: ExplorerDiskMapVisualID
        let center: CGPoint
        let radius: CGFloat
    }

    private struct CandidateScore: Comparable {
        let span: CGFloat
        let area: CGFloat
        let centerOffset: CGFloat
        let angleDelta: CGFloat
        let x: CGFloat
        let y: CGFloat

        static func < (lhs: Self, rhs: Self) -> Bool {
            if abs(lhs.span - rhs.span) > 0.000_000_1 { return lhs.span < rhs.span }
            if abs(lhs.area - rhs.area) > 0.000_000_1 { return lhs.area < rhs.area }
            if abs(lhs.centerOffset - rhs.centerOffset) > 0.000_000_1 {
                return lhs.centerOffset < rhs.centerOffset
            }
            if abs(lhs.angleDelta - rhs.angleDelta) > 0.000_000_1 {
                return lhs.angleDelta < rhs.angleDelta
            }
            if abs(lhs.x - rhs.x) > 0.000_000_1 { return lhs.x < rhs.x }
            return lhs.y < rhs.y
        }
    }

    private struct Extents {
        let minX: CGFloat
        let maxX: CGFloat
        let minY: CGFloat
        let maxY: CGFloat
    }

    static func circles(
        for items: [ExplorerDiskMapLayoutItem],
        in bounds: CGRect,
        inset: CGFloat = 8
    ) -> [ExplorerDiskMapLayoutCircle] {
        guard
            bounds.width.isFinite,
            bounds.height.isFinite,
            bounds.width > inset * 2,
            bounds.height > inset * 2
        else { return [] }

        let positive = items.filter { $0.allocatedBytes > 0 }
        guard let maximum = positive.map(\.allocatedBytes).max(), maximum > 0 else {
            return []
        }

        let packingOrder = positive.enumerated().sorted {
            if $0.element.allocatedBytes == $1.element.allocatedBytes {
                return $0.offset < $1.offset
            }
            return $0.element.allocatedBytes > $1.element.allocatedBytes
        }.map(\.element)
        var packed = [UnitCircle]()
        packed.reserveCapacity(packingOrder.count)
        for (index, item) in packingOrder.enumerated() {
            let radius = CGFloat(sqrt(Double(item.allocatedBytes) / Double(maximum)))
            let center: CGPoint
            switch index {
            case 0:
                center = .zero
            case 1:
                center = CGPoint(x: packed[0].radius + radius, y: 0)
            default:
                center = compactTangentCenter(
                    radius: radius,
                    packed: packed,
                    preferredAngle: CGFloat(index) * .pi * (3 - sqrt(5))
                ) ?? CGPoint(
                    x: (packed.map { $0.center.x + $0.radius }.max() ?? 0) + radius,
                    y: 0
                )
            }
            packed.append(UnitCircle(id: item.id, center: center, radius: radius))
        }

        let minX = packed.map { $0.center.x - $0.radius }.min() ?? 0
        let maxX = packed.map { $0.center.x + $0.radius }.max() ?? 0
        let minY = packed.map { $0.center.y - $0.radius }.min() ?? 0
        let maxY = packed.map { $0.center.y + $0.radius }.max() ?? 0
        let localCenter = CGPoint(x: (minX + maxX) / 2, y: (minY + maxY) / 2)
        let finalEnclosingRadius = packed.map {
            hypot($0.center.x - localCenter.x, $0.center.y - localCenter.y) + $0.radius
        }.max() ?? 1
        let stageRadius = max(0, min(bounds.width, bounds.height) / 2 - inset)
        let scale = stageRadius / max(finalEnclosingRadius, .leastNonzeroMagnitude)
        let stageCenter = CGPoint(x: bounds.midX, y: bounds.midY)

        let projected = packed.map {
            ExplorerDiskMapLayoutCircle(
                id: $0.id,
                center: CGPoint(
                    x: stageCenter.x + ($0.center.x - localCenter.x) * scale,
                    y: stageCenter.y + ($0.center.y - localCenter.y) * scale
                ),
                radius: $0.radius * scale
            )
        }
        let circlesByID: [ExplorerDiskMapVisualID: ExplorerDiskMapLayoutCircle] =
            Dictionary(uniqueKeysWithValues: projected.map { ($0.id, $0) })
        return positive.compactMap { circlesByID[$0.id] }
    }

    private static func compactTangentCenter(
        radius: CGFloat,
        packed: [UnitCircle],
        preferredAngle: CGFloat
    ) -> CGPoint? {
        var best: (center: CGPoint, score: CandidateScore)?
        var minX = CGFloat.greatestFiniteMagnitude
        var maxX = -CGFloat.greatestFiniteMagnitude
        var minY = CGFloat.greatestFiniteMagnitude
        var maxY = -CGFloat.greatestFiniteMagnitude
        for circle in packed {
            minX = min(minX, circle.center.x - circle.radius)
            maxX = max(maxX, circle.center.x + circle.radius)
            minY = min(minY, circle.center.y - circle.radius)
            maxY = max(maxY, circle.center.y + circle.radius)
        }
        let extents = Extents(minX: minX, maxX: maxX, minY: minY, maxY: maxY)
        let frontier = candidateFrontier(in: packed)
        for leftOffset in frontier.indices {
            let leftIndex = frontier[leftOffset]
            for rightOffset in frontier.indices where rightOffset > leftOffset {
                let rightIndex = frontier[rightOffset]
                for center in tangentCenters(
                    radius: radius,
                    left: packed[leftIndex],
                    right: packed[rightIndex]
                ) where doesNotOverlap(center: center, radius: radius, packed: packed) {
                    let score = candidateScore(
                        center: center,
                        radius: radius,
                        extents: extents,
                        preferredAngle: preferredAngle
                    )
                    if best == nil || score < best!.score {
                        best = (center, score)
                    }
                }
            }
        }
        return best?.center
    }

    /// Bounded contact frontier: the largest anchors, the newest placements,
    /// and the outer hull. Every candidate is still checked against all packed
    /// circles, so the bound changes search cost without weakening separation.
    private static func candidateFrontier(in packed: [UnitCircle]) -> [Int] {
        var indices = Set<Int>()
        indices.formUnion(packed.indices.prefix(4))
        indices.formUnion(packed.indices.suffix(7))
        indices.formUnion(
            packed.indices.sorted {
                let left = hypot(packed[$0].center.x, packed[$0].center.y) + packed[$0].radius
                let right = hypot(packed[$1].center.x, packed[$1].center.y) + packed[$1].radius
                if abs(left - right) > 0.000_000_1 { return left > right }
                return $0 < $1
            }.prefix(5)
        )
        return indices.sorted()
    }

    private static func tangentCenters(
        radius: CGFloat,
        left: UnitCircle,
        right: UnitCircle
    ) -> [CGPoint] {
        let dx = right.center.x - left.center.x
        let dy = right.center.y - left.center.y
        let distance = hypot(dx, dy)
        guard distance > .leastNonzeroMagnitude else { return [] }
        let leftDistance = left.radius + radius
        let rightDistance = right.radius + radius
        guard
            distance <= leftDistance + rightDistance + 0.000_001,
            distance + 0.000_001 >= abs(leftDistance - rightDistance)
        else { return [] }

        let along = (
            pow(leftDistance, 2) - pow(rightDistance, 2) + pow(distance, 2)
        ) / (2 * distance)
        let perpendicular = sqrt(max(0, pow(leftDistance, 2) - pow(along, 2)))
        let base = CGPoint(
            x: left.center.x + along * dx / distance,
            y: left.center.y + along * dy / distance
        )
        let offset = CGPoint(
            x: -dy * perpendicular / distance,
            y: dx * perpendicular / distance
        )
        if perpendicular < 0.000_000_1 {
            return [base]
        }
        return [
            CGPoint(x: base.x + offset.x, y: base.y + offset.y),
            CGPoint(x: base.x - offset.x, y: base.y - offset.y),
        ]
    }

    private static func doesNotOverlap(
        center: CGPoint,
        radius: CGFloat,
        packed: [UnitCircle]
    ) -> Bool {
        packed.allSatisfy {
            hypot(center.x - $0.center.x, center.y - $0.center.y) + 0.000_001
                >= radius + $0.radius
        }
    }

    private static func candidateScore(
        center: CGPoint,
        radius: CGFloat,
        extents: Extents,
        preferredAngle: CGFloat
    ) -> CandidateScore {
        let minX = min(center.x - radius, extents.minX)
        let maxX = max(center.x + radius, extents.maxX)
        let minY = min(center.y - radius, extents.minY)
        let maxY = max(center.y + radius, extents.maxY)
        let width = maxX - minX
        let height = maxY - minY
        let angle = atan2(center.y, center.x)
        let rawAngleDelta = abs(angle - preferredAngle).truncatingRemainder(dividingBy: 2 * .pi)
        let angleDelta = min(rawAngleDelta, 2 * .pi - rawAngleDelta)
        return CandidateScore(
            span: max(width, height),
            area: width * height,
            centerOffset: hypot((minX + maxX) / 2, (minY + maxY) / 2),
            angleDelta: angleDelta,
            x: center.x,
            y: center.y
        )
    }
}

struct ExplorerDiskMapVolumeEnvelope: Equatable, Sendable {
    let totalBytes: UInt64
    let usedBytes: UInt64
    let availableBytes: UInt64
    let measuredBytes: UInt64
    let unmappedBytes: UInt64

    static func make(
        capacity: VolumeCapacitySnapshot,
        measuredAllocatedBytes: UInt64?
    ) -> Self? {
        guard
            capacity.totalBytes > 0,
            let available = capacity.filesystemAvailableBytes,
            available <= capacity.totalBytes
        else { return nil }
        let used = capacity.totalBytes - available
        let measured = min(measuredAllocatedBytes ?? 0, used)
        return Self(
            totalBytes: capacity.totalBytes,
            usedBytes: used,
            availableBytes: available,
            measuredBytes: measured,
            unmappedBytes: used - measured
        )
    }

    var layoutItems: [ExplorerDiskMapLayoutItem] {
        overviewLayoutItems
    }

    var overviewLayoutItems: [ExplorerDiskMapLayoutItem] {
        [
            ExplorerDiskMapLayoutItem(id: .used, allocatedBytes: usedBytes),
            ExplorerDiskMapLayoutItem(id: .available, allocatedBytes: availableBytes),
        ]
    }

    var usedLayoutItems: [ExplorerDiskMapLayoutItem] {
        [
            ExplorerDiskMapLayoutItem(id: .scanned, allocatedBytes: measuredBytes),
            ExplorerDiskMapLayoutItem(id: .unmapped, allocatedBytes: unmappedBytes),
        ]
    }
}
