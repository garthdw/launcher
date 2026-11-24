# Bundles the app and runs it in the foreground, so its log shows here; Ctrl-C quits.
# A query opens the panel with it at launch: `just run '100 N to lbf'`.
# Stops a running copy first, since it holds ⌥Space.
run query="":
    scripts/bundle
    -pkill -x Launcher
    {{ if query == "" { "" } else { "LAUNCHER_OPEN=" + quote(query) } }} target/release/Launcher.app/Contents/MacOS/Launcher
