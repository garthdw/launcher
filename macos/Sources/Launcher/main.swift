import AppKit
import Carbon.HIToolbox

let started = CFAbsoluteTimeGetCurrent()

/// Geometry shared by the list and the detail pane.
enum Metrics {
    static let width: CGFloat = 640
    static let headerHeight: CGFloat = 56
    static let glassRadius: CGFloat = 18
    /// How far rows and the detail's highlight sit inside the panel's edges.
    static let inset: CGFloat = 6
    /// Concentric with the glass: its radius less the inset.
    static let selectionRadius = glassRadius - inset
}

/// Borderless panels refuse key status by default, and the query field needs it.
final class Panel: NSPanel {
    override var canBecomeKey: Bool { true }

    /// The app has no menu bar to route editing shortcuts, so the panel sends them itself.
    override func performKeyEquivalent(with event: NSEvent) -> Bool {
        guard event.modifierFlags.intersection(.deviceIndependentFlagsMask).subtracting(.capsLock) == .command
        else { return super.performKeyEquivalent(with: event) }
        let action: Selector? = switch event.charactersIgnoringModifiers?.lowercased() {
        case "c": #selector(NSText.copy(_:))
        case "v": #selector(NSText.paste(_:))
        case "x": #selector(NSText.cut(_:))
        case "a": #selector(NSText.selectAll(_:))
        default: nil
        }
        guard let action else { return super.performKeyEquivalent(with: event) }
        return NSApp.sendAction(action, to: nil, from: self)
    }
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

/// An item's detail in place of the query field and list: a header, then its sections, scrolling
/// once they pass `maxBodyHeight`.
final class DetailPane: NSView {
    let text: DetailText
    let bodyHeight: CGFloat

    init(item: Item) {
        let maxBodyHeight: CGFloat = 420
        text = DetailText(sections: item.detail)
        let contentHeight = text.contentHeight
        bodyHeight = min(contentHeight, maxBodyHeight)
        super.init(frame: NSRect(x: 0, y: 0, width: Metrics.width, height: Metrics.headerHeight + bodyHeight))

        let title = titleLabel(item.title)
        title.lineBreakMode = .byTruncatingTail
        title.frame = NSRect(x: 18, y: bodyHeight + 17, width: 380, height: 22)
        title.autoresizingMask = [.minYMargin]
        let hint = hintLabel("⌘C copies · esc goes back")
        hint.frame = NSRect(x: 402, y: bodyHeight + 19, width: 220, height: 18)
        hint.autoresizingMask = [.minYMargin]

        let scroll = NSScrollView(frame: NSRect(x: 0, y: 0, width: Metrics.width, height: bodyHeight))
        scroll.autoresizingMask = [.width, .height]
        scroll.drawsBackground = false
        scroll.hasVerticalScroller = true
        scroll.autohidesScrollers = true
        let document = FlippedView(frame: NSRect(x: 0, y: 0, width: Metrics.width, height: contentHeight))
        document.autoresizingMask = [.width]
        text.frame = document.bounds
        text.autoresizingMask = [.width]
        document.addSubview(text.highlight)
        document.addSubview(text)
        scroll.documentView = document
        addSubview(title)
        addSubview(hint)
        addSubview(scroll)
        text.placeHighlight()
    }

    required init?(coder: NSCoder) {
        fatalError("built in code")
    }
}

/// Top-down, like the text view it holds, so a long detail opens scrolled to its start.
final class FlippedView: NSView {
    override var isFlipped: Bool { true }
}

/// Read-only and selectable, so the mouse can copy any part; ↑ and ↓ step through the sections
/// for ⌘C to copy whole.
final class DetailText: NSTextView {
    var back: () -> Void = {}
    var run: () -> Void = {}
    /// Each section's span in the text, and what ⌘C copies when it's picked.
    private var sections: [(range: NSRange, text: String)] = []
    private var picked = 0 {
        didSet {
            // Otherwise ⌘C would copy an earlier mouse selection instead of the section just picked.
            setSelectedRange(NSRange(location: sections[picked].range.location, length: 0))
            placeHighlight()
            scrollRangeToVisible(sections[picked].range)
        }
    }

