import AppKit
import Carbon.HIToolbox

let started = CFAbsoluteTimeGetCurrent()

/// Borderless panels refuse key status by default, and the query field needs it.
final class Panel: NSPanel {
    override var canBecomeKey: Bool { true }
}

final class Row: NSView {
    var clicked: () -> Void = {}
    var hovered: () -> Void = {}

    override init(frame: NSRect) {
        super.init(frame: frame)
        // `activeAlways`: the panel never makes the app active, so other tracking modes stay off.
        // `mouseMoved` rather than entered: rows rebuilt under a resting pointer mustn't take the
        // selection from the top result.
        addTrackingArea(NSTrackingArea(
            rect: .zero, options: [.mouseMoved, .activeAlways, .inVisibleRect], owner: self
        ))
    }

    required init?(coder: NSCoder) {
        fatalError("rows are built in code")
    }

    override func mouseMoved(with _: NSEvent) {
        hovered()
    }

    // Clicks on the title, subtitle or icon belong to the row.
    override func hitTest(_ point: NSPoint) -> NSView? {
        frame.contains(point) ? self : nil
    }

    // AppKit sends `mouseUp` only to the view that took `mouseDown`.
    override func mouseDown(with _: NSEvent) {}

    // Like a button: dragging off the row before letting go cancels the click.
    override func mouseUp(with event: NSEvent) {
        if bounds.contains(convert(event.locationInWindow, from: nil)) { clicked() }
    }
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
    var picked = 0 {
        didSet { highlightPicked() }
    }
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
        field.placeholderString = "Search apps, calculate, convert…"
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
        layout()
        picked = 0
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
            let row = Row(frame: NSRect(
                x: 6, y: listHeight - 6 - CGFloat(index + 1) * (rowHeight + 2), width: 628, height: rowHeight
            ))
            row.clicked = { [unowned self] in choose(index) }
            row.hovered = { [unowned self] in if picked != index { picked = index } }
            row.wantsLayer = true
            // Concentric with the glass: its 18 pt radius less the row's 6 pt inset.
            row.layer?.cornerRadius = 12
            var titleX: CGFloat = 12
            if case let .open(path) = item.action {
                let icon = NSImageView(image: NSWorkspace.shared.icon(forFile: path))
                icon.frame = NSRect(x: 10, y: 8, width: 28, height: 28)
                row.addSubview(icon)
                titleX = 46
            }
            let title = NSTextField(labelWithString: item.title)
            title.font = .systemFont(ofSize: 17, weight: .semibold)
            title.frame = NSRect(x: titleX, y: 11, width: 392 - titleX, height: 22)
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

    /// Moving the selection only recolours rows; rebuilding them would fetch every icon again.
    func highlightPicked() {
        for (index, row) in results.subviews.enumerated() {
            row.layer?.backgroundColor = index == picked ? NSColor.controlAccentColor.cgColor : nil
        }
    }

    func choose(_ index: Int) {
        if index < items.count { run(items[index]) }
        field.stringValue = ""
        refresh()
        panel.orderOut(nil)
    }

    func run(_ item: Item) {
        switch item.action {
        case let .copy(text):
            NSPasteboard.general.clearContents()
            if NSPasteboard.general.setString(text, forType: .string) {
                launcher.ran(choice: item.choice)
            } else {
                log("can't copy \(text)")
            }
        case let .open(path):
            NSWorkspace.shared.open(
                URL(fileURLWithPath: path), configuration: NSWorkspace.OpenConfiguration()
            ) { _, error in
                let failure = error?.localizedDescription
                Task { @MainActor in
                    guard let failure else {
                        self.launcher.ran(choice: item.choice)
                        return
                    }
                    log("can't open \(path): \(failure)")
                    // The panel has already closed, so nothing else would show the failure.
                    let alert = NSAlert()
                    alert.messageText = "Can't open \(FileManager.default.displayName(atPath: path))"
                    alert.informativeText = failure
                    NSApp.activate()
                    alert.runModal()
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
            choose(picked)
            return true
        case #selector(NSResponder.cancelOperation(_:)):
            panel.orderOut(nil)
            return true
        default:
            return false
        }
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
