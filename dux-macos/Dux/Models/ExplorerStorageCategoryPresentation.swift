import Foundation

enum ExplorerStorageCategoryPalette: String, CaseIterable, Equatable, Sendable {
    case neutral
    case developer
    case applicationCache
    case browserCache
    case diagnostic
    case download
    case device
    case cloud
    case review
    case protected
    case unknown

    /// A category-specific non-color treatment for compact treemap cells.
    var strokeDash: [CGFloat] {
        switch self {
        case .neutral: [1, 3]
        case .developer: [6, 2]
        case .applicationCache: [2, 2]
        case .browserCache: [8, 2, 2, 2]
        case .diagnostic: [4, 2, 1, 2]
        case .download: [10, 3]
        case .device: [3, 1]
        case .cloud: [7, 2, 1, 2]
        case .review: [5, 1, 1, 1]
        case .protected: [9, 2, 3, 2]
        case .unknown: [2, 1, 2, 4]
        }
    }
}

struct ExplorerStorageCategoryPresentation: Equatable, Sendable {
    let title: String
    let accessibilityPhrase: String
    let symbol: String
    let palette: ExplorerStorageCategoryPalette
}

extension ExplorerStorageCategory {
    var presentation: ExplorerStorageCategoryPresentation {
        switch self {
        case .unclassified:
            ExplorerStorageCategoryPresentation(
                title: "Unclassified",
                accessibilityPhrase: "not classified",
                symbol: "questionmark",
                palette: .neutral
            )
        case .developerArtifact:
            ExplorerStorageCategoryPresentation(
                title: "Developer artifacts",
                accessibilityPhrase: "developer artifacts category",
                symbol: "hammer.fill",
                palette: .developer
            )
        case .applicationCache:
            ExplorerStorageCategoryPresentation(
                title: "Application caches",
                accessibilityPhrase: "application caches category",
                symbol: "shippingbox.fill",
                palette: .applicationCache
            )
        case .browserCache:
            ExplorerStorageCategoryPresentation(
                title: "Browser caches",
                accessibilityPhrase: "browser caches category",
                symbol: "globe",
                palette: .browserCache
            )
        case .logAndDiagnostic:
            ExplorerStorageCategoryPresentation(
                title: "Logs & diagnostics",
                accessibilityPhrase: "logs and diagnostics category",
                symbol: "doc.text.magnifyingglass",
                palette: .diagnostic
            )
        case .installerAndDownload:
            ExplorerStorageCategoryPresentation(
                title: "Installers & downloads",
                accessibilityPhrase: "installers and downloads category",
                symbol: "arrow.down.circle.fill",
                palette: .download
            )
        case .deviceAndSimulatorData:
            ExplorerStorageCategoryPresentation(
                title: "Device & simulator data",
                accessibilityPhrase: "device and simulator data category",
                symbol: "iphone",
                palette: .device
            )
        case .cloudFile:
            ExplorerStorageCategoryPresentation(
                title: "Cloud files",
                accessibilityPhrase: "cloud files category",
                symbol: "icloud.fill",
                palette: .cloud
            )
        case .largeReviewItem:
            ExplorerStorageCategoryPresentation(
                title: "Large review items",
                accessibilityPhrase: "large review items category",
                symbol: "doc.badge.ellipsis",
                palette: .review
            )
        case .protectedSystemData:
            ExplorerStorageCategoryPresentation(
                title: "Protected system data",
                accessibilityPhrase: "protected system data category",
                symbol: "lock.shield.fill",
                palette: .protected
            )
        case .unknownStorage:
            ExplorerStorageCategoryPresentation(
                title: "Unknown storage",
                accessibilityPhrase: "unknown storage category",
                symbol: "questionmark.folder.fill",
                palette: .unknown
            )
        }
    }
}
