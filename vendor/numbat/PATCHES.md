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

## 2. jiff 1.0 (`numbat/Cargo.toml`, `numbat/src/{datetime,value,vm}.rs`, `numbat/src/ffi/datetime.rs`)

So the app links one jiff, shared with its own crates, numbat depends on `jiff` 1.0 instead of
0.2. jiff 1.0 renamed `Zoned` to `ZonedDateTime` and `to_zoned` to `to_zoned_datetime`; nothing
else needed changing. `js` still exists in 1.0 and only matters on wasm, so it stays. numbat's
tests give the same results as before this patch.

## 3. `Context::interpret_and_forget` (`numbat/src/{lib,vm,bytecode_interpreter,resolver,diagnostic}.rs`)

Upstream keeps everything a `Context` compiles: each `interpret` appends its bytecode and spans to
the VM's main chunk, its numbers to the constant table, and its source to the resolver's files.
`add_constant` asserts there are at most 65,535 constants, so a context answering every keystroke
panics after about 32,000 queries, slowing down and growing on the way, since the constants are
cloned on every call. `interpret_and_forget` records the lengths of those tables, interprets, and
truncates them again, whether the code succeeded or not. It refuses any statement other than an
expression or procedure call with the new `ResolverError::Definition`, as a definition or import
would outlive the call while referring to what it dropped. The resolver's files are a `SourceFiles`
instead of codespan's `SimpleFiles`, which can't be truncated; it implements the same `Files` trait.

## 4. `interpret_and_forget` copies nothing and suggests no names (`numbat/src/{lib.rs,typechecker/mod.rs}`)

`interpret` clones the prefix transformer, typechecker and interpreter on every call, to restore
them on error; profiling the launcher put those clones and their drops at about 45% of a
keystroke. An expression leaves them as they were (only `ans` changes, replaced by the next
expression before use), and patch 3 drops what it compiled, so `interpret_and_forget` runs the
three stages directly, without on-demand currency loading. It also sets the typechecker's new
`skip_suggestions`, since computing "did you mean" against every identifier for each unknown word
was another 34%, for errors the caller doesn't show.
