# AGENTS.md

This file provides guidance to AI coding agents when working with code in this repository.

Launcher is a keyboard launcher (calculator, unit and currency conversion, window management) built to use as little memory as possible. The plugins are Rust and shared; each platform gets a native shell. macOS, the main target, is Swift/AppKit. A Linux shell (GTK or Qt) will follow and use the core directly in Rust.

## Commands

The toolchain is pinned in `rust-toolchain.toml`. The macOS shell targets macOS 26 and needs Xcode 26 or later. The target is set in three places that must agree: `platforms` in `macos/Package.swift`, `LSMinimumSystemVersion` in `scripts/bundle` and `MACOSX_DEPLOYMENT_TARGET` in `.cargo/config.toml`.

```sh
cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test
cargo test -p launcher-calc units            # single test, by name substring
just                                         # bundle, stop any running copy, run in the foreground
just run '100 N to lbf'                      # the same, opening the panel with this query
scripts/bundle                               # macOS: Rust core, Swift bindings, signed target/release/Launcher.app
swift test --package-path macos              # the Swift shell's tests, after scripts/bundle
scripts/install                              # bundle, install as ~/Applications/Launcher.app, quit any running copy and open it
LAUNCHER_OPEN='100 N to lbf' target/release/Launcher.app/Contents/MacOS/Launcher   # open the panel at launch
target/release/Launcher.app/Contents/MacOS/Launcher config init   # write ~/.config/launcher/config.toml, every setting commented out
target/release/Launcher.app/Contents/MacOS/Launcher login status   # launch at login: on, off or status; registers this copy of the app
pbpaste | target/release/Launcher.app/Contents/MacOS/Launcher secret set kagi   # store the Kagi API token in the keychain; `secret remove kagi` removes it
scripts/measure                              # bundle, then check memory and summon time against the budget over 6 launches
footprint "$(pgrep -x Launcher)"             # memory, as Activity Monitor reports it
cargo build --profile profiling              # release with full symbols, for samply or xctrace
```

`scripts/bundle` signs with the identity in `MAC_APP_SIGN`, or ad hoc without one, in which case macOS asks again for the Accessibility permission after every build, and may ask again for access to the keychain item holding the Kagi token. It writes the UniFFI bindings into `macos/Sources/launcher_ffiFFI` and `macos/Sources/Launcher/Generated` (both ignored), so run it before `swift build --package-path macos`.

The app logs to stderr (`log(…)` in Swift, `launcher_core::log!` in Rust). When stderr is `/dev/null`, as it is when launched by Finder or `open`, `Log.swift` points it at `~/Library/Logs/launcher.log`.

## Architecture

- `crates/launcher-core`: the `Plugin` trait, `Item` (with its `Icon`, `Action` and detail `Section`s), `Plugins`, which orders results by `Item::relevance` (0–100, shared by every plugin), and `Hotkey`. No FFI or UI dependencies.
- `crates/launcher-config`: settings from `~/.config/launcher/config.toml` over the defaults, and the `config` command. A mistake rejects the whole file with a message naming its line. Tests keep `default.toml`, `config.schema.json` and the defaults in step.
- `crates/launcher-plugins`: `Launcher` builds the plugins the settings turn on, starts their background work and reloads the settings when the shell asks (⌘R; nothing watches the file). Shells hold a `Launcher` rather than choosing plugins.
- `crates/launcher-apps`: app search over the macOS app folders, indexed on a background thread and rescanned on change, with fuzzy matching and frecency.
- `crates/launcher-calc`: numbat for arithmetic, units and currency, with ECB exchange rates cached on disk and refreshed in the background. The numbat context builds in the background, and each query is interpreted and forgotten so the context doesn't grow.
- `vendor/numbat`: numbat with the patches in its `PATCHES.md`, through `[patch.crates-io]`.
- `crates/launcher-jwt`: decodes a pasted JSON Web Token (signature not verified).
- `crates/launcher-kagi`: web search with Kagi for queries starting with its prefix: a row that opens kagi.com in the browser, and with an API token, results from Kagi's Search API fetched on a background thread once typing pauses.
- `crates/launcher-secret`: secrets such as API tokens in the platform's secret store (the login keychain through Security.framework on macOS; Linux to come), read on demand, and the `secret` command.
- `crates/launcher-window`: window layouts and moves between displays as pure geometry, and their default hotkeys. Shells only read and move windows.
- `crates/launcher-ffi`: the UniFFI boundary the Swift shell calls, mirroring the core's types so the core stays free of FFI derives. Built as a static library linked into the app. `crates/uniffi-bindgen` is the matching bindgen CLI.
- `macos/`: the AppKit shell. A non-activating `NSPanel`, so the app the user was in stays frontmost for window management; global hotkeys through Carbon's `RegisterEventHotKey`; no Dock icon or main menu, so the panel handles ⌘Q, ⌘, and ⌘R itself; an optional menu bar item; launch at login through `SMAppService`. Given arguments, the binary runs the `login`, `config` and `secret` commands before starting AppKit. `Windows.swift` moves windows through the Accessibility API. `PanelLayout` holds arithmetic the shell can't leave to the core, such as how a row shares its width, free of AppKit so `swift test` covers it.