    /// The picked section's background: a layer's colour, not drawn by this view. Drawing in the
    /// text view gives it a backing store and Metal state, about 8 MB while the detail shows.
    let highlight: NSView = {
        let highlight = NSView()
        highlight.wantsLayer = true
        // Translucent, unlike the list's solid highlight, so the section's text stays readable.
        highlight.layer?.backgroundColor = NSColor.controlAccentColor.withAlphaComponent(0.25).cgColor
        highlight.layer?.cornerRadius = Metrics.selectionRadius
        return highlight
    }()

    convenience init(sections: [Section]) {
        // TextKit 1, for the glyph rects `placeHighlight` reads.
        self.init(usingTextLayoutManager: false)
        isEditable = false
        drawsBackground = false
        isVerticallyResizable = true
        maxSize = NSSize(width: CGFloat.greatestFiniteMagnitude, height: CGFloat.greatestFiniteMagnitude)
        textContainerInset = NSSize(width: 14, height: 12)
        // The container tracks the view's width, which starts at zero: without the panel's width
        // before the first layout, nothing lays out and `contentHeight` is only the inset.
        frame.size.width = Metrics.width
        textContainer?.widthTracksTextView = true
        let heading: [NSAttributedString.Key: Any] = [
            .font: NSFont.systemFont(ofSize: 11, weight: .semibold),
            .foregroundColor: NSColor.secondaryLabelColor,
        ]
        let body: [NSAttributedString.Key: Any] = [
            .font: NSFont.monospacedSystemFont(ofSize: 13, weight: .regular),
            .foregroundColor: NSColor.labelColor,
        ]
        let string = NSMutableAttributedString()
        self.sections = sections.enumerated().map { index, section in
            if index > 0 { string.append(NSAttributedString(string: "\n\n", attributes: body)) }
            let start = string.length
            string.append(NSAttributedString(string: section.title.uppercased() + "\n", attributes: heading))
            string.append(NSAttributedString(string: section.text, attributes: body))
            return (NSRange(location: start, length: string.length - start), section.text)
        }
        textStorage?.setAttributedString(string)
    }

    var contentHeight: CGFloat {
        guard let layoutManager, let textContainer else { return 0 }
        layoutManager.ensureLayout(for: textContainer)
        return layoutManager.usedRect(for: textContainer).height + textContainerInset.height * 2
    }

    override func doCommand(by selector: Selector) {
        switch selector {
        case #selector(cancelOperation(_:)), #selector(moveLeft(_:)): back()
        case #selector(insertNewline(_:)): run()
        case #selector(moveDown(_:)) where !sections.isEmpty: picked = step(picked, by: 1, count: sections.count)
        case #selector(moveUp(_:)) where !sections.isEmpty: picked = step(picked, by: -1, count: sections.count)
        default: super.doCommand(by: selector)
        }
    }

    /// Copies the mouse's selection, or else the picked section. Plain text either way: rich text
    /// would carry the monospaced font into whatever it's pasted into.
    override func copy(_: Any?) {
        let selected = selectedRange()
        if selected.length > 0 {
            copyToPasteboard((string as NSString).substring(with: selected))
        } else if sections.indices.contains(picked) {
            copyToPasteboard(sections[picked].text)
        }
    }

