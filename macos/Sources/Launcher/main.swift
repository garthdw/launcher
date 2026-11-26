import AppKit
import Carbon.HIToolbox
import PanelLayout
import ServiceManagement

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
    /// A row's icon, square.
    static let icon: CGFloat = 28
}

/// Borderless panels refuse key status by default, and the query field needs it.
final class Panel: NSPanel {
    override var canBecomeKey: Bool { true }

    /// The app has no main menu to route shortcuts, so the panel sends them itself: editing, ⌘Q,
    /// ⌘, and ⌘R.
    override func performKeyEquivalent(with event: NSEvent) -> Bool {
        guard event.modifierFlags.intersection(.deviceIndependentFlagsMask).subtracting(.capsLock) == .command
        else { return super.performKeyEquivalent(with: event) }
        let action: Selector? = switch event.charactersIgnoringModifiers?.lowercased() {
        case "c": #selector(NSText.copy(_:))
        case "v": #selector(NSText.paste(_:))
        case "x": #selector(NSText.cut(_:))
        case "a": #selector(NSText.selectAll(_:))
        case "q": #selector(NSApplication.terminate(_:))
        case ",": #selector(AppDelegate.openSettingsFile)
        case "r": #selector(AppDelegate.reloadSettings)
        default: nil
        }
        guard let action else { return super.performKeyEquivalent(with: event) }
        return NSApp.sendAction(action, to: nil, from: self)
    }
}

final class Row: NSView {
    var clicked: () -> Void = {}
    var hovered: () -> Void = {}
    /// What scrolling to the row brings into view: the row with room around it, and its group's
    /// header when it's the group's first row.
    var reach = NSRect.zero

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

    // Hidden rows, as under an open detail, keep their tracking areas and still hear the pointer.
    override func mouseMoved(with _: NSEvent) {
        if !isHiddenOrHasHiddenAncestor { hovered() }
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

        let document = FlippedView(frame: NSRect(x: 0, y: 0, width: Metrics.width, height: contentHeight))
        document.autoresizingMask = [.width]
        text.frame = document.bounds
        text.autoresizingMask = [.width]
        document.addSubview(text.highlight)
        document.addSubview(text)
        let scroll = scrollView(holding: document)
        scroll.frame = NSRect(x: 0, y: 0, width: Metrics.width, height: bodyHeight)
        scroll.autoresizingMask = [.width, .height]
        addSubview(title)
        addSubview(hint)
        addSubview(scroll)
        text.placeHighlight()
    }

    required init?(coder: NSCoder) {
        fatalError("built in code")
    }
}

/// Top-down, so scrolled content opens at its start, and a scroll view's document grows down.
final class FlippedView: NSView {
    override var isFlipped: Bool { true }
}

/// Its scroller always overlays the content: a legacy one, as with "Show scroll bars: Always" or a
/// mouse plugged in, would cover the right end of rows laid out at the panel's width.
final class OverlayScrollView: NSScrollView {
    override var scrollerStyle: NSScroller.Style {
        get { .overlay }
        set {}
    }
}

