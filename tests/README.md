# Tests

Cargo treats every `*.rs` file in this directory as a standalone
**integration test binary**: linked against the public surface of
`markdoc-pdf` (and nothing else), compiled and run by
`cargo test --all-targets`. CI runs that on every push.

Use this directory for tests that need to exercise the public API
end-to-end. Unit tests that touch crate internals stay alongside the
code in `src/**.rs` under `#[cfg(test)] mod tests`.

## Contents

### `parley_krilla_spike.rs` — dependency-integration smoke

A standalone end-to-end test that proves [parley](https://crates.io/crates/parley)
(text shaping) and [krilla](https://crates.io/crates/krilla) (PDF
emit) can be wired together — the two crates form the rendering
substrate that the rest of `src/render/` builds on. If either
publishes an API-breaking release, this file stops compiling and CI
fails before any of the higher-level rendering paths even get
exercised.

The file has a `fn main()` (writes `spike.pdf` with a styled paragraph,
mixed-script text, emoji, and an SVG callout) inherited from when it
lived in `examples/`. Under the default test harness cargo's own
`main` is used, so the spike's `main` doesn't run on `cargo test`;
the value here is purely "did it compile?". To actually execute the
spike, move the file back under `examples/` or convert `main()` into
a `#[test]` function and the test harness will run it.

Notable requirements: the Noto Sans family must be discoverable by
parley's `FontContext`. On Fedora/toolbox: install
`google-noto-sans-fonts`, `google-noto-sans-arabic-fonts`,
`google-noto-sans-devanagari-fonts`, `google-noto-color-emoji-fonts`.
The CI runners already have these.

## Running

```sh
cargo test --all-targets        # everything (lib + integration + examples)
cargo test --test parley_krilla_spike   # just this one
```
