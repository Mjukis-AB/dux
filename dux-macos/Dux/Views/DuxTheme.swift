import SwiftUI

/// Shared visual language for DUX surfaces: card containers, sidebar icon
/// tiles, and disk-pressure tinting. Purely presentational; no state.
enum DuxTheme {
    static func tint(for pressure: DiskPressureLevel) -> Color {
        switch pressure {
        case .healthy: .blue
        case .warning: .orange
        case .critical: .red
        case .unknown: .gray
        }
    }
}

/// Card look for `GroupBox`: a small tinted header row above content in a
/// rounded container. Apply once per surface with `.groupBoxStyle(.duxCard)`.
struct DuxCardGroupBoxStyle: GroupBoxStyle {
    func makeBody(configuration: Configuration) -> some View {
        VStack(alignment: .leading, spacing: 12) {
            configuration.label
                .labelStyle(DuxCardHeaderLabelStyle())
            configuration.content
        }
        .padding(16)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(
            RoundedRectangle(cornerRadius: 12, style: .continuous)
                .fill(Color(nsColor: .textBackgroundColor).opacity(0.55))
        )
        .overlay(
            RoundedRectangle(cornerRadius: 12, style: .continuous)
                .strokeBorder(.quaternary, lineWidth: 1)
        )
    }
}

private struct DuxCardHeaderLabelStyle: LabelStyle {
    func makeBody(configuration: Configuration) -> some View {
        HStack(spacing: 7) {
            configuration.icon
                .font(.subheadline.weight(.semibold))
                .foregroundStyle(.tint)
            configuration.title
                .font(.subheadline.weight(.semibold))
                .foregroundStyle(.secondary)
        }
    }
}

extension GroupBoxStyle where Self == DuxCardGroupBoxStyle {
    static var duxCard: DuxCardGroupBoxStyle { DuxCardGroupBoxStyle() }
}

/// Sidebar rows with a colored rounded icon tile, System Settings style.
struct DuxSidebarLabelStyle: LabelStyle {
    let tint: Color

    func makeBody(configuration: Configuration) -> some View {
        Label {
            configuration.title
        } icon: {
            configuration.icon
                .font(.system(size: 11, weight: .semibold))
                .foregroundStyle(.white)
                .frame(width: 24, height: 24)
                .background(
                    tint.gradient,
                    in: RoundedRectangle(cornerRadius: 7, style: .continuous)
                )
        }
    }
}

extension LabelStyle where Self == DuxSidebarLabelStyle {
    static func duxSidebar(_ tint: Color) -> DuxSidebarLabelStyle {
        DuxSidebarLabelStyle(tint: tint)
    }
}