Logic belongs in the shared Rust crates; shells stay thin: they draw results, register hotkeys, name the keys and run actions.

Plugins answer `query` synchronously on the UI thread for every keystroke, so they must stay well inside a frame; anything slow (network, disk) runs in the background ahead of time.

Settings and state live in `~/.config/launcher`, or in `LAUNCHER_CONFIG_DIR` when it's set, so a test or measurement can run without the user's settings (`launcher_core::config_directory`). Downloads that can be fetched again, such as exchange rates, live in the platform's cache folder (`launcher_core::cache_directory`).

## Memory

Memory is the main constraint: prefer work done on demand over state kept resident. Run `scripts/measure` before and after a change that adds a dependency or keeps state; it checks the bundled app against the budget at its top, with the panel open, hidden, and showing a detail long enough that Core Animation draws it on the GPU. `crates/launcher-plugins/tests/memory.rs` checks the plugins' live heap in CI. Raise either budget only deliberately, in the commit that needs it, and record what was measured in the PR.

`scripts/measure` needs a screen and its numbers depend on the Mac, so compare builds side by side in one session rather than against old figures. It leaves out LaunchServices' `CoreServices` region, which swings by several MB in every app with the system's state.

## Rust coding guidelines

- Prioritize code correctness and clarity. Speed and efficiency are secondary priorities unless otherwise specified.
- Comments only explain why code is written as it is, where that's not obvious; don't summarize the code.
- Prefer implementing functionality in existing files unless it is a new logical component. Avoid creating many small files.
- Avoid functions that panic like `unwrap()`, and indexing that may go out of bounds; propagate errors with `?`.
- Log with `launcher_core::log!`, not `eprintln!`: the installed app's stderr is a file, and `eprintln!` panics when a write to it fails. Clippy enforces it; only the `config` command, writing to its terminal, is exempt.
- Never silently discard errors with `let _ =` on fallible operations: propagate them, log them, or handle them explicitly.
- Never create files with `mod.rs` paths - prefer `src/some_module.rs` instead of `src/some_module/mod.rs`.
- When creating new crates, set the library root in `Cargo.toml` with `[lib] path = "...rs"` instead of the default `lib.rs`, for descriptive naming.
- Avoid creative additions unless explicitly requested.
- Use full words for variable names (no abbreviations like "q" for "queue").

## Verifying changes

A change is done when it works in the real app, not just when tests pass. Run it with `just run '<query>'`, check the `summon to focus` line it logs, then drive the panel with `scripts/press` (real key presses, including ⌥Space) and check the screen and clipboard. The `drive-app` skill (`.claude/skills/drive-app/SKILL.md`) walks through it and the traps that look like app bugs. `scripts/press` needs the Accessibility permission for the terminal.
