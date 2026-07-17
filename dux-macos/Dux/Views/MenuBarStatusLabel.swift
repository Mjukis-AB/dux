import SwiftUI

struct MenuBarStatusLabel: View {
    let mode: MenuBarLabelMode
    let volumeState: VolumeCapacityState

    var body: some View {
        let presentation = MenuBarLabelPresentation.make(
            mode: mode,
            volumeState: volumeState
        )
        Group {
            if let visibleText = presentation.visibleText {
                Label {
                    Text(verbatim: visibleText)
                        .monospacedDigit()
                } icon: {
                    statusImage(presentation.symbolName)
                }
                .labelStyle(.titleAndIcon)
            } else {
                statusImage(presentation.symbolName)
            }
        }
        .foregroundStyle(.primary)
        .accessibilityElement(children: .ignore)
        .accessibilityIdentifier(MenuBarLabelPresentation.accessibilityIdentifier)
        .accessibilityLabel(Text(verbatim: presentation.accessibilityLabel))
        .help(Text(verbatim: presentation.accessibilityLabel))
    }

    private func statusImage(_ systemName: String) -> some View {
        Image(systemName: systemName)
            .symbolRenderingMode(.monochrome)
    }
}
