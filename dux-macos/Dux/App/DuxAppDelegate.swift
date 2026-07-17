import AppKit

@MainActor
protocol DuxAppRuntimeServing: AnyObject {
    func start() async
    func signalMaintenance(_ trigger: DuxMaintenanceTrigger) async
    func signalCapacity(_ trigger: DuxCapacitySamplingTrigger) async
    func shutdown() async
}

enum DuxTerminationDisposition {
    case terminateNow
    case waitForExistingShutdown
    case beginShutdown
}

@MainActor
final class DuxTerminationGate {
    private var isApproved = false
    private var isInProgress = false

    func begin() -> DuxTerminationDisposition {
        if isApproved {
            return .terminateNow
        }
        if isInProgress {
            return .waitForExistingShutdown
        }
        isInProgress = true
        return .beginShutdown
    }

    func approve() {
        isApproved = true
        isInProgress = false
    }
}

@MainActor
final class DuxAppDelegate: NSObject, NSApplicationDelegate {
    private var observers: [NSObjectProtocol] = []
    private let terminationGate = DuxTerminationGate()
    private let runtime: any DuxAppRuntimeServing

    override convenience init() {
        self.init(runtime: AppRuntime.shared)
    }

    init(runtime: any DuxAppRuntimeServing) {
        self.runtime = runtime
        super.init()
    }

    func applicationDidFinishLaunching(_ notification: Notification) {
        _ = notification
        installObservers()
        Task {
            await runtime.start()
        }
    }

    func applicationDidBecomeActive(_ notification: Notification) {
        _ = notification
        Task {
            await runtime.signalMaintenance(.applicationBecameActive)
        }
    }

    func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
        switch terminationGate.begin() {
        case .terminateNow:
            return .terminateNow
        case .waitForExistingShutdown:
            return .terminateLater
        case .beginShutdown:
            Task {
                await runtime.shutdown()
                terminationGate.approve()
                sender.reply(toApplicationShouldTerminate: true)
            }
            return .terminateLater
        }
    }

    func applicationWillTerminate(_ notification: Notification) {
        _ = notification
        removeObservers()
    }

    private func installObservers() {
        guard observers.isEmpty else {
            return
        }
        observers.append(
            NSWorkspace.shared.notificationCenter.addObserver(
                forName: NSWorkspace.didWakeNotification,
                object: nil,
                queue: .main
            ) { _ in
                Task { @MainActor in
                    await self.handleWake()
                }
            }
        )
        for name in [
            NSWorkspace.didMountNotification,
            NSWorkspace.didUnmountNotification,
            NSWorkspace.didRenameVolumeNotification,
        ] {
            observers.append(
                NSWorkspace.shared.notificationCenter.addObserver(
                    forName: name,
                    object: nil,
                    queue: .main
                ) { _ in
                    Task { @MainActor in
                        await self.handleVolumesChanged()
                    }
                }
            )
        }
        for (name, trigger) in [
            (Notification.Name.NSSystemClockDidChange, DuxMaintenanceTrigger.significantTimeChange),
            (Notification.Name.NSProcessInfoPowerStateDidChange, .energyPolicyChanged),
            (
                Notification.Name("NSProcessInfoThermalStateDidChangeNotification"),
                .energyPolicyChanged
            ),
        ] {
            observers.append(
                NotificationCenter.default.addObserver(
                    forName: name,
                    object: nil,
                    queue: .main
                ) { _ in
                    Task { @MainActor in
                        await self.runtime.signalMaintenance(trigger)
                    }
                }
            )
        }
    }

    func handleWake() async {
        await runtime.signalCapacity(.wake)
        await runtime.signalMaintenance(.wake)
    }

    func handleVolumesChanged() async {
        await runtime.signalCapacity(.volumesChanged)
    }

    private func removeObservers() {
        for observer in observers {
            NotificationCenter.default.removeObserver(observer)
            NSWorkspace.shared.notificationCenter.removeObserver(observer)
        }
        observers.removeAll(keepingCapacity: false)
    }
}

extension AppRuntime: DuxAppRuntimeServing {}
