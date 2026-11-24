# Launcher

A launcher written with the core written in Rust and "plugins" written directly as part of the launcher for now.

Plans for a WASM plugin system in the future if needed.

Built to use as little memory as possible while running or idle.

## Build

You need the Rust toolchain pinned in `rust-toolchain.toml` (rustup installs it) and Xcode 26 or
later.

```sh
just              # builds and runs the app, logging to the terminal
scripts/bundle    # or just builds target/release/Launcher.app
open target/release/Launcher.app
```

Set `MAC_APP_SIGN` to a codesigning identity (`security find-identity -v -p codesigning` lists
yours) to keep macOS permissions across rebuilds; without it the app is signed ad hoc.

## Settings

Settings live in `~/.config/launcher/config.toml`: the launcher's hotkey, each window action's
hotkey (or `"none"`), and which plugins are on. Write one with every setting commented out at its
default, with comments explaining them; uncomment only what you change, so later defaults still
reach the rest:

```sh
target/release/Launcher.app/Contents/MacOS/Launcher config init
```

Hotkeys are modifiers and a key joined by `+`, such as `"ctrl+alt+left"` or `"cmd+shift+k"`. The
launcher reloads the file when it's saved. If the file has a mistake, the launcher keeps the
settings it had and says what's wrong; if another app holds a hotkey exclusively, it says which
setting it can't use. Apps that registered a hotkey without exclusivity lose it to the launcher
while it runs, and clashes with macOS's own shortcuts (such as Spotlight's ⌘Space) can't be
detected. Editors that read `#:schema` lines
(Taplo, the Even Better TOML extension) check and complete the file against
[`config.schema.json`](crates/launcher-config/config.schema.json).

## Contributing

See [`AGENTS.md`](AGENTS.md) for the architecture, commands and coding guidelines.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT)
at your option.
