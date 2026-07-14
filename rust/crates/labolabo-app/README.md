# labolabo-app (Rust, gpui)

A [gpui](https://www.gpui.rs/) binary: the first UI layer of LaboLabo's Rust
cross-platform port. This is **wave 5a/5b** — a bootable skeleton, not the
production terminal UI. It exists to prove the shape of the gpui <->
`labolabo-term` wiring (window, keyboard input, resize, event-driven
redraw, a minimal tab bar, the user's Ghostty font *and color* settings)
before a real task/tile UI is built on top of it.

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
`labolabo-term`'s default feature) sized to the window, and renders it with
the font from your Ghostty config (see "Ghostty configuration" below). Type
into it like a normal terminal. Click "+" to open another tab, click a tab's
title to switch to it, click its "×" to close it. A tab whose shell exits
(`exit`, Ctrl-D, crash) closes itself; closing the last tab — either way —
quits the app, like Ghostty.

To exercise the intended production VT backend instead
(`backend-ghostty-vt`, real `libghostty-vt` — needs a local Ghostty source
tree built with Zig 0.16; see `crates/labolabo-term/README.md` for the full
setup):

```sh
GHOSTTY_SOURCE_DIR=/path/to/ghostty-zig016-src \
  PATH="/path/to/zig-0.16:$PATH" \
  cargo run -p labolabo-app --no-default-features --features backend-ghostty-vt
```

(Not exercised in CI — local-only, same as `labolabo-term`'s own
`rust-term-ghostty` job.)

## Design

### Module layout

| Module | Responsibility |
|---|---|
| `main.rs` | Entry point: reads the Ghostty font config, opens the one window at a starting size. |
| `app.rs` | The gpui root view (`TerminalApp`): tab model, key/click routing, the redraw-bridge thread, session-exit handling, the render tree. |
| `ghostty_config.rs` | Pure-ish loader for the user's Ghostty config (`font-family`/`font-size`, `background`/`foreground`/`cursor-color`/`palette`/`theme`, `config-file` includes). Fixture-tested; never reads `$HOME` in tests. |
| `grid.rs` | Pure function: pixel area + cell size -> terminal column/row count. No gpui types — unit-tested without a gpui `Application`. |
| `keys.rs` | Pure function: `gpui::Keystroke` -> PTY input bytes. `Keystroke`/`Modifiers` are plain data, so this is unit-tested directly too. |
| `render.rs` | `RenderSpec` (font resolution + cell measurement) and painting one `labolabo_term::GridSnapshot` into a gpui canvas (background, glyphs, cursor). |

### Ghostty configuration (font-family / font-size / colors)

At startup, `ghostty_config::load_user_font_config` and `::
load_user_color_config` read the user's existing Ghostty configuration so
the embedded terminal matches their normal Ghostty look — same idea as the
Swift app's `GhosttyConfig.swift`, but as a (partial) parser instead of
handing the file to libghostty. The loading rules are ported from the
actual Ghostty source (`ghostty-zig016-src`, upstream PR #12726 state) and
are documented key-by-key with source references in `ghostty_config.rs`'s
module docs. The short version:

- **Files, in load order (later wins)**: `$XDG_CONFIG_HOME/ghostty/config`
  (legacy) then `.../config.ghostty` (Ghostty >= 1.3.0), then on macOS
  `~/Library/Application Support/com.mitchellh.ghostty/config` and
  `.../config.ghostty` on top. All that exist are loaded (matching
  `Config.loadDefaultFiles`); empty root files are skipped like upstream.
- **Line syntax** matches Ghostty's `LineIterator`: trim, `#` full-line
  comments only, split at the first `=`, one quote layer stripped from the
  value.
- **`config-file` includes** are processed *after* all root files, in
  order, recursively (queue semantics, cycle-safe), `?` marks an include
  optional, relative paths resolve against the including file, `~/`
  against home — matching `Config.loadRecursiveFiles` + `path.zig`.
- **Keys read**: `font-family` (repeatable; empty value resets the list),
  `font-size` (float, last valid wins), `background`/`foreground`/
  `cursor-color` (scalar, last *valid* value wins), `palette` (repeatable
  `N=COLOR`, one index per occurrence), and `theme`. Everything else is
  skipped.

Font resolution + measurement (`render::RenderSpec::resolve`): the first
`font-family` entry that exists on the system (case-insensitive match
against `TextSystem::all_font_names`) becomes the terminal font; if none
resolve (or none are configured) it falls back to Menlo with a stderr
warning. The cell box is **measured**, not hardcoded: a reference glyph is
shaped through gpui's text system and its advance width / line ascent +
descent become `cell_width`/`cell_height` (ceil-rounded so cell backgrounds
tile without hairline gaps), used by both the renderer and the grid-size
math. gpui 0.2 exposes no public line-gap metric, so rows can be slightly
tighter than Ghostty.app's for fonts with a non-zero line gap.

Color resolution (`ghostty_config::extract_color_config`, full rationale +
source references in the module doc comment): a `theme = NAME` value (if
any) is resolved to a theme file (absolute path, else searched for in the
user's own `ghostty/themes` dir, then a best-effort macOS guess at
Ghostty.app's bundled themes) and loaded as a color *baseline*; the user's
own explicit `background`/`foreground`/`cursor-color`/`palette` settings
then override it field-by-field (and, for `palette`, index-by-index) —
matching upstream's documented "additional colors... override the colors
specified in the theme" rule, regardless of where `theme = ` appears in the
user's own files. The result is a `labolabo_term::ColorScheme`, passed to
every tab's `Terminal::spawn_with_options` call
(`app::TerminalApp::open_tab`) so the embedded terminal's default
fg/bg/cursor/palette match what the user configured. Color *values* only
support `#rgb`/`#rrggbb` hex (with or without the `#`) — see "Known
limitations" for what's deliberately unsupported.

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
subtracts the tab bar's fixed height, then floor-divides by the *measured*
cell size in `RenderSpec`) -> for every tab whose column/row count actually
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
   in a loop and forwards each `TermEvent::Wakeup`/`Exit` as a `BridgeMsg`
   over an unbounded `futures` channel.
2. A gpui `Task` (`cx.spawn`) owns the receiving end. It awaits the channel,
   drains any burst of already-queued wakeups into a single `cx.notify()`
   (so a flood of PTY output collapses into one redraw), then sleeps
   `FRAME_INTERVAL` (16ms, matching `labolabo_term::session`'s own snapshot
   throttle) before draining and notifying again. An `Exit` seen at any
   point (awaited or drained) instead closes the tab
   (`TerminalApp::handle_session_exit`) and ends the task.

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

`EVENT_POLL_TIMEOUT` (250ms) is *not* a redraw-cadence knob — real events
are delivered immediately regardless of its value. It only bounds how
quickly a bridge thread notices its gpui `Task` was dropped (tab closed, so
no one is listening any more) and exits.

### Tab bar and session lifecycle

A row of `div`s above the terminal canvas: a title (click to switch) and a
"×" (click to close) per tab as *sibling* elements (not nested, on purpose —
gpui's click-hit-testing doesn't stop a parent's `on_click` from also firing
when a nested child inside its bounds is clicked, so overlapping hitboxes
were avoided by construction rather than needing a stop-propagation
workaround), plus a trailing "+" to open a new one.

A tab closes two ways, both funneled through id-based removal (never by
index — indices shift as tabs come and go):

- **The shell exits**: the redraw bridge sees `TermEvent::Exit` and calls
  `handle_session_exit(tab_id)`.
- **"×" click**: `close_tab(tab_id)` first calls `TermSession::shutdown()`
  (added to `labolabo-term` this wave: SIGHUP to the child, the same signal
  a real terminal delivers on window close; the session then winds down
  through its normal exit path), then removes the tab.

Either way, when the last tab is gone the app quits (`cx.quit()`), matching
Ghostty's close-last-surface behavior.

**TODO(W5b):** the `Tab`/`tabs: Vec<Tab>` model in `app.rs` is a deliberately
minimal placeholder. `plans/012-task-model-and-control-cli.md` describes a
real task/tile model (`labolabo-core::tiling`, already ported from the
Swift `PaneTilingModel`) that a later wave will replace this window's tab
model with. Do not build further UI on top of `app::Tab` expecting it to
survive that replacement.

## Known limitations

- **No IME support** (see "Keyboard input path" above) — the biggest
  functional gap in this wave.
- **Colors: light/dark theme switching is out of scope.** Ghostty's `theme
  = light:NAME,dark:NAME` syntax picks a theme based on the desktop
  appearance; this port only ever resolves the **light** side and has no
  appearance-tracking logic at all (see `ghostty_config.rs`'s module doc
  comment, "Scope limitation"). A config that relies on the dark variant
  will render with the light theme's colors regardless of the window's
  actual appearance.
- **Colors: unsupported color value syntax is skipped, not approximated.**
  Only `#rgb`/`#rrggbb` hex (with or without the leading `#`) is parsed;
  Ghostty's `rgb:`/`rgbi:` device syntax, the 12-/16-bit-per-channel forms,
  and the ~750 X11 named colors are reported (stderr) and left as whatever
  the value was before, same as an unparseable `font-size`. A scan of all
  463 themes bundled with a real Ghostty.app install found none of these
  forms in use, so this covers every built-in theme; a hand-written config
  using an X11 color name (e.g. `background = "royal blue"`) would need to
  switch to hex to be picked up here.
- **Colors: the macOS theme-resources-dir lookup is a hardcoded guess.**
  `theme = NAME` search order matches upstream (user's own `ghostty/themes`
  dir, then the app bundle's bundled themes), but the second location is
  hardcoded to `/Applications/Ghostty.app/Contents/Resources/ghostty/themes`
  rather than resolved via a real bundle/LaunchServices lookup — a
  differently-installed Ghostty.app (a non-`/Applications` location) won't
  be found for *bundled* (not user-authored) themes.
- **Cursor color tints, rather than recolors, the existing overlay.**
  `render::paint_cursor`'s block-cursor is still a translucent overlay (not
  a solid block with an inverted glyph, as some terminals draw it) — a
  configured `ColorScheme::cursor` (via `CursorSnapshot::color`) changes the
  overlay's tint at the same alpha as before; an unconfigured cursor renders
  exactly as it did before this wave (hardcoded translucent white).
- **`shutdown` signals the shell, not its descendants.** `TermSession::
  shutdown` SIGHUPs the direct child (the shell); a descendant process that
  detaches from the PTY and ignores SIGHUP can outlive the tab, same as in
  other terminal emulators.
- **Per-cell text shaping.** `render.rs` shapes one `TextRun` per non-blank
  cell rather than batching same-style runs per row — simple and correct,
  not the most efficient; a reasonable follow-up if it ever shows up as a
  bottleneck (see the module doc comment).
- **Font availability matching is exact-name.** `RenderSpec::resolve`
  matches `font-family` case-insensitively against installed family names;
  Ghostty's own font discovery is fuzzier, so a family Ghostty finds under a
  variant name may fall back to Menlo here (a stderr warning says so).
- Linux gpui builds are not exercised by CI yet (`rust-app` is macOS-only —
  see `.github/workflows/ci.yml`'s comment on that job for why: gpui's Linux
  system-dependency footprint is heavier to provision than this wave wanted
  to take on).

## What was and wasn't verified

Verified locally:

- `cargo build -p labolabo-app`, `cargo clippy -p labolabo-app --all-targets
  -- -D warnings`, and workspace-wide `cargo fmt --check` all pass, on
  **both** backends (`--no-default-features --features backend-ghostty-vt`
  too, with `GHOSTTY_SOURCE_DIR`/Zig 0.16 on `PATH`).
- `cargo test -p labolabo-app`: the `grid`/`keys` pure-function unit tests
  plus the `ghostty_config` parser/loader tests -- font *and* color/theme
  extraction, all fixture-tree-based (`fixtures/ghostty_config/`; no test
  touches `$HOME`, `/Applications`, or the real user config/theme files).
- `cargo test -p labolabo-term` on **both** backends (alacritty and
  ghostty-vt), including the new `shutdown` integration tests
  (`shutdown_kills_child_and_fires_exit`,
  `shutdown_is_idempotent_and_safe_after_natural_exit`) and the new color
  integration tests (`colors_background_override_reflected_in_snapshot`,
  `colors_foreground_override_reflected_in_unstyled_cell`,
  `colors_palette_override_reflected_in_sgr_colored_cell`,
  `default_color_scheme_matches_spawn_with_command`) -- headless, real PTY
  children whose SGR-colored/unstyled output is asserted against the
  extracted `GridSnapshot`.
- While building this wave's color support, the headless ghostty-vt
  integration tests caught a real upstream quirk before it shipped:
  `libghostty-vt`'s `RenderState.update` only resolves the effective
  background/foreground pair when **both** are set at the `Terminal` level
  (a labeled-block `orelse break` in `terminal/render.zig` bails on the
  *whole pair* if either is left unset) -- worked around in
  `backend/ghostty.rs` by always configuring both, falling back to this
  crate's own default constants for whichever side a `ColorScheme` leaves
  unconfigured (see that file's comment for the exact upstream source
  quoted).
- Root `cargo build`/`cargo test`/`cargo clippy` (the workspace's
  `default-members`, unchanged) still pass and do not build gpui.
- `cargo run -p labolabo-app`, run for ~5 seconds and killed, on **both**
  backends: the window opens and the process does not crash or panic, with
  a real user Ghostty config present (`font-family` resolved without a
  fallback warning; no color/theme parsing warnings on stderr against a
  config using a user-authored theme file with bare-hex colors).

**Not verified — no synthetic keyboard/mouse input and no window
inspection were used anywhere in this work, on explicit instruction
(`osascript keystroke`/`cliclick t:`/`kp:` and similar are banned).** As a
direct consequence:

- **The actual on-screen colors have not been visually confirmed.** The
  parsing/merging/backend-plumbing is unit- and integration-tested (see
  above), and the headless ghostty-vt tests assert on real resolved cell
  colors, but "does the running window actually look like my Ghostty" is
  the user's call -- this is the single biggest thing to check by hand.

- `keys::keystroke_to_bytes` has never been exercised against a real
  keypress by the author — only its unit tests (hand-constructed
  `gpui::Keystroke` values) have run. (The user has since exercised basic
  typing interactively and reported it working.)
- The **look** of the configured font (correct family/size/metrics on a
  real screen) has not been visually confirmed by the author — the config
  loader and measurement are unit-tested/compiled, but "does it look like
  my Ghostty" is the user's call.
- Exit-closes-tab / last-tab-quits and "×"-closes-with-shutdown were not
  interactively exercised by the author (mouse clicks on a live window);
  the underlying `shutdown`/Exit-event path *is* covered headlessly by the
  `labolabo-term` integration tests.
- Window drag-resizing was not interactively exercised; the grid math is
  unit-tested and the `observe_window_bounds` wiring compiles.
