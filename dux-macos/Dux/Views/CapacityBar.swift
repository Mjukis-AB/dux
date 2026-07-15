import Foundation
import SwiftUI

struct CapacityBar: View {
    let snapshot: VolumeCapacitySnapshot

    var body: some View {
        ProgressView(value: snapshot.usedFraction)
            .progressViewStyle(.linear)
            .tint(.accentColor)
            .accessibilityLabel("Storage used")
            .accessibilityValue(Text(verbatim: "\(snapshot.usedPercentage)%"))
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
