# labolabo-term (Rust)

The cross-platform **terminal-session core** for LaboLabo: a real PTY
(`portable-pty`) driving a VT parser on a background worker thread, exposing a
UI-independent [`GridSnapshot`] (cell text, resolved fg/bg RGB, style flags,
cursor) plus a wakeup/exit event channel. **No UI dependency** — rendering is
the future `labolabo-ui`'s job; this crate only produces snapshots.

Distilled from the `term-poc` spike in
[`labolabo-spikes`](../../../../labolabo-spikes) (M1–M6), with the spike's two
one-off session structs (`term_session.rs` = alacritty, `ghostty_session.rs` =
libghostty-vt) refactored into one shared machine plus a small backend trait.

## Backends

The VT core is pluggable behind the `VtBackend` trait. Exactly one backend is
active per build; `ActiveBackend`/`Terminal` resolve to it, so call sites (and
the tests) name no backend.

| Feature | Backend | Default? | Needs |
|---|---|---|---|
| `backend-ghostty-vt` | `libghostty-vt` (real Ghostty VT engine) | no (opt-in) | `GHOSTTY_SOURCE_DIR` + Zig 0.16 |
| `backend-alacritty` | `alacritty_terminal` | **yes** | crates.io only |

**`backend-ghostty-vt` is the intended production backend** — it is the same
VT engine the macOS app embeds. It is *not* the default only because building
it requires a local Ghostty source tree compiled with Zig 0.16 (see below), a
heavyweight and currently fork-pinned external dependency we keep off the
always-on CI path. `backend-alacritty` is a parallel implementation that
resolves entirely from crates.io, so a plain `cargo test` and the standing
`rust` CI job are always green with no extra toolchain. It is kept honest by
running the **same** integration tests as ghostty (`tests/backend_common.rs`).

## Architecture

```
                 caller thread                worker thread            reader thread
  TermSession ── write_input ─────────────▶ PTY writer (mutex)
              ── resize ───────┐
                               ▼
                        WorkerMsg channel ◀── PTY read chunks ◀── tight blocking read() loop
                               │
                          VtBackend (VT core, owned here)
                               │  feed / resize / build_snapshot
                               ▼
                     latest GridSnapshot  +  TermEvent::{Wakeup,Exit}
```

- **PTY is unified on `portable-pty` for both backends.** The alacritty
  backend does **not** use `alacritty_terminal`'s bundled `tty::EventLoop`
  (the spike's M1–M5 path); instead it feeds bytes straight into a
  `vte::ansi::Processor`→`Term`, so the entire PTY/spawn/thread layer is
  shared verbatim with ghostty. See "PTY unification" below.
- **Reader and snapshot are decoupled** (the spike's M6 bug #2 fix, now
  structural): a dedicated reader thread does a tight, never-throttled
  blocking `read()`; the worker throttles only *snapshot construction* to
  ~60fps. Bytes and resize requests share one channel so a resize is applied
  promptly even with no PTY output flowing.
- **`env` injection is first-class** in `spawn_with_command(cols, rows,
  command, env)` — the mechanism LaboLabo's hooks protocol uses to tag a pane
  (`LABOLABO_PANE`, `LABOLABO_TASK`, …) for the spawned agent.
- **Explicit teardown via `shutdown()`** (added for the gpui shell's
  close-tab action): signals the child through `portable-pty`'s
  `ChildKiller` (SIGHUP on Unix — what a real terminal sends on window
  close). There is no separate teardown state machine: the dying child
  closes the PTY slave, the reader sees EOF, and the session ends through
  the same final-snapshot + `TermEvent::Exit` path as a natural exit.
  Idempotent; see the method docs for the (inherited) stale-pid caveat and
  the fact that only the direct child is signalled, not its descendants.

### PTY unification (design decision)

The brief asked whether the alacritty backend could also go through
`portable-pty` rather than its bundled `tty` module. **Yes, and it does.**
`alacritty_terminal::Term<T: EventListener>` implements `vte::ansi::Handler`,
so bytes can be driven directly with `Processor::advance(&mut term, bytes)`,
bypassing `tty::EventLoop` entirely. VT *responses* (device-status reports,
cursor-position queries) surface as `Event::PtyWrite` through the `Term`'s
`EventListener`, which we forward to the shared PTY writer — the same role
ghostty's `Terminal::on_pty_write` plays. Net result: one `portable-pty`
spawn/read/resize/thread layer for both backends; the only backend-specific
code is the ~100-line VT-core slice (`feed` / `resize` / `build_snapshot` +
color mapping). Reusing `EventLoop` would instead have required reimplementing
alacritty's private `EventedPty` trait (SIGCHLD tracking, fd registration for
`polling`) on top of a `portable-pty` master — substantial and fragile — for
no benefit, since we don't need its child-signal machinery.