    /// Moves `highlight`, a sibling behind this view, onto the picked section.
    func placeHighlight() {
        guard let layoutManager, let textContainer, let container = highlight.superview,
              sections.indices.contains(picked)
        else { return }
        let glyphs = layoutManager.glyphRange(forCharacterRange: sections[picked].range, actualCharacterRange: nil)
        let box = layoutManager.boundingRect(forGlyphRange: glyphs, in: textContainer)
        highlight.frame = convert(NSRect(
            x: Metrics.inset, y: box.minY + textContainerOrigin.y - Metrics.inset,
            width: bounds.width - Metrics.inset * 2, height: box.height + Metrics.inset * 2
        ), to: container)
    }
}

@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate, NSTextFieldDelegate, NSWindowDelegate {
    let launcher = Launcher()
    // Non-activating, so the app the user was in stays frontmost: window snapping acts on it.
    let panel = Panel(
        contentRect: NSRect(x: 0, y: 0, width: Metrics.width, height: Metrics.headerHeight),
        styleMask: [.borderless, .nonactivatingPanel],
        backing: .buffered,
        defer: false
    )
    let content = NSView()
    let field = NSTextField()
    let results = NSView()
    /// Shown in place of the field and list while an item's detail is open.
    var detail: DetailPane?
    var items: [Item] = []
    var picked = 0 {
        didSet { highlightPicked() }
    }
    var summoned: CFAbsoluteTime?
    var hotkey: EventHotKeyRef?

    func applicationDidFinishLaunching(_: Notification) {
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
        glass.cornerRadius = Metrics.glassRadius
        glass.autoresizingMask = [.width, .height]
        // Only the glass's own `contentView` is guaranteed to sit inside the effect.
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
        // A fixed height: `resize` reads it as the list's, so the panel growing mustn't stretch it.
        results.autoresizingMask = [.width]
        content.addSubview(field)
        content.addSubview(results)
        resize()
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
        // ⌥Space while the panel is still key comes here without hiding it first.
        closeDetail()
        if let screen = NSScreen.main {
            let frame = screen.visibleFrame
            panel.setFrameTopLeftPoint(NSPoint(x: frame.midX - panel.frame.width / 2, y: frame.maxY - 180))
        }
        panel.makeKeyAndOrderFront(nil)
        panel.makeFirstResponder(field)
    }

    func hide() {
        closeDetail()
        panel.orderOut(nil)
    }

    func windowDidBecomeKey(_: Notification) {
        guard let summoned else { return }
        self.summoned = nil
        log(String(format: "summon to focus: %.1f ms", (CFAbsoluteTimeGetCurrent() - summoned) * 1000))
    }

    func windowDidResignKey(_: Notification) {
        hide()
    }

    func refresh() {
        items = launcher.query(query: field.stringValue)
        let rowHeight: CGFloat = 44
        let listHeight = items.isEmpty ? 0 : CGFloat(items.count) * (rowHeight + 2) + Metrics.inset * 2
        results.frame = NSRect(x: 0, y: 0, width: Metrics.width, height: listHeight)
        results.subviews.forEach { $0.removeFromSuperview() }
        for (index, item) in items.enumerated() {
            let row = Row(frame: NSRect(
                x: Metrics.inset, y: listHeight - Metrics.inset - CGFloat(index + 1) * (rowHeight + 2),
                width: Metrics.width - Metrics.inset * 2, height: rowHeight
            ))
            row.clicked = { [unowned self] in choose(index) }
            row.hovered = { [unowned self] in if picked != index { picked = index } }
            row.wantsLayer = true
            row.layer?.cornerRadius = Metrics.selectionRadius
            var titleX: CGFloat = 12
            if case let .open(path) = item.action {
                let icon = NSImageView(image: NSWorkspace.shared.icon(forFile: path))
                icon.frame = NSRect(x: 10, y: 8, width: 28, height: 28)
                row.addSubview(icon)
                titleX = 46
            }
            let title = titleLabel(item.title)
            // The shell names its own keys, so plugins' subtitles say only what an item is.
            let keys = switch item.action {
            case .copy: "↵ copies"
            case .open: "↵ opens"
            }
            let subtitle = hintLabel(
                ([item.subtitle, keys] + (item.detail.isEmpty ? [] : ["→ details"]))
                    .filter { !$0.isEmpty }
                    .joined(separator: " · ")
            )
            // Long subtitles such as app folders lose their start, not the keys. A subtitle may
            // take room a short title leaves, but never squeezes a long title further.
            subtitle.lineBreakMode = .byTruncatingHead
            let end = row.frame.width - 12
            let subtitleX = min(396, max(titleX + title.fittingSize.width + 4, end - subtitle.fittingSize.width))
            subtitle.frame = NSRect(x: subtitleX, y: 13, width: end - subtitleX, height: 18)
            title.frame = NSRect(x: titleX, y: 11, width: subtitleX - 4 - titleX, height: 22)
            row.addSubview(title)
            row.addSubview(subtitle)
            results.addSubview(row)
        }
        resize()
        picked = 0
    }

    /// Fits the panel to what it shows, keeping its top edge where it is.
    func resize() {
        let height = Metrics.headerHeight + (detail?.bodyHeight ?? results.frame.height)
        var frame = panel.frame
        frame.origin.y += frame.height - height
        frame.size.height = height
        panel.setFrame(frame, display: true)
        field.frame = NSRect(x: 18, y: height - Metrics.headerHeight + 16, width: Metrics.width - 36, height: 26)
        field.isHidden = detail != nil
        results.isHidden = detail != nil
        detail?.frame = NSRect(x: 0, y: 0, width: Metrics.width, height: height)
    }

    /// Moving the selection only recolours rows; rebuilding them would fetch every icon again.
    func highlightPicked() {
        for (index, row) in results.subviews.enumerated() {
            row.layer?.backgroundColor = index == picked ? NSColor.controlAccentColor.cgColor : nil
        }
    }

    func showDetail(of index: Int) {
        let pane = DetailPane(item: items[index])
        pane.text.back = { [unowned self] in
            closeDetail()
            panel.makeFirstResponder(field)
            field.currentEditor()?.moveToEndOfDocument(nil)
        }
        // The shown item, not `picked`: hidden rows under the pane may still track the pointer.
        pane.text.run = { [unowned self] in choose(index) }
        detail = pane
        content.addSubview(pane)
        resize()
        panel.makeFirstResponder(pane.text)
    }

    /// The pane is rebuilt each time it opens, so nothing stays resident once it closes.
    func closeDetail() {
        guard let detail else { return }
        detail.removeFromSuperview()
        self.detail = nil
        resize()
    }

    func choose(_ index: Int) {
        if index < items.count { run(items[index]) }
        field.stringValue = ""
        refresh()
        hide()
    }

    func run(_ item: Item) {
        switch item.action {
        case let .copy(text):
            if copyToPasteboard(text) { launcher.ran(choice: item.choice) }
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
                    let previous = NSWorkspace.shared.frontmostApplication
                    NSApp.activate()
                    alert.runModal()
                    // Window management acts on the frontmost app, so hand focus back.
                    previous?.activate()
                }
            }
        }
    }

    func controlTextDidChange(_: Notification) {
        refresh()
    }

    func control(_: NSControl, textView: NSTextView, doCommandBy selector: Selector) -> Bool {
        switch selector {
        case #selector(NSResponder.moveDown(_:)) where !items.isEmpty:
            picked = step(picked, by: 1, count: items.count)
        case #selector(NSResponder.moveUp(_:)) where !items.isEmpty:
            picked = step(picked, by: -1, count: items.count)
        // Only from the end of the query, so → still moves the caret elsewhere.
        case #selector(NSResponder.moveRight(_:))
            where textView.selectedRange() == NSRange(location: (textView.string as NSString).length, length: 0)
            && items.indices.contains(picked) && !items[picked].detail.isEmpty:
            showDetail(of: picked)
        case #selector(NSResponder.insertNewline(_:)):
            choose(picked)
            return true
        case #selector(NSResponder.cancelOperation(_:)):
            hide()
            return true
        default:
            return false
        }
        return true
    }
}

/// Moves a selection through `count` entries, wrapping at either end.
func step(_ index: Int, by offset: Int, count: Int) -> Int {
    (index + offset + count) % count
}

@MainActor
func titleLabel(_ string: String) -> NSTextField {
    let label = NSTextField(labelWithString: string)
    label.font = .systemFont(ofSize: 17, weight: .semibold)
    return label
}

@MainActor
func hintLabel(_ string: String) -> NSTextField {
    let label = NSTextField(labelWithString: string)
    label.font = .systemFont(ofSize: 13)
    label.textColor = .secondaryLabelColor
    label.alignment = .right
    return label
}

@discardableResult
func copyToPasteboard(_ text: String) -> Bool {
    NSPasteboard.general.clearContents()
    if NSPasteboard.general.setString(text, forType: .string) { return true }
    log("can't copy \(text)")
    return false
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
