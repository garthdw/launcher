import AppKit
import Carbon.HIToolbox

let started = CFAbsoluteTimeGetCurrent()

/// Borderless panels refuse key status by default, and the query field needs it.
final class Panel: NSPanel {
    override var canBecomeKey: Bool { true }
}

@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate, NSTextFieldDelegate, NSWindowDelegate {
    let launcher = Launcher()
    // Non-activating, so the app the user was in stays frontmost: window snapping acts on it.
    let panel = Panel(
        contentRect: NSRect(x: 0, y: 0, width: 640, height: 56),
        styleMask: [.borderless, .nonactivatingPanel],
        backing: .buffered,
        defer: false
    )
    let field = NSTextField()
    let results = NSView()
    var items: [Item] = []
    var picked = 0
    var summoned: CFAbsoluteTime?
    var hotkey: EventHotKeyRef?

    func applicationDidFinishLaunching(_: Notification) {
        prefetchExchangeRates()
        buildPanel()
        registerHotkey()
        // Opens the panel at launch with this query, for measuring without the hotkey.
        if let query = ProcessInfo.processInfo.environment["LAUNCHER_OPEN"] {
            field.stringValue = query
            refresh()
            summon(at: started)
        }
    }

    func buildPanel() {
        panel.isFloatingPanel = true
        panel.level = .floating
        panel.backgroundColor = .clear
        panel.isOpaque = false
        panel.hasShadow = true
        panel.delegate = self
        let glass = NSGlassEffectView()
        glass.cornerRadius = 18
        glass.autoresizingMask = [.width, .height]
        // Only the glass's own `contentView` is guaranteed to sit inside the effect.
        let content = NSView()
        content.autoresizingMask = [.width, .height]
        glass.contentView = content
        panel.contentView = glass

        field.isBordered = false
        field.drawsBackground = false
        field.focusRingType = .none
        field.font = .systemFont(ofSize: 20)
        field.placeholderString = "Calculate, convert…"
        field.delegate = self
        field.autoresizingMask = [.width, .minYMargin]
        results.autoresizingMask = [.width, .height]
        content.addSubview(field)
        content.addSubview(results)
        layout()
    }

    func registerHotkey() {
        var pressed = EventTypeSpec(eventClass: OSType(kEventClassKeyboard), eventKind: UInt32(kEventHotKeyPressed))
        let delegate = Unmanaged.passUnretained(self).toOpaque()
        InstallEventHandler(GetApplicationEventTarget(), { _, _, context in
            guard let context else { return OSStatus(eventNotHandledErr) }
            let delegate = Unmanaged<AppDelegate>.fromOpaque(context).takeUnretainedValue()
            MainActor.assumeIsolated { delegate.summon(at: CFAbsoluteTimeGetCurrent()) }
            return noErr
        }, 1, &pressed, delegate, nil)
        let status = RegisterEventHotKey(
            UInt32(kVK_Space), UInt32(optionKey), EventHotKeyID(signature: 0x4C4E_4348, id: 1),
            GetApplicationEventTarget(), 0, &hotkey
        )
        if status != noErr { log("can't register ⌥Space: \(status)") }
    }

    func summon(at time: CFAbsoluteTime) {
        summoned = time
        if let screen = NSScreen.main {
            let frame = screen.visibleFrame
            panel.setFrameTopLeftPoint(NSPoint(x: frame.midX - panel.frame.width / 2, y: frame.maxY - 180))
        }
        panel.makeKeyAndOrderFront(nil)
        panel.makeFirstResponder(field)
    }

    func windowDidBecomeKey(_: Notification) {
        guard let summoned else { return }
        self.summoned = nil
        log(String(format: "summon to focus: %.1f ms", (CFAbsoluteTimeGetCurrent() - summoned) * 1000))
    }

    func windowDidResignKey(_: Notification) {
        panel.orderOut(nil)
    }

    func refresh() {
        items = launcher.query(query: field.stringValue)
        picked = 0
        layout()
    }

    func layout() {
        let rowHeight: CGFloat = 44
        let listHeight = items.isEmpty ? 0 : CGFloat(items.count) * (rowHeight + 2) + 12
        let height = 56 + listHeight
        var frame = panel.frame
        frame.origin.y += frame.height - height
        frame.size.height = height
        panel.setFrame(frame, display: true)
        field.frame = NSRect(x: 18, y: height - 56 + 16, width: 640 - 36, height: 26)
        results.frame = NSRect(x: 0, y: 0, width: 640, height: listHeight)
        results.subviews.forEach { $0.removeFromSuperview() }
        for (index, item) in items.enumerated() {
            let row = NSView(frame: NSRect(
                x: 6, y: listHeight - 6 - CGFloat(index + 1) * (rowHeight + 2), width: 628, height: rowHeight
            ))
            row.wantsLayer = true
            // Concentric with the glass: its 18 pt radius less the row's 6 pt inset.
            row.layer?.cornerRadius = 12
            row.layer?.backgroundColor = index == picked ? NSColor.controlAccentColor.cgColor : nil
            let title = NSTextField(labelWithString: item.title)
            title.font = .systemFont(ofSize: 17, weight: .semibold)
            title.frame = NSRect(x: 12, y: 11, width: 380, height: 22)
            let subtitle = NSTextField(labelWithString: item.subtitle)
            subtitle.font = .systemFont(ofSize: 13)
            subtitle.textColor = .secondaryLabelColor
            subtitle.alignment = .right
            subtitle.frame = NSRect(x: 396, y: 13, width: 220, height: 18)
            row.addSubview(title)
            row.addSubview(subtitle)
            results.addSubview(row)
        }
    }

    func run(_ item: Item) {
        switch item.action {
        case let .copy(text):
            NSPasteboard.general.clearContents()
            NSPasteboard.general.setString(text, forType: .string)
        case let .open(path):
            NSWorkspace.shared.openApplication(
                at: URL(fileURLWithPath: path), configuration: NSWorkspace.OpenConfiguration()
            ) { _, error in
                if let error {
                    log("can't open \(path): \(error.localizedDescription)")
                } else {
                    Task { @MainActor in self.launcher.opened(path: path) }
                }
            }
        }
    }

    func controlTextDidChange(_: Notification) {
        refresh()
    }

    func control(_: NSControl, textView _: NSTextView, doCommandBy selector: Selector) -> Bool {
        switch selector {
        case #selector(NSResponder.moveDown(_:)) where !items.isEmpty:
            picked = (picked + 1) % items.count
        case #selector(NSResponder.moveUp(_:)) where !items.isEmpty:
            picked = (picked + items.count - 1) % items.count
        case #selector(NSResponder.insertNewline(_:)):
            if picked < items.count { run(items[picked]) }
            field.stringValue = ""
            refresh()
            panel.orderOut(nil)
            return true
        case #selector(NSResponder.cancelOperation(_:)):
            panel.orderOut(nil)
            return true
        default:
            return false
        }
        layout()
        return true
    }
}

func log(_ message: String) {
    FileHandle.standardError.write(Data((message + "\n").utf8))
}

MainActor.assumeIsolated {
    let app = NSApplication.shared
    app.setActivationPolicy(.accessory)
    let delegate = AppDelegate()
    app.delegate = delegate
    app.run()
}
