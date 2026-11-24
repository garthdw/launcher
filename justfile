# Runs in the foreground, so the app's log shows here; Ctrl-C quits. Stops a running copy
# first, since it holds ⌥Space. A query opens the panel with it: `just run '100 N to lbf'`.
# Bundle the app and run it
run query="":
    scripts/bundle
    -pkill -x Launcher
    {{ if query == "" { "" } else { "LAUNCHER_OPEN=" + quote(query) } }} target/release/Launcher.app/Contents/MacOS/Launcher

# Keys by name with cmd+, opt+, ctrl+, shift+, or `--text '2 x 3'`; see scripts/press.swift.
# Press keys in the open panel: `just press right down cmd+c`
[positional-arguments]
press *keys:
    scripts/press "$@"
