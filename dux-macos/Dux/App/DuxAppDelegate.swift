import AppKit
import UserNotifications

@MainActor
protocol DuxAutomaticTerminationControlling: AnyObject {
    var automaticTerminationSupportEnabled: Bool { get set }
    func disableAutomaticTermination(_ reason: String)
    func enableAutomaticTermination(_ reason: String)
}

extension ProcessInfo: DuxAutomaticTerminationControlling {}

@MainActor
final class DuxAutomaticTerminationLease {
    private static let reason = "DUX must remain available as a menu-bar application"

    private let controller: any DuxAutomaticTerminationControlling
    private var isHeld = false
    private var disableCount = 0

    init(controller: any DuxAutomaticTerminationControlling = ProcessInfo.processInfo) {
        self.controller = controller
    }

    func acquire() {
        guard !isHeld else {
            return
        }
        // The counter only participates when automatic-termination support is
        // enabled. Keep one process-lifetime opt-out and balance that exact
        // lease during ordered shutdown; scene callbacks only restore the
        // support flag and must not increment the counter again.
        controller.automaticTerminationSupportEnabled = true
        controller.disableAutomaticTermination(Self.reason)
        disableCount = 1
        isHeld = true
    }

    func reassert() {
        guard isHeld else {
            return
        }
        // AppKit can reset the support flag while it tears down and restores
        // MenuBarExtra's transient window. Restore it, but do not touch the
        // counter: release() owns exactly one matching enable call.
        controller.automaticTerminationSupportEnabled = true
    }

    func release() {
        guard isHeld else {
            return
        }
        while disableCount > 0 {
            controller.enableAutomaticTermination(Self.reason)
            disableCount -= 1
        }
        isHeld = false
    }

}

@MainActor
protocol DuxAppRuntimeServing: AnyObject {
    func start() async
    func signalMaintenance(_ trigger: DuxMaintenanceTrigger) async
    func signalCapacity(_ trigger: DuxCapacitySamplingTrigger) async
    func revealMenuBarItemForSession()
    func refreshStorageAccessEvidenceAfterActivation() async
    func handleUrgentRecommendations(_ payload: DiskPressureNotificationPayload) async
    func shutdown() async
}

extension DuxAppRuntimeServing {
    func handleUrgentRecommendations(_ payload: DiskPressureNotificationPayload) async {
        _ = payload
    }
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
    private let automaticTerminationLease: DuxAutomaticTerminationLease
    private let runtime: any DuxAppRuntimeServing

    override convenience init() {
        self.init(runtime: AppRuntime.shared)
    }

    init(
        runtime: any DuxAppRuntimeServing,
        automaticTerminationLease: DuxAutomaticTerminationLease = DuxAutomaticTerminationLease()
    ) {
        self.runtime = runtime
        self.automaticTerminationLease = automaticTerminationLease
        super.init()
    }

    func applicationDidFinishLaunching(_ notification: Notification) {
        _ = notification
        automaticTerminationLease.acquire()
        UNUserNotificationCenter.current().delegate = self
        installObservers()
        Task {
            await runtime.start()
        }
        // SwiftUI restores MenuBarExtra's transient scene after the delegate
        // callback and AppKit may re-enable automatic termination while doing
        // so. Reassert the lease after that restoration turn has settled.
        DispatchQueue.main.asyncAfter(deadline: .now() + .milliseconds(250)) { [weak self] in
            Task { @MainActor in
                self?.automaticTerminationLease.reassert()
            }
        }
    }

    func applicationDidBecomeActive(_ notification: Notification) {
        _ = notification
        scheduleAutomaticTerminationReassertion()
        Task {
            await handleApplicationBecameActive()
        }
    }

    func applicationDidResignActive(_ notification: Notification) {
        _ = notification
        // Closing a MenuBarExtra window can resign the agent application before
        // AppKit has finished restoring its transient scene. Reassert after the
        // scene turn as well as immediately so automatic termination cannot win
        // that race.
        scheduleAutomaticTerminationReassertion()
    }

    func handleApplicationBecameActive() async {
        await runtime.signalMaintenance(.applicationBecameActive)
        await runtime.refreshStorageAccessEvidenceAfterActivation()
    }

    func applicationShouldHandleReopen(
        _ sender: NSApplication,
        hasVisibleWindows flag: Bool
    ) -> Bool {
        _ = sender
        _ = flag
        runtime.revealMenuBarItemForSession()
        return true
    }

    func applicationShouldTerminateAfterLastWindowClosed(
        _ sender: NSApplication
    ) -> Bool {
        _ = sender
        scheduleAutomaticTerminationReassertion()
        return false
    }

    func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
        guard DuxTerminationIntent.consumeExplicitQuitRequest() else {
            // MenuBarExtra owns a transient window. AppKit may ask to terminate
            // an agent app when that window closes; that is not a user Quit.
            scheduleAutomaticTerminationReassertion()
            return .terminateCancel
        }
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
        automaticTerminationLease.release()
    }

    private func installObservers() {
        guard observers.isEmpty else {
            return
        }
        for name in [
            NSWindow.willCloseNotification,
            NSWindow.didResignKeyNotification,
        ] {
            observers.append(
                NotificationCenter.default.addObserver(
                    forName: name,
                    object: nil,
                    queue: .main
                ) { [weak self] _ in
                    Task { @MainActor in
                        self?.scheduleAutomaticTerminationReassertion()
                    }
                }
            )
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

    private func scheduleAutomaticTerminationReassertion() {
        automaticTerminationLease.reassert()
        for delay in [0, 50, 250] {
            DispatchQueue.main.asyncAfter(deadline: .now() + .milliseconds(delay)) {
                [weak self] in
                Task { @MainActor in
                    self?.automaticTerminationLease.reassert()
                }
            }
        }
    }
}

extension DuxAppDelegate: UNUserNotificationCenterDelegate {
    nonisolated func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        didReceive response: UNNotificationResponse,
        withCompletionHandler completionHandler: @escaping () -> Void
    ) {
        _ = center
        guard let payload = DiskPressureNotificationPayload(
            userInfo: response.notification.request.content.userInfo
        ) else {
            completionHandler()
            return
        }
        Task { @MainActor [weak self] in
            await self?.runtime.handleUrgentRecommendations(payload)
        }
        completionHandler()
    }
}

extension AppRuntime: DuxAppRuntimeServing {}
