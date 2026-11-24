import Foundation
import ServiceManagement

/// What to do when the app is registered to open at login but turned off in System Settings.
let loginItemApproval = "Launcher is in System Settings > General > Login Items, but turned off. Turn it on there to open Launcher at login."

/// Runs `login on|off|status`, which reaches launch at login without the menu bar item: `nil`
/// when `arguments` aren't a `login` command, so the UI starts; else the exit status. It's the
/// shell's rather than `launcher_config::run_command`'s, since login items are macOS's.
func loginCommand(_ arguments: [String]) -> Int32? {
    guard arguments.first == "login" else { return nil }
    let app = SMAppService.mainApp
    let command = arguments.dropFirst()
    do {
        switch command {
        case ["status"]:
            break
        case ["on"] where app.status != .enabled:
            try app.register()
        case ["off"] where [.enabled, .requiresApproval].contains(app.status):
            try app.unregister()
        case ["on"], ["off"]:
            break
        default:
            log("usage: login on|off|status\n  opens \(Bundle.main.bundlePath) at login, or says whether it does")
            return 64
        }
    } catch {
        log("can't turn \(command.first ?? "") launch at login: \(error.localizedDescription)")
        return 1
    }
    let status = app.status
    let description = switch status {
    case .enabled: "enabled"
    case .notRegistered: "not registered"
    case .requiresApproval: "requires approval\n\(loginItemApproval)"
    case .notFound: "not found"
    @unknown default: "unknown (\(status.rawValue))"
    }
    print(description)
    return switch command {
    case ["on"]: status == .enabled ? 0 : 1
    case ["off"]: [.notRegistered, .notFound].contains(status) ? 0 : 1
    default: 0
    }
}
