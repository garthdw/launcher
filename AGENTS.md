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
scripts/install                              # bundle, install as ~/Applications/Launcher.app, quit any running copy and open it
LAUNCHER_OPEN='100 N to lbf' target/release/Launcher.app/Contents/MacOS/Launcher   # open the panel at launch
target/release/Launcher.app/Contents/MacOS/Launcher config init   # write ~/.config/launcher/config.toml, every setting commented out
target/release/Launcher.app/Contents/MacOS/Launcher login status   # launch at login: on, off or status; registers this copy of the app
scripts/measure                              # bundle, then check memory and summon time against the budget over 6 launches
footprint "$(pgrep -x Launcher)"             # memory, as Activity Monitor reports it
cargo build --profile profiling              # release with full symbols, for samply or xctrace
```

`scripts/bundle` signs with the identity in `MAC_APP_SIGN`, or ad hoc without one, in which case macOS asks again for the Accessibility permission after every build. It writes the UniFFI bindings into `macos/Sources/launcher_ffiFFI` and `macos/Sources/Launcher/Generated` (both ignored), so run it before `swift build --package-path macos`.

The app logs to stderr (`log(…)` in Swift, `launcher_core::log!` in Rust). When stderr is `/dev/null`, as it is when launched by Finder or `open`, `Log.swift` points it at `~/Library/Logs/launcher.log` and rotates that to `launcher.log.1` at launch past 1 MB. Each launch logs whether it has the Accessibility permission.

`.cargo/config.toml` sets `MACOSX_DEPLOYMENT_TARGET` to the app's minimum macOS, so C code in dependencies links into the Swift binary without warnings.

## Architecture

- `crates/launcher-core` (`core.rs`): the `Plugin` trait, `Item`, its `Icon` (a file's own, or a kind such as `Calculator` that each shell draws its own way), `Action` and an item's detail `Section`s, `Plugins`, which asks each plugin and orders their groups of results by `Item::relevance`, a 0–100 scale every plugin shares, and `Hotkey`, which reads and prints as text such as `ctrl+alt+left`. No FFI or UI dependencies.
- `crates/launcher-config` (`config.rs`): settings from `~/.config/launcher/config.toml`: the launcher hotkey, whether the menu bar item shows, each window action's hotkey or `none`, and which plugins are on, over the defaults, with the window hotkeys' defaults from `launcher-window`. A mistake rejects the whole file with a message naming its line. Also `config init`, which writes the commented `default.toml` (`file_to_edit` writes it too when the shell opens a missing file), and a watcher that reports saves. `config.schema.json` is the JSON schema editors read through the file's `#:schema` line; tests keep it, `default.toml` and the defaults in step.
- `crates/launcher-plugins` (`plugins.rs`): `start()` builds every plugin the settings turn on, in the order equally relevant results show, and starts their background work, which calls the shell's `ResultsChanged` when it may have changed results so the shell asks again for the shown query. `Launcher` holds the plugins and the settings, watches the config file and reloads it on the shell's call: an invalid file keeps the settings from before and reports the problem, and only the plugins a reload turns on or off start or stop, each in a fixed place, so items already shown still reach theirs. Shells hold a `Launcher` rather than choosing plugins, so they stay thin: they draw results, register hotkeys, name the keys and run actions.
- `crates/launcher-apps` (`apps.rs`): app search. Indexes `.app` bundles in the macOS app folders on a background thread and rescans when `notify` reports a change; a small fuzzy matcher favouring word starts, plus frecency kept in `~/.config/launcher/frecency`. The shell opens results (`Action::Open`) and draws their icons (`Icon::File`).
- `crates/launcher-calc` (`calc.rs`, `rates.rs`): numbat for arithmetic, units and currency. The first numbat `Context` builds on a background thread, as the prelude takes a good part of launch, so the calculator answers nothing for its first few tens of milliseconds. Exchange rates come from the ECB, cached on disk and fetched again in the background when a day old, while the app runs too; each new set builds a fresh `Context` there too, swapped in at the next query. Until rates load, a currency query shows a muted `Action::Nothing` item, as a calculation does if the prelude fails to build. Each query goes through numbat's `interpret_and_forget`, which drops what it compiled, so the context doesn't grow with use, and refuses definitions. numbat's own fetch is off, since it would run on the UI thread.
- `vendor/numbat`: numbat 1.24.0 with the patches in its `PATCHES.md`, through `[patch.crates-io]`.
- `crates/launcher-jwt` (`jwt.rs`): decodes a pasted JSON Web Token (signature not verified) into its claims and expiry, with the header, payload and times as its detail.
- `crates/launcher-window` (`window.rs`): window layouts (halves, quarters, thirds, maximise, centre, and restore to before the launcher moved it) and moves to the next or previous display as pure geometry over top-left-origin frames, which display holds a window, and the default ⌃⌥ and ⌃⌥⌘ hotkeys (`DEFAULT_BINDINGS`). Shells only read and move windows.
- `crates/launcher-ffi` (`ffi.rs`): the UniFFI boundary the Swift shell calls: a `Launcher` object wrapping `launcher_plugins::Launcher`, and records mirroring the core's types, so the core stays free of FFI derives (plain-data types such as `Rect` use `#[uniffi::remote]` instead). Built as a static library linked into the app.
- `crates/uniffi-bindgen`: the bindgen CLI, pinned to the same `uniffi` as the FFI crate.
- `macos/`: the SwiftPM package for the AppKit shell. A non-activating `NSPanel`, so the app the user was in stays frontmost (window management acts on it); global hotkeys from the settings through Carbon's `RegisterEventHotKey`; `LSUIElement`, so no Dock icon or main menu: the panel routes ⌘Q, ⌘, and the editing keys itself, and an optional `NSStatusItem`, built only while the settings show it, opens the panel or the settings file, reloads settings, quits and turns launch at login on or off through `SMAppService.mainApp`. Launch at login isn't a setting, since the system owns it and System Settings can change it; the menu reads it each time it opens, and `Launcher login on|off|status` reaches it without the menu. Hotkeys are registered with `kEventHotKeyExclusive`. A hotkey another app registered exclusively first fails, and an alert names each setting that can't be used. Apps that registered the same hotkey without exclusivity lose it to the launcher while it runs. Clashes with macOS's own shortcuts (Spotlight's ⌘Space, Mission Control's ⌃←) register without error, so they can't be detected. The app's binary runs commands when given them as arguments, before starting AppKit: `login` (`LoginItem.swift`) first, since login items are macOS's, then `launcher_config::run_command`, through the FFI's `config_command`, for the shared `config` commands; each returns no exit status for arguments it doesn't own, so neither needs to know the other. `login on` registers the bundle it runs from, so a `target/release` copy registers that build, which `scripts/bundle` then replaces. `Windows.swift` moves the frontmost app's focused window through the Accessibility API, flipping between AppKit's bottom-left and AX's top-left coordinates.

