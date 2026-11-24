// Presses keys as the keyboard would, so the launcher's panel receives them.
//
// The panel never makes the app active, so System Events keystrokes (sent to the frontmost app)
// miss it; events posted at the HID level are routed like real ones. Needs the Accessibility
// permission for the terminal running it.
//
//   press right right esc cmd+c       keys by name, with cmd+, opt+, ctrl+, shift+ modifiers
//   press --text '100 N to lbf'       types text
import ApplicationServices
import Carbon.HIToolbox
import Foundation

let keys: [String: Int] = [
    "return": kVK_Return, "enter": kVK_Return, "esc": kVK_Escape, "tab": kVK_Tab,
    "space": kVK_Space, "delete": kVK_Delete, "left": kVK_LeftArrow, "right": kVK_RightArrow,
    "up": kVK_UpArrow, "down": kVK_DownArrow,
    "a": kVK_ANSI_A, "c": kVK_ANSI_C, "v": kVK_ANSI_V, "x": kVK_ANSI_X, "z": kVK_ANSI_Z,
    // The window layouts' hotkeys.
    "d": kVK_ANSI_D, "f": kVK_ANSI_F, "g": kVK_ANSI_G,
    "i": kVK_ANSI_I, "j": kVK_ANSI_J, "k": kVK_ANSI_K, "u": kVK_ANSI_U,
]
let modifiers: [String: CGEventFlags] = [
    "cmd": .maskCommand, "opt": .maskAlternate, "ctrl": .maskControl, "shift": .maskShift,
]

func post(_ code: Int, flags: CGEventFlags, text: String? = nil) {
    for down in [true, false] {
        guard let event = CGEvent(keyboardEventSource: nil, virtualKey: CGKeyCode(code), keyDown: down)
        else { continue }
        // Set every time: flags otherwise carry over from earlier posted events, turning → into ⌥→.
        event.flags = flags
        if let text {
            let units = Array(text.utf16)
            event.keyboardSetUnicodeString(stringLength: units.count, unicodeString: units)
        }
        event.post(tap: .cghidEventTap)
    }
    // Lets the app handle each key before the next, as a person's typing would.
    usleep(150_000)
}

guard AXIsProcessTrusted() else {
    FileHandle.standardError.write(Data(
        "press: give the terminal Accessibility permission (System Settings › Privacy & Security)\n".utf8
    ))
    exit(1)
}

var arguments = CommandLine.arguments.dropFirst()
while let argument = arguments.popFirst() {
    if argument == "--text" {
        guard let text = arguments.popFirst() else { break }
        for character in text { post(kVK_ANSI_A, flags: [], text: String(character)) }
        continue
    }
    var parts = argument.lowercased().split(separator: "+").map(String.init)
    guard let name = parts.popLast(), let code = keys[name] else {
        FileHandle.standardError.write(Data("press: unknown key \(argument)\n".utf8))
        exit(1)
    }
    // A keyboard marks arrow keys as Fn and keypad keys; global hotkeys on arrows only match then.
    var flags: CGEventFlags = [kVK_LeftArrow, kVK_RightArrow, kVK_UpArrow, kVK_DownArrow].contains(code)
        ? [.maskSecondaryFn, .maskNumericPad] : []
    for modifier in parts {
        guard let flag = modifiers[modifier] else {
            FileHandle.standardError.write(Data("press: unknown modifier \(modifier)\n".utf8))
            exit(1)
        }
        flags.insert(flag)
    }
    post(code, flags: flags)
}
