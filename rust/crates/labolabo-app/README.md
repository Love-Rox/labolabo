# labolabo-app (Rust, gpui)

A [gpui](https://www.gpui.rs/) binary: the first UI layer of LaboLabo's Rust
cross-platform port. Wave 5a proved the shape of the gpui <-> `labolabo-term`
wiring (window, keyboard input, resize, event-driven redraw, the user's
Ghostty font *and color* settings) with a placeholder flat tab bar. Wave
5b-2 replaced that placeholder with the real tile/tab tree
(`labolabo_core::tiling::PaneTilingModel`, ported from the Swift app's
`PaneTilingModel.swift`): split panes, each holding its own tab group, with
keyboard-driven split/tab/focus operations. **Wave 5b-3** layers the Task
model (`plans/012-task-model-and-control-cli.md` §1) on top: a left sidebar
lists Tasks grouped by repository, **one Task owns one tile/tab tree**, each
Task's terminal panes spawn in that Task's working directory (a dedicated
git worktree, or an "attached" existing directory), and Tasks + layouts
persist to a Rust-only SQLite database and are restored on relaunch. Still
not the full production UI — the control CLI (plan §2), drag & drop (plan
§3), and Task rename/done/archive are later waves' scope.

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

Opens one window: a Task sidebar on the left, and the selected Task's
tile/tab tree (backend-alacritty, `labolabo-term`'s default feature) filling
the rest, rendered with the font from your Ghostty config (see "Ghostty
configuration" below). On first launch there are no Tasks yet — use the
sidebar's "+ Attached" (pick any directory via the native OS directory
picker; work happens directly there) or "+ Worktree" (pick an existing git
repository checkout; a fresh branch `labolabo/<YYYYMMDD>-<n>` is generated
and `git worktree add`-ed under `<repo>/.worktrees/`, and the Task starts
there). Each Task starts with a single terminal pane in its working
directory; split it (Cmd+D / Cmd+Shift+D), add tabs (Cmd+T or a pane's "+"),
switch panes/tabs by clicking or with the keybindings below. Click a Task in
the sidebar to switch to it — the Task you switched away from keeps its
ptys/scrollback alive in memory, same semantics as switching tabs.

Quitting and relaunching restores every Task (sidebar entries + each one's
split/tab layout + which Task was selected) from the database; each restored
terminal pane gets a **fresh shell** in the Task's working directory (the
container is restored, not terminal scrollback or agent-session content —
that's future hooks-integration work).

### Where the data lives

`~/Library/Application Support/LaboLabo-rs/tasks.db` on macOS
(`$XDG_DATA_HOME/LaboLabo-rs/` on Linux, `%APPDATA%\LaboLabo-rs\` on
Windows) — `labolabo_core::store::TaskDatabase::default_path()`.
**Deliberately a different directory tree and file from the Swift app's**
`~/Library/Application Support/LaboLabo/labolabo.db`: the Rust port never
opens the Swift `SessionDatabase` (two live apps must never write the same
SQLite file, and this schema shares nothing with GRDB's) — see
`labolabo-core`'s `store::task_database` module docs for the full contract.

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
| `app.rs` | The gpui root view (`LaboLaboApp`): owns the `TaskDatabase`, the Task list, one `TaskWorkspace` per loaded Task, Task selection/persistence, the new-Task flows' orchestration, key routing, and the action handlers for every keybinding. |
| `task_workspace.rs` | One Task's live workspace: its `PaneTilingModel` + one `PaneRuntime` (real `Terminal` session + redraw bridge) per terminal pane, and the recursive split/tab-bar render tree (wave 5b-2's tree, made per-Task — every render/click path carries a `task_id`). |
| `sidebar.rs` | The Task sidebar: pure, unit-tested repo-grouping (`group_tasks_by_repo`) + minimal rendering (title + a one-glyph worktree/attached marker, "+ Attached"/"+ Worktree" buttons, error banner). |
| `new_task.rs` | The new-Task flows' git side (gpui-free, integration-tested against real temp repos): repo-identity resolution for attached Tasks, and branch-generation + `git worktree add` for worktree Tasks. |
| `focus.rs` | Pure tile-tree focus logic (gpui-independent, unit-tested): which pane to focus after a close, next/previous-pane cycling, Cmd+N tab lookup. See its module doc comment for the "focus is a `PaneId`, not a `NodeId`" invariant. |
| `ghostty_config.rs` | Pure-ish loader for the user's Ghostty config (`font-family`/`font-size`, `background`/`foreground`/`cursor-color`/`palette`/`theme`, `config-file` includes). Fixture-tested; never reads `$HOME` in tests. |
| `grid.rs` | Pure function: pixel area + cell size -> terminal column/row count. No gpui types — unit-tested without a gpui `Application`. |
| `keys.rs` | Pure function: `gpui::Keystroke` -> PTY input bytes. `Keystroke`/`Modifiers` are plain data, so this is unit-tested directly too. |
| `render.rs` | `RenderSpec` (font resolution + cell measurement) and painting one `labolabo_term::GridSnapshot` into a gpui canvas (background, glyphs, cursor). |

### The Task model (wave 5b-3)

Data model and persistence live in `labolabo-core` (usable later by the
control CLI without gpui): `store::Task` (`id` = UUID v4, repo identity from
`GitEngine::repo_info` — `repo_key` is the shared git dir, the sidebar's
grouping key —, `kind` = `Worktree { branch, base, path }` \|
`Attached { directory }`, `title`, `layout: TileLayout`, `status`
(active/done/archived — schema-level reservation, nothing transitions past
active yet), `created_at`/`last_active_at`, `sort_order`, and an
`agent_bindings` column reserved for the plan's per-tab agent-session
bindings), and `store::TaskDatabase` (rusqlite; own
`schemaMigrations`-ledger migrations, **no GRDB compatibility constraint** —
see its module docs). A Task's `layout` column stores the exact same
`TileLayout` JSON the tile tree has always serialized to (`TileLayout::
to_json`/`from_json`), so the tree round-trips byte-compatibly with
everything already tested in `labolabo-core`.

In this crate, `LaboLaboApp` holds `HashMap<TaskId, TaskWorkspace>`:
selecting a Task for the first time decodes its layout
(`PaneTilingModel::model_from`, falling back to a fresh single-terminal tree
if missing/corrupt) and spawns a `Terminal` per terminal-kind pane **in the
Task's working directory** (`Terminal::spawn_with_cwd_options`, added to
`labolabo-term` this wave); selecting it again just re-renders the kept
workspace. Layout persistence is save-on-mutation: every layout-affecting
action (add tab / split / close / select tab) snapshots and upserts the Task
row immediately (a single cheap SQLite upsert — simpler than, but satisfying
the same requirement as, the plan's "revision 変化で debounce 保存"
suggestion). The selected Task id persists under the `selectedTask`
app-state key.

A Task's **last** pane is special (the Task must never become pane-less and
unrecoverable, and Task deletion/done is out of scope this wave): a user
close ("×"/Cmd+W) is refused outright — the shell keeps running untouched —
unless it's also the app's only Task, which quits the app like wave 5b-2
did. If the last pane's shell **exits on its own** (`exit`, Ctrl-D, crash),
the pane stays in the tree with an empty canvas as a recoverable anchor:
its "+"/Cmd+T opens a fresh tab in the Task's cwd, after which the dead tab
closes normally. (Auto-respawning a shell into the dead pane was
deliberately avoided — an immediately-exiting shell would respawn-loop.)

## Keybindings

Registered globally (`main.rs`'s `cx.bind_keys`) and dispatched to the
**selected Task's** focused pane via `app.rs`'s action handlers.
Cmd-modified keystrokes never reach a terminal's own input —
`keys::keystroke_to_bytes` reserves the whole `platform` modifier for these,
so there's no conflict with typing.

| Keys | Action |
|---|---|
| Cmd+T | New tab in the focused pane |
| Cmd+W | Close the focused pane's active tab (refused for a Task's last pane, unless it's the app's only Task — then quits) |
| Cmd+D | Split the focused pane right (new terminal in the Task's working directory, focus moves to it) |
| Cmd+Shift+D | Split the focused pane down (same) |
| Cmd+1 .. Cmd+9 | Select the Nth tab in the focused pane (no-op if it has fewer tabs) |
| Cmd+] | Focus the next pane (cycles leaves in tree order, wraps around) |
| Cmd+[ | Focus the previous pane (cycles leaves in tree order, wraps around) |

"Next/previous pane" cycles the tree's leaves in depth-first order rather
than true on-screen (left/right/up/down) adjacency — the simpler of the two
options wave 5b-2's brief allowed, and the one `focus::adjacent_pane` is
unit-tested against. Clicking a tab chip or a pane's terminal area also moves
focus there, and the focused pane's frame gets a subtle accent-colored
border. There is no keybinding (or UI) for divider drag-resize —
see "Known limitations". There are no Task-switching keybindings yet (click
the sidebar).

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
every pane's `Terminal::spawn_with_cwd_options` call (`app::LaboLaboApp::
spawn_runtime_for_task`) so the embedded terminal's default fg/bg/cursor/
palette match what the user configured. Color *values* only support `#rgb`/
`#rrggbb` hex (with or without the `#`) — see "Known limitations" for
what's deliberately unsupported.

### Keyboard input path

gpui delivers a `KeyDownEvent` (via `div::on_key_down`, on a focused,
`track_focus`-ed root div) -> `keys::keystroke_to_bytes` turns it into raw
bytes (pure function, see `grid.rs`/`keys.rs` unit tests) -> the selected
Task's focused pane's `Terminal::write_input` writes them to its PTY.
Handled: printable characters (via gpui's own `key_char`),
Enter/Backspace/Tab/Escape/Space, the four arrow keys (CSI sequences), and a
bare Ctrl-<letter> (C0 control codes, Ctrl-A..Ctrl-Z). Cmd/Super
combinations are never forwarded to a terminal (reserved for
application-level shortcuts — see "Keybindings" above for what they're bound
to as of this wave).

**Not implemented (TODO, see `keys.rs`'s module doc comment):**

- **IME composition.** gpui's `EntityInputHandler` (marked text, composition
  events) is not wired up. This means CJK input methods, dead-key
  compositions, and similar multi-keystroke-per-character input do not work
  — only single dispatched key-down events are handled. This remains the
  headline input gap; see "What was and wasn't verified" below.
- Delete (forward-delete)/Home/End/PageUp/PageDown/function keys.
- Ctrl combined with anything other than a single letter, and any
  Ctrl+Alt/Ctrl+Shift combination beyond "fall back to whatever `key_char`
  gpui reports".

### Resize path (per pane)

Each leaf's terminal canvas is sized from its own laid-out on-screen area,
not the whole window — necessary once panes can be split into unequal
fractions of it (and now that a sidebar occupies part of the window).
`task_workspace::render_leaf` builds each pane's canvas with a `prepaint`
closure that receives that canvas's actual `Bounds<Pixels>` (post-flex,
post-split-ratio — gpui has already done the layout math by the time
`prepaint` runs, so this module never reimplements it): `grid::
grid_size_for_area` turns that into a column/row count, and if it differs
from the pane's last known size (tracked in a small `Rc<Cell<(u16, u16)>>`
shared between repaints — `prepaint` runs outside any `&mut LaboLaboApp`
borrow, so this is the one piece of state it needs on its own), `Terminal::
resize` is called. This reacts uniformly to a window resize (`Context::
observe_window_bounds` just forces a fresh layout/paint pass via
`cx.notify()`), to a split/tab-count change, and to a Task switch (the newly
shown tree lays out and self-corrects the very next frame) without a
separate code path for each.

The launch-restored Task's panes are sized from the window viewport (minus
the sidebar width and tab-bar height) up front; panes created afterward (new
tab, split, a new or lazily-loaded Task) start at a fixed default (80x24)
and are corrected on the next frame by the same per-pane `prepaint` path.

**Not implemented:** interactive divider drag-resize. Split ratios come only
from where a leaf was created (always 0.5) — there is no keybinding or
mouse-drag UI to change a `TileNode::ratio` after the fact (unchanged from
wave 5b-2; ratios *do* persist through the Task's `TileLayout` now). The
sidebar width is likewise a fixed constant (`sidebar::SIDEBAR_WIDTH`).

### Event-driven redraw (no polling)

`labolabo_term::Terminal` has no async event stream — `recv_event` blocks
the calling thread until a `TermEvent` arrives or a timeout elapses.
`task_workspace::spawn_redraw_bridge` reconciles that with gpui's async,
`cx.notify()`-driven redraw model per pane:

1. A dedicated OS thread blocks on `session.recv_event(EVENT_POLL_TIMEOUT)`
   in a loop and forwards each `TermEvent::Wakeup`/`Exit` as a `BridgeMsg`
   over an unbounded `futures` channel.
2. A gpui `Task` (`cx.spawn`) owns the receiving end. It awaits the channel,
   drains any burst of already-queued wakeups into a single `cx.notify()`
   (so a flood of PTY output collapses into one redraw), then sleeps
   `FRAME_INTERVAL` (16ms, matching `labolabo_term::session`'s own snapshot
   throttle) before draining and notifying again. An `Exit` seen at any
   point (awaited or drained) instead closes the pane
   (`LaboLaboApp::handle_pane_exit(task_id, pane_id)`) and ends the task.

This is the same two-stage "coalesce, then pace" design as the
`gpui-term-poc` spike's `spawn_redraw_task` (see `labolabo-spikes`), adapted
from an async-native event source to `labolabo-term`'s blocking `recv_event`
API — hence the extra thread-to-channel bridge step. `Render::render` only
ever re-runs from an actual `cx.notify()` call, so there is no polling
redraw loop and no idle CPU cost. Note that a **hidden** Task's panes still
notify (their bridges don't know they're offscreen) — each notify is cheap
(the hidden tree isn't rendered), but suppressing them for unselected Tasks
is a reasonable follow-up if idle cost ever shows up.

`EVENT_POLL_TIMEOUT` (250ms) is *not* a redraw-cadence knob — real events
are delivered immediately regardless of its value. It only bounds how
quickly a bridge thread notices its gpui `Task` was dropped (pane closed, so
no one is listening any more) and exits.

### Tile tree, tab bars, and session lifecycle

Each `TaskWorkspace` owns one `labolabo_core::tiling::PaneTilingModel` (the
same tile/tab tree ported from the Swift app's `PaneTilingModel.swift`) and
a `PaneRuntime` (real `Terminal` session + redraw bridge) per
`terminal`-kind pane in the tree — including hidden (non-selected) tabs
*and* every pane of unselected Tasks, so pty and scrollback survive tab
switches, structural changes elsewhere in the tree, and Task switches.
`task_workspace::render_tile` recursively turns a split node into a flex
row/column sized by `node.ratio` and a leaf node into `render_leaf`: a
per-pane tab bar (chip per tab — click to select, "×" to close — plus a
trailing "+") above that pane's terminal canvas.

Focus (which pane's active tab receives keystrokes) is tracked as a single
`PaneId` per `TaskWorkspace`; see `focus.rs`'s module doc comment for why a
`PaneId` (not a leaf's `NodeId`, which doesn't survive a split) is the right
thing to track, and for the pure, unit-tested resolution logic
split/close/Cmd+]/Cmd+[/Cmd+1..9 all go through.

A tab closes two ways, both funneled through `LaboLaboApp::remove_pane`
(id-based, never by tree position — positions shift as the tree changes):

- **The shell exits**: the redraw bridge sees `TermEvent::Exit` and calls
  `handle_pane_exit(task_id, pane_id)` (no shutdown signal — the child is
  already dead).
- **A tab's "×", or Cmd+W** (the focused pane's active tab):
  `close_pane_user(task_id, pane_id)` first calls `Terminal::shutdown()`
  (SIGHUP to the child, the same signal a real terminal delivers on window
  close), then removes the pane.

A Task's **last** pane is handled specially (see "The Task model" above for
the full rules): user close refused / natural exit leaves a recoverable
empty pane / app's-only-Task quits (`cx.quit()`) — Ghostty's
close-last-surface behavior, scoped to the degenerate one-Task case now
that Tasks outlive their panes' sessions.

**Not implemented this wave** (see `plans/012-task-model-and-control-cli.md`
for where these land in the product model): the control CLI (§2), drag &
drop (§3 — pane/tab DnD, sidebar reordering, OS file drops), Task
rename/done/archive (§1's completion flow), and per-tab agent-session
bindings (`agent_bindings` is a reserved column only).

## Known limitations

- **No Task rename/done/archive/delete.** The schema has `status`, but the
  UI never transitions it; a created Task stays in the sidebar until a
  future wave adds lifecycle UI. (Deleting the row by hand from `tasks.db`
  works in a pinch — the app only reads it at launch.)
- **The "+ Worktree" flow has no branch/base input.** The branch name is
  always auto-generated (`labolabo/<YYYYMMDD>-<n>`), the base is the repo's
  current branch (fallback `main`), and the worktree always lands under
  `<repo>/.worktrees/` — the plan's fuller "new Task" dialog is future work.
- **No repo registry.** "+ Worktree" asks for a repository directory every
  time via the OS picker; the plan's "registered repositories" notion (and
  reinterpreting "open folder" as registration) isn't built yet.
- **Restore does not resume agent sessions or terminal content.** Fresh
  shells in the right directories, with the right splits/tabs — nothing
  else. Per-tab `--resume` (the Swift app's tab-resume behavior) needs the
  hooks integration and `agent_bindings`, both future work.
- **Keyboard focus placement is not persisted.** After a restart, a Task's
  focus defaults to its first leaf's selected tab.
- **No interactive divider drag-resize**, and the sidebar width is fixed
  (see "Resize path" above).
- **No drag & drop.** Moving a tab into another pane (merge) or splitting it
  out by dragging is a later wave, per `plans/012` §3; `PaneTilingModel::
  move_pane` (the underlying operation) is already ported and tested in
  `labolabo-core`, just not wired to any gesture here.
- **"Next/previous pane" is DFS tree order, not geometric adjacency.**
  Cmd+]/Cmd+[ cycle `TileNode::leaves()` in depth-first order, not by
  on-screen position — the simplest option wave 5b-2's brief explicitly
  allowed. In a layout where DFS order doesn't match visual left-to-right/
  top-to-bottom order (e.g. after several splits), the cycle direction may
  feel surprising.
- **No IME support** (see "Keyboard input path" above) — the biggest
  functional gap, carried since wave 5a.
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

This section reflects wave 5b-3 (the Task model). See git history for
earlier waves' own verification notes (colors, ghostty-vt backend, IME
scope, the tile/tab tree itself — unchanged mechanics not re-verified
here beyond compiling and their existing tests staying green).

Verified locally:

- `cargo build -p labolabo-app`, `cargo clippy -p labolabo-app --all-targets
  -- -D warnings`, and workspace-wide `cargo fmt --check` all pass (default
  `backend-alacritty` feature; `backend-ghostty-vt` was **not** re-checked
  this wave — no Zig 0.16 Ghostty tree was available on the machine this
  was developed on; the change to `labolabo-term` is backend-independent
  session-layer code).
- `cargo test -p labolabo-app` (67 tests): the new `sidebar` grouping tests
  and `new_task` integration tests (real temp git repos: worktree creation
  on a generated branch, repo-identity fallback for non-repo directories),
  plus all pre-existing `focus`/`grid`/`keys`/`ghostty_config` tests.
- `cargo test -p labolabo-core` (186 lib tests + goldens): the new
  `store::task_record`/`store::task_database` suites (CRUD, migration-ledger
  idempotence, `TileLayout` JSON round-trip through the DB, on-disk
  reopen persistence, malformed-value error surfacing) and `branch_naming`
  tests, plus the full pre-existing suite (tiling goldens' byte-for-byte
  Swift-compatibility contract untouched).
- `cargo test -p labolabo-term` (12 tests): the new
  `spawn_with_cwd_options` tests (cwd reaches the child's `pwd`; `None`
  matches the old behavior) plus the pre-existing suite (alacritty backend).
- Root `cargo build`/`cargo test`/`cargo clippy -- -D warnings` (the
  workspace's `default-members`: `labolabo-core` + `labolabo-term`) all pass
  and do not build gpui.
- `cargo run -p labolabo-app`, run for ~6 seconds and killed: the window
  opens, the process does not crash or panic, and
  `~/Library/Application Support/LaboLabo-rs/tasks.db` is created with the
  expected schema (verified with `sqlite3 .schema`) and migration-ledger row.

**Not verified — no synthetic keyboard/mouse input and no window inspection
were used anywhere in this work, on explicit instruction.** The Task model
is largely interactive UI, so the user needs to close these by hand:

- **Neither "+ Attached" nor "+ Worktree" has been clicked.** The OS
  directory picker (`cx.prompt_for_paths`), the async flow around it
  (`cx.spawn` + `background_spawn`), and the end-to-end "picked directory ->
  Task appears in sidebar, selected, shell opens there" path are exercised
  only at the `new_task.rs` layer (real git operations, tested) — not
  through the UI.
- **Task switching via sidebar clicks has not been exercised** (pty
  preservation across switches is by-construction — runtimes are never
  dropped on switch — but not observed live).
- **Restart restoration has not been end-to-end verified**: the DB
  round-trip (layout JSON in/out, selected task) is covered by
  `labolabo-core`'s tests, and launch-time code paths compile and don't
  crash with an empty DB, but "create Tasks, split some panes, quit,
  relaunch, see the same sidebar + layouts + cwds" was not performed (needs
  real UI interaction to create the Tasks first).
- **No keybinding has been exercised against a real keypress**, and no
  click on tab chips/"×"/"+" — same gap as wave 5b-2, now with the extra
  `task_id` routing layer in those handlers.
- The per-Task `persist_workspace` save-on-mutation path is unit-covered on
  the DB side but its UI triggering (split -> row updated) was not observed
  live.