Plugins answer `query` synchronously on the UI thread for every keystroke, so they must stay well inside a frame; anything slow (network, disk) runs in the background ahead of time.

Memory is the main constraint. Run `scripts/measure` before and after a change that adds a dependency or keeps state, and prefer work done on demand over state kept resident. It checks the bundled app against a budget at its top, leaving out LaunchServices' pages of its database (the `CoreServices` region), which change by several MB in every app with the system's state, as `crates/launcher-plugins/tests/memory.rs` checks the live heap of the calculator and token plugins and that typing doesn't grow it; raise either only deliberately, in the commit that needs it. The heap test runs in CI with the others; `scripts/measure` needs a screen, and its numbers depend on the Mac and its apps, so run it before merging such a change. The bundled app uses about 24.1 MB with the panel open and 24.5 MB after Esc hides it, since hiding frees nothing, and takes about 68 ms from launch to a focused panel. In the core alone the calculator adds about 6.8 MB, of which only about 2 MB is live heap: the rest is fragmentation from building numbat's prelude. App search adds about 2.3 MB, almost all macOS's FSEvents and LaunchServices rather than the ~20 KB index. AppKit, SwiftUI and the shell make up the other ~14 MB. Building the calculator on a background thread rather than the main one costs about 0.45 MB and saves about 30 ms of launch. Moving to macOS 26 and Liquid Glass (`NSGlassEffectView`) from `NSVisualEffectView` added about 3 MB. The app index and its watcher add about 0.1 MB for ~100 apps; showing app rows with icons adds about 1 MB, and the calculator's and token's SF Symbols nothing measurable. Window snapping adds nothing measurable until first used, then about 0.1 MB. The config file and its watcher add about 0.1 MB, and the TOML parser about 0.4 MB of binary. The menu bar item adds about 1.8 MB while it shows, almost all the `NSStatusItem` itself rather than its menu or image, so it's off by default; hidden, it and launch at login add nothing measurable.

Settings and state live in `~/.config/launcher` on every platform (`launcher_core::config_directory`). Downloads that can be fetched again, such as exchange rates, live in the platform's cache folder: `~/Library/Caches/dev.dewet.launcher` on macOS, `~/.cache/launcher` elsewhere (`launcher_core::cache_directory`).

## Rust coding guidelines

- Prioritize code correctness and clarity. Speed and efficiency are secondary priorities unless otherwise specified.
- Do not write organizational or comments that summarize the code. Comments should only be written in order to explain "why" the code is written in some way in the case there is a reason that is tricky / non-obvious.
- Prefer implementing functionality in existing files unless it is a new logical component. Avoid creating many small files.
- Avoid using functions that panic like `unwrap()`, instead use mechanisms like `?` to propagate errors.
- Be careful with operations like indexing which may panic if the indexes are out of bounds.
- Log with `launcher_core::log!`, not `eprintln!`: the installed app's stderr is a file, and `eprintln!` panics when a write to it fails. Clippy enforces it; only the `config` command, writing to its terminal, is exempt.
- Never silently discard errors with `let _ =` on fallible operations. Always handle errors appropriately:
  - Propagate errors with `?` when the calling function should handle them
  - Use `.log_err()` or similar when you need to ignore errors but want visibility
  - Use explicit error handling with `match` or `if let Err(...)` when you need custom logic
  - Example: avoid `let _ = client.request(...).await?;` - use `client.request(...).await?;` instead
- When implementing async operations that may fail, ensure errors propagate to the UI layer so users get meaningful feedback.
- Never create files with `mod.rs` paths - prefer `src/some_module.rs` instead of `src/some_module/mod.rs`.
- When creating new crates, prefer specifying the library root path in `Cargo.toml` using `[lib] path = "...rs"` instead of the default `lib.rs`, to maintain consistent and descriptive naming (e.g., `main.rs`).
- Avoid creative additions unless explicitly requested
- Use full words for variable names (no abbreviations like "q" for "queue")
- Use variable shadowing to scope clones in async contexts for clarity, minimizing the lifetime of borrowed references.
  Example:
  ```rust
  executor.spawn({
      let task_ran = task_ran.clone();
      async move {
          *task_ran.borrow_mut() = true;
      }
  });
  ```

## Verifying changes

A change is done when it works in the real app, not just when tests pass. Run it with `just run '<query>'`, check the `summon to focus` line it logs, then drive the panel with `scripts/press` (real key presses, including ⌥Space) and check the screen and clipboard. The `drive-app` skill (`.claude/skills/drive-app/SKILL.md`) walks through it and the traps that look like app bugs. `scripts/press` needs the Accessibility permission for the terminal.
