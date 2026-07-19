import Foundation

enum DiskPressureNotificationUrgency: String, Equatable, Sendable {
    case warning
    case critical
}

struct DiskPressureNotificationPayload: Equatable, Sendable {
    static let recordVersion = "1"
    static let route = "recommendations"
    static let maximumVolumeIDBytes = 256

    let stableVolumeID: String
    let urgency: DiskPressureNotificationUrgency

    init?(stableVolumeID: String, urgency: DiskPressureNotificationUrgency) {
        guard Self.isValidVolumeID(stableVolumeID) else {
            return nil
        }
        self.stableVolumeID = stableVolumeID
        self.urgency = urgency
    }

    init?(fields: [String: String]) {
        guard
            fields["recordVersion"] == Self.recordVersion,
            fields["route"] == Self.route,
            let stableVolumeID = fields["stableVolumeID"],
            let rawUrgency = fields["urgency"],
            let urgency = DiskPressureNotificationUrgency(rawValue: rawUrgency),
            fields.count == 4,
            Self.isValidVolumeID(stableVolumeID)
        else {
            return nil
        }
        self.stableVolumeID = stableVolumeID
        self.urgency = urgency
    }

    var fields: [String: String] {
        [
            "recordVersion": Self.recordVersion,
            "route": Self.route,
            "stableVolumeID": stableVolumeID,
            "urgency": urgency.rawValue,
        ]
    }

    private static func isValidVolumeID(_ value: String) -> Bool {
        !value.isEmpty
            && value.utf8.count <= maximumVolumeIDBytes
            && !value.unicodeScalars.contains(where: {
                CharacterSet.controlCharacters.contains($0)
            })
    }
}

struct DiskPressureNotificationCandidate: Equatable, Sendable {
    let payload: DiskPressureNotificationPayload
    let sampledAt: Date
    let availableBytes: UInt64
    let displayName: String?

    var delivery: DiskPressureNotificationDelivery {
        DiskPressureNotificationDelivery(
            identifier: "dux.disk-pressure.\(payload.urgency.rawValue).\(payload.stableVolumeID)",
            title: payload.urgency == .critical
                ? "Storage space is critically low"
                : "Storage space is getting low",
            body: "Open DUX Recommendations to review safe ways to reclaim space.",
            userInfo: payload.fields
        )
    }
}

struct DiskPressureNotificationDelivery: Equatable, Sendable {
    let identifier: String
    let title: String
    let body: String
    let userInfo: [String: String]
}

@MainActor
protocol DiskPressureNotificationCooldownStoring {
    func lastAcceptedAt(
        volumeID: String,
        urgency: DiskPressureNotificationUrgency
    ) -> Date?
    func recordAccepted(
        at date: Date,
        volumeID: String,
        urgency: DiskPressureNotificationUrgency
    )
}

@MainActor
struct UserDefaultsDiskPressureNotificationCooldownStore:
    DiskPressureNotificationCooldownStoring
{
    static let keyPrefix = "diskPressure.notification.v1"

    private let defaults: UserDefaults

    init(defaults: UserDefaults = .standard) {
        self.defaults = defaults
    }

    func lastAcceptedAt(
        volumeID: String,
        urgency: DiskPressureNotificationUrgency
    ) -> Date? {
        defaults.object(forKey: key(volumeID: volumeID, urgency: urgency)) as? Date
    }

    func recordAccepted(
        at date: Date,
        volumeID: String,
        urgency: DiskPressureNotificationUrgency
    ) {
        defaults.set(date, forKey: key(volumeID: volumeID, urgency: urgency))
    }

    private func key(
        volumeID: String,
        urgency: DiskPressureNotificationUrgency
    ) -> String {
        "\(Self.keyPrefix).\(urgency.rawValue).\(volumeID)"
    }
}

enum DiskPressureNotificationGate {
    static let cooldown: TimeInterval = 24 * 60 * 60

    static func candidate(
        for snapshot: VolumeCapacitySnapshot,
        lastAcceptedAtForUrgency: Date?
    ) -> DiskPressureNotificationCandidate? {
        guard
            snapshot.historyDisposition == .stored,
            let stableVolumeID = snapshot.stableVolumeID,
            let urgency = urgency(
                previous: snapshot.previousDurablePressure,
                current: snapshot.pressure
            ),
            let payload = DiskPressureNotificationPayload(
                stableVolumeID: stableVolumeID,
                urgency: urgency
            ),
            cooldownAllows(
                sampledAt: snapshot.sampledAt,
                lastAcceptedAt: lastAcceptedAtForUrgency
            )
        else {
            return nil
        }
        return DiskPressureNotificationCandidate(
            payload: payload,
            sampledAt: snapshot.sampledAt,
            availableBytes: snapshot.effectiveAvailableBytes,
            displayName: snapshot.displayName
        )
    }

    private static func urgency(
        previous: DiskPressureLevel?,
        current: DiskPressureLevel
    ) -> DiskPressureNotificationUrgency? {
        switch (previous, current) {
        case (.critical, .warning), (.warning, .warning), (.critical, .critical):
            nil
        case (_, .warning):
            .warning
        case (_, .critical):
            .critical
        case (_, .healthy), (_, .unknown):
            nil
        }
    }

    private static func cooldownAllows(
        sampledAt: Date,
        lastAcceptedAt: Date?
    ) -> Bool {
        guard let lastAcceptedAt else {
            return true
        }
        return sampledAt.timeIntervalSince(lastAcceptedAt) >= cooldown
    }
}
