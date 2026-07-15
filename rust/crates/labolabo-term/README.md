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
- **Color configuration is opt-in via `spawn_with_options(cols, rows,
  command, env, &ColorScheme)`** — `spawn`/`spawn_with_command` are thin
  wrappers passing `ColorScheme::default()` (every backend's own built-in
  colors, unchanged). `ColorScheme` (`src/color.rs`) carries optional
  foreground/background/cursor overrides plus `(index, Rgb)` palette
  entries; `VtBackend::new` takes it and each backend applies it to its own
  VT core (alacritty: an in-crate 256-color table built from `ANSI_16` +
  the standard xterm cube/grayscale ramp, with the scheme's overrides
  applied on top).
- **The child's working directory is opt-in via
  `spawn_with_cwd_options(cols, rows, command, env, &ColorScheme, cwd:
  Option<&Path>)`** — the fullest entry point; `spawn_with_options` is a
  thin wrapper passing `cwd: None` (the child inherits this process's own
  working directory, same as before this option existed). This is what
  `labolabo-app`'s Task model (`plans/012-task-model-and-control-cli.md`
  §1) uses to spawn a Task's panes inside that Task's worktree/attached
  directory rather than wherever the app process happens to be running
  from.
  layered on top; ghostty: `libghostty-vt`'s own
  `set_default_fg_color`/`set_default_bg_color`/`set_default_cursor_color`/
  `set_default_color_palette` setters) — see `backend/alacritty.rs` and
  `backend/ghostty.rs` for the exact mapping, and that ghostty file's
  comment on `set_default_fg_color`/`set_default_bg_color` for a real
  upstream quirk (must be set together, never just one) this crate works
  around. `labolabo-app`'s `ghostty_config.rs` is the intended source of a
  real `ColorScheme` (the user's own Ghostty config); this crate has no
  opinion on where one comes from.
- **Explicit teardown via `shutdown()`** (added for the gpui shell's
  close-tab action): signals the child through `portable-pty`'s
  `ChildKiller` (SIGHUP on Unix — what a real terminal sends on window
  close). There is no separate teardown state machine: the dying child
  closes the PTY slave, the reader sees EOF, and the session ends through
  the same final-snapshot + `TermEvent::Exit` path as a natural exit.
  Idempotent; see the method docs for the (inherited) stale-pid caveat and
  the fact that only the direct child is signalled, not its descendants.
- **Scrollback via `TermSession::scroll`/`scroll_to_bottom` +
  `GridSnapshot::{scroll_offset, scrollback_len}`** (added for the gpui
  shell's trackpad/wheel scrolling and text selection over history):
  `VtBackend` gained `scroll_display(delta_lines: i64)`,
  `scroll_to_bottom()`, and `alt_screen_active() -> bool`. Both backends
  already have native scrollback + viewport support -- **the "fall back to
  our own N-line ring buffer" design this feature's brief flagged as a
  possibility was not needed**: `alacritty_terminal::Grid` has
  `display_offset`/`scroll_display(Scroll::Delta/Top/Bottom)` built in, and
  `libghostty-vt`'s `Terminal::scroll_viewport(ScrollViewport::Delta/Top/
  Bottom)` + `Terminal::scrollbar()` turned out to be an equally complete,
  independently-discovered native API (confirmed by reading
  `libghostty-vt-0.2.0/src/terminal.rs` directly, not assumed) -- so both
  backends wrap their own native mechanism instead.
  - **Sign convention, unified across both backends and
    `GridSnapshot::scroll_offset`**: positive `delta_lines` scrolls *up*,
    into history; negative scrolls *down*, toward the live tail;
    `scroll_offset` is `0` at the live tail and increases toward
    `scrollback_len` (fully scrolled back). This matches
    `alacritty_terminal`'s own native `Scroll::Delta` convention directly
    (verified against its `Grid::scroll_display`/`Term::scroll_to_point`
    source) and is also the convention real Ghostty's own apprt layer
    normalizes trackpad/wheel deltas to before touching its VT core
    (verified against the vendored Ghostty source's `Surface.zig`
    `ScrollAmount` doc comment, "positive is up, right", and
    `SurfaceView_AppKit.swift`'s unmodified forwarding of
    `NSEvent.scrollingDeltaY`) -- so `labolabo-app`'s wheel handler can feed
    gpui's raw platform delta straight through with no sign flip.
    `libghostty-vt`'s own `ScrollViewport::Delta` is the *opposite*
    convention ("up is negative", its own doc comment) and its
    `Scrollbar { total, offset, len }` reports offset from the *top* of
    scrollback (`0` = fully scrolled back) rather than from the live tail
    -- `backend/ghostty.rs` negates the delta and re-derives
    `scroll_offset`/`scrollback_len` from `total`/`offset`/`len` internally,
    so nothing above this crate ever needs to know the two backends
    disagree.
  - **History size defaults to 1000 lines, and is now caller-configurable**
    (`DEFAULT_MAX_SCROLLBACK` in `session.rs`; `VtBackend::new`'s
    `max_scrollback: usize` parameter, threaded through by both backends'
    `Config.scrolling_history` / `TerminalOptions.max_scrollback`). The
    spike's M3 milestone measured `scrolling_history: 10_000` (alacritty's
    own default) costing ~21% lower steady-state throughput than 1000, with
    0 and 1000 performing about the same (see `labolabo-spikes/term-poc/
    README.md`, "M3: frame-pacing + throughput efficiency" ->
    "Throughput efficiency: `scrolling_history`"). 1000 stays the default
    for exactly that reason, but `TermSession::spawn_with_scrollback_options`
    (added for `labolabo-app`'s Cmd+, settings screen, `plans` wave 5i §3)
    lets a caller opt into a different cap per spawn -- every pre-existing
    `spawn_*` entry point (`spawn_with_command`/`spawn_with_options`/
    `spawn_with_cwd_options`) is unchanged and still funnels down to
    `DEFAULT_MAX_SCROLLBACK`, so this is purely additive: a caller who never
    heard of the new method sees identical behavior to before it existed.
  - **Alt screen has no scrollback of its own on either backend**
    (alacritty: `Term::new`'s inactive/alternate grid is constructed with
    `max_scroll_limit: 0`; ghostty-vt's alternate screen is not part of the
    primary screen's scrollback page list) -- `scroll_display` is a
    harmless no-op while `alt_screen_active()` is true, on both backends,
    without either needing special-case code for it.
  - **`GridSnapshot`'s row-index math already generalizes**: `display_iter`
    (alacritty) / the row iterator (ghostty-vt, described in its own docs
    as tracking "a visible screen (a viewport), which changes when
    scrolled") both already iterate whatever the *current* viewport is;
    `build_snapshot` re-bases their absolute/viewport-relative line numbers
    into a plain `0..rows` grid the same way regardless of `scroll_offset`,
    so no separate "scrolled" vs. "live" code path exists in either
    backend's cell-walk.
  - **Not independently verified against real `libghostty-vt`**: this
    development machine has Zig 0.15.2 only (the `backend-ghostty-vt`
    feature needs 0.16, see "Building the ghostty-vt backend" below), so
    `backend/ghostty.rs`'s scroll/alt-screen/scrollbar code is reviewed
    carefully against the vendored crate's own doc comments and source
    (not guessed), and compiles logically against its API shape, but has
    not been built or run. Flagged for whoever next has the 0.16 toolchain
    available -- `tests/backend_common.rs`'s scrollback/alt-screen tests
    are written backend-agnostically specifically so they're ready to run
    the moment that's possible.
  - **`Terminal::scrollbar()` cost caveat, inherited from libghostty-vt's
    own doc comment**: "may be expensive to calculate depending on where
    the viewport is (arbitrary pins are expensive)." Called once per
    `build_snapshot` (already throttled to `FRAME_INTERVAL`, ~60fps, by
    `session.rs` -- never per PTY byte), which should be fine at this
    crate's scale, but is a real, vendor-flagged cost worth remembering if
    scrollback ever needs to scale beyond the current 1000-line cap.
  - **A worker-thread throttle bug found and fixed while building this
    feature**: `run_worker`'s snapshot-publish throttle (`FRAME_INTERVAL`)
    previously had a real gap -- if an entire burst of PTY output landed
    within one `FRAME_INTERVAL` window (routine: anything that prints a lot
    at once, since the reader thread's `read()`s typically return
    microseconds apart) and the child then went quiet (idled at a prompt,
    inside a long `sleep`, ...), the burst's *last* `Bytes` message could
    hit the "already snapshotted within this frame" branch and skip
    publishing -- and since nothing else was arriving to trigger a
    recheck, that final, truest state of the burst would never reach a
    published `GridSnapshot` until *something else* eventually nudged it
    (a keypress, a resize, or the child finally exiting). Found via this
    feature's own tests (a fast 40-line flood followed by an idle `sleep`,
    to leave the session alive long enough to scroll) reliably hanging;
    root-caused with temporary tracing before being fixed, not
    guessed. Fixed by tracking a `dirty` flag across throttled messages and
    switching the worker's blocking `rx.recv()` to a bounded
    `rx.recv_timeout()` once something is pending, so the worker wakes
    itself up and force-publishes right when the throttle window ends, even
    with zero new incoming messages -- see `run_worker`'s doc comments for
    the full mechanism. An idle session (no pending change) still blocks
    on a plain `recv()`, so this costs nothing when nothing is happening.
  - **Alt-screen scroll-to-cursor-keys translation lives in `labolabo-app`,
    not here** (`app::LaboLaboApp::handle_pane_scroll`) -- this crate only
    exposes `alt_screen_active()` as a plain query; converting a wheel
    delta into `ESC[A`/`ESC[B` sequences is an input-handling policy, the
    same layer `keys.rs`'s keystroke-to-bytes translation already lives in,
    not a VT-core concern.
- **Bracketed-paste mode query via `TermSession::bracketed_paste()`**
  (added for the gpui shell's Cmd+V paste handler, `labolabo-app`'s
  `app::LaboLaboApp::action_paste`): a small `bracketed_paste(&self) ->
  bool` addition to `VtBackend`, reporting whether the foreground program
  has enabled DECSET `2004` (`alacritty_terminal::term::TermMode::
  BRACKETED_PASTE`; `libghostty_vt::terminal::Mode::BRACKETED_PASTE` --
  identical semantics on both backends). Like `GridSnapshot`, this is
  backend state that lives on the worker thread; the flag is mirrored into
  a plain `AtomicBool` the caller thread reads without blocking, refreshed
  by the worker after every processed PTY byte batch (not throttled to the
  snapshot cadence -- it's a single cheap bool, and a paste can land at any
  time). Covered by a shared (`tests/backend_common.rs`) headless test on
  both backends: `printf '\033[?2004h'`/`...l` in a spawned shell toggles
  `bracketed_paste()`.

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
the child ends; and (via `spawn_with_options`) a configured `background`
shows up as `GridSnapshot::background`, a configured `foreground` shows up
as the fg color of an unstyled cell, and a `palette` override shows up as
the fg color of an SGR-colored cell.

Also covered, backend-agnostically: a fresh session reports `scroll_offset`/
`scrollback_len` of `0`/`0`; flooding more lines than fit the viewport then
scrolling back reveals a line that had scrolled off (and `scroll_to_bottom`
returns to the live tail); an oversized `scroll` delta clamps to
`scrollback_len` (and an oversized negative one clamps to `0`) rather than
panicking or drifting; `alt_screen_active()` reflects DECSET `1049`
(entering/leaving the alternate screen, the mode `vim`/`less`/`htop` use);
and (via `spawn_with_scrollback_options`) a small explicit `max_scrollback`
is accepted and reaches the VT core. On `backend-alacritty` this is a tight
assertion (`Grid::update_history` trims synchronously and exactly, so
`scrollback_len` lands precisely at the configured cap after a flood); on
`backend-ghostty-vt` the same test asserts a weaker "spawns, floods, and
stays readable without erroring" contract instead -- CI's `rust-term-ghostty`
job caught a stricter version of this assertion failing there (the
pagelist reclaims scrollback in coarse page-sized chunks rather than
trimming to an exact line count after a small burst), so see the test's own
doc comment (`tests/backend_common.rs`) for what's actually verified per
backend, rather than assuming both match.

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
