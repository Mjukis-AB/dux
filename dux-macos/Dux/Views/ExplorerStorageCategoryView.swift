import SwiftUI

struct ExplorerStorageCategoryLabel: View {
    let category: ExplorerStorageCategory

    var body: some View {
        let presentation = category.presentation
        HStack(spacing: 6) {
            Image(systemName: presentation.symbol)
                .foregroundStyle(presentation.palette.color)
            Text(verbatim: presentation.title)
                .foregroundStyle(.primary)
                .lineLimit(1)
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(presentation.accessibilityPhrase)
    }
}

struct ExplorerStorageCategoryLegend: View {
    let categories: [ExplorerStorageCategory]

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text("Categories for represented cells")
                .font(.caption2)
                .foregroundStyle(.secondary)
            ScrollView(.horizontal) {
                HStack(spacing: 12) {
                    ForEach(categories, id: \.self) { category in
                        ExplorerStorageCategoryLabel(category: category)
                            .font(.caption)
                            .accessibilityIdentifier(
                                ExplorerAccessibility.snapshotCategoryLegend(category: category)
                            )
                    }
                }
                .padding(.horizontal, 2)
            }
            .scrollIndicators(.hidden)
        }
        .accessibilityElement(children: .contain)
        .accessibilityLabel("Categories for represented cells")
        .accessibilityIdentifier(ExplorerAccessibility.snapshotCategoryLegend)
    }
}

extension ExplorerStorageCategoryPalette {
    var color: Color {
        switch self {
        case .neutral: .secondary
        case .developer: .purple
        case .applicationCache: .blue
        case .browserCache: .teal
        case .diagnostic: .orange
        case .download: .indigo
        case .device: .pink
        case .cloud: .cyan
        case .review: .brown
        case .protected: .red
        case .unknown: .gray
        }
    }
}
