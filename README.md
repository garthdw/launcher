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

## Install

```sh
scripts/install   # or `just install`
```

This builds the app, installs it as `~/Applications/Launcher.app`, quits any running copy (only
one can hold ⌥Space) and starts the installed one. Started from Finder, `open` or at login, the app
logs to `~/Library/Logs/launcher.log`, rotating it to `launcher.log.1` at launch once it passes
1 MB; started from a terminal, it logs there instead.

### Signing

macOS ties the Accessibility permission, which moving windows needs, to the app's code signature.
Without a signing identity the app is signed ad hoc, which gives every build a new identity, so
macOS asks for the permission again after each build and install. To keep it, sign with a
certificate, such as the free Apple Development one Xcode creates when you sign in under Settings
› Accounts. List your identities:

```sh
security find-identity -v -p codesigning
```

and set `MAC_APP_SIGN` to one, by its name or hash, in your shell's profile:

```sh
export MAC_APP_SIGN='Apple Development: Jane Doe (ABCDE12345)'
```

## Settings

Settings live in `~/.config/launcher/config.toml`: the launcher's hotkey, whether its menu bar
item shows, each window action's hotkey (or `"none"`), and which plugins are on. ⌘, in the
launcher opens the file, first writing one with every setting commented out at its default, with
comments explaining them; uncomment only what you change, so later defaults still reach the rest.
`LAUNCHER_CONFIG_DIR` moves the folder, with the app's other state, elsewhere. `config init`
writes the same file from a terminal:

```sh
target/release/Launcher.app/Contents/MacOS/Launcher config init
```

The menu bar item, off by default since it takes about 2 MB (`menu_bar = true` shows it), opens
the launcher or the settings file, reloads the settings, quits, and turns Launch at Login on or
off. Launch at Login isn't in the file, since macOS keeps it: System Settings › General › Login
Items changes it too, and it stays as set when the item is hidden. Without the item, set it from
a terminal. It registers the copy of the app you run, so run the installed one (see Install), not
a `target/release` build that the next bundle replaces:

```sh
~/Applications/Launcher.app/Contents/MacOS/Launcher login on   # or off, or status
```

⌘Q in the launcher quits, with or without the item.

Hotkeys are modifiers and a key joined by `+`, such as `"ctrl+alt+left"` or `"cmd+shift+k"`. The
launcher reads the file again when you press ⌘R in it, or choose Reload Settings from the menu
bar item. If the file has a mistake, the launcher keeps the
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
