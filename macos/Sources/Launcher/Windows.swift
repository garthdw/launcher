import AppKit
import ApplicationServices

/// Lays out the frontmost app's focused window through the Accessibility API. Where each action
/// puts it comes from the Rust core, shared with other shells; this only reads and moves windows.
@MainActor
final class Windows {
    /// Each window's frame from before its first layout since the last restore, for `.restore`.
    /// Kept even if the user moves the window in between: telling their move from an app adjusting
    /// the frame it was given (to a minimum size, a terminal's cell grid, late) isn't reliable.
    private var originals: [AXUIElement: CGRect] = [:]
    private var askedForPermission = false
    private var explaining = false

    func perform(_ action: WindowAction) {
        // Frontmost only while showing an alert, which isn't for moving.
        guard isTrusted(), let app = NSWorkspace.shared.frontmostApplication, app != .current else { return }
        let element = AXUIElementCreateApplication(app.processIdentifier)
        // Hotkeys run on the main thread: a hung app mustn't stall the launcher for AX's default 6 s.
        // Each element has its own timeout, so the window gets it too.
        AXUIElementSetMessagingTimeout(element, 1)
        let name = app.localizedName ?? "the frontmost app"
        guard let window = element.focusedWindow else {
            log("\(name) has no focused window to move")
            return
        }
        AXUIElementSetMessagingTimeout(window, 1)
        // Sheets, dialogs and palettes belong where their app put them.
        guard window.isStandard, let frame = window.frame else {
            log("\(name)'s focused window isn't one to move")
            return
        }
        if window.isFullScreen {
            log("leaving \(name)'s full-screen window alone")
            return
        }
        if action == .restore {
            guard let original = originals.removeValue(forKey: window) else { return }
            window.setFrame(original)
            return
        }
        let displays = NSScreen.screens.map {
            Display(frame: Rect(axFrame($0.frame)), visible: Rect(axFrame($0.visibleFrame)))
        }
        guard let placement = windowPlacement(action: action, window: Rect(frame), displays: displays)
        else { return }
        if originals[window] == nil { originals[window] = frame }
        window.setFrame(CGRect(placement.frame))
        // A window whose minimum size is larger than the target hangs off the display's edge. Apps
        // that resize asynchronously (Electron, Chrome) still report the old frame, so leave those.
        if let placed = window.frame, placed != frame {
            let within = CGRect(windowMovedWithin(frame: Rect(placed), visible: placement.visible))
            if within.origin != placed.origin { window.setPosition(within.origin) }
        }
    }

    /// Asks for the Accessibility permission the first time it's missing, then explains how to
    /// grant it.
    private func isTrusted() -> Bool {
        if AXIsProcessTrusted() { return true }
        log("no Accessibility permission to move windows")
        guard askedForPermission else {
            askedForPermission = true
            // Lists Launcher, switched off, under Accessibility and shows the system's prompt. The
            // key is `kAXTrustedCheckOptionPrompt`'s value: Swift 6 rejects reading that global var.
            AXIsProcessTrustedWithOptions(["AXTrustedCheckOptionPrompt": true] as CFDictionary)
            return false
        }
        // Hotkeys still arrive while the alert is up; one alert is enough.
        guard !explaining else { return false }
        explaining = true
        defer { explaining = false }
        let response = showAlert(
            "Launcher can't move windows",
            "Turn Launcher on in System Settings › Privacy & Security › Accessibility, then press the shortcut again.",
            buttons: ["Open System Settings", "Cancel"]
        )
        if response == .alertFirstButtonReturn,
           let settings = URL(string: "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility") {
            NSWorkspace.shared.open(settings)
        }
        return false
    }
}

/// Converts between AppKit's global coordinates, with the origin at the primary display's bottom
/// left, and AX's, at its top left. The flip is its own inverse, so it works both ways.
@MainActor
func axFrame(_ frame: CGRect) -> CGRect {
    let primaryHeight = NSScreen.screens.first?.frame.height ?? 0
    return CGRect(x: frame.minX, y: primaryHeight - frame.maxY, width: frame.width, height: frame.height)
}

extension Rect {
    init(_ rect: CGRect) {
        self.init(x: rect.minX, y: rect.minY, width: rect.width, height: rect.height)
    }
}

extension CGRect {
    init(_ rect: Rect) {
        self.init(x: rect.x, y: rect.y, width: rect.width, height: rect.height)
    }
}

extension AXUIElement {
    var focusedWindow: AXUIElement? {
        guard let value = copyValue(kAXFocusedWindowAttribute), CFGetTypeID(value) == AXUIElementGetTypeID()
        else { return nil }
        return (value as! AXUIElement)
    }

    var isStandard: Bool {
        copyValue(kAXSubroleAttribute) as? String == kAXStandardWindowSubrole
    }

    var isFullScreen: Bool {
        copyValue("AXFullScreen") as? Bool == true
    }

    /// In AX coordinates.
    var frame: CGRect? {
        guard let origin = value(kAXPositionAttribute, .cgPoint, CGPoint.zero),
              let size = value(kAXSizeAttribute, .cgSize, CGSize.zero)
        else { return nil }
        return CGRect(origin: origin, size: size)
    }

    /// Resizes, moves, then resizes again: the first resize lets a shrinking window fit at its new
    /// position, and the second applies what the first couldn't where the window was, since apps
    /// keep their windows on screen.
    func setFrame(_ frame: CGRect) {
        set(kAXSizeAttribute, .cgSize, frame.size)
        setPosition(frame.origin)
        set(kAXSizeAttribute, .cgSize, frame.size)
    }

    func setPosition(_ origin: CGPoint) {
        set(kAXPositionAttribute, .cgPoint, origin)
    }

    private func copyValue(_ attribute: String) -> CFTypeRef? {
        var value: CFTypeRef?
        return AXUIElementCopyAttributeValue(self, attribute as CFString, &value) == .success ? value : nil
    }

    private func value<T: BitwiseCopyable>(_ attribute: String, _ type: AXValueType, _ empty: T) -> T? {
        guard let value = copyValue(attribute), CFGetTypeID(value) == AXValueGetTypeID() else { return nil }
        var result = empty
        return AXValueGetValue(value as! AXValue, type, &result) ? result : nil
    }

    private func set<T: BitwiseCopyable>(_ attribute: String, _ type: AXValueType, _ value: T) {
        var value = value
        guard let wrapped = AXValueCreate(type, &value) else { return }
        let error = AXUIElementSetAttributeValue(self, attribute as CFString, wrapped)
        if error != .success { log("can't set \(attribute): AXError \(error.rawValue)") }
    }
}
