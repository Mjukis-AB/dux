import SwiftUI

#if DUX_CLEANUP_QUALIFICATION
enum CleanupQualificationAccessibility {
    static let notice = "cleanup-qualification.notice"
}

struct CleanupQualificationNotice: View {
    let compact: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            Label(
                "Signed cleanup qualification",
                systemImage: "exclamationmark.shield.fill"
            )
            .font(compact ? .subheadline.weight(.semibold) : .headline)

            Text(
                "Disposable test data only. This is not a public DUX build and uses the production app identity."
            )
            .font(.caption)
        }
        .foregroundStyle(.orange)
        .padding(compact ? 8 : 10)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(.orange.opacity(0.12), in: RoundedRectangle(cornerRadius: 9))
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier(CleanupQualificationAccessibility.notice)
        .accessibilityLabel(
            "Signed cleanup qualification. Disposable test data only. Not a public DUX build."
        )
    }
}
#endif
