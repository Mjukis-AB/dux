import Foundation
import SwiftUI

struct CapacityBar: View {
    let snapshot: VolumeCapacitySnapshot

    @ViewBuilder
    var body: some View {
        if let usedFraction = snapshot.usedFraction {
            ProgressView(value: usedFraction)
                .progressViewStyle(.linear)
                .tint(.accentColor)
                .accessibilityLabel(Text(verbatim: Self.accessibilityLabel()))
                .accessibilityValue(
                    Text(verbatim: Self.accessibilityValue(for: snapshot))
                )
        } else {
            Capsule()
                .fill(.quaternary)
                .frame(height: 6)
                .accessibilityLabel(Text(verbatim: Self.accessibilityLabel()))
                .accessibilityValue(
                    Text(verbatim: Self.accessibilityValue(for: snapshot))
                )
        }
    }

    static func accessibilityLabel(locale: Locale = .current) -> String {
        String(localized: "Storage used", locale: locale)
    }

    static func accessibilityValue(
        for snapshot: VolumeCapacitySnapshot,
        locale: Locale = .current
    ) -> String {
        guard let usedFraction = snapshot.usedFraction else {
            return String(localized: "Unavailable", locale: locale)
        }
        return usedFraction.formatted(
            .percent.locale(locale).precision(.fractionLength(0))
        )
    }
}

struct DiskPressureBadge: View {
    let pressure: DiskPressureLevel

    var body: some View {
        Label {
            Text(verbatim: Self.localizedTitle(for: pressure))
        } icon: {
            Image(systemName: symbol)
        }
            .font(.caption.weight(.semibold))
            .foregroundStyle(color)
            .padding(.horizontal, 8)
            .padding(.vertical, 3)
            .background(color.opacity(0.14), in: Capsule())
            .accessibilityLabel(
                Text(verbatim: Self.localizedAccessibilityLabel(for: pressure))
            )
    }

    static func localizedTitle(
        for pressure: DiskPressureLevel,
        locale: Locale = .current
    ) -> String {
        switch pressure {
        case .healthy: String(localized: "Healthy", locale: locale)
        case .warning: String(localized: "Low space", locale: locale)
        case .critical: String(localized: "Critically low space", locale: locale)
        case .unknown: String(localized: "Pressure unknown", locale: locale)
        }
    }

    static func localizedAccessibilityLabel(
        for pressure: DiskPressureLevel,
        locale: Locale = .current
    ) -> String {
        let title = localizedTitle(for: pressure, locale: locale)
        return String(localized: "Disk pressure: \(title)", locale: locale)
    }

    private var symbol: String {
        switch pressure {
        case .healthy: "checkmark.circle.fill"
        case .warning: "exclamationmark.triangle.fill"
        case .critical: "exclamationmark.octagon.fill"
        case .unknown: "questionmark.circle"
        }
    }

    private var color: Color {
        switch pressure {
        case .healthy: .green
        case .warning: .orange
        case .critical: .red
        case .unknown: .secondary
        }
    }
}

enum StorageByteFormatter {
    static func string(from bytes: UInt64) -> String {
        ByteCountFormatter.string(
            fromByteCount: Int64(min(bytes, UInt64(Int64.max))),
            countStyle: .file
        )
    }
}