## Testing

Headless integration tests (`tests/backend_common.rs`) — no window required:

```sh
# default (alacritty), always green:
cargo test -p labolabo-term

# ghostty-vt (needs the toolchain below):
GHOSTTY_SOURCE_DIR=/path/to/ghostty-zig016-src \
  PATH="/path/to/zig-0.16:$PATH" \
  cargo test -p labolabo-term --no-default-features --features backend-ghostty-vt
```

The same test file runs on both backends: `spawn_with_command("echo hello &&
sleep 0.2")` → snapshot contains `hello`; injected `$LABOLABO_PANE` reaches the
child; resize changes the reported grid dimensions; the `Exit` event fires when
the child ends.

## Building the ghostty-vt backend

`libghostty-vt-sys`'s `build.rs` builds a vendored `libghostty-vt.a` from a
Ghostty source tree via Zig. Point it at a checkout and put the right Zig on
`PATH`:

- `GHOSTTY_SOURCE_DIR` → a Ghostty tree that builds under **Zig 0.16**. Upstream
  `ghostty-org/ghostty` still requires the broken-here Zig 0.15.2; the working
  tree today is the fork `vancluever/ghostty`'s `zig-0.16` branch (the draft PR
  [ghostty-org/ghostty#12726](https://github.com/ghostty-org/ghostty/pull/12726),
  "Update to Zig 0.16.0"). **This is a fork-pinned dependency: re-pin
  `GHOSTTY_SOURCE_DIR` to upstream once #12726 (or equivalent) merges.**
- Zig **0.16.0** on `PATH`.

### Build traps carried over from the spike

1. **`LIBGHOSTTY_VT_SYS_OPTIMIZE=ReleaseFast` is mandatory.**
   `libghostty-vt-sys`'s `build.rs` otherwise picks the *Zig* optimize mode
   from cargo's `DEBUG` env var, defaulting to Zig `Debug` whenever
   `DEBUG=true` — which cargo reports for any profile carrying debug symbols,
   so the native VT parser silently builds **unoptimized** even under
   `--release`. In the spike this was a ~20000× throughput cliff (`yes | pv`
   ~4KiB/s vs. ~90MiB/s) — a broken build, not a slow one. This repo pins the
   value in [`rust/.cargo/config.toml`](../../.cargo/config.toml) `[env]`
   (overridable), so the trap can't be hit by accident from the workspace.
2. **Reads must stay tight; throttle only snapshots.** Folding a per-frame
   sleep into the read loop caps throughput at `pty_buffer / frame_interval`.
   Here the reader thread and the snapshot throttle are separate by
   construction (see Architecture).
3. **`libghostty-vt-sys`'s `bindings.rs` is a checked-in static artifact**, not
   regenerated from `GHOSTTY_SOURCE_DIR`. Building against a Ghostty commit
   newer than those bindings risks a silent C-ABI mismatch. If a future change
   starts calling libghostty-vt functions this crate doesn't use today,
   regenerate/diff bindings against the pinned source first (the spike found
   one genuine by-value→by-pointer ABI break in `ghostty_color_rgb_get`, which
   nothing on our path calls). The `gen-bindings` tool in `libghostty-vt-sys`
   does this.
