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
persist to a Rust-only SQLite database and are restored on relaunch. **Wave
5c** adds Claude Code hooks integration (`docs/hooks-protocol.md`): agent
status dots (tab chip + sidebar row), per-tab Claude session memory, and
resume-at-restore — see "Claude Code hooks integration" below. **This wave**
adds the control CLI (`plans/012-task-model-and-control-cli.md` §2,
`docs/control-protocol.md`): the `labolabo` binary and a second, separate
control socket let scripts/agents/the app's own Claude sessions open tabs,
list Tasks/tabs, and switch focus from outside the gpui process — see
"Control CLI" below. Still not the full production UI — drag & drop (plan
§3) and Task rename/done/archive are later waves' scope.

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
terminal pane spawns a fresh Claude session unless it previously observed one
via hooks, in which case it spawns `claude --resume <id>` directly instead
(gated on the recorded transcript still existing — see "Claude Code hooks
integration" below). The container (splits, tabs, cwd) is always restored;
scrollback itself is not (a fresh PTY either way).

### Where the data lives

`~/Library/Application Support/LaboLabo-rs/tasks.db` on macOS
(`$XDG_DATA_HOME/LaboLabo-rs/` on Linux, `%APPDATA%\LaboLabo-rs\` on
Windows) — `labolabo_core::store::TaskDatabase::default_path()`.
**Deliberately a different directory tree and file from the Swift app's**
`~/Library/Application Support/LaboLabo/labolabo.db`: the Rust port never
opens the Swift `SessionDatabase` (two live apps must never write the same
SQLite file, and this schema shares nothing with GRDB's) — see
`labolabo-core`'s `store::task_database` module docs for the full contract.

### Smoke runs: always isolate the data directory

Launching against the real database is not a harmless read: every restored
Task spawns shells in — and, since wave 5c, **injects Claude Code hooks
into** — that Task's real working directory. An exploratory "does it start"
run must therefore never see the real `tasks.db`. Set
`LABOLABO_RS_DATA_DIR` (developer escape hatch, honored by
`labolabo_core::store::rust_app_data_dir`; empty value = unset) to a scratch
directory:

```sh
LABOLABO_RS_DATA_DIR=$(mktemp -d) timeout 5 cargo run -p labolabo-app
```

(macOS ships no `timeout`; use coreutils' `gtimeout`, or just quit the app
by hand.) Prefer quitting gracefully (window close / the last pane's Cmd+W
quit path) over `kill -9`: hooks cleanup (`HookRuntime::restore_all`, which
puts every injected `.claude/settings.local.json` back) runs from
`on_app_quit` and never gets a chance under SIGKILL.

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
| `hooks.rs` | Claude Code hooks integration (wave 5c): the app-wide `AgentStatusBus`, `.claude/settings.local.json` injection/restore, and the `LABOLABO_PANE` routing table. See "Claude Code hooks integration" below. |
| `control.rs` | Control CLI wiring: `ControlRuntime` (the app-wide control socket/server) and the gpui bridge that routes each request through a `WindowHandle` into `LaboLaboApp::dispatch_control` (`app.rs`). See "Control CLI" below. |
| `bin/labolabo.rs` | The `labolabo` CLI binary — a thin client for the control socket (argv parsing, `ControlRequest` construction, printing the response). See "Control CLI" below. |
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

### Claude Code hooks integration (wave 5c)

Implements `docs/hooks-protocol.md` (the canonical wire spec, checked in at
the repo root) end to end: hooks injection, the AF_UNIX bus, agent status
display, per-tab session memory, and resume-at-restore. Ported from the
Swift app's `app/Sources/AgentSessionModel.swift`/`HookForwarder.swift` at
the logic level (`labolabo-core`'s `hook_settings`/`hooks`/`tiling`/
`store::agent_bindings` modules — pure, unit-tested), with the app-layer
wiring (`labolabo-app`'s `hooks.rs` + `app.rs`) making one deliberate
architectural change from Swift, detailed below.

**One shared socket per app process, not one per session.** Swift runs a
dedicated `AgentStatusBus`/socket per `RepoSession` (1 worktree = 1 socket).
This port instead starts exactly one `AgentStatusBus` for the whole
`labolabo-app` process (`hooks::HookRuntime::new`, called once in
`LaboLaboApp::new`) and routes every incoming event to the right
`(task_id, PaneId)` purely via the `LABOLABO_PANE`/`LABOLABO_TASK` env vars
injected at pane-spawn time (`docs/hooks-protocol.md` §7; `LABOLABO_TASK` is
new in this port — `plans/012` §1's reserved `labolabo_task_id` wire field).
Every injected directory's hook `command` therefore points at the *same*
socket path regardless of which Task it belongs to. This sidesteps
`plans/012` §1's flagged "同一 cwd の複数 Task と hooks の衝突" design
question entirely (there is only ever one socket to route through, so two
Tasks sharing a directory no longer implies two competing sockets) at the
cost of a global (not per-Task) routing table, which `hooks::HookRuntime`
owns (`register_pane`/`unregister_pane`/`resolve_pane`, updated at pane
spawn/close in `app::LaboLaboApp::spawn_runtime_for_task`/`remove_pane`).

**Injection** (`hooks::HookRuntime::ensure_injected`, called from
`ensure_workspace_loaded` before a Task's panes spawn, idempotent per
directory per process run): merges LaboLabo's hook entry into the Task's
`.claude/settings.local.json` for all 7 events
(`labolabo_core::hook_settings::HOOK_EVENTS`), preserving any existing
entries (including another LaboLabo instance's or another tool's) exactly
like the Swift app does. The forwarder binary is resolved as the sibling of
the running `labolabo-app` executable (`labolabo-core`'s `labolabo-hook` bin
target) — if it isn't there (e.g. `cargo run -p labolabo-app` without ever
building `labolabo-core`'s bin targets), injection is skipped for the whole
run with one stderr warning, not per directory. **Restore** happens at app
quit (`cx.on_app_quit` in `LaboLaboApp::new`, which — unlike the plain
`gpui::App::on_app_quit` — hands the closure `&mut LaboLaboApp` directly, so
`HookRuntime::restore_all` needs no separate shared-ownership handle): a
directory LaboLabo created the settings file for gets it deleted; a
directory that had a real prior file gets it restored from the
`.labolabo-bak` snapshot `ensure_injected` wrote. A stale backup found at
injection time (a previous crash) is restored-then-re-snapshotted first, so
double-injection can't happen across restarts.

**Status display**: `TaskWorkspace::pane_status: HashMap<PaneId, AgentStatus>`
is updated by `LaboLaboApp::handle_agent_event` (the sink
`hooks::spawn_agent_event_bridge` dispatches every bus event to, mirroring
`task_workspace::spawn_redraw_bridge`'s OS-thread → channel → gpui-task
pattern) and rendered as a small colored dot — deliberately minimal, per the
wave's brief, not the Swift sidebar's `PhaseAnimator` ping/breathing-halo
treatment (`task_workspace::status_dot_color`) — on each tab chip and, as
the highest-priority status across a Task's panes
(`LaboLaboApp::task_agent_status`), on its sidebar row.

**Session memory**: an event carrying `session_id` updates two records, both
keyed off the routed pane/Task:

- **Per-tab** (docs/hooks-protocol.md §6(b)): `PaneTilingModel::
  record_agent_session` sets the routed `PaneItem`'s `agent_session_id`/
  `agent_transcript_path`, which already round-trips through the Task's
  `TileLayout` (`layout` column) — this is the primary mechanism, unchanged
  from the tiling port done in an earlier wave.
- **Task-level fallback** (docs/hooks-protocol.md §6(a)):
  `labolabo_core::AgentBindings` (JSON in the reserved `Task::
  agent_bindings` column) records the last-seen `(session_id,
  transcript_path)` for the Task as a whole, resolved via the event's own
  `labolabo_task_id` if present (validated against still-known Tasks) or
  else the routed pane's Task. This is **not currently consulted at
  restore** — restore only reads the per-tab record, which is populated
  from the very first hooks event a fresh Rust-app Task ever sees, so there
  is no "old data with no per-tab record" case to fall back for (unlike
  Swift, which grew per-tab tracking after per-session tracking already
  existed). It's kept for the docs' own (a) semantics and as a
  ready-to-use Task-level record for a future control-CLI/introspection use.

**Resume at restore** (`app::LaboLaboApp::spawn_runtime_for_task`): for each
terminal pane about to spawn, if it already carries a persisted
`agent_session_id` and `PaneItem::is_resumable` says its transcript is
either unrecorded (old data) or actually present on disk, the pane is
spawned with `claude --resume '<id>'` (`labolabo_core::
claude_resume_command`) as its command instead of a login shell. **This
differs from the Swift app's tab-resume behavior**: Swift always spawns a
plain shell first and then *types* the resume command into it once ready
(`PaneTilingModel.sendToTerminal`), so the pane still has a live shell
prompt after Claude exits; this port execs `claude --resume` **directly**
(`Terminal::spawn_with_cwd_options`'s `command: Some(...)` path is `sh -c
<command>`, no login shell), which is simpler and race-free (no "was the
shell ready yet" timing to get right) but means the pane's PTY exits along
with `claude` when the session ends, rather than dropping back to a shell.
Documented here per the porting brief's "どちらにしたか README に明記".

**Known limitation — same-directory concurrent use.** Two LaboLabo
instances (Rust and Rust, or Rust and the Swift app) with a Task/session
open on the *same* directory at the *same* time is not fully safe: both
independently merge their own hook entry into the shared
`settings.local.json` (harmless — merges append, never overwrite each
other's entries), but the `.labolabo-bak` snapshot/restore dance assumes
single-writer semantics for that one file, and there is a real race if both
processes inject or restore for that directory concurrently (e.g. one
process's restore-at-quit racing the other's crash-recovery
restore-then-reinject at startup). **Running the Rust and Swift apps (or
two Rust instances) on the same worktree/attached directory at the same
time is not recommended** — this matches `plans/012` §1's own "要設計" note
on the same-cwd-multiple-Tasks case, which remains only partially resolved
(the *socket* collision is solved by this wave's one-socket-per-process
design; the *settings-file* backup race is not).

### Control CLI

Implements `docs/control-protocol.md` (the canonical wire spec, checked in
at the repo root) end to end: a second AF_UNIX socket (separate from the
hooks socket above — `control_protocol::control_socket_path_from_uuid`,
`/tmp/labolabo/control-<10hex>.sock`), the `labolabo` CLI binary, and the
`tab_open`/`task_list`/`tab_list`/`focus` commands. The flagship use case
(`plans/012` §2): a Claude session running inside a LaboLabo pane opens a
teammate as a new tab in its own Task with
`labolabo tab open --title reviewer -- claude ...`, with no `--task` flag
needed (resolved from the ambient `LABOLABO_TASK` env var LaboLabo injects
into every pane it spawns, alongside the pre-existing `LABOLABO_PANE`).

**Same accept-loop-on-a-dedicated-thread shape as the hooks bus, but
bidirectional.** `labolabo_core::control::ControlServer` mirrors
`hooks::UnixSocketEventTransport`'s bind/chmod/accept-loop/`stop()`
structure, but each connection gets a response written back before it
closes (docs/control-protocol.md §3: "書いて half-close → 読む"). The actual
Task/tab mutation has to happen on the gpui main thread, so
`labolabo-app::control::ControlRuntime`'s handler hands each request off
over a channel and blocks (`std::sync::mpsc`, 15s timeout) for the reply —
`control::spawn_control_bridge` is the gpui-side task that receives
requests and calls `LaboLaboApp::dispatch_control` via a `WindowHandle`
(not the plain `WeakEntity` update the hooks bridge uses — command handlers
like `open_tab_for_control`/`select_task` need a live `&mut Window`).

**`tab_open` reuses the exact UI "+"-button code path**
(`LaboLaboApp::open_tab_for_control`, which `add_tab_to` now also calls):
env injection (`LABOLABO_PANE`/`LABOLABO_TASK`/`LABOLABO_CONTROL_SOCKET`,
the last one new this wave), hooks routing-table registration, and layout
persistence are identical whether the tab came from a click or a CLI
request — this is also the enforcement mechanism for
`docs/control-protocol.md` §2's "no invisible execution" invariant: there
is no code path that spawns a pane without also adding it to the visible
tile/tab tree.

**The CLI is a separate small binary in this same package** (`labolabo`,
`src/bin/labolabo.rs`) rather than living in `labolabo-core` like
`labolabo-hook` does — see that file's module doc comment for the
trade-off this implies (it pulls in the package's `gpui` dependency at
*build* time, though the linker doesn't include unreferenced gpui code in
the produced binary). `cargo run -p labolabo-app` (bare) still launches the
gpui app (`default-run = "labolabo-app"` in `Cargo.toml`, needed once a
second bin target existed); use `cargo run -p labolabo-app --bin labolabo
-- ...` (or the built `target/debug/labolabo` binary directly) for the CLI.

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
for where these land in the product model): drag & drop (§3 — pane/tab DnD,
sidebar reordering, OS file drops), and Task rename/done/archive (§1's
completion flow). Claude Code hooks integration (agent status, per-tab
session memory, resume-at-restore) landed in wave 5c — see "Claude Code
hooks integration" above; the control CLI (§2 — `labolabo tab open`/
`task list`/`tab list`/`focus`) landed this wave — see "Control CLI" above.
`task new` (§2's own scope note) and exposing the same RPC as an MCP server
remain reserved/future work (docs/control-protocol.md §5.5).

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
- **Restore resumes Claude sessions per tab (wave 5c), not terminal
  scrollback.** A pane with a previously-observed Claude session (and an
  existing or unrecorded transcript) spawns `claude --resume` directly
  instead of a shell on restore -- see "Claude Code hooks integration"
  above for how this differs from the Swift app's type-into-a-running-shell
  approach. Raw terminal scrollback itself is never restored (a fresh PTY
  either way).
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

### Wave 5c (Claude Code hooks integration)

Verified locally:

- `cargo build -p labolabo-app`, `cargo clippy -p labolabo-app --all-targets
  -- -D warnings`, and workspace-wide `cargo fmt --check` all pass.
- `cargo test -p labolabo-app` (78 tests): the new `hooks` module's 11 tests
  — routing-table round-trips (register/resolve/unregister/overwrite), real-
  filesystem `ensure_injected`/`restore_all` coverage (fresh injection
  writes all 7 events; idempotent re-injection; preserves another tool's
  existing hook entries; restore deletes a freshly-created file vs. restores
  a real prior file's exact original bytes; crash recovery from a stale
  `.labolabo-bak` before re-injecting), and one real-socket end-to-end test
  (`hook_runtime_receives_a_real_socket_event_and_resolves_its_route`: the
  real `AgentStatusBus`/channel/binary-resolution construction path bound to
  a temp-dir socket, a real `labolabo_core::forward_hook` call over that
  socket with a `LABOLABO_PANE`-annotated payload, delivered through the
  real channel and resolved through the real routing table) — plus all
  pre-existing tests unchanged.
- `cargo test -p labolabo-core` (224 lib tests + goldens, up from 186 at the
  end of wave 5b-3): the new `hook_settings` module (`shell_quote`/
  `hook_command`/`claude_resume_command`/`socket_path_from_uuid`, and
  `merge_hooks`'s create-vs-preserve/malformed-input/preserves-other-
  entries/idempotent-shape/all-seven-events behaviors), `store::
  agent_bindings` (round-trip, dedup, malformed-input degrade, plus a
  DB-level `agent_bindings_round_trips_through_upsert_and_all_tasks` in
  `task_database.rs`), `tiling::PaneItem::is_resumable`, the
  `LABOLABO_RS_DATA_DIR` override in `store::data_dir` (used-verbatim /
  empty-means-unset / absent-falls-back, tested through the pure
  env-value-as-parameter core, no env mutation), and `agent_event_parser`/
  `hooks::annotate_ids`'s new `labolabo_task_id` coverage — plus the full
  pre-existing suite (goldens untouched).
- Root `cargo build`/`cargo test`/`cargo clippy -- -D warnings` (workspace
  `default-members`: `labolabo-core` + `labolabo-term`) all pass.
- `cargo run -p labolabo-app` smoke run, done the "Smoke runs" way
  (`LABOLABO_RS_DATA_DIR=$(mktemp -d)`, ~8 seconds, then killed): no
  panic/crash output, a fresh `tasks.db` was created **inside the scratch
  directory** (not the real Application Support path, whose mtime was
  verified untouched), and — the scratch DB having no Tasks — no directory
  anywhere received a hooks injection.

**Incident that motivated the smoke-run isolation above.** An earlier
version of this check ran against the machine's real, shared `tasks.db`
(populated by other agents/sessions developing this port in parallel),
whose pre-existing selected Task pointed at a real worktree directory
outside this crate's scope. That Task was auto-restored at launch, so
`ensure_workspace_loaded` really injected a hooks entry into that
directory's `.claude/settings.local.json`; and because the process was
killed (not gracefully quit), `on_app_quit` never fired and the injected
file was left behind. The leftover file and the run's stale socket were
verified by hand to contain only the injection artifact (no user settings,
no Swift-app entries) and removed. Two preventions landed as a result: the
`LABOLABO_RS_DATA_DIR` escape hatch (`labolabo-core`'s
`store::rust_app_data_dir`, unit-tested) and the "Smoke runs: always
isolate the data directory" section above.

**Not verified — no synthetic keyboard/mouse input, on explicit
instruction, same as wave 5b-3 above:**

- **No real Claude Code session was launched or observed end to end.** The
  hooks wire protocol (forwarder → socket → bus → routing → UI) is
  exercised for real (see the socket integration test above), but nothing
  here has confirmed that a real `claude` process, hooked via a real
  injected `settings.local.json`, actually fires these events as expected
  in practice — that needs the user to run a real Claude Code session in a
  Task's terminal and watch the tab/sidebar dot.
- **The resume-at-restore path (`claude --resume` spawned directly as a
  pane's command) has not been observed against a real prior session.**
  `PaneItem::is_resumable`'s gating logic is unit-tested; the actual
  `claude --resume <id>` invocation succeeding (or gracefully failing) was
  not.
- **Status dots have not been visually inspected** (colors, tab-chip/
  sidebar placement) — no screenshot or window inspection was taken.
- **The same-directory-concurrent-use race** (two LaboLabo instances
  injecting/restoring hooks for the same directory at once — see "Known
  limitations" above) is understood but not reproduced/tested; the advice
  to avoid it is precautionary, not empirically validated.
