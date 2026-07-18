# alacritty_terminal (LaboLabo vendored copy -- W17 keyboard-mode-stack patch)

This is a local, `[patch.crates-io]`-pinned copy of upstream
[`alacritty_terminal` 0.26.0](https://crates.io/crates/alacritty_terminal)
(https://github.com/alacritty/alacritty), with one deliberate change from
the published crate: `src/term/mod.rs`'s `push_keyboard_mode` now trims
`self.keyboard_mode_stack` instead of `self.title_stack` once the stack
hits its max depth. See the "LOCAL PATCH" comment right above that line for
the full rationale; short version: upstream copy-pasted the bound-check
branch from the unrelated `push_title` method and never updated which stack
it operates on, so `self.keyboard_mode_stack.len() >=
KEYBOARD_MODE_STACK_MAX_DEPTH` guards a `self.title_stack.remove(0)` --
against a stack this function never pushes to, so `title_stack` is still
empty at that point in every realistic case. `Vec::remove(0)` on an empty
`Vec` panics, unwinding the worker thread this workspace's
`labolabo-term::session::run_worker` (`crates/labolabo-term/src/
session.rs`) runs the VT core on, with no `catch_unwind` there -- so a pane
that triggers this doesn't just error, it freezes permanently (the worker
thread is gone; no more snapshots, no more input processing).

This is reachable in this workspace specifically because
`AlacrittyBackend::new` (`crates/labolabo-term/src/backend/alacritty.rs`)
sets `Config { kitty_keyboard: true, .. }` -- without that, `push_keyboard_
mode`'s own early-return (`if !self.config.kitty_keyboard { return; }`)
makes the buggy branch dead code, which is presumably why upstream hasn't
noticed: `kitty_keyboard` defaults to `false` and most embedders (including
Alacritty's own `alacritty` binary, historically) don't opt in. 4096
uninterrupted `CSI > <flags> u` pushes with no intervening pop (`CSI < u`)
-- a genuinely malformed/adversarial byte stream, but not one this crate's
`vte::ansi::Processor` rejects at parse time -- panics the worker on every
build in this workspace that reaches this backend, including the default
`backend-alacritty` feature and any release binary built from it.

Upstream status (checked 2026-07-18): the exact bug was already reported as
https://github.com/alacritty/alacritty/issues/8957 ("keyboard mode stack
panic", same copy-paste diagnosis) and **closed by the maintainers** with
"There's no point in trying to defend against a DOS from a malicious
application." -- i.e. upstream knows and declines to fix. Don't re-file a
duplicate; this vendored patch is expected to stay for as long as the
`backend-alacritty` feature ships with `kitty_keyboard: true`. Re-check
that issue (and the `title_stack.remove(0)` line, see below) when bumping
the pinned version, in case upstream changes their mind.

Everything else in this directory (`src/`, `Cargo.toml`, `LICENSE-APACHE`)
is the unmodified published crate source, copied verbatim from
`~/.cargo/registry/src/*/alacritty_terminal-0.26.0/`. Deliberately *not*
included: `Cargo.toml.orig`, `Cargo.lock`, `CHANGELOG.md`,
`.cargo_vcs_info.json`, `.cargo-ok`, and `tests/` (the crate's own `ref.rs`
integration test and its fixtures) -- none of these affect how `cargo`
resolves or builds this crate as a `path`-patched dependency of
`labolabo-term`, and `tests/` in particular is never built by this
workspace's own `cargo test --workspace` (or `-p labolabo-term`), since
this directory is a patched dependency, not a workspace member, so its own
test targets are out of scope for that invocation. The crate's own
`Cargo.toml` declares `license = "Apache-2.0"` (single-licensed, not the
dual MIT/Apache-2.0 pattern common elsewhere in the Rust ecosystem) and
ships only `LICENSE-APACHE` in the published tarball -- both carried over
here unchanged.

Update by re-vendoring `src/`, `Cargo.toml`, and `LICENSE-APACHE` from
`~/.cargo/registry/src/*/alacritty_terminal-<version>/` and re-applying the
same one-line fix (confirm the bug is still present upstream first --
`rg 'title_stack.remove\(0\)' src/term/mod.rs` inside the freshly-fetched
source -- it may have been fixed independently) when bumping the pinned
version in `Cargo.lock`. Drop this patch entirely once the fix lands
upstream.

---

<p align="center">
    <img width="200" alt="Alacritty Logo" src="https://raw.githubusercontent.com/alacritty/alacritty/master/extra/logo/compat/alacritty-term%2Bscanlines.png">
</p>

<h1 align="center">Alacritty - A fast, cross-platform, OpenGL terminal emulator</h1>

## About

Alacritty is a modern terminal emulator that comes with sensible defaults, but
allows for extensive configuration. By integrating with other
applications, rather than reimplementing their functionality, it manages to
provide a flexible set of features with high performance.
The supported platforms currently consist of BSD, Linux, macOS and Windows.

`alacritty_terminal` is the library crate within that project this
workspace depends on (the VT parser/grid core, not the GPU-rendered
`alacritty` binary itself) -- see `crates/labolabo-term/src/backend/
alacritty.rs`'s module doc comment for how it's used here.

## License

Alacritty is released under the [Apache License, Version 2.0](https://github.com/alacritty/alacritty/blob/master/LICENSE-APACHE).
