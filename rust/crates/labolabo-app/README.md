# labolabo-app (Rust, gpui)

A [gpui](https://www.gpui.rs/) binary: the first UI layer of LaboLabo's Rust
cross-platform port. Wave 5a proved the shape of the gpui <-> `labolabo-term`
wiring (window, keyboard input, resize, event-driven redraw, the user's
Ghostty font *and color* settings) with a placeholder flat tab bar. **Wave
5b-2** replaces that placeholder with the real tile/tab tree
(`labolabo_core::tiling::PaneTilingModel`, ported from the Swift app's
`PaneTilingModel.swift`): split panes, each holding its own tab group, with
keyboard-driven split/tab/focus operations. Still not the full production
UI — the sidebar/Task list, the control CLI, drag & drop, and layout
persistence (`plans/012-task-model-and-control-cli.md`) are later waves'
scope.

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

Opens one window with a single terminal pane (backend-alacritty,
`labolabo-term`'s default feature), filling the window and rendered with the
font from your Ghostty config (see "Ghostty configuration" below). Type into
it like a normal terminal. Split it (Cmd+D / Cmd+Shift+D), add tabs to a pane
(Cmd+T or its "+"), switch panes/tabs by clicking or with the keybindings
below (see "Keybindings"). A tab whose shell exits (`exit`, Ctrl-D, crash)
closes itself; closing the tree's last tab — either way — quits the app,
like Ghostty.

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
| `main.rs` | Entry point: reads the Ghostty font config, registers the tile/tab keybindings (`cx.bind_keys`), opens the one window at a starting size. |
| `app.rs` | The gpui root view (`TerminalApp`): owns a `labolabo_core::tiling::PaneTilingModel` + one `PaneRuntime` (real `Terminal` session + redraw bridge) per terminal pane, key/click routing, the recursive split/tab-bar render tree, action handlers for every keybinding. |
| `focus.rs` | Pure tile-tree focus logic (gpui-independent, unit-tested): which pane to focus after a close, next/previous-pane cycling, Cmd+N tab lookup. See its module doc comment for the "focus is a `PaneId`, not a `NodeId`" invariant. |
| `ghostty_config.rs` | Pure-ish loader for the user's Ghostty config (`font-family`/`font-size`, `background`/`foreground`/`cursor-color`/`palette`/`theme`, `config-file` includes). Fixture-tested; never reads `$HOME` in tests. |
| `grid.rs` | Pure function: pixel area + cell size -> terminal column/row count. No gpui types — unit-tested without a gpui `Application`. |
| `keys.rs` | Pure function: `gpui::Keystroke` -> PTY input bytes. `Keystroke`/`Modifiers` are plain data, so this is unit-tested directly too. |
| `render.rs` | `RenderSpec` (font resolution + cell measurement) and painting one `labolabo_term::GridSnapshot` into a gpui canvas (background, glyphs, cursor). |

## Keybindings

Registered globally (`main.rs`'s `cx.bind_keys`) and dispatched to whichever
pane/tab is affected via `app.rs`'s action handlers. Cmd-modified keystrokes
never reach a terminal's own input — `keys::keystroke_to_bytes` reserves the
whole `platform` modifier for these, so there's no conflict with typing.

| Keys | Action |
|---|---|
| Cmd+T | New tab in the focused pane |
| Cmd+W | Close the focused pane's active tab (last tab in the tree quits the app) |
| Cmd+D | Split the focused pane right (new terminal, focus moves to it) |
| Cmd+Shift+D | Split the focused pane down (new terminal, focus moves to it) |
| Cmd+1 .. Cmd+9 | Select the Nth tab in the focused pane (no-op if it has fewer tabs) |
| Cmd+] | Focus the next pane (cycles leaves in tree order, wraps around) |
| Cmd+[ | Focus the previous pane (cycles leaves in tree order, wraps around) |

"Next/previous pane" cycles the tree's leaves in depth-first order rather
than true on-screen (left/right/up/down) adjacency — the simpler of the two
options the wave's brief allowed, and the one `focus::adjacent_pane` is
unit-tested against. Clicking a tab chip or a pane's terminal area also moves
focus there, and the focused pane's frame gets a subtle accent-colored
border. There is no keybinding (or UI) for divider drag-resize this wave —
see "Known limitations".

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
every pane's `Terminal::spawn_with_options` call (`app::TerminalApp::
spawn_runtime`) so the embedded terminal's default fg/bg/cursor/palette
match what the user configured. Color *values* only support `#rgb`/
`#rrggbb` hex (with or without the `#`) — see "Known limitations" for
what's deliberately unsupported.

### Keyboard input path

gpui delivers a `KeyDownEvent` (via `div::on_key_down`, on a focused,
`track_focus`-ed root div) -> `keys::keystroke_to_bytes` turns it into raw
bytes (pure function, see `grid.rs`/`keys.rs` unit tests) -> the focused
pane's `Terminal::write_input` writes them to its PTY. Handled: printable
characters (via gpui's own `key_char`), Enter/Backspace/Tab/Escape/Space,
the four arrow keys (CSI sequences), and a bare Ctrl-<letter> (C0 control
codes, Ctrl-A..Ctrl-Z). Cmd/Super combinations are never forwarded to a
terminal (reserved for application-level shortcuts — see "Keybindings"
above for what they're bound to as of this wave).

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

### Resize path (per pane)

Each leaf's terminal canvas is sized from its own laid-out on-screen area,
not the whole window — necessary once panes can be split into unequal
fractions of it. `render_leaf` builds each pane's canvas with a `prepaint`
closure that receives that canvas's actual `Bounds<Pixels>` (post-flex,
post-split-ratio — gpui has already done the layout math by the time
`prepaint` runs, so this module never reimplements it): `grid::
grid_size_for_area` turns that into a column/row count, and if it differs
from the pane's last known size (tracked in a small `Rc<Cell<(u16, u16)>>`
shared between repaints — `prepaint` runs outside any `&mut TerminalApp`
borrow, so this is the one piece of state it needs on its own), `Terminal::
resize` is called. This reacts uniformly to a window resize (`Context::
observe_window_bounds` just forces a fresh layout/paint pass via
`cx.notify()`) and to a split/tab-count change (a fresh tree shape lays out
differently the very next frame) without a separate code path for each.

The very first pane (`TerminalApp::new`) is sized from the full window
viewport up front (`grid::grid_size_for_window`, subtracting the tab bar's
height) so it doesn't have to wait a frame to reach its correct size; every
pane created afterward (new tab, split) starts at a fixed default (80x24)
and is corrected on the next frame by the same per-pane `prepaint` path.

**Not implemented:** interactive divider drag-resize. Split ratios come only
from where a leaf was created (always 0.5) — there is no keybinding or
mouse-drag UI to change a `TileNode::ratio` after the fact this wave (the
wave's keybinding list doesn't call for it, and it wasn't in scope). The
model itself already supports arbitrary ratios (persisted, clamped to
`[0.05, 0.95]`); wiring a divider drag up to it is a reasonable follow-up.

### Event-driven redraw (no polling)

`labolabo_term::Terminal` has no async event stream — `recv_event` blocks
the calling thread until a `TermEvent` arrives or a timeout elapses.
`app::spawn_redraw_bridge` reconciles that with gpui's async, `cx.notify()`-
driven redraw model per pane:

1. A dedicated OS thread blocks on `session.recv_event(EVENT_POLL_TIMEOUT)`
   in a loop and forwards each `TermEvent::Wakeup`/`Exit` as a `BridgeMsg`
   over an unbounded `futures` channel.
2. A gpui `Task` (`cx.spawn`) owns the receiving end. It awaits the channel,
   drains any burst of already-queued wakeups into a single `cx.notify()`
   (so a flood of PTY output collapses into one redraw), then sleeps
   `FRAME_INTERVAL` (16ms, matching `labolabo_term::session`'s own snapshot
   throttle) before draining and notifying again. An `Exit` seen at any
   point (awaited or drained) instead closes the pane
   (`TerminalApp::handle_pane_exit`) and ends the task.

This is the same two-stage "coalesce, then pace" design as the
`gpui-term-poc` spike's `spawn_redraw_task` (see `labolabo-spikes`), adapted
from an async-native event source (the spike's own `term_session.rs`, a
one-off session type built directly on a `futures::channel::mpsc` stream) to
`labolabo-term`'s blocking `recv_event` API — hence the extra
thread-to-channel bridge step. `Render::render` (and therefore the paint
work in `render.rs`) only ever re-runs from an actual `cx.notify()` call, so
there is no polling redraw loop and no idle CPU cost: an idle pane's bridge
thread sits blocked in `recv_event` doing no work until either real PTY
output arrives or the pane is closed.

`EVENT_POLL_TIMEOUT` (250ms) is *not* a redraw-cadence knob — real events
are delivered immediately regardless of its value. It only bounds how
quickly a bridge thread notices its gpui `Task` was dropped (pane closed, so
no one is listening any more) and exits.

### Tile tree, tab bars, and session lifecycle

`TerminalApp` owns one `labolabo_core::tiling::PaneTilingModel` (the same
tile/tab tree ported from the Swift app's `PaneTilingModel.swift`) and a
`PaneRuntime` (real `Terminal` session + redraw bridge) per `terminal`-kind
pane in the tree — including hidden (non-selected) tabs, so their pty and
scrollback survive tab switches and structural changes elsewhere in the
tree, matching the Swift app's behavior. `app::render_tile` recursively
turns a split node into a flex row/column sized by `node.ratio` and a leaf
node into `render_leaf`: a per-pane tab bar (chip per tab — click to select,
"×" to close — plus a trailing "+") above that pane's terminal canvas.

Focus (which pane's active tab receives keystrokes) is tracked as a single
`PaneId` on `TerminalApp`; see `focus.rs`'s module doc comment for why a
`PaneId` (not a leaf's `NodeId`, which doesn't survive a split) is the right
thing to track, and for the pure, unit-tested resolution logic
split/close/Cmd+]/Cmd+[/Cmd+1..9 all go through.

A tab closes two ways, both funneled through `TerminalApp::remove_pane`
(id-based, never by tree position — positions shift as the tree changes):

- **The shell exits**: the redraw bridge sees `TermEvent::Exit` and calls
  `handle_pane_exit(pane_id)` (no shutdown signal — the child is already
  dead).
- **A tab's "×", or Cmd+W** (the focused pane's active tab):
  `close_pane_user(pane_id)` first calls `Terminal::shutdown()` (SIGHUP to
  the child, the same signal a real terminal delivers on window close),
  then removes the pane.

Either way, when the tree's last pane's last tab is gone the app quits
(`cx.quit()`), matching Ghostty's close-last-surface behavior — the same
rule wave 5a's flat tab bar had, now decided from `model.panes().len() ==
1` instead of an empty `Vec<Tab>`.

**Not implemented this wave** (see `plans/012-task-model-and-control-cli.md`
for where these land in the product model): the sidebar/Task list, the
control CLI, drag & drop (moving a tab between panes by dragging), and
layout persistence (`PaneTilingModel::snapshot`/`apply` exist and are
tested in `labolabo-core`, but nothing in this crate calls them yet — every
window starts from a single terminal pane, and the tree is lost on quit).

## Known limitations

- **No interactive divider drag-resize** (see "Resize path" above) — split
  ratios are fixed at 0.5 for the life of the pane; there is no mouse-drag
  or keybinding to change them.
- **No drag & drop.** Moving a tab into another pane (merge) or splitting it
  out by dragging is a later wave, per `plans/012`; `PaneTilingModel::
  move_pane` (the underlying operation) is already ported and tested in
  `labolabo-core`, just not wired to any gesture here.
- **No layout persistence.** Every window starts from a single terminal
  pane; the tree (and any splits/tabs the user made) is not saved and is
  lost on quit. `PaneTilingModel::snapshot`/`::apply` (the serialize/restore
  API) exist and are tested, but nothing in this crate calls them yet.
- **"Next/previous pane" is DFS tree order, not geometric adjacency.**
  Cmd+]/Cmd+[ cycle `TileNode::leaves()` in depth-first order, not by
  on-screen position — the simplest option the wave's brief explicitly
  allowed. In a layout where DFS order doesn't match visual left-to-right/
  top-to-bottom order (e.g. after several splits), the cycle direction may
  feel surprising.
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

This section reflects wave 5b-2 (the tile/tab tree). See git history for
earlier waves' own verification notes (colors, ghostty-vt backend, IME
scope, etc. — unchanged by this wave and not re-verified here).

Verified locally:

- `cargo build -p labolabo-app`, `cargo clippy -p labolabo-app --all-targets
  -- -D warnings`, and workspace-wide `cargo fmt --check` all pass (default
  `backend-alacritty` feature; `backend-ghostty-vt` was not re-checked this
  wave — this change doesn't touch backend selection).
- `cargo test -p labolabo-app` (60 tests): `focus.rs`'s pure focus-resolution
  tests (close-time refocus, next/previous-pane DFS cycling with wraparound,
  Cmd+N tab lookup) plus the pre-existing `grid`/`keys`/`ghostty_config`
  tests, all still green.
- `cargo test -p labolabo-core` (165 lib tests + golden fixtures): the new
  `PaneTilingModel::add_tab` method's 3 tests, plus the full existing
  `tiling` suite (unaffected) and the `tiling_golden` fixture tests (byte-
  for-byte Swift-compatibility contract untouched — `add_tab` doesn't change
  `TileLayout`'s serialized shape).
- Root `cargo build`/`cargo test`/`cargo clippy -- -D warnings` (the
  workspace's `default-members`: `labolabo-core` + `labolabo-term`,
  unchanged) all pass and do not build gpui.
- `cargo run -p labolabo-app`, run for several seconds and killed: the
  window opens and the process does not crash, panic, or print anything to
  stderr.

**Not verified — no synthetic keyboard/mouse input and no window inspection
were used anywhere in this work, on explicit instruction.** This wave is
almost entirely interactive UI (split/tab/focus operations, click targets,
the focus border, per-pane resize-on-layout), so this is a large gap the
user needs to close by hand:

- **No keybinding has been exercised against a real keypress.** Cmd+T/W/D/
  Shift+D/]/[/1..9 are covered only by `focus.rs`'s pure-logic unit tests
  (which test the tree-resolution math, not gpui's key dispatch/keymap
  matching) and by reading gpui's own dispatch code (`dispatch_key_event`/
  `dispatch_action_on_node` in the vendored `gpui-0.2.2` source) to confirm
  a matched action keybinding consumes the keystroke before it would
  otherwise reach `on_key_down`/`keystroke_to_bytes` — not by pressing the
  keys.
- **No click has been exercised** (tab chip select/close, "+", clicking a
  pane's terminal area to focus it, the focused-pane border rendering
  correctly).
- **Split rendering/resizing has not been visually confirmed.** The
  `flex_row`/`flex_col` + `relative(ratio)` layout, the per-pane canvas
  `prepaint`-driven resize, and the focus border are all unverified beyond
  "compiles and the app doesn't crash with the single default pane" — no
  split was ever actually created and observed on screen.
- **Exit-closes-tab / last-pane-quits / "×"-closes-with-shutdown on the new
  tree model** were not interactively exercised (only `remove_pane`'s logic
  was read/reasoned through against the model's own tested `close`/`panes`
  behavior).
- Whether the tile tree's visual behavior (split proportions, tab bar
  layout, border highlight) matches the Swift app's *look* (not required by
  the brief, which only asks for matching modeling semantics) was not
  assessed at all.
