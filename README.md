# Launcher

A launcher written with the core written in Rust and "plugins" written directly as part of the launcher for now.

Plans for a WASM plugin system in the future if needed.

Built to use as little memory as possible while running or idle.

## Build

You need the Rust toolchain pinned in `rust-toolchain.toml` (rustup installs it) and Xcode 26 or
later.

```sh
scripts/bundle    # builds target/release/Launcher.app
open target/release/Launcher.app
```

Set `MAC_APP_SIGN` to a codesigning identity (`security find-identity -v -p codesigning` lists
yours) to keep macOS permissions across rebuilds; without it the app is signed ad hoc.

## Contributing

See [`AGENTS.md`](AGENTS.md) for the architecture, commands and coding guidelines.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT)
at your option.