/// Scrolls `document` on the glass, its scroller showing only while it scrolls.
@MainActor
func scrollView(holding document: FlippedView) -> NSScrollView {
    // Sized as the document, so a document that tracks the width doesn't grow by the whole
    // panel width when the caller sizes the scroll view.
    let scroll = OverlayScrollView(frame: document.frame)
    scroll.drawsBackground = false
    scroll.hasVerticalScroller = true
    scroll.autohidesScrollers = true
    scroll.documentView = document
    return scroll
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
final class AppDelegate: NSObject, NSApplicationDelegate, NSTextFieldDelegate, NSWindowDelegate, NSMenuDelegate {
    lazy var launcher = Launcher(listener: MainRelay { [weak self] in self?.resultsChanged() })
    let windows = Windows()
    // Non-activating, so the app the user was in stays frontmost: window snapping acts on it.
    let panel = Panel(
        contentRect: NSRect(x: 0, y: 0, width: Metrics.width, height: Metrics.headerHeight),
        styleMask: [.borderless, .nonactivatingPanel],
        backing: .buffered,
        defer: false
    )
    let content = NSView()
    let field = NSTextField()
    /// The scrolled content of `results`.
    let list = FlippedView()
    lazy var results = scrollView(holding: list)
    /// Shown in place of the field and list while an item's detail is open.
    var detail: DetailPane?
    /// Every group's items, in the order shown.
    var items: [Item] = []
    /// Each item's row; group headers sit between them in `list`.
    var rows: [Row] = []
    /// The core decides which item is picked; this only mirrors it, to draw.
    var picked = 0 {
        didSet { highlightPicked() }
    }
    var summoned: CFAbsoluteTime?
    /// A plugin's results changed while the panel was hidden or showed a detail.
    var resultsStale = false
    /// The registered hotkeys by their Carbon hotkey IDs: the ref to unregister each, and what it
    /// does. IDs aren't reused, so a press queued before a reload can't run another binding.
    var hotkeys: [UInt32: (ref: EventHotKeyRef, action: () -> Void)] = [:]
    var nextHotkeyID: UInt32 = 0
    /// Problems with the settings that came while an alert about earlier ones showed; shown
    /// after it, rather than in an alert on top.
    var pendingProblems: [String]?
    var showingProblems = false
    /// Built only while the settings show it, so hiding it frees it.
    var statusItem: NSStatusItem?

    func applicationDidFinishLaunching(_: Notification) {
        // Lazy only so the relay can reach `self`; plugins start their background work now rather
        // than on the first keystroke.
        _ = launcher
        buildPanel()
        handleHotkeys()
        apply(launcher.settings())
        // Opens the panel at launch with this query, for measuring without the hotkey.
        if let query = ProcessInfo.processInfo.environment["LAUNCHER_OPEN"] {
            // Apps the launcher opens inherit its environment.
            unsetenv("LAUNCHER_OPEN")
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
        field.placeholderString = "Search, calculate, convert…"
        // One line that scrolls sideways: a pasted token would otherwise wrap onto a clipped second line.
        field.usesSingleLineMode = true
        field.cell?.isScrollable = true
        field.delegate = self
        field.autoresizingMask = [.width, .minYMargin]
        // A fixed height: `resize` reads it as the list's, so the panel growing mustn't stretch it.
        results.autoresizingMask = [.width]
        content.addSubview(field)
        content.addSubview(results)
        resize()
    }

    func handleHotkeys() {
        var pressed = EventTypeSpec(eventClass: OSType(kEventClassKeyboard), eventKind: UInt32(kEventHotKeyPressed))
        let delegate = Unmanaged.passUnretained(self).toOpaque()
        InstallEventHandler(GetApplicationEventTarget(), { _, event, context in
            guard let event, let context else { return OSStatus(eventNotHandledErr) }
            var hotkeyID = EventHotKeyID()
            let status = GetEventParameter(
                event, EventParamName(kEventParamDirectObject), EventParamType(typeEventHotKeyID),
                nil, MemoryLayout<EventHotKeyID>.size, nil, &hotkeyID
            )
            guard status == noErr else { return status }
            let delegate = Unmanaged<AppDelegate>.fromOpaque(context).takeUnretainedValue()
            return MainActor.assumeIsolated {
                guard let hotkey = delegate.hotkeys[hotkeyID.id] else { return OSStatus(eventNotHandledErr) }
                hotkey.action()
                return noErr
            }
        }, 1, &pressed, delegate, nil)
    }

    @objc func reloadSettings() {
        if let settings = launcher.reloadSettings() { apply(settings) }
    }

    /// Registers the settings' hotkeys in place of those from before, and shows what's wrong
    /// with the settings, if anything.
    func apply(_ settings: Settings) {
        for hotkey in hotkeys.values {
            UnregisterEventHotKey(hotkey.ref)
        }
        hotkeys = [:]
        showStatusItem(settings.menuBar)
        let problems = [settings.problem].compactMap { $0 } + settings.bindings.compactMap(register)
        problems.forEach(log)
        // The alert's modal loop still runs reloads, so a save while it shows lands here.
        guard !showingProblems else {
            pendingProblems = problems
            return
        }
        showingProblems = true
        var showing = problems
        while !showing.isEmpty {
            pendingProblems = nil
            showAlert("Some settings don't work", showing.joined(separator: "\n\n"))
            showing = pendingProblems ?? []
        }
        showingProblems = false
    }

    func showStatusItem(_ shown: Bool) {
        if !shown, let statusItem {
            NSStatusBar.system.removeStatusItem(statusItem)
            self.statusItem = nil
        } else if shown, statusItem == nil {
            let item = NSStatusBar.system.statusItem(withLength: NSStatusItem.squareLength)
            item.button?.image = NSImage(systemSymbolName: "command", accessibilityDescription: "Launcher")
            let menu = NSMenu()
            for (title, action, key) in [
                ("Open Launcher", #selector(openPanel), ""),
                ("Open Settings", #selector(openSettingsFile), ","),
                ("Reload Settings", #selector(reloadSettings), ""),
                ("Launch at Login", #selector(toggleLaunchAtLogin), ""),
            ] {
                menu.addItem(withTitle: title, action: action, keyEquivalent: key).target = self
            }
            menu.addItem(.separator())
            menu.addItem(withTitle: "Quit Launcher", action: #selector(NSApplication.terminate(_:)), keyEquivalent: "q")
                .target = NSApp
            menu.delegate = self
            item.menu = menu
            statusItem = item
        }
    }

    @objc func openPanel() {
        summon(at: CFAbsoluteTimeGetCurrent())
    }

    /// Opens the config file in the app that opens `.toml` files, else TextEdit, writing it first
    /// when there's none.
    @objc func openSettingsFile() {
        let file: URL
        do {
            file = try URL(fileURLWithPath: settingsFile())
        } catch let SettingsFileError.Unavailable(reason) {
            showAlert("Can't open the settings", reason)
            return
        } catch {
            showAlert("Can't open the settings", "\(error)")
            return
        }
        let workspace = NSWorkspace.shared
        guard let editor = workspace.urlForApplication(toOpen: file)
            ?? workspace.urlForApplication(withBundleIdentifier: "com.apple.TextEdit")
        else {
            showAlert("Can't open the settings", "No app opens \(file.path).")
            return
        }
        open(file, with: editor)
    }

    /// Ticks Launch at Login from the system's state each time the menu opens, since System
    /// Settings can change it too.
    func menuNeedsUpdate(_ menu: NSMenu) {
        let state: NSControl.StateValue = switch SMAppService.mainApp.status {
        case .enabled: .on
        case .requiresApproval: .mixed
        default: .off
        }
        menu.items.first { $0.action == #selector(toggleLaunchAtLogin) }?.state = state
    }

    @objc func toggleLaunchAtLogin() {
        let on = SMAppService.mainApp.status != .enabled
        do {
            try setLaunchAtLogin(on)
        } catch {
            log("can't change launch at login: \(error)")
            showAlert("Can't turn \(on ? "on" : "off") Launch at Login", error.localizedDescription)
            return
        }
        // Registered, but off in System Settings, as when the user turned it off there.
        guard SMAppService.mainApp.status == .requiresApproval else { return }
        if showAlert(
            "Turn on Launcher in Login Items",
            loginItemApproval,
            buttons: ["Open Login Items", "Cancel"]
        ) == .alertFirstButtonReturn {
            SMAppService.openSystemSettingsLoginItems()
        }
    }

    /// Its action runs when the hotkey is pressed, whichever app is frontmost. Returns why it
    /// couldn't be registered.
    func register(_ binding: HotkeyBinding) -> String? {
        guard let keyCode = binding.hotkey.keyCode else { return "\(binding.setting): no such key." }
        var registered: EventHotKeyRef?
        let id = nextHotkeyID
        nextHotkeyID &+= 1
        // Exclusive, so the hotkey isn't acted on twice: registering fails while another app
        // holds it exclusively, and apps that registered it plainly stop getting it.
        let status = RegisterEventHotKey(
            UInt32(keyCode), UInt32(binding.hotkey.carbonModifiers),
            EventHotKeyID(signature: 0x4C4E_4348, id: id),
            GetApplicationEventTarget(), OptionBits(kEventHotKeyExclusive), &registered
        )
        guard status == noErr, let registered else {
            let reason = status == eventHotKeyExistsErr ? "another app holds this hotkey exclusively" : "error \(status)"
            return "\(binding.setting): \(reason)."
        }
        hotkeys[id] = (registered, { [unowned self, action = binding.action] in
            if let action { windows.perform(action) } else { summon(at: CFAbsoluteTimeGetCurrent()) }
        })
        return nil
    }

    func summon(at time: CFAbsoluteTime) {
        summoned = time
        // ⌥Space while the panel is still key comes here without hiding it first.
        closeDetail()
        refreshIfStale()
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
        launcher.hidden()
    }

    func windowDidBecomeKey(_: Notification) {
        guard let summoned else { return }
        self.summoned = nil
        log(String(format: "summon to focus: %.1f ms", (CFAbsoluteTimeGetCurrent() - summoned) * 1000))
    }

    func windowDidResignKey(_: Notification) {
        hide()
    }

    /// A plugin's background work finished, so the shown query may have other results now.
    func resultsChanged() {
        guard panel.isVisible, detail == nil else {
            resultsStale = true
            return
        }
        refresh()
    }

    func refreshIfStale() {
        if resultsStale { refresh() }
    }

    /// Rebuilds the rows. For the same query, the item the user moved to stays picked, as rows
    /// may come and go; otherwise the top result is.
    func refresh() {
        resultsStale = false
        let shown = launcher.query(query: field.stringValue)
        let groups = shown.groups
        items = groups.flatMap(\.items)
        rows = []
        list.subviews.forEach { $0.removeFromSuperview() }
        let rowHeight: CGFloat = 44
        let headerHeight: CGFloat = 28
        // Eight rows under their header, and room for the next group's header to show there's more.
        let maxHeight = Metrics.inset * 2 + headerHeight * 2 + 8 * (rowHeight + 2)
        let titleX = Metrics.inset + 12
        var y = Metrics.inset
        for group in groups {
            let header = NSTextField(labelWithString: group.name)
            header.font = .systemFont(ofSize: 12, weight: .semibold)
            header.textColor = .secondaryLabelColor
            header.frame = NSRect(x: titleX, y: y + 9, width: Metrics.width - titleX * 2, height: 16)
            list.addSubview(header)
            // A group's first row reaches up over its header.
            var top = y - Metrics.inset
            y += headerHeight
            for item in group.items {
                let row = row(for: item, at: rows.count, frame: NSRect(
                    x: Metrics.inset, y: y, width: Metrics.width - Metrics.inset * 2, height: rowHeight
                ))
                row.reach = NSRect(x: 0, y: top, width: Metrics.width, height: y + rowHeight + Metrics.inset - top)
                list.addSubview(row)
                rows.append(row)
                y += rowHeight + 2
                top = y - Metrics.inset
            }
        }
        let height = groups.isEmpty ? 0 : y + Metrics.inset
        list.frame = NSRect(x: 0, y: 0, width: Metrics.width, height: height)
        results.frame = NSRect(x: 0, y: 0, width: Metrics.width, height: min(height, maxHeight))
        resize()
        show(Int(shown.picked))
    }

    /// `item`'s row, the `index`th in `items`.
    func row(for item: Item, at index: Int, frame: NSRect) -> Row {
        let row = Row(frame: frame)
        row.clicked = { [unowned self] in choose(index) }
        row.hovered = { [unowned self] in if picked != index { picked = Int(launcher.pick(index: UInt32(index))) } }
        row.wantsLayer = true
        row.layer?.cornerRadius = Metrics.selectionRadius
        let icon = NSImageView(image: item.icon.image(side: Metrics.icon))
        icon.frame = NSRect(x: 10, y: 8, width: Metrics.icon, height: Metrics.icon)
        icon.contentTintColor = .secondaryLabelColor
        row.addSubview(icon)
        let titleX = icon.frame.maxX + 8
        let title = titleLabel(item.title)
        // The shell names its own keys, so plugins' subtitles say only what an item is.
        var keys = ""
        switch item.action {
        case .copy: keys = "↵ copies"
        case .open, .openUrl: keys = "↵ opens"
        case .nothing: title.textColor = .secondaryLabelColor
        }
        let subtitle = hintLabel(
            ([item.subtitle, keys] + (item.detail.isEmpty ? [] : ["→ details"]))
                .filter { !$0.isEmpty }
                .joined(separator: " · ")
        )
        // A long subtitle loses its start, such as an app's folder, rather than the keys.
        subtitle.lineBreakMode = .byTruncatingHead
        title.lineBreakMode = .byTruncatingTail
        let layout = RowLayout(
            titleX: titleX, end: row.frame.width - 12,
            title: title.fittingSize.width, subtitle: subtitle.fittingSize.width
        )
        subtitle.frame = NSRect(x: layout.subtitleX, y: 13, width: layout.subtitleWidth, height: 18)
        title.frame = NSRect(x: titleX, y: 11, width: layout.titleWidth, height: 22)
        row.addSubview(title)
        row.addSubview(subtitle)
        return row
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
        for (index, row) in rows.enumerated() {
            row.layer?.backgroundColor = index == picked ? NSColor.controlAccentColor.cgColor : nil
        }
    }

    /// Shows the `index`th item picked and scrolls its row into view. Hover sets `picked` alone:
    /// scrolling under the pointer would pick the row that slides beneath it, and scroll again.
    func show(_ index: Int) {
        picked = index
        if rows.indices.contains(index) { list.scrollToVisible(rows[index].reach) }
    }

    func showDetail(of index: Int) {
        // Going back after a refresh returns to the item the detail showed.
        picked = Int(launcher.pick(index: UInt32(index)))
        let pane = DetailPane(item: items[index])
        pane.text.back = { [unowned self] in
            closeDetail()
            refreshIfStale()
            panel.makeFirstResponder(field)
            field.currentEditor()?.moveToEndOfDocument(nil)
        }
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
        let item = items.indices.contains(index) ? items[index] : nil
        if let item, case .nothing = item.action { return }
        // Hidden before the action runs, so plugins drop work for the query before an open, which
        // finishes later, starts.
        field.stringValue = ""
        refresh()
        hide()
        if let item { run(item) }
    }

    func run(_ item: Item) {
        switch item.action {
        case let .copy(text):
            if copyToPasteboard(text) { launcher.ran(choice: item.choice) }
        case let .open(path):
            open(URL(fileURLWithPath: path)) { self.launcher.ran(choice: item.choice) }
        case let .openUrl(address):
            guard let url = URL(string: address) else {
                log("can't open a malformed web address")
                return
            }
            open(url) { self.launcher.ran(choice: item.choice) }
        case .nothing:
            break
        }
    }

    func controlTextDidChange(_: Notification) {
        refresh()
    }

    func control(_: NSControl, textView: NSTextView, doCommandBy selector: Selector) -> Bool {
        switch selector {
        case #selector(NSResponder.moveDown(_:)) where !items.isEmpty:
            show(Int(launcher.down()))
        case #selector(NSResponder.moveUp(_:)) where !items.isEmpty:
            show(Int(launcher.up()))
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

/// Hands the core's calls, which come on its background threads, to the main actor.
final class MainRelay: ResultsListener, Sendable {
    let changed: @MainActor @Sendable () -> Void

    init(_ changed: @escaping @MainActor @Sendable () -> Void) {
        self.changed = changed
    }

    func resultsChanged() {
        DispatchQueue.main.async { MainActor.assumeIsolated { self.changed() } }
    }
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

/// Opens `url`, a file or web address, in `app`, or else its default app, then runs `opened`. A
/// failure shows an alert, since the panel has already closed, so nothing else would show it.
@MainActor
func open(_ url: URL, with app: URL? = nil, opened: @escaping @MainActor @Sendable () -> Void = {}) {
    let finished: @Sendable (NSRunningApplication?, (any Error)?) -> Void = { _, error in
        let failure = error?.localizedDescription
        Task { @MainActor in
            guard let failure else { return opened() }
            if url.isFileURL {
                log("can't open \(url.path): \(failure)")
                showAlert("Can't open \(FileManager.default.displayName(atPath: url.path))", failure)
            } else {
                // The host only: the rest can be a search, and the log is kept on disk.
                let host = url.host() ?? "the web address"
                log("can't open \(host): \(failure)")
                showAlert("Can't open \(host)", failure)
            }
        }
    }
    let configuration = NSWorkspace.OpenConfiguration()
    if let app {
        NSWorkspace.shared.open([url], withApplicationAt: app, configuration: configuration, completionHandler: finished)
    } else {
        NSWorkspace.shared.open(url, configuration: configuration, completionHandler: finished)
    }
}

/// Activates the app to show the alert, then gives focus back: window management acts on the
/// frontmost app. Returns which of `buttons` was clicked, from `.alertFirstButtonReturn`.
@MainActor
@discardableResult
func showAlert(_ message: String, _ detail: String, buttons: [String] = []) -> NSApplication.ModalResponse {
    let alert = NSAlert()
    alert.messageText = message
    alert.informativeText = detail
    for button in buttons {
        alert.addButton(withTitle: button)
    }
    let previous = NSWorkspace.shared.frontmostApplication
    NSApp.activate()
    let response = alert.runModal()
    previous?.activate()
    return response
}

extension Icon {
    /// A file's own icon, or an SF Symbol for the kinds the core names, shown `side` points square.
    @MainActor
    func image(side: CGFloat) -> NSImage {
        switch self {
        case let .file(path): fileIcon(path, side: side)
        case .calculator: symbol("plus.forwardslash.minus")
        case .token: symbol("key.horizontal")
        case .search: symbol("magnifyingglass")
        }
    }

    /// The file's icon without representations of more than twice the pixels `side` points take
    /// on the sharpest screen. NSImageView prepares the one it picks off the main thread, and for
    /// some apps' icons renders a 4 MB bitmap, whose pages the allocator keeps after freeing it.
    @MainActor
    private func fileIcon(_ path: String, side: CGFloat) -> NSImage {
        let icon = NSWorkspace.shared.icon(forFile: path)
        let pixels = side * (NSScreen.screens.map(\.backingScaleFactor).max() ?? 2)
        let small = icon.representations.filter { CGFloat($0.pixelsWide) < 2 * pixels }
        guard !small.isEmpty else { return icon }
        let image = NSImage(size: NSSize(width: side, height: side))
        image.addRepresentations(small)
        return image
    }

    private func symbol(_ name: String) -> NSImage {
        NSImage(systemSymbolName: name, accessibilityDescription: nil)?
            .withSymbolConfiguration(NSImage.SymbolConfiguration(pointSize: 18, weight: .medium)) ?? NSImage()
    }
}

extension Hotkey {
    /// Carbon's virtual key code: the key's position on a US layout.
    var keyCode: Int? {
        switch key {
        case .left: kVK_LeftArrow
        case .right: kVK_RightArrow
        case .up: kVK_UpArrow
        case .down: kVK_DownArrow
        case .return: kVK_Return
        case .delete: kVK_Delete
        case .space: kVK_Space
        case let .character(character): characterKeyCodes[character]
        }
    }

    var carbonModifiers: Int {
        (control ? controlKey : 0) | (option ? optionKey : 0) | (shift ? shiftKey : 0) | (command ? cmdKey : 0)
    }
}

let characterKeyCodes: [String: Int] = [
    "a": kVK_ANSI_A, "b": kVK_ANSI_B, "c": kVK_ANSI_C, "d": kVK_ANSI_D, "e": kVK_ANSI_E, "f": kVK_ANSI_F,
    "g": kVK_ANSI_G, "h": kVK_ANSI_H, "i": kVK_ANSI_I, "j": kVK_ANSI_J, "k": kVK_ANSI_K, "l": kVK_ANSI_L,
    "m": kVK_ANSI_M, "n": kVK_ANSI_N, "o": kVK_ANSI_O, "p": kVK_ANSI_P, "q": kVK_ANSI_Q, "r": kVK_ANSI_R,
    "s": kVK_ANSI_S, "t": kVK_ANSI_T, "u": kVK_ANSI_U, "v": kVK_ANSI_V, "w": kVK_ANSI_W, "x": kVK_ANSI_X,
    "y": kVK_ANSI_Y, "z": kVK_ANSI_Z, "0": kVK_ANSI_0, "1": kVK_ANSI_1, "2": kVK_ANSI_2, "3": kVK_ANSI_3,
    "4": kVK_ANSI_4, "5": kVK_ANSI_5, "6": kVK_ANSI_6, "7": kVK_ANSI_7, "8": kVK_ANSI_8, "9": kVK_ANSI_9,
]

@discardableResult
func copyToPasteboard(_ text: String) -> Bool {
    NSPasteboard.general.clearContents()
    if NSPasteboard.general.setString(text, forType: .string) { return true }
    // Not the text itself: it can be a token's claims, and the log is kept on disk.
    log("can't copy \(text.count) characters to the pasteboard")
    return false
}

func log(_ message: String) {
    // The non-throwing `write` raises when stderr is a file a full disk refuses, and there's
    // nowhere left to report that.
    try? FileHandle.standardError.write(contentsOf: Data((message + "\n").utf8))
}

// `Launcher config init`, `Launcher secret set kagi` and `Launcher login on`, from the app's own binary, so there's no separate
// tool to build and sign.
do {
    // A block, so `arguments` stays local rather than a global every file could see.
    let arguments = Array(CommandLine.arguments.dropFirst())
    if let status = loginCommand(arguments) ?? runCommand(arguments: arguments) {
        exit(status)
    }
}

startLog()
MainActor.assumeIsolated {
    let app = NSApplication.shared
    app.setActivationPolicy(.accessory)
    let delegate = AppDelegate()
    app.delegate = delegate
    app.run()
}
