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
LAUNCHER_OPEN='100 N to lbf' target/release/Launcher.app/Contents/MacOS/Launcher   # open the panel at launch
footprint "$(pgrep -x Launcher)"             # memory, as Activity Monitor reports it
cargo build --profile profiling              # release with full symbols, for samply or xctrace
```

`scripts/bundle` signs with the identity in `MAC_APP_SIGN`, or ad hoc without one, in which case macOS asks again for the Accessibility permission after every build. It writes the UniFFI bindings into `macos/Sources/launcher_ffiFFI` and `macos/Sources/Launcher/Generated` (both ignored), so run it before `swift build --package-path macos`.

`.cargo/config.toml` sets `MACOSX_DEPLOYMENT_TARGET` to the app's minimum macOS, so C code in dependencies links into the Swift binary without warnings.

## Architecture

- `crates/launcher-core` (`core.rs`): the `Plugin` trait, `Item`, `Action` and an item's detail `Section`s. No FFI or UI dependencies.
- `crates/launcher-plugins` (`plugins.rs`): `start()` builds every plugin in result order and starts their background work. Shells call it rather than choosing plugins, so they stay thin: they draw results, name the keys and run actions.
- `crates/launcher-apps` (`apps.rs`): app search. Indexes `.app` bundles in the macOS app folders on a background thread and rescans when `notify` reports a change; a small fuzzy matcher favouring word starts, plus frecency kept in `~/.config/launcher/frecency`. The shell opens results (`Action::Open`) and draws their icons.
- `crates/launcher-calc` (`calc.rs`): numbat for arithmetic, units and currency. Exchange rates come from the ECB, cached on disk and fetched again in the background at launch when a day old; numbat's own fetch is off, since it would run on the UI thread.
- `crates/launcher-jwt` (`jwt.rs`): decodes a pasted JSON Web Token (signature not verified) into its claims and expiry, with the header, payload and times as its detail.
- `crates/launcher-ffi` (`ffi.rs`): the UniFFI boundary the Swift shell calls: a `Launcher` object holding the plugins, and records mirroring the core's types, so the core stays free of FFI derives. Built as a static library linked into the app.
- `crates/uniffi-bindgen`: the bindgen CLI, pinned to the same `uniffi` as the FFI crate.
- `macos/`: the SwiftPM package for the AppKit shell. A non-activating `NSPanel`, so the app the user was in stays frontmost (window management acts on it); the global hotkey (⌥Space) through Carbon's `RegisterEventHotKey`; `LSUIElement`, so no Dock icon.

Plugins answer `query` synchronously on the UI thread for every keystroke, so they must stay well inside a frame; anything slow (network, disk) runs in the background ahead of time.

Memory is the main constraint. Check `footprint` before and after a change that adds a dependency or keeps state, and prefer work done on demand over state kept resident. The bundled app used about 15 MB with the panel open and 12 MB hidden, about 8 MB of which is numbat. Moving to macOS 26 and Liquid Glass (`NSGlassEffectView`) from `NSVisualEffectView` added about 3 MB. The app index and its watcher add about 0.1 MB for ~100 apps; showing app rows with icons adds about 1 MB.

Settings and state live in `~/.config/launcher` on every platform (`launcher_core::config_directory`). Downloads that can be fetched again, such as exchange rates, live in the platform's cache folder: `~/Library/Caches/dev.dewet.launcher` on macOS, `~/.cache/launcher` elsewhere (`launcher_core::cache_directory`).

## Rust coding guidelines

- Prioritize code correctness and clarity. Speed and efficiency are secondary priorities unless otherwise specified.
- Do not write organizational or comments that summarize the code. Comments should only be written in order to explain "why" the code is written in some way in the case there is a reason that is tricky / non-obvious.
- Prefer implementing functionality in existing files unless it is a new logical component. Avoid creating many small files.
- Avoid using functions that panic like `unwrap()`, instead use mechanisms like `?` to propagate errors.
- Be careful with operations like indexing which may panic if the indexes are out of bounds.
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
