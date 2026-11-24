---
name: drive-app
description: Run the bundled launcher, drive its panel with real key presses, and check the result by screenshot, clipboard and log. Use to verify a shell or plugin change in the real app (AGENTS.md says a change is done only when it works there), or to reproduce a UI bug.
---

# Drive the app

## Before starting

- The screen must be unlocked: with it locked the panel never becomes key (no `summon to focus` line) and screenshots come back black. `ioreg -n Root -d1 -a | rg -A1 IOConsoleLocked` says `<true/>` when locked; ask the user to unlock.
- `scripts/press` needs the Accessibility permission for the terminal; it says so and exits if missing. Ask the user to grant it.
- Only one copy can run: a second steals the panel's focus and can't register ⌥Space. `just run` stops the old one, so warn the user first if they may be using it.

## Run

Start it in the background (Bash `run_in_background`), opening the panel with a query:

```sh
just run '100 N to lbf'
```

Wait for its log to print `summon to focus: … ms` before pressing anything: that line means the panel is key. Poll the task's output file in a loop (`until rg -q summon "$log"; do osascript -e 'delay 0.2'; done`); a bare `sleep` is blocked.

## Press keys

```sh
scripts/press right right        # keys by name: return esc tab space delete left right up down a c v x z
scripts/press down cmd+c         # modifiers: cmd+ opt+ ctrl+ shift+
scripts/press opt+space          # the global hotkey, to summon the panel again
scripts/press --text '2 x 3'     # type text
```

Things that look like app bugs but aren't:

- The panel opens with the query selected, so the first → only moves the caret to the end; a second → opens a detail.
- ↵ runs the action, clears the query and hides the panel. Start a fresh `just run` for the next case rather than summoning the empty panel.
- Another window manager (Raycast, Rectangle) may own the same ⌃⌥ window hotkeys. The launcher registers its hotkeys exclusively: an app that registered one exclusively first keeps it (the launcher's alert names the setting), and one that registered it plainly stops getting it while the launcher runs. To test while another manager runs, rebind in `~/.config/launcher/config.toml` (such as `left_half = "ctrl+alt+shift+left"`); the app reloads it on save. Back up the user's file first and restore it after. Read window frames with `osascript -e 'tell application "System Events" to tell (first process whose frontmost is true) to get {position, size} of window 1'`.
- An alert about the settings may show without focus, since macOS lets an app take focus only when the frontmost app gives it up; `scripts/press return` then reaches the frontmost app instead. Click its button (post a mouse click with `CGEvent`).
- System Events (`osascript … key code`) sends keys to the frontmost app, never to this non-activating panel. Use `scripts/press`, which posts at the HID level like a keyboard.

## Check the result

- Screen: `screencapture -x "$file"`, then crop to the panel, which sits centred near the top: `sips -c 700 1000 --cropOffset 400 850 "$file" --out "$crop"` on a 3024-pixel-wide Retina screen; adjust for others. Read the crop.
- Clipboard, after ↵ or ⌘C: set a sentinel first (`printf before | pbcopy`), then `pbpaste`.
- Log: the background task's output file holds the app's stderr (`log(…)` in `main.swift`).
- Memory: `vmmap -summary "$(pgrep -x Launcher)" | rg 'Physical footprint'` gives a precise figure (`footprint` rounds to whole MB). Its categories (Malloc Small, IOSurface, IOAccelerator) show whether growth is heap or graphics.

Stop the app with `pkill -x Launcher` when done.
