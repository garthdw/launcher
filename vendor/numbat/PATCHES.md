# Patches to numbat 1.24.0

This is numbat's `v1.24.0` tag, used through `[patch.crates-io]` in the workspace `Cargo.toml`.
Each patch below is its own commit. Remove a patch once upstream releases an equivalent, and the
vendored copy once none are left.

Run numbat's own tests with
`cargo test --manifest-path vendor/numbat/numbat/Cargo.toml --target-dir target/vendor-numbat`.
Unpatched, `test_datetime_runtime_errors` already fails with jiff 0.2.46: it matches jiff's error
text, which changed after 1.24.0's lockfile.

## 1. `Context::set_exchange_rates` replaces rates (`numbat/src/currency.rs`)

Upstream keeps exchange rates in a `OnceLock` and `set_from_xml` calls `.set(..).unwrap()`, so a
second call panics, as does a first call after a currency lookup has already stored `None`. The
launcher runs for weeks and refreshes rates daily, so `set_from_xml` assigns through the `Mutex`
inside the lock instead. The signature is unchanged, and so is the rest: XML that doesn't parse
still leaves no rates (the launcher checks XML before passing it), and the lock is unwrapped as
everywhere else in `currency.rs`.
