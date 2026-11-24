import Foundation
import ServiceManagement

/// What to do when the app is registered to open at login but turned off in System Settings.
let loginItemApproval = "Launcher is in System Settings > General > Login Items, but turned off. Turn it on there to open Launcher at login."

/// Registers or unregisters the app as a login item. Registered but awaiting approval counts as
/// on, since registering again can't approve it; only the user can, in System Settings.
func setLaunchAtLogin(_ on: Bool) throws {
    let app = SMAppService.mainApp
    let registered = [.enabled, .requiresApproval].contains(app.status)
    if on, !registered {
        try app.register()
    } else if !on, registered {
        try app.unregister()
    }
}

/// Runs `login on|off|status`, which reaches launch at login without the menu bar item: `nil`
/// when `arguments` aren't a `login` command, so the UI starts; else the exit status. It's the
/// shell's rather than `launcher_config::run_command`'s, since login items are macOS's.
func loginCommand(_ arguments: [String]) -> Int32? {
    guard arguments.first == "login" else { return nil }
    let on: Bool?
    switch arguments.dropFirst() {
    case ["on"]: on = true
    case ["off"]: on = false
    case ["status"]: on = nil
    default:
        log("usage: login on|off|status\n  opens \(Bundle.main.bundlePath) at login, or says whether it does")
        return 64
    }
    if let on {
        do {
            try setLaunchAtLogin(on)
        } catch {
            log("can't turn \(on ? "on" : "off") launch at login: \(error.localizedDescription)")
            return 1
        }
    }
    let status = SMAppService.mainApp.status
    let description = switch status {
    case .enabled: "enabled"
    case .notRegistered: "not registered"
    case .requiresApproval: "requires approval\n\(loginItemApproval)"
    case .notFound: "not found"
    @unknown default: "unknown (\(status.rawValue))"
    }
    print(description)
    return switch on {
    case true?: status == .enabled ? 0 : 1
    case false?: [.enabled, .requiresApproval].contains(status) ? 1 : 0
    case nil: 0
    }
}
