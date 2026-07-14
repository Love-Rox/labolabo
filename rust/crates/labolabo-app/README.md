# labolabo-app (Rust, gpui)

A [gpui](https://www.gpui.rs/) binary: the first UI layer of LaboLabo's Rust
cross-platform port. This is **wave 5a** — a bootable skeleton, not the
production terminal UI. It exists to prove the shape of the gpui <->
`labolabo-term` wiring (window, keyboard input, resize, event-driven
redraw, a minimal tab bar) before a real task/tile UI is built on top of it.

Not in the workspace's `default-members` (see `rust/Cargo.toml`): gpui is a
heavy desktop-UI dependency, and the existing `rust` CI job's fast,
toolchain-light `cargo build`/`test`/`clippy` at the workspace root must stay
that way. Build/test/lint this crate explicitly with `-p labolabo-app`; it
has its own CI job, `rust-app` (see `.github/workflows/ci.yml`).

## Running it

```sh
cd rust
cargo run -p labolabo-app
```

Opens one window, spawns a login-shell `TermSession` (backend-alacritty,
`labolabo-term`'s default feature) sized to the window, and renders it. Type
into it like a normal terminal. Click "+" to open another tab, click a tab's
title to switch to it, click its "×" to close it (see "Known limitations"
below for what closing a tab does *not* do yet).

To exercise the intended production VT backend instead
(`backend-ghostty-vt`, real `libghostty-vt` — needs a local Ghostty source
tree built with Zig 0.16; see `crates/labolabo-term/README.md` for the full
setup):

```sh
GHOSTTY_SOURCE_DIR=/path/to/ghostty-zig016-src \
  PATH="/path/to/zig-0.16:$PATH" \
  cargo run -p labolabo-app --no-default-features --features backend-ghostty-vt
```

(Not exercised in CI or during this wave's own verification — local-only,
same as `labolabo-term`'s own `rust-term-ghostty` job.)

## Design

### Module layout

| Module | Responsibility |
|---|---|
| `main.rs` | Entry point: opens the one window at a starting size. |
| `app.rs` | The gpui root view (`TerminalApp`): tab model, key/click routing, the redraw-bridge thread, the render tree. |
| `grid.rs` | Pure function: pixel area + cell size -> terminal column/row count. No gpui types — unit-tested without a gpui `Application`. |
| `keys.rs` | Pure function: `gpui::Keystroke` -> PTY input bytes. `Keystroke`/`Modifiers` are plain data, so this is unit-tested directly too. |
| `render.rs` | Paints one `labolabo_term::GridSnapshot` into a gpui canvas (background, glyphs, cursor). No session/tab state — snapshot in, paint calls out. |

### Keyboard input path

gpui delivers a `KeyDownEvent` (via `div::on_key_down`, on a focused,
`track_focus`-ed root div) -> `keys::keystroke_to_bytes` turns it into raw
bytes (pure function, see `grid.rs`/`keys.rs` unit tests) -> `TermSession::
write_input` writes them to the PTY. Handled: printable characters (via
gpui's own `key_char`), Enter/Backspace/Tab/Escape/Space, the four arrow
keys (CSI sequences), and a bare Ctrl-<letter> (C0 control codes,
Ctrl-A..Ctrl-Z). Cmd/Super combinations are swallowed (reserved for
future application-level shortcuts, e.g. Cmd-T/Cmd-W for tabs) rather than
forwarded to the terminal.

**Not implemented (TODO, see `keys.rs`'s module doc comment):**

- **IME composition.** gpui's `EntityInputHandler` (marked text, composition
  events) is not wired up. This means CJK input methods, dead-key
  compositions, and similar multi-keystroke-per-character input do not work
  — only single dispatched key-down events are handled. This is the
  headline gap for this wave; see "What was and wasn't verified" below.
- Delete (forward-delete)/Home/End/PageUp/PageDown/function keys.
- Ctrl combined with anything other than a single letter, and any
  Ctrl+Alt/Ctrl+Shift combination beyond "fall back to whatever `key_char`
  gpui reports".

### Resize path

`Context::observe_window_bounds` fires on window resize ->
`TerminalApp::handle_window_resized` re-derives the terminal grid size from
the new `Window::viewport_size()` via `grid::grid_size_for_window` (which
subtracts the tab bar's fixed height, then floor-divides by the fixed cell
size in `render.rs`) -> for every tab whose column/row count actually
changed, `TermSession::resize` is called (which resizes both the kernel PTY,
so full-screen programs see `SIGWINCH`, and the VT core's own grid). The
same `grid_size_for_window` function computes the *initial* grid at tab
creation, so there's exactly one place that answers "how big is a tab's
grid right now" — no separately-hardcoded initial column/row count to drift
out of sync with the resize path.

### Event-driven redraw (no polling)

`labolabo_term::TermSession` has no async event stream — `recv_event`
blocks the calling thread until a `TermEvent` arrives or a timeout elapses.
`app::spawn_redraw_bridge` reconciles that with gpui's async, `cx.notify()`-
driven redraw model per tab:

1. A dedicated OS thread blocks on `session.recv_event(EVENT_POLL_TIMEOUT)`
   in a loop and forwards each `TermEvent::Wakeup`/`Exit` over an unbounded
   `futures` channel.
2. A gpui `Task` (`cx.spawn`) owns the receiving end. It awaits the channel,
   drains any burst of already-queued wakeups into a single `cx.notify()`
   (so a flood of PTY output collapses into one redraw), then sleeps
   `FRAME_INTERVAL` (16ms, matching `labolabo_term::session`'s own snapshot
   throttle) before draining and notifying again.

This is the same two-stage "coalesce, then pace" design as the
`gpui-term-poc` spike's `spawn_redraw_task` (see `labolabo-spikes`), adapted
from an async-native event source (the spike's own `term_session.rs`, a
one-off session type built directly on a `futures::channel::mpsc` stream) to
`labolabo-term`'s blocking `recv_event` API — hence the extra
thread-to-channel bridge step. `Render::render` (and therefore the paint
work in `render.rs`) only ever re-runs from an actual `cx.notify()` call, so
there is no polling redraw loop and no idle CPU cost: an idle tab's bridge
thread sits blocked in `recv_event` doing no work until either real PTY
output arrives or the tab is closed.

`EVENT_POLL_TIMEOUT` (250ms) is *not* a redraw-cadence knob — real wakeups
are delivered immediately regardless of its value. It only bounds how
quickly a bridge thread notices its gpui `Task` was dropped (tab closed, so
no one is listening any more) and exits; see "Known limitations" below for
why this can't be instant without a `labolabo-term` API change.

### Tab bar

A row of `div`s above the terminal canvas: a title (click to switch) and a
"×" (click to close) per tab as *sibling* elements (not nested, on purpose —
gpui's click-hit-testing doesn't stop a parent's `on_click` from also firing
when a nested child inside its bounds is clicked, so overlapping hitboxes
were avoided by construction rather than needing a stop-propagation
workaround), plus a trailing "+" to open a new one.

**TODO(W5b):** the `Tab`/`tabs: Vec<Tab>` model in `app.rs` is a deliberately
minimal placeholder. `plans/012-task-model-and-control-cli.md` describes a
real task/tile model (`labolabo-core::tiling`, already ported from the
Swift `PaneTilingModel`) that a later wave will replace this window's tab
model with. Do not build further UI on top of `app::Tab` expecting it to
survive that replacement.

## Known limitations

- **No IME support** (see "Keyboard input path" above) — the biggest
  functional gap in this wave.
- **Closing a tab does not terminate its child process.**
  `labolabo_term::TermSession` exposes no "close/kill this session" API —
  by its own design, it's meant to run for the whole session's lifetime (see
  `crates/labolabo-term/README.md`). `close_tab` only removes the tab from
  this app's UI and lets the redraw-bridge thread wind down (within
  `EVENT_POLL_TIMEOUT`, once its `Task` is dropped); the underlying PTY
  reader/worker threads and the child shell process are **not** torn down —
  they linger until the child exits on its own or the app quits. No change
  was made to `labolabo-term` to add a teardown API for this, per this
  wave's brief ("keep `labolabo-term` changes minimal") — flagged instead as
  follow-up work for whenever real session lifecycle management lands
  alongside the tab-model replacement above.
- **Cell size and font are hardcoded** (`render::CELL_WIDTH`/`CELL_HEIGHT`,
  14pt Menlo) rather than measured from real font metrics or made
  configurable.
- **Per-cell text shaping.** `render.rs` shapes one `TextRun` per non-blank
  cell rather than batching same-style runs per row — simple and correct,
  not the most efficient; a reasonable follow-up if it ever shows up as a
  bottleneck (see the module doc comment).
- Linux gpui builds are not exercised by CI yet (`rust-app` is macOS-only —
  see `.github/workflows/ci.yml`'s comment on that job for why: gpui's Linux
  system-dependency footprint is heavier to provision than this wave wanted
  to take on).

## What was and wasn't verified

Verified locally this wave:

- `cargo build -p labolabo-app`, `cargo clippy -p labolabo-app --all-targets
  -- -D warnings`, and workspace-wide `cargo fmt --check` all pass.
- `cargo test -p labolabo-app` (the `grid`/`keys` pure-function unit tests)
  passes.
- Root `cargo build`/`cargo test`/`cargo clippy` (the workspace's
  `default-members`, unchanged) still pass and do not build gpui.
- `cargo run -p labolabo-app`, run for 5 seconds and killed: the window
  opens and the process does not crash or panic during that window.

**Not verified — no synthetic keyboard input was used anywhere in this
wave's development, on explicit instruction (`osascript keystroke`/
`cliclick t:`/`kp:` and similar are banned).** As a direct consequence:

- `keys::keystroke_to_bytes` has never been exercised against a real
  keypress in a running window — only its unit tests (hand-constructed
  `gpui::Keystroke` values) have run. The keystroke-name strings it matches
  (`"enter"`, `"backspace"`, `"up"`, ...) are gpui's own documented/observed
  key-name shape (cross-checked against the `gpui-term-poc` spike's
  `key_to_bytes`, which used the same strings), not independently confirmed
  against gpui's macOS platform layer for this crate.
- Manually typing into a `cargo run -p labolabo-app` window (no synthetic
  input) is the first thing to check if picking this up next — same caveat
  the `gpui-term-poc` spike's own README documents for its `key_to_bytes`.
- Window resizing (drag-to-resize) was not interactively exercised either;
  `grid::grid_size_for_window`'s math is unit-tested, and
  `Context::observe_window_bounds` wiring compiles and type-checks, but the
  end-to-end "drag the window edge, watch the grid follow" behavior has not
  been eyeballed.
- Tab click/switch/close and the "+" button have not been interactively
  exercised (would require mouse clicks on a live window, which the 5-second
  headless-ish smoke run does not do).
