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
resume-at-restore — see "Claude Code hooks integration" below. The control
CLI (`plans/012-task-model-and-control-cli.md` §2, `docs/control-protocol.md`)
followed: the `labolabo` binary and a second, separate control socket let
scripts/agents/the app's own Claude sessions open tabs, list Tasks/tabs, and
switch focus from outside the gpui process — see "Control CLI" below. **This
wave** adds drag & drop (`plans/012` §3): dragging a tab chip onto another
pane splits (edge) or merges (center) it via `PaneTilingModel::move_pane`,
dragging a sidebar Task row reorders it within its repo group, and dropping
OS files/folders inserts shell-quoted paths into a terminal pane or starts a
new attached Task from a dropped folder — see "Drag & drop" below. Wave 6c
adds the menu bar (see "Menu bar"), Task archive/delete (see "Task archive /
delete"), and window-bounds persistence (see "Window bounds persistence").
Still not the full production UI — Task rename/done and interactive divider
drag-resize are later waves' scope.

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

### Importing from the Swift app

On a genuinely fresh install (no Task at all, active or archived) that also
has a Swift `labolabo.db` (`~/Library/Application Support/LaboLabo/
labolabo.db`), the app automatically imports every Swift session as a Task
on first launch, and shows a one-line, closable result banner at the top of
the sidebar ("Swift 版から n 件の作業を取り込みました…"). You can also
trigger it manually at any time via **ファイル > Swift 版からインポート…**
(`crate::swift_import`, `labolabo_core::import_from_swift`) — this always
runs (not just on a fresh install) and always leaves a banner, using the
same duplicate-directory skip rule (a Swift session whose directory already
matches an existing Task, active or archived, is skipped).

The read from the Swift database is strictly read-only
(`SQLITE_OPEN_READ_ONLY`, never `ensure_schema`'s `ALTER TABLE`) — safe to
run while a real Swift `LaboLabo.app` is open at the same time. **After
importing, don't open the same directory in both apps at once**: each app
injects its own Claude Code hooks (`.claude/settings.local.json`) into a
Task/session's working directory while it's open, and two concurrent
injections into the same directory will conflict (see "Claude Code hooks
integration" below for what that file does).

`LABOLABO_SWIFT_DB_PATH` (empty = unset) overrides the Swift database path
the importer reads — a `LABOLABO_RS_DATA_DIR`-style developer escape hatch,
used by the smoke-run recipe below so an exploratory launch never reads a
real Swift `labolabo.db` either.

### Smoke runs: always isolate the data directory

Launching against the real database is not a harmless read: every restored
Task spawns shells in — and, since wave 5c, **injects Claude Code hooks
into** — that Task's real working directory. An exploratory "does it start"
run must therefore never see the real `tasks.db`, and (since the Swift
importer above runs automatically whenever `tasks.db` starts out empty)
must not see a real Swift `labolabo.db` either. Set both
`LABOLABO_RS_DATA_DIR` (developer escape hatch, honored by
`labolabo_core::store::rust_app_data_dir`; empty value = unset) and
`LABOLABO_SWIFT_DB_PATH` (see above) to scratch/nonexistent paths:

```sh
LABOLABO_RS_DATA_DIR=$(mktemp -d) \
  LABOLABO_SWIFT_DB_PATH=/nonexistent \
  timeout 5 cargo run -p labolabo-app
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

## Linux (wave 7a)

This crate builds, lints, and unit-tests on Linux — the `rust-app-linux`
CI job (`.github/workflows/ci.yml`, ubuntu-latest) runs the same
`cargo build`/`clippy`/`test -p labolabo-app` gauntlet as the macOS
`rust-app` job on every PR, and `scripts/package-linux.sh` (run by
`rust-app-bundle.yml`'s `package-linux` job, `workflow_dispatch`-only, same
as the macOS bundle) packages a release build into a portable
`LaboLabo-rs-linux-<version>-<arch>.tar.gz` (three binaries + a
freedesktop.org `.desktop` launcher + a root-less per-user `install.sh` +
icon + README).

> **Verification honesty**: what CI proves is "compiles, clippy-clean, and
> the headless unit/integration tests (no window ever opened) pass on
> Linux". **Actually displaying a window on a real X11/Wayland desktop has
> not been verified** — there is no Linux machine in this project's
> development loop yet. Treat the first real-desktop launch as an open
> verification task (rendering, keyboard, clipboard, IME are all
> theoretically wired through gpui's cross-platform APIs but unproven here).

### System dependencies

Build-time (link) dependencies on Debian/Ubuntu — what the CI job installs:

```sh
sudo apt-get install -y pkg-config libxcb1-dev libxkbcommon-dev \
  libxkbcommon-x11-dev libfreetype-dev
```

Only three libraries are actually *linked*: `libxcb` (gpui's `x11rb`
dependency compiles its `xcb_ffi` module unconditionally under the
`allow-unsafe-code` feature gpui enables) and `libxkbcommon` /
`libxkbcommon-x11` (the `xkbcommon` crate's `#[link]` attributes).
`libfreetype-dev` is optional (freetype-sys falls back to building its
vendored copy via `cc` when pkg-config can't find the system one — the
system one is just faster). Everything else gpui needs on Linux is
**dlopen'd at runtime, not linked**, by explicit upstream feature choices
(`wayland-backend` with its `dlopen` feature, `font-kit` with
`source-fontconfig-dlopen`, Vulkan via `ash`'s default `loaded` =
`libloading`): at runtime you additionally need `libwayland-client` *or* an
X11 server's client libs, `libfontconfig`, and a working Vulkan driver
(`libvulkan` + the GPU's ICD; gpui's blade renderer is Vulkan-only on
Linux) — all present on any mainstream desktop install.

### Wayland vs. X11

gpui picks its backend at runtime: Wayland when `WAYLAND_DISPLAY` is set,
else X11 (this is upstream's standard detection — nothing in this crate
chooses). Neither has been exercised for this app (see the verification
note above); gpui itself ships Zed on both daily.

### Known Linux differences / limitations

- **"IDE で開く" / "Finder で表示" are macOS-only and hidden on Linux** (the
  menu items are `#[cfg(target_os = "macos")]`-gated out; detection returns
  empty). Decision for this wave: *hide* rather than degrade to a single
  `xdg-open` item — editor detection is Spotlight-based (`mdfind`) with no
  Linux equivalent implemented, and this wave deliberately avoids adding
  new user-facing menu strings while the i18n pass is in flight. Revisit as
  a follow-up (an `xdg-open <dir>` "open in file manager" item is the
  natural shape).
- **The `platform` modifier is Super, not Ctrl.** All "Cmd+…" keybindings
  in the table above are gpui `platform` bindings, which map to the
  Super/Windows key on Linux (upstream keystroke semantics). Ctrl stays
  reserved for the terminal itself, same as on macOS — but Super+T/W/D…
  may collide with desktop-environment shortcuts; remapping is a follow-up.
- **Fallback font is per-OS.** Menlo doesn't exist on Linux, and the CSS
  generic `"monospace"` doesn't resolve through gpui's cosmic-text font
  database (it matches face families literally; "monospace" is a
  fontconfig alias) — so when no configured Ghostty `font-family` resolves,
  `render::FALLBACK_FONT_FAMILIES` probes DejaVu Sans Mono → Noto Sans
  Mono → Liberation Mono → Ubuntu Mono (first installed wins). The Git
  panes' monospace bits (diff/whole-file/commit hash/date) reuse the same
  resolved font instead of hardcoding Menlo.
- **Ghostty config discovery is already XDG-native**: `$XDG_CONFIG_HOME/
  ghostty/config` (`~/.config/ghostty/config`) etc. — the macOS-only
  `~/Library/Application Support` and `/Applications/Ghostty.app` bundled-
  themes locations are skipped on Linux (`ghostty_config.rs`). Data lives
  in `$XDG_DATA_HOME/LaboLabo-rs` (`~/.local/share/LaboLabo-rs`).
- **Hooks + control CLI are unix-domain-socket based and shared with
  macOS** (`#[cfg(unix)]` in `labolabo-core`) — no Linux-specific work was
  needed; the same `labolabo-hook`-as-sibling-binary resolution applies
  (`package-linux.sh` keeps all three binaries side by side in `bin/`).
- **IME on Linux (xim / wayland text-input) is untested** — the overlay
  logic (`ime.rs`) is platform-independent gpui API, but no Linux IME has
  ever been driven against it.
- **Window menu's しまう/拡大・縮小** call gpui's cross-platform
  `Window::minimize_window`/`zoom_window`; behavior under Wayland (where
  minimize-self is compositor-dependent) is unverified.

## Windows (wave 7c)

This crate builds, lints, and unit/integration-tests on Windows — the
`rust-app-windows` CI job (`.github/workflows/ci.yml`, `windows-latest`)
runs the same `cargo build`/`clippy`/`test -p labolabo-app` gauntlet as the
macOS `rust-app`/Linux `rust-app-linux` jobs on every PR, and
`scripts/package-windows.ps1` (run by `rust-app-bundle.yml`'s
`package-windows` job, `workflow_dispatch`-only, same as the macOS/Linux
packaging jobs) packages a release build into a portable
`LaboLabo-rs-windows-<version>-<arch>.zip` (three binaries + an icon +
README — no installer, see "Icon" below). This wave builds directly on the
Windows *core* wave (`rust/README.md`'s "Windows core wave" section):
`labolabo-core`'s Named Pipe hooks/control transports, `ToolLocator`, and
`taskkill`-based process kill were already Windows-real before this wave —
this one closes the remaining gaps in `labolabo-app`/`labolabo-term`
themselves (shell resolution, the hooks/control socket-path minting bug
below, font fallback, Ghostty config discovery) and adds the CI job +
packaging.

> **Verification honesty**: same caveat as the Linux section above — what CI
> proves is "compiles, clippy-clean, and the headless unit/integration
> tests (no window ever opened) pass on Windows". This includes more *real*
> OS-level plumbing than the Linux job currently exercises, though: the
> hooks/control Named Pipe round-trip tests (`labolabo-core`, already
> real on `windows-latest` since the core wave), this crate's own
> `HookRuntime`/`ControlRuntime` construction tests (fixed this wave to
> mint valid `\\.\pipe\...` names instead of unix-shaped paths — see "Hooks
> and control socket naming" below), and `labolabo-term`'s new
> `tests/windows_spawn.rs` (real `cmd.exe`-spawned child processes) all run
> for real. **Actually displaying a window on a real Windows desktop has
> not been verified** — there is no Windows machine in this project's
> development loop yet. Treat the first real-desktop launch as an open
> verification task (rendering, DirectWrite text shaping, ConPTY-vs-real-
> Ghostty differences, IME are all theoretically wired through gpui's
> cross-platform APIs but unproven here).

### Default shell resolution

`labolabo-term`'s `TermSession::spawn_with_scrollback_options`
(`crates/labolabo-term/src/session.rs`) previously hardcoded `/bin/sh -c
<cmd>` for the one-shot-command spawn path and delegated entirely to
`portable-pty`'s own `CommandBuilder::new_default_prog` for the interactive
default-shell path — the latter already resolved to `%ComSpec%` (i.e.
`cmd.exe`) on Windows via `portable-pty` itself, so this "just worked" in
the sense of not crashing, but the former (`/bin/sh`) doesn't exist on
Windows at all, and neither path ever considered PowerShell. Fixed
per-platform:

- **One-shot command** (used by `claude --resume`/the control CLI's
  `tab_open -- <command>`, never by a user typing): `%ComSpec% /C <cmd>`
  (`cmd.exe`) on Windows, unchanged `/bin/sh -c <cmd>` elsewhere. No
  PowerShell preference here — deliberately the simplest universal
  one-shot invocation, matching every stock Windows install.
- **Interactive default shell** (a fresh terminal pane with no explicit
  command): Windows now prefers `pwsh.exe` (PowerShell 7+, preinstalled on
  GitHub's `windows-latest` runner and common on developer machines) →
  `powershell.exe` (Windows PowerShell 5.1, present on every Windows
  install since Windows 7) → `%ComSpec%` (`cmd.exe`, `portable-pty`'s own
  fallback) — a `PATH` scan for the first two, self-contained in
  `labolabo-term` (see `session.rs`'s private `windows` module and its
  doc comment for why this doesn't reuse `labolabo_core::ToolLocator`
  despite the near-identical problem: pulling in `labolabo-core`'s GRDB-
  compatible persistence/tiling/hooks/control layers into this otherwise
  standalone terminal-session crate for a ~15-line `PATH` scan would be a
  much heavier coupling than the problem calls for). No unix-style login-
  shell flag (`-l` has no Windows equivalent, and there is no analogous
  "profile wasn't sourced" problem to solve for). Covered by
  `labolabo-term/tests/windows_spawn.rs` (real `cmd.exe`-wrapped spawn +
  env injection + resize survival) and pure unit tests of the `PATH`-scan
  preference order (`session.rs`'s `windows::tests`).

### Hooks and control socket naming

**Found and fixed while porting this wave** (a behavioral bug, not a
compile error — it wouldn't have surfaced until a real Windows launch):
`hooks::HookRuntime::new`/`control::ControlRuntime::new` unconditionally
built a `/tmp/labolabo/<hash>.sock`-shaped string via
`labolabo_core::socket_path_from_uuid`/`control_socket_path_from_uuid` and
handed it to `AgentStatusBus::new`/`ControlServer::new` — correct for the
AF_UNIX transport those functions build a *path* for, but on Windows
`AgentStatusBus`/`ControlServer` expect a `\\.\pipe\...` Named Pipe *name*
in that same slot (`labolabo_core::hook_pipe_name_from_uuid`/
`control_pipe_name_from_uuid`, already implemented since the Windows core
wave but never called from this crate). Both `hooks.rs` and `control.rs`
now mint the platform-appropriate identity (`mint_socket_path`, `#[cfg]`-
branched) — without this fix, hooks injection (agent status dots, session
memory, resume-at-restore) and the control CLI would have silently been
dead on every Windows launch even after everything else compiled and ran.
The crate's own real-socket construction tests (`hooks::tests::
hook_runtime_receives_a_real_socket_event_and_resolves_its_route`,
`control::tests::control_runtime_*`) were fixed the same way so they mint
real Named Pipe names on Windows instead of unix-shaped path strings, and
now run for real on the `rust-app-windows` CI job (previously these tests
existed but had never run on Windows at all, since there was no Windows
`labolabo-app` CI job before this wave).

`hooks::resolve_hook_binary` also gained `std::env::consts::EXE_SUFFIX`
(portable: empty on unix, `.exe` on Windows) — without it, the sibling-
binary lookup for `labolabo-hook` always missed on Windows (`labolabo-hook`
vs. the real `labolabo-hook.exe`), silently skipping hooks injection on
every Windows run with no compile-time signal at all.

### Ghostty configuration on Windows

Real Ghostty does **not** ship an official Windows build as of this
writing (macOS/Linux only) — so unlike the Linux XDG search above, there is
no documented "where does Ghostty look for its config on Windows" upstream
spec to port faithfully. Per the task brief's explicit fallback, this wave
implements only the XDG-equivalent search: `%APPDATA%\ghostty\config` then
`\config.ghostty` (`ghostty_config::windows_config_paths`, later file
wins — same two-filename convention as the unix search), with `%APPDATA%`
also standing in for `$HOME` (via `%USERPROFILE%`, falling back to
`%APPDATA%` itself) for `config-file` include resolution. This is a
best-effort, **unverified-against-any-real-upstream** guess (see that
function's doc comment) — it exists so a user who manually places a config
file there (e.g. anticipating a future official Windows Ghostty build, or
running Ghostty under WSL with a Windows-side copy) gets it picked up, not
because there is a confirmed Windows Ghostty to test against. No macOS-
style bundled-themes-directory guess on Windows (no app-bundle concept to
guess a path inside of) — a Windows user's `theme = ` only ever resolves
against their own `ghostty/themes` subdirectory.

### Font fallback on Windows

`render::FALLBACK_FONT_FAMILIES` gets its own Windows list (rather than
falling into the Linux DejaVu/Noto/Liberation/Ubuntu list, none of which
ship on Windows by default): **Cascadia Mono** (the modern monospace face
Microsoft ships with Windows Terminal/VS Code, and — since Windows 11 —
sometimes with the OS itself; not guaranteed on an older/minimal install,
hence not the only candidate) → **Consolas** (bundled with any install that
has Visual Studio/Office's shared fonts — near-universal on a real
developer machine) → **Courier New** (ships with every Windows install
unconditionally — the final safety net).

### "IDE で開く" is hidden on Windows, like Linux

No new `#[cfg(windows)]` branch was needed here: `ide_open.rs`'s detection/
open functions already have a `#[cfg(not(target_os = "macos"))]` stub arm
(empty detection, `Err`-returning open — see that module's doc comment),
and the menu items that would call them (`menus.rs`, `task_menu.rs`) are
`#[cfg(target_os = "macos")]`-gated at the call site, so the stubs are
simply never reached on Windows, exactly as on Linux today. Same decision
as the Linux section above: *hide* rather than degrade to a single
`explorer.exe`-based "reveal" item — that's a natural follow-up, not
attempted this wave.

### Icon

`crates/labolabo-app/resources/windows/labolabo-rs.ico` (committed to the
repo, generated from the Swift app's own `icon_512x512@2x.png` artwork via
Pillow — `Image.open(png).save(ico, format="ICO", sizes=[(16,16),(24,24),
(32,32),(48,48),(64,64),(128,128),(256,256)])` — regenerate the same way if
the source artwork ever changes) is **not embedded into `labolabo-app.exe`
itself** — deliberately the lighter of the two options the task brief
allowed ("重ければ zip 内 .ico 同梱+ショートカット案内に縮退可"). Build-time
icon embedding is real and low-risk in principle (`embed-resource`, the
same crate gpui's own `build.rs` already pulls in as a
`[target.'cfg(target_os = "windows")'.build-dependencies]` for its DPI-
awareness manifest — see `gpui`'s `build.rs`/`resources/windows/gpui.rc`
for the exact pattern this crate could follow), but this repo has no
Windows machine in its development loop to visually confirm the result
against, and the embedding only actually takes effect when `build.rs`
itself compiles *on* a Windows host (gpui's own `mod windows` is
`#[cfg(target_os = "windows")]`-gated at the Rust source level, i.e. host-
conditional, not just target-conditional — a macOS→Windows cross-compile
never exercises it either). Shipping the `.ico` alongside the binaries in
`scripts/package-windows.ps1`'s zip for a user to point their own
Start Menu/taskbar shortcut at is lower-risk and still gives every user a
real, branded icon option (see that script's own "Icon" comment for the
full write-up and the exact shortcut steps in the package's own README).
Revisit build-time embedding if/when this port gets a proper installer and
a Windows machine joins the dev loop to verify it against.

### Known Windows differences / limitations

- **Default shell prefers PowerShell over `cmd.exe`** — see "Default shell
  resolution" above; this is a deliberate UX choice (mirroring Windows
  Terminal's own default), not a compatibility requirement.
- **No PowerShell/cmd shell-kind metadata per pane** — `labolabo_core::
  quote_dropped_paths` (OS file drops onto a terminal pane, see "Drag &
  drop" below) is POSIX-shell-quoting-only; a dropped path's quoting may
  not parse correctly under `cmd.exe`/PowerShell on Windows. Same for
  `hook_settings::claude_resume_command`'s POSIX single-quoting of a
  resume session id, now wrapped in `%ComSpec% /C ...` on Windows (see
  "Default shell resolution" above) — the wrapping makes the *spawn* work,
  but `cmd.exe` doesn't strip single quotes the way a POSIX shell does, so
  a resume id containing characters `shell_quote` would need to escape
  could still reach `claude` malformed. Both are pre-existing, already-
  documented POSIX-only-quoting gaps (see "Drag & drop"'s own note) that
  this wave widens the visibility of rather than fixes — a Windows-aware
  `quote_dropped_paths`/`shell_quote` variant is a natural follow-up.
- **Ghostty config/theme discovery is unverified against any real
  upstream** — see "Ghostty configuration on Windows" above; Ghostty
  itself has no official Windows build to test against.
- **IME on Windows is untested** — same caveat as Linux: the overlay logic
  (`ime.rs`) is platform-independent gpui API, but no Windows IME has ever
  been driven against it.
- **ConPTY-vs-real-Ghostty rendering differences are unexplored.** This
  app's PTY layer (`portable-pty`) uses Windows' ConPTY on Windows
  (transparently, no code change needed in this crate — `portable-pty`
  0.9's own dependency graph already includes `winapi`/`miow` for this);
  whether ConPTY's own VT-sequence translation layer interacts with either
  VT backend (`backend-alacritty` default, `backend-ghostty-vt` opt-in) any
  differently than a real PTY does on unix has not been investigated.
- **A `--release` build needs gpui's own HLSL shader compilation, which
  needs `fxc.exe` on `PATH` — unverified on a real Windows host.** gpui's
  `build.rs` only compiles its DirectX shaders in a non-debug build
  (`#[cfg(not(debug_assertions))]`); this is what `scripts/
  package-windows.ps1`'s `cargo build --release` exercises, but the
  `rust-app-windows` CI job's own `cargo build`/`test`/`clippy` steps are
  all debug-mode (matching `rust-app`/`rust-app-linux`'s own convention),
  so they never hit this path — confirmed by cross-compiling this repo's
  own debug build cleanly for `x86_64-pc-windows-gnu` from macOS, then
  hitting exactly this `fxc.exe`-not-found error the moment a `--release`
  cross-build was attempted (mingw has no `fxc.exe` to find — a real
  Windows host with the Windows SDK does, which is presumably what GitHub's
  `windows-latest` runner has, the same assumption gpui's own upstream
  (Zed) relies on for its release Windows builds — but this repo has no
  Windows machine to confirm it against directly). If `package-windows.ps1`
  (run only by the `workflow_dispatch`-only `rust-app-bundle.yml`, not a
  required PR check) ever fails on `windows-latest` with this error, that
  is the mechanism to look at first.
- **No code signing.** `scripts/package-windows.ps1`'s zip is unsigned (no
  code-signing certificate obtained) — Windows SmartScreen will likely warn
  on first run of an unsigned, freshly-downloaded `.exe`, same category of
  friction the Linux tarball's lack of any signing convention already has,
  worse in Windows' case since SmartScreen actively surfaces it.
- **No installer, no Start Menu registration.** `package-windows.ps1`
  produces a flat, portable zip (bin\ + icon + README) — no MSI/MSIX/
  Inno Setup installer, no registry entries, no Start Menu shortcut created
  automatically (see "Icon" above for the manual shortcut steps the
  package's own README documents). A proper installer is future work, same
  category as the Linux package's own root-less `install.sh` being the
  "no real package manager integration yet" placeholder for that platform.

## Design

### Module layout

| Module | Responsibility |
|---|---|
| `main.rs` | Entry point: reads the Ghostty font config, resolves the UI language (`rust_i18n::i18n!` + `crate::i18n`), registers the tile/tab keybindings (`cx.bind_keys`), installs the menu bar (`cx.set_menus`), and opens the one window at the persisted (or centered-default) bounds. |
| `i18n.rs` | UI-chrome i18n (wave 6f): `LocaleSetting` (自動/ja/en, persisted in `appState`), OS-locale detection (`sys-locale`), and the localized pane-title helper. The locale tables themselves live in `locales/{ja,en}.yml`, compiled in by `rust_i18n::i18n!` and looked up via `t!()`. See "UI language" below. |
| `menus.rs` | The menu bar definition (wave 6c, `gpui::Menu`/`MenuItem` over the same actions the keybindings dispatch) and the About overlay (version + `build.rs`-injected build number). Takes the locale explicitly (see its module doc comment's "i18n (wave 6f)" section). |
| `task_menu.rs` | The sidebar row "…" menu and delete-confirmation overlay (wave 6c): a pure, unit-tested phase state machine (`TaskMenuState`) + rendering. See "Task archive / delete" below. |
| `task_lifecycle.rs` | Archive/delete logic (gpui-free): next-selection math (pure, unit-tested) and worktree removal — `git worktree remove` (never forced) + optional `git branch -d` — integration-tested against real temp repos. |
| `ide_open.rs` | "IDE で開く" (macOS): the Swift app's editor-candidate list, Spotlight (`mdfind`) installed-detection, and `open -b`/`open -R` launching. Pure detection/filter helpers unit-tested. |
| `window_bounds.rs` | Window bounds persistence (wave 6c): JSON encode/decode of `{x,y,w,h}` and the "does it still intersect any display" restore validation — pure, unit-tested. |
| `update_check.rs` | RC release wave: once-per-launch background GitHub-releases check + the dismissible sidebar banner's data/state (`ReleaseInfo`, `is_update_available`, `should_notify`). See `rust/README.md`'s "RC リリース手順" section. |
| `swift_import.rs` | Thin glue around `labolabo_core::import_from_swift`: locates the Swift database (`LABOLABO_SWIFT_DB_PATH` override), persists the resulting Tasks, and formats the sidebar result banner. See "Importing from the Swift app" above. |
| `app.rs` | The gpui root view (`LaboLaboApp`): owns the `TaskDatabase`, the Task list, one `TaskWorkspace` per loaded Task, Task selection/persistence, the new-Task flows' orchestration, key routing, the action handlers for every keybinding (including Cmd+V paste), and the `EntityInputHandler` impl that wires up IME composition. |
| `task_workspace.rs` | One Task's live workspace: its `PaneTilingModel` + one `PaneRuntime` (real `Terminal` session + redraw bridge) per terminal pane, and the recursive split/tab-bar render tree (wave 5b-2's tree, made per-Task — every render/click path carries a `task_id`). The focused pane's leaf also registers the IME input handler and paints the preedit overlay each frame. |
| `sidebar.rs` | The Task sidebar: pure, unit-tested repo-grouping (`group_tasks_by_repo`) + minimal rendering (title + a one-glyph worktree/attached marker, "+ Attached"/"+ Worktree" buttons, error banner). |
| `new_task.rs` | The new-Task flows' git side (gpui-free, integration-tested against real temp repos): repo-identity resolution for attached Tasks, and branch-generation + `git worktree add` for worktree Tasks. |
| `focus.rs` | Pure tile-tree focus logic (gpui-independent, unit-tested): which pane to focus after a close, next/previous-pane cycling, Cmd+N tab lookup. See its module doc comment for the "focus is a `PaneId`, not a `NodeId`" invariant. |
| `hooks.rs` | Claude Code hooks integration (wave 5c): the app-wide `AgentStatusBus`, `.claude/settings.local.json` injection/restore, and the `LABOLABO_PANE` routing table. See "Claude Code hooks integration" below. |
| `control.rs` | Control CLI wiring: `ControlRuntime` (the app-wide control socket/server) and the gpui bridge that routes each request through a `WindowHandle` into `LaboLaboApp::dispatch_control` (`app.rs`). See "Control CLI" below. |
| `bin/labolabo.rs` | The `labolabo` CLI binary — a thin client for the control socket (argv parsing, `ControlRequest` construction, printing the response). See "Control CLI" below. |
| `ghostty_config.rs` | Pure-ish loader for the user's Ghostty config (`font-family`/`font-size`, `background`/`foreground`/`cursor-color`/`palette`/`theme`, `config-file` includes). Fixture-tested; never reads `$HOME` in tests. |
| `grid.rs` | Pure functions: pixel area + cell size -> terminal column/row count (`grid_size_for_area`/`_for_window`); pixel position -> `(col, row)` cell (`cell_at`); wheel/trackpad pixel delta -> whole scroll lines, with cross-event fractional carry (`accumulate_scroll_lines`). No gpui types — unit-tested without a gpui `Application`. |
| `keys.rs` | Pure function: `gpui::Keystroke` -> PTY input bytes, for the keys that must bypass the platform's text-input/IME machinery (control keys, a bare Ctrl-<letter>) — everything else is `app.rs`'s `EntityInputHandler` impl's job. `Keystroke`/`Modifiers` are plain data, so this is unit-tested directly too. |
| `ime.rs` | Pure IME helpers: `layout_preedit` (column layout of an in-progress composition string on the terminal grid, unicode-width-aware) and UTF-8/UTF-16 length/slice conversions gpui's `EntityInputHandler` trait needs. No gpui types — unit-tested directly. |
| `paste.rs` | Pure function: a clipboard string -> the PTY bytes for a paste (`encode_paste`) — unsafe control byte stripping, newline normalization to `"\r"`, optional bracketed-paste wrapping. Unit-tested directly. |
| `render.rs` | `RenderSpec` (font resolution + cell measurement) and painting one `labolabo_term::GridSnapshot` into a gpui canvas (background, glyphs, a selection highlight, cursor, and — via `ime.rs` — the IME preedit overlay). |
| `selection.rs` | Pure text-selection geometry (`CellPos`/`Selection`) and cell-range -> string extraction (`selected_text`) over a `GridSnapshot`. No gpui types — unit-tested directly. See "Text selection, scroll & copy" below. |

### The Task model (wave 5b-3)

Data model and persistence live in `labolabo-core` (usable later by the
control CLI without gpui): `store::Task` (`id` = UUID v4, repo identity from
`GitEngine::repo_info` — `repo_key` is the shared git dir, the sidebar's
grouping key —, `kind` = `Worktree { branch, base, path }` \|
`Attached { directory }`, `title`, `layout: TileLayout`, `status`
(active/done/archived — archive/restore transitions landed in wave 6c, see
"Task archive / delete"; `done` is still a schema-level reservation),
`created_at`/`last_active_at`, `sort_order`, and an
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
| Cmd+V | Paste the system clipboard's text into the focused pane (see "Text selection, scroll & copy" below) |
| Cmd+C | Copy the focused pane's current text selection to the system clipboard, if any (see "Text selection, scroll & copy" below) |
| Cmd+Q | Quit (same graceful path as closing the last pane — hooks `settings.local.json` restore runs) |
| Cmd+M | Minimize the window |

"Next/previous pane" cycles the tree's leaves in depth-first order rather
than true on-screen (left/right/up/down) adjacency — the simpler of the two
options wave 5b-2's brief allowed, and the one `focus::adjacent_pane` is
unit-tested against. Clicking a tab chip or a pane's terminal area also moves
focus there, and the focused pane's frame gets a subtle accent-colored
border. There is no keybinding (or UI) for divider drag-resize —
see "Known limitations". There are no Task-switching keybindings yet (click
the sidebar).

### Menu bar (wave 6c)

`menus::app_menus()` (wired via gpui's `cx.set_menus`) provides a standard
menu bar; every item references the same gpui actions the keybindings above
dispatch, so there is a single handler per operation:

- **LaboLabo-rs**: LaboLabo-rs について (About overlay: version + build
  number, the latter injected at compile time by `build.rs` from
  `git rev-list --count HEAD`) / 設定… (Cmd+,) / LaboLabo-rs を終了 (Cmd+Q)
- **ファイル**: 新しい作業（フォルダ直付け）… / 新しい作業（worktree を作成）…
  / 選択中の作業を IDE で開く (macOS のみ、検出済みエディタの先頭で開く。
  未検出なら Finder で表示) / 新しいタブ (Cmd+T) / タブを閉じる (Cmd+W)
- **編集**: コピー (Cmd+C) / ペースト (Cmd+V)
- **表示**: Git ペインを表示/非表示 (Cmd+Shift+G) / 右に分割 (Cmd+D) /
  下に分割 (Cmd+Shift+D) / 次のペイン (Cmd+]) / 前のペイン (Cmd+[)
- **ウィンドウ**: しまう (Cmd+M) / 拡大/縮小 — gpui は英語名 `"Window"` の
  メニューにしか OS のウィンドウリストを差し込まないため、日本語文言を
  優先して自前アクション（`Window::minimize_window`/`zoom_window`）で配線
  している（単一ウィンドウアプリなのでウィンドウリストは実害なし）。

### Task archive / delete (wave 6c)

Hovering a sidebar Task row reveals a "…" button → a small overlay menu
(`task_menu.rs`): エディタで開く (macOS, installed editors only —
`ide_open.rs`, Spotlight `mdfind` detection cached at startup) /
Finder で表示 / アーカイブ / 削除….

- **アーカイブ**: sessions are shut down, the Task moves (status
  `archived`) to a collapsed "アーカイブ済み (n)" section at the sidebar's
  bottom, and selection moves to the next active Task. 復元 brings it back
  (appended at the end, fresh shells spawn on selection).
- **削除…** (confirmation required): an *attached* Task is only
  unregistered from the database — its directory is never touched. A
  *worktree* Task shuts its sessions down, then runs `git worktree remove`
  (never `--force`: uncommitted changes make git refuse, and the dialog
  shows 未コミットの変更があるため削除できません — the Task stays). Only
  after a successful removal, the optional「ブランチも削除」checkbox
  (default off) runs `git branch -d` (never `-D`; an unmerged branch shows
  a warning but the worktree deletion still counts as done). All git work
  runs on a background thread via `GitEngine`/`git_runner`
  (`task_lifecycle.rs`, integration-tested against throwaway repos).

### Window bounds persistence (wave 6c)

The window's position/size is saved (debounced ~500ms) to the `appState`
table (`windowBounds` key, JSON `{x,y,w,h}`) and restored at launch
(`window_bounds.rs`). If the saved bounds no longer intersect any connected
display (e.g. an unplugged external monitor), the window falls back to the
centered default. **Fullscreen/maximized state is not restored in this
first version** — the restore size is saved and the window always reopens
as a normal window.

### UI language (wave 6f)

All UI-chrome text (menu bar, sidebar, settings overlay, Git pane/tiles,
Task menu/dialogs, importer banner, About) is looked up through
[rust-i18n](https://crates.io/crates/rust-i18n)'s `t!()` macro against
compile-time-embedded ja/en tables (`locales/{ja,en}.yml` — ~100 keys,
semantic dotted names like `menu.file.new_tab`). The language resolves at
startup from the `locale` `appState` key (`crate::i18n::LocaleSetting`):
`auto` (the default; picks `ja` when the OS locale starts with `ja`, via
`sys-locale`, else `en`), or an explicit `ja`/`en` from the settings
overlay's 言語/Language row. **Switching applies live** — every render
function re-reads `t!()` each frame, and the one non-per-frame surface (the
native menu bar) is rebuilt by `LaboLaboApp::set_locale` — no restart
needed. Deliberately *not* localized: terminal content, Ghostty config, the
`labolabo` control CLI's output (fixed to English by
`docs/control-protocol.md`), stderr developer warnings, and *persisted*
pane titles (a new pane's default title is stamped in the UI language at
creation and then behaves as data, like a rename). Three source-level tests
(`tests/i18n_parity.rs`) gate regressions: ja/en key-set parity, every
`t!()` key existing in both tables, and no hardcoded Japanese string
literal in production code.

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

Two parallel paths feed a pane's PTY, split by *what kind* of input a
keystroke represents — see `keys.rs`'s module doc comment for the full
reasoning and `app::LaboLaboApp`'s `EntityInputHandler` impl doc comment for
the IME side:

- **Control keys** (Enter/Backspace/Tab/Escape/arrows, a bare
  Ctrl-<letter>): gpui delivers a `KeyDownEvent` (via `div::on_key_down`, on
  a focused, `track_focus`-ed root div) -> `keys::keystroke_to_bytes` turns
  it into raw bytes (pure function, see `keys.rs` unit tests) -> the
  selected Task's focused pane's `Terminal::write_input` writes them to its
  PTY. `app::LaboLaboApp::key_down` calls `cx.stop_propagation()` on a
  claimed keystroke, which is what stops gpui from *also* forwarding it to
  the platform's text-input/IME machinery once one is registered (see
  below) — without it, e.g. Ctrl-A would additionally reach macOS's
  `doCommandBySelector:` (Cocoa's default key bindings map it to
  `moveToBeginningOfLine:`), re-dispatching the same handler a second time.
- **Everything else — plain printable text, space, and IME composition**:
  routed through gpui's `EntityInputHandler` trait instead (implemented on
  `LaboLaboApp` in `app.rs`), which gpui wires to the platform's real
  text-input machinery (macOS's `NSTextInputContext`; X11/Wayland's
  IBus/fcitx bridge). `task_workspace::render_leaf` registers an
  `ElementInputHandler<LaboLaboApp>` (via `Window::handle_input`) against
  the focused pane's canvas every frame. `replace_text_in_range` writes a
  committed string (a plain character, or an IME composition's final
  confirmed text) straight to the focused pane's PTY;
  `replace_and_mark_text_in_range` tracks an in-progress composition's
  preedit string *without* writing anything to the PTY, and
  `task_workspace::render_leaf` paints it inline over the cursor
  (`render::paint_preedit`, underlined, using `ime::layout_preedit`'s pure
  column-layout math — unicode-width-aware, so CJK fullwidth characters
  occupy two cells); `unmark_text` clears it on cancel (e.g. Escape while
  composing) with nothing written to the PTY.

  **Design decision — no double-send.** `keys::keystroke_to_bytes`
  deliberately does *not* handle any keystroke carrying a `key_char` other
  than a bare Ctrl-<letter> (this used to include a "printable fallback"
  and a `"space"` case; both were removed this wave). Traced through gpui's
  own macOS/X11/Wayland backends: once an input handler is registered, all
  three platforms route every plain/shift-only `key_char` keystroke through
  it (self-inserting, or starting an IME composition) rather than through
  the raw `KeyDownEvent` a `div::on_key_down` listener sees. Handling such
  a keystroke in both places would either double-send it, or — worse —
  send the *unconverted* Roman letter to the PTY before the IME ever gets
  a chance to compose it. So printable text now flows through exactly one
  path, chosen by whether it needs IME/text-input's cooperation.

- Cmd/Super combinations are never forwarded to a terminal (reserved for
  application-level shortcuts — see "Keybindings" above for what they're
  bound to as of this wave), including Cmd+V/Cmd+C (paste/copy — see "Text
  selection, scroll & copy" below, a separate path from either of the
  above).

**Not implemented:**

- Delete (forward-delete)/Home/End/PageUp/PageDown/function keys.
- Ctrl combined with anything other than a single letter, and any
  Ctrl+Alt/Ctrl+Shift combination (falls to the `EntityInputHandler` path
  like any other `key_char`-carrying keystroke, same as a plain letter).
- IME candidate-window positioning that depends on an existing selection
  range — `EntityInputHandler::selected_text_range` always reports an empty
  selection at the composition's end, never the terminal-grid text
  selection described below (there is no addressable "document" to select
  from in the IME sense — see the impl's doc comment). The two "selection"
  concepts (IME's document-selection query, and mouse-driven terminal text
  selection below) are unrelated despite the shared word.

### Text selection, scroll & copy

**Scrollback** (`labolabo_term::Terminal::scroll`/`scroll_to_bottom`,
`GridSnapshot::{scroll_offset, scrollback_len}` — see
`crates/labolabo-term/README.md`'s own writeup for the backend-level design
and sign convention): `task_workspace::render_leaf` registers
`on_scroll_wheel` on each pane's canvas, routed to `app::LaboLaboApp::
handle_pane_scroll`. A raw gpui `ScrollWheelEvent::delta` (either
`ScrollDelta::Lines` or `::Pixels` — trackpad and traditional wheel both
supported, unified via gpui's own `pixel_delta(line_height)` so both
resolve through the same code path) is accumulated into whole lines via
`grid::accumulate_scroll_lines` (a per-pane `PaneRuntime::pending_scroll`
carries the fractional remainder across events, so a slow trackpad
gesture's individual sub-cell-height deltas still eventually produce a
scroll step instead of each rounding to zero). While the alternate screen
is active (`vim`/`less`/`htop`, ...), the accumulated line count is instead
converted into that many Up/Down (`ESC[A`/`ESC[B`) key sequences written to
the PTY — mirroring real Ghostty's default "alternate scroll mode" behavior
for full-screen TUI programs (see `VtBackend::alt_screen_active`'s doc
comment for the source-level confirmation); this app does not track DECCKM
(application cursor-key mode), same simplification `keys.rs`'s own literal
arrow-key handling already makes. **Any keystroke that reaches a pane's PTY
snaps that pane's scroll back to the live tail** (`Terminal::
scroll_to_bottom`, called from the single `write_focused_pane_input` choke
point both the control-key and IME-committed-text input paths write
through) — the terminal-UI convention every mainstream terminal follows.
Mouse-reporting TUIs (`vim -mouse`, ...) that would rather receive raw
scroll-wheel button events instead of either of the above are **out of
scope** — see "Known limitations".

**Text selection** (`selection.rs`'s `CellPos`/`Selection`/`selected_text`,
all pure/unit-tested; `app::LaboLaboApp::begin_selection`/
`extend_selection`/`finish_selection` drive them from mouse events on a
pane's canvas): mouse-down converts the click position into a grid cell
(`grid::cell_at`, using the canvas's live paint bounds tracked in
`PaneRuntime::last_bounds`) and starts a zero-length selection there (also
still focusing the pane, unchanged from before selection existed);
mouse-move while the left button is held (`MouseMoveEvent::dragging()`)
extends the selection's cursor cell; mouse-up clears it back to `None` if
it never grew past that zero-length start (a plain click, not a drag) so
click-to-focus never leaves a stray highlight or blocks a later Cmd+C with
an empty range. `render::paint_grid` paints a translucent highlight
(`render::SELECTION_HIGHLIGHT_RGB`, the same accent hue as the
focused-pane border) over every cell `Selection::contains` reports, under
the glyph so selected text stays legible. Selection is character-based
(not line or box/rectangular mode) and works over scrolled-back history
exactly the same as the live view, since it just reads whatever's in the
pane's *current* `GridSnapshot` — including one `VtBackend::scroll_display`
scrolled back.

**Known limitation, by design — a selection's coordinates aren't stable
across a scroll or new output mid-drag.** A selection's endpoints are
plain `(row, col)` cell coordinates within whichever snapshot was current
when the mouse last moved, not a persistent per-line buffer identity. If
the view scrolls (or new PTY output arrives) between two events of the
*same* drag, or between finishing a drag and pressing Cmd+C, those
coordinates are reinterpreted against whatever is at that position in the
*next* snapshot — which can shift what ends up highlighted/copied. This is
the simplest class of terminal-selection design (see `selection.rs`'s
module doc comment for the fuller writeup); a persistent per-line-id design
is future work if this proves to matter in practice.

**Not implemented (flagged, not attempted this wave):** double-click
word-select and triple-click line-select (a plain click-drag is the only
gesture wired up); mouse reporting to TUI programs that request it (out of
scope per this wave's brief — `vim -mouse` and similar always get the
scroll-to-cursor-keys/text-selection behavior above, never raw mouse
button/motion escape sequences); a visible scrollbar widget (`GridSnapshot`
already carries `scroll_offset`/`scrollback_len` for one, but no UI element
draws it yet).

**Copy** (`app::LaboLaboApp::action_copy`, Cmd+C): extracts the focused
pane's selection via `selection::selected_text` and writes it to the system
clipboard (`cx.write_to_clipboard`) — a no-op with no selection, an empty
one, or empty extracted text. Deliberately never touches the pane's PTY:
`Ctrl+C` (a bare control byte via `keys::keystroke_to_bytes`) is the only
way to send `SIGINT`; Cmd+C and Ctrl+C are different keystrokes entirely
(the whole `platform` modifier is reserved for application shortcuts, so
there's no ambiguity to resolve at dispatch time). The selection is left
exactly as it was after copying — matching every mainstream terminal's
"copy doesn't clear the selection" convention.

**Paste** (`app::LaboLaboApp::action_paste`, Cmd+V): reads the system
clipboard (`cx.read_from_clipboard()`), and — if it has text — encodes it
via `paste::encode_paste` (pure function, unit tested): unsafe control
bytes stripped (in particular `ESC`, so a crafted clipboard payload can't
embed a literal bracketed-paste end marker to break out early), line
endings normalized to `"\r"` (a real terminal's Enter-key convention;
`"\r\n"` and lone `"\n"` both collapse to it), and — when the focused
pane's `Terminal::bracketed_paste()` reports the foreground program has
enabled DECSET `2004` — wrapped in `ESC[200~...ESC[201~`. The mode query
itself is a small addition to `labolabo-term`'s `VtBackend` trait
(`bracketed_paste(&self) -> bool`), implemented identically for both
backends (`alacritty_terminal::Term::mode().contains(TermMode::
BRACKETED_PASTE)`; `libghostty_vt::Terminal::mode(Mode::BRACKETED_PASTE)`)
and exposed to the caller thread as a plain, non-blocking flag
(`TermSession::bracketed_paste`) refreshed by the worker thread after every
processed PTY byte batch — the same "publish a cheap plain-data value for
the caller thread" shape `GridSnapshot` itself already uses, just for a
single `bool`. Covered by a shared (`tests/backend_common.rs`) headless test
on both backends: enabling/disabling DECSET `2004` via `printf` in the
spawned shell is reflected in `bracketed_paste()`.

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
for where these land in the product model): Task rename/done (§1's
completion flow; archive/delete landed in wave 6c — see "Task archive /
delete") and interactive divider drag-resize (explicitly out of
this DnD wave's scope, per the wave's own brief). Claude Code hooks
integration (agent status, per-tab session memory, resume-at-restore)
landed in wave 5c — see "Claude Code hooks integration" above; the control
CLI (§2 — `labolabo tab open`/`task list`/`tab list`/`focus`) landed the
following wave — see "Control CLI" above; drag & drop (§3 — pane/tab DnD,
sidebar reordering, OS file drops) landed this wave — see "Drag & drop"
below. `task new` (§2's own scope note) and exposing the same RPC as an MCP
server remain reserved/future work (docs/control-protocol.md §5.5).

## Drag & drop (`plans/012-task-model-and-control-cli.md` §3)

Three independent DnD systems, all built on gpui 0.2's `on_drag`/
`on_drag_move`/`on_drop`/`drag_over`/`can_drop` (`crate::task_workspace`,
`crate::sidebar`):

- **Pane/tab DnD** (`task_workspace::render_pane_tab_bar`'s per-chip
  `.on_drag`, `task_workspace::render_leaf`'s per-leaf drop target):
  dragging a tab chip and dropping it on another pane's outer 25% margin
  splits toward that edge (`DropEdge::Left/Right/Top/Bottom`); dropping on
  the inner 50% merges it into that pane's tab group
  (`DropEdge::Center`) — the exact geometry `app/Sources/PaneTiling.swift`'s
  `PaneFrameView.edge(at:)` used, ported to gpui's top-left-origin
  coordinates (`labolabo_core::drop_edge_for_point`, unit-tested). The
  actual tree mutation is `PaneTilingModel::move_pane` — nothing here
  reimplements tiling logic. A translucent blue overlay
  (`task_workspace::MOVE_DROP_HIGHLIGHT_COLOR`) previews the drop zone while
  dragging; a same-leaf drop onto its own tab group's center, or its own
  edge when it's the group's only tab, shows no highlight (meaningless —
  `move_pane` already no-ops on it). **Not implemented:** reordering tabs
  *within* the same group by dragging (only cross-pane split/merge) — out of
  this wave's scope per the task brief.
- **Sidebar Task reorder** (`sidebar::render`'s per-row `.on_drag`/
  `.on_drop`): dragging a Task row and dropping it on another row within the
  same repo group reorders it there (`LaboLaboApp::
  reorder_tasks_in_sidebar`, pure ordering math in `labolabo_core::
  reorder_task_ids`, unit-tested — other repos' interleaved positions are
  preserved exactly). Cross-repo drops are rejected (`can_drop`). `sort_order`
  is renumbered densely and persisted for every Task on each successful
  reorder.
- **OS file/folder drops** (§3.1): gpui translates a platform file drag into
  a synthetic internal drag of its own `ExternalPaths` type, dispatched
  through the same hit-tested `on_drop`/`drag_over`/`can_drop` machinery as
  the in-app drags above — so "which pane is under the pointer" falls out of
  normal event dispatch, no separate coordinate-to-pane resolution needed.
  - **Onto a terminal pane**: every dropped path is POSIX-shell-quoted and
    space-joined (`labolabo_core::quote_dropped_paths`, reusing
    `shell_quote`), with one trailing space and **no newline** — the user
    finishes the command themselves. A distinct amber overlay
    (`task_workspace::FILE_DROP_HIGHLIGHT_COLOR`) marks this as "insert",
    visually different from the pane-move highlight. Dropping on a
    non-terminal pane (diff/files/commits) is a no-op (`can_drop` rejects
    it).
  - **Onto the sidebar**: every dropped *directory* starts a new attached
    Task there (`LaboLaboApp::handle_sidebar_folder_drop`, reusing "+
    Attached"'s own tail); dropped files are ignored. No confirmation
    dialog — matches "+ Attached"'s existing no-confirmation flow.
    **TODO:** a future wave may want a confirmation step here per the plan's
    own note (§3: "確認 UI を挟む" for the general case; this wave's brief
    explicitly allowed skipping it for the no-destructive-side-effect
    attached-Task case).
  - **Not implemented**: the relative-path/filename-only path variants (§3.1
    lists absolute-path as the default and only variant landed here), and
    Windows PowerShell/cmd quoting (§3.1's OS×shell matrix) — this app is
    macOS-only today and has no shell-kind metadata per pane yet;
    `quote_dropped_paths` is POSIX-only pending that.
  - **Not implemented**: window-level (non-sidebar, non-pane) OS drops —
    e.g. dropping a folder directly onto empty canvas space outside any
    pane/sidebar has no handler; the plan's "フォルダをサイドバー/ウィンドウ
    へドロップ" window-level case is covered only via the sidebar today.

## Known limitations

- **No Task rename/done.** Archive/delete landed in wave 6c (see "Task
  archive / delete"); rename and the `done` status transition are still a
  future wave's scope (the "…" menu deliberately shows no rename item).
- **Fullscreen/maximized window state is not restored.** Wave 6c persists
  the window's (restore-size) bounds only; the window always reopens as a
  normal window (see "Window bounds persistence").
- **"IDE で開く" detection relies on Spotlight.** Installed editors are
  found via `mdfind` by bundle id at startup; with Spotlight indexing
  disabled the editor entries disappear (Finder で表示 always remains).
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
- **Interactive divider drag-resize landed in wave 5j** (`task_workspace.rs`'s
  `render_tile` split branch, `app.rs`'s `update_divider_drag`/
  `finish_divider_drag`, `labolabo_core::tiling`'s `TileNode::set_ratio`/
  `find_node_mut`/`PaneTilingModel::set_split_ratio`) — a thin draggable
  handle at a split's boundary, resize cursor + hover highlight, `Terminal::
  resize` throttled to "once, on drag end" rather than every intermediate
  frame (see "Wave 5j" below for the real-mouse-drag caveat). The sidebar
  width is still fixed — that was never in scope for this addition.
- **No intra-group tab reorder by dragging**, and **no Windows shell
  quoting for OS file drops** — see "Drag & drop" above for both.
- **"Next/previous pane" is DFS tree order, not geometric adjacency.**
  Cmd+]/Cmd+[ cycle `TileNode::leaves()` in depth-first order, not by
  on-screen position — the simplest option wave 5b-2's brief explicitly
  allowed. In a layout where DFS order doesn't match visual left-to-right/
  top-to-bottom order (e.g. after several splits), the cycle direction may
  feel surprising.
- **IME composition support landed this wave** (see "Keyboard input path"
  above) but real Japanese/CJK input has not been verified interactively —
  synthetic keyboard/IME input is off-limits for this port's own
  verification discipline (`README.md`'s "What was and wasn't verified"
  convention), so this needs a human to actually type through a real IME
  before it's considered confirmed working, not just "compiles and the
  design traces correctly through gpui's platform backends".
- **Scrollback, text selection & copy landed in wave 5g; double/triple-click
  word/line select and SGR mouse-report forwarding landed in wave 5j** (see
  "Text selection, scroll & copy" above, and "Wave 5j" below). Remaining
  scope trims after 5j:
  - **Mouse reporting is SGR-format only** (`crate::mouse_report`'s module
    doc comment): a program that requests tracking (`1000`/`1002`/`1003`)
    *without* SGR (`1006`) — rare in practice since virtually every
    mouse-aware TUI written since SGR's introduction requests both — still
    falls back to local selection/scrollback, exactly as before 5j.
  - **No bare-hover motion forwarding.** Under `MouseTracking::Any`, a
    program can ask for motion reports with *no* button held (e.g. link
    highlighting on hover); this app only forwards motion while the left
    button is actively held (a drag) — see `render_leaf`'s mouse-move
    wiring comment.
  - **No horizontal-scroll forwarding.** `grid::accumulate_scroll_lines`
    only accumulates the vertical wheel axis (unchanged from before 5j);
    `mouse_report::MouseButtonKind::WheelLeft`/`WheelRight` exist (mirroring
    Ghostty's own button set, exercised by this module's own tests) but
    have no accumulated horizontal delta to encode from yet.
  - **Word/line selection is single-row and doesn't re-snap while
    dragging.** `selection::word_bounds_at` doesn't cross a soft-wrapped
    line the way Ghostty's own algorithm does; continuing to drag after a
    double/triple-click extends character-by-character, not by whole
    words/lines, unlike Ghostty's own gesture (`selection::
    selection_for_click`'s doc comment).
  - No scrollbar widget drawn yet (the data for one, `GridSnapshot::
    scroll_offset`/`scrollback_len`, already exists); a selection's
    coordinates are not stable across a scroll or new PTY output that lands
    mid-drag (see that section's "known limitation, by design" for why);
    and real interactive scroll/drag-select/double-click/mouse-report/
    Cmd+C behavior has not been verified by an actual mouse — see "What was
    and wasn't verified" below.
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
  variant name may fall back to `render::FALLBACK_FONT_FAMILIES` (Menlo on
  macOS, the first installed of DejaVu Sans Mono/Noto Sans Mono/Liberation
  Mono/Ubuntu Mono on Linux) here — a stderr warning says so.
- Linux builds/tests run in CI (`rust-app-linux`), but **no window has ever
  been opened on a real Linux desktop** — see "Linux (wave 7a)" above for
  what is and isn't verified there, and the Linux-specific limitations.
- **Cross-session conflict detection only sees Tasks whose Git status has
  already been fetched at least once this run** (`LaboLaboApp::
  task_conflicts`/`changed_files_cache`, `app.rs`). The Git pane's
  `FileWatcher` — and therefore `git status` — is only ever active for the
  *selected* Task (see "The Task model" above), so a Task that has never
  been selected has no entry in the cache and neither contributes to nor
  triggers a conflict warning, even if it really is editing the same file.
  This is a deliberate wave-5i scope decision (no all-Tasks background
  polling) rather than an oversight: the warning badge (sidebar row, an
  orange ⚠) only ever reflects the *last-known* status of whichever Tasks
  happen to have been visited, refreshed on the selected Task's own Git
  pane refresh cadence (FSEvents-debounced, not polled).
- **Transcript usage display is best-effort and per-tab, not per-Task.**
  `TaskWorkspace::pane_usage` re-reads a pane's transcript
  (`labolabo_core::transcript_usage::read`) only when that pane's own hook
  status transitions to `Idle`/`Ended` — never polled — and is shown as a
  compact `"1.2k tok · $0.08"` label on that pane's own tab chip (no
  popover/tooltip surface exists yet to show the fuller per-field
  breakdown Swift's `UsagePopover` does). A Task with several tabs shows
  one usage figure per tab, not a combined total for the Task.
- **The settings screen (`Cmd+,`) is an in-window overlay, not a native
  macOS window** — gpui has no `Settings`-scene equivalent to SwiftUI's,
  so `crate::settings::render_settings_overlay` paints a backdrop + panel
  over the existing window instead. There is no click-outside-to-close
  (see that function's doc comment for why); use the panel's own "×" or
  toggle `Cmd+,` again. The Git-pane-default-visible and scrollback-lines
  settings only affect Tasks/panes loaded *after* the change — see
  `crate::settings`'s module doc comment.

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

### Wave 5e (IME composition + clipboard paste)

Landed: `EntityInputHandler` wired to the focused pane's canvas
(`app::LaboLaboApp`'s impl; `task_workspace::render_leaf` registers it and
paints the preedit overlay), `keys::keystroke_to_bytes` narrowed to only the
keys that must bypass the platform's text-input machinery (see "Keyboard
input path" above for the full double-send-prevention reasoning), Cmd+V
clipboard paste (`app::LaboLaboApp::action_paste`, `paste::encode_paste`),
and a small `bracketed_paste(&self) -> bool` addition to `labolabo-term`'s
`VtBackend` trait (implemented for both backends, exposed to the caller
thread via `TermSession::bracketed_paste`).

Verified locally:

- `cargo build -p labolabo-app`, `cargo clippy -p labolabo-app --all-targets
  -- -D warnings`, and workspace-wide `cargo fmt --check` all pass.
- `cargo test -p labolabo-app` (105 lib tests + 8 `control_cli` integration
  tests): the new `ime` module (preedit column layout, including the
  wide-character and right-edge-shift cases mirrored from the vendored
  Ghostty source's own `Preedit.range` tests, plus UTF-16 length/slice
  helpers), the new `paste` module (newline normalization, unsafe-
  control-byte stripping including the bracketed-paste-end-marker-injection
  case, bracketed wrapping), and `keys.rs`'s updated contract (printable
  text/space now assert `None`, since they're the input handler's job) —
  plus the full pre-existing suite (unchanged where not directly touched).
- `cargo build/clippy/test -p labolabo-term` on **both** backends: default
  `backend-alacritty`, and `backend-ghostty-vt` (a Zig 0.16 toolchain and
  the fork-pinned Ghostty source tree were both available this time — see
  the crate README's "Building the ghostty-vt backend"). The new
  `bracketed_paste_mode_reflects_decset_2004` test (in the shared
  `tests/backend_common.rs`, so it runs against both backends unmodified)
  enables/disables DECSET `2004` via `printf` in a spawned shell and asserts
  `Terminal::bracketed_paste()` tracks it, off -> on -> off.
- Root `cargo build`/`cargo test`/`cargo clippy -- -D warnings` (workspace
  `default-members`: `labolabo-core` + `labolabo-term`) all pass.
- `cargo run -p labolabo-app` smoke run (`LABOLABO_RS_DATA_DIR=$(mktemp
  -d)`, ~6 seconds, then killed): no panic/crash output (one benign AppKit
  console line, `error messaging the mach port for
  IMKCFRunLoopWakeUpReliable` — a known harmless Input Method Kit warning
  Cocoa apps commonly print on window creation, not an error from this
  code), process exited cleanly on kill, no leftover process.

**Not verified — no synthetic keyboard/mouse input, on explicit
instruction, same as every wave above:**

- **Real Japanese (or any CJK) IME input has not been typed through the
  app.** This is the one thing that actually proves the feature works —
  everything above confirms the design traces correctly through gpui's own
  platform-backend source (macOS's `NSTextInputContext` dispatch, X11/
  Wayland's IBus/fcitx bridge, all read directly rather than assumed) and
  that the pure layout/encoding logic is correct in isolation, but **the
  user needs to actually switch to a Japanese input source, type a romaji
  sequence, watch the preedit render inline with the correct underline/
  column position, confirm it, and see the composed characters land in the
  shell** before this is confirmed working end to end.
- **Plain (non-IME) typing has not been re-verified interactively either.**
  This wave changed the base path (printable text/space no longer write
  directly from `key_down`; they rely on `EntityInputHandler::
  replace_text_in_range` being reached instead) — the design was traced
  through gpui's own dispatch code on all three platforms and is exercised
  transitively by the app not crashing at startup, but no actual keystroke
  was sent, so a regression in ordinary typing (not just IME) can't be
  ruled out without a human trying it.
- **Cmd+V paste has not been exercised against a real clipboard or a real
  bracketed-paste-aware program** (e.g. a shell with readline's bracketed
  paste enabled, or `vim`/`less`). `paste::encode_paste`'s logic is unit-
  tested and `bracketed_paste()`'s mode tracking is integration-tested
  against a real PTY, but the two have not been observed wired together
  through an actual Cmd+V keypress.
- **The IME candidate window's on-screen position has not been visually
  confirmed** — `bounds_for_range`'s cursor-cell math is straightforward
  (mirrors `render::paint_cursor`'s own coordinate math) but was not
  screenshotted against a real candidate popover.

### Drag & drop (`plans/012-task-model-and-control-cli.md` §3)

Landed: the three DnD systems described in "Drag & drop" above --
`labolabo_core::drop_edge_for_point` (pane-move drop-zone geometry, ported
from `PaneFrameView.edge(at:)`), `labolabo_core::quote_dropped_paths`
(§3.1's terminal file-drop path encoding, built on the existing
`shell_quote`), `labolabo_core::reorder_task_ids` (sidebar reorder ordering
math), and their gpui wiring in `task_workspace.rs`/`sidebar.rs`/`app.rs`
(`on_drag`/`on_drag_move`/`on_drop`/`drag_over`/`can_drop`).

Verified locally:

- `cargo build -p labolabo-app`, `cargo clippy -p labolabo-app --all-targets
  -- -D warnings`, and workspace-wide `cargo fmt --check` all pass.
- `cargo test -p labolabo-app` (105 lib tests + 8 `control_cli` integration
  tests, unchanged counts): no new app-crate unit tests this wave -- the
  gpui-level drag/drop wiring itself isn't meaningfully unit-testable
  without a real `Application`/window (per the task brief, verified instead
  by the smoke run below and left for manual UI verification); every piece
  of *pure logic* the wiring calls into lives in, and is tested in,
  `labolabo-core` instead (see below).
- `cargo test -p labolabo-core` (284 lib tests + 23 golden/integration
  tests): the new `drop_edge_for_point` suite in `tiling.rs` (center/edge/
  corner-priority/non-square-rectangle/degenerate-rectangle cases, plus an
  explicit test documenting the AppKit-flipped-vs-gpui-top-left coordinate
  difference from the Swift source), the new `quote_dropped_paths` suite in
  `hook_settings.rs` (single/multiple paths, embedded-quote escaping, empty
  input, no-newline invariant), and the new `task_order` module's
  `reorder_task_ids` suite (move-before, move-to-end, cross-repo-slot
  preservation, self-drop/unknown-id/cross-repo no-ops) -- plus the full
  pre-existing suite unchanged (goldens untouched; `move_pane`'s existing
  tests already covered the underlying tree mutation these both call into).
- Root `cargo build`/`cargo test`/`cargo clippy -- -D warnings` (workspace
  `default-members`: `labolabo-core` + `labolabo-term`) all pass.
- `cargo run -p labolabo-app` smoke run (`LABOLABO_RS_DATA_DIR=$(mktemp
  -d)`, ~6 seconds, then killed): no panic/crash output, process exited
  cleanly on kill, no leftover process.

**Not verified -- no synthetic keyboard/mouse input, on explicit
instruction, same as every wave above. Drag & drop is inherently a gesture
feature, so this gap is larger than usual here:**

- **No actual drag has been performed.** Dragging a tab chip onto another
  pane's edge/center, dragging a sidebar Task row onto another row, and
  dropping an OS file/folder onto a terminal pane or the sidebar have all
  been traced through gpui's own source (`on_drag`/`on_drag_move`/
  `on_drop`/`drag_over`/`can_drop`'s exact dispatch code, read directly --
  see this README's "Drag & drop" section above for what was confirmed that
  way, including how `FileDropEvent` becomes a synthetic internal drag
  routed through the same hit-tested dispatch, which is what makes
  per-pane/per-row drop-target resolution work with no extra coordinate
  bookkeeping) and compiles/runs without crashing, but none of the five
  interactions above (pane split, pane merge, same-leaf-meaningless-drop
  suppression, sidebar reorder, file/folder drop) has been watched happen
  on screen.
- **The drop-zone highlight overlays have not been visually confirmed** --
  `move_drop_highlight_overlay`'s fractional-quadrant math (left/right/top/
  bottom half, or full-pane center) and the distinct move/insert/reorder
  colors are straightforward but unscreenshotted.
- **The sidebar folder-drop -> new-Task flow has not been exercised via an
  actual OS drag** (dropping a Finder folder onto the sidebar) -- the
  underlying `resolve_attached_repo`/`Task::new_attached`/
  `add_task_and_select` path is exercised by "+ Attached"'s own (also
  unverified-via-click) flow and by `new_task.rs`'s real-git-repo tests, but
  the drop-triggered entry point itself was not driven end to end.
- **Whether gpui correctly reports drop coordinates on Linux's X11/Wayland
  backends for `FileDropEvent`** was not checked -- this wave's development
  and smoke run were macOS-only (see "macOS 専用" in the repo's top-level
  `CLAUDE.md`); the dispatch-code reading above covers all three platforms'
  source, but only macOS was actually run.

### Scrollback, text selection & copy (wave 5g)

Landed: `labolabo-term`'s scrollback API (`Terminal::scroll`/
`scroll_to_bottom`, `GridSnapshot::{scroll_offset, scrollback_len}`,
`VtBackend::{scroll_display, scroll_to_bottom, alt_screen_active}` on both
backends -- see `crates/labolabo-term/README.md` for the backend-level
design, sign convention, and a worker-thread throttle bug found and fixed
along the way); wheel/trackpad scroll wiring on each pane's canvas
(`app::LaboLaboApp::handle_pane_scroll`, `grid::accumulate_scroll_lines`),
including alt-screen -> cursor-key conversion; mouse-drag text selection
(`selection.rs`'s `CellPos`/`Selection`/`selected_text`, `grid::cell_at`,
`app::LaboLaboApp::begin_selection`/`extend_selection`/`finish_selection`,
`render::paint_grid`'s highlight pass); and Cmd+C copy
(`app::LaboLaboApp::action_copy`) alongside the existing Cmd+V paste. See
"Text selection, scroll & copy" above for the full design and its
by-design limitations (no word/line select, no mouse reporting, a
selection's coordinates aren't stable across a scroll/new-output mid-drag).

Verified locally:

- `cargo build -p labolabo-term`, `cargo clippy -p labolabo-term
  --all-targets -- -D warnings`, and `cargo test -p labolabo-term` (17
  tests, default `backend-alacritty`) all pass, including the new
  scrollback/alt-screen tests in the shared `tests/backend_common.rs`: a
  fresh session reports `scroll_offset`/`scrollback_len` of `0`/`0`;
  flooding more lines than fit the viewport then scrolling all the way back
  reveals the very first line printed (and `scroll_to_bottom` returns to
  the live tail showing the most recent line again); an oversized `scroll`
  delta in either direction clamps to `[0, scrollback_len]` rather than
  panicking; `alt_screen_active()` tracks DECSET `1049` on and back off. Run
  repeatedly (not just once) to rule out flakiness after the throttle fix.
- **`backend-ghostty-vt` was not built or tested this wave** -- this
  development machine has Zig 0.15.2 only (the feature needs 0.16); the
  ghostty backend's scroll/alt-screen/scrollbar code was written by close
  reading of the vendored `libghostty-vt` crate's own source and doc
  comments (not guessed), and is exercised by the same
  backend-agnostic `tests/backend_common.rs` suite the alacritty backend
  is, but has not actually compiled or run. Flagged prominently rather than
  reported as done -- see the crate README's own note on this.
- `cargo build -p labolabo-app`, `cargo clippy -p labolabo-app --all-targets
  -- -D warnings`, and workspace-wide `cargo fmt --check` all pass.
- `cargo test -p labolabo-app` (126 lib tests + 8 `control_cli` integration
  tests): the new `grid` tests (`cell_at`'s exact-boundary/negative-clamp/
  past-the-grid-clamp/degenerate-cell-size cases, `accumulate_scroll_lines`'s
  sub-cell-delta accumulation/carry/negative-direction/degenerate-input
  cases) and the new `selection` module's full suite (`Selection::
  is_empty`/`normalized`/`contains` for single-row, multi-row, and
  backward-dragged selections; `selected_text`'s single-row substring,
  trailing-blank trimming, multi-row newline-joining, and out-of-range
  row/column clamping without panicking) -- plus the full pre-existing
  suite unchanged.
- Root `cargo build`/`cargo test`/`cargo clippy -- -D warnings`/`cargo fmt
  --check` (workspace `default-members`: `labolabo-core` + `labolabo-term`)
  all pass.
- `cargo run -p labolabo-app` smoke run (`LABOLABO_RS_DATA_DIR=$(mktemp
  -d)`, ~5 seconds, then killed): no panic/crash output, process exited
  cleanly on kill, no leftover process.

**Not verified -- no synthetic keyboard/mouse input, on explicit
instruction, same as every wave above. Scroll/selection/copy are gesture
features, so (like drag & drop above) this gap is larger than usual here:**

- **No actual wheel/trackpad scroll has been performed against a running
  app.** The sign convention, pixel-to-line accumulation, and alt-screen
  detection were each traced against the vendored Ghostty source and
  `alacritty_terminal`'s own code (not assumed), and the underlying
  `Terminal::scroll` mechanism is integration-tested headlessly, but the
  actual gpui `ScrollWheelEvent` -> `handle_pane_scroll` -> visibly-scrolled
  pane chain has not been watched happen on screen. **In particular, the
  scroll *direction* (does scrolling up on a real trackpad actually reveal
  older content, not newer) rests on reading macOS/Ghostty source, not on
  physically testing a trackpad gesture -- flagged as the single highest-
  value thing for a human to check first.**
- **No actual mouse-drag text selection has been performed.** The highlight
  paints (`render::paint_grid`'s new pass), the click-starts/drag-extends/
  release-finalizes state machine, and `grid::cell_at`'s pixel-to-cell math
  are each unit-tested or traced through gpui's own mouse-event dispatch,
  but no real click-and-drag over a rendered terminal has been observed.
- **Cmd+C copy has not been exercised against a real selection or a real
  system clipboard.** `selection::selected_text`'s extraction logic is
  unit-tested in isolation; whether a real drag-then-Cmd+C round-trips into
  a paste-able clipboard entry has not been checked end to end.
- **The alt-screen -> cursor-key conversion has not been exercised against
  a real full-screen program** (e.g. scrolling while `vim` or `less` is
  running) -- `alt_screen_active()` itself is integration-tested (DECSET
  `1049` on/off), but scrolling the mouse wheel while actually inside one
  of these programs and confirming it moves the cursor/pages the view
  (rather than scrolling LaboLabo's own history) was not performed.
- **The selection highlight's visual legibility has not been screenshotted**
  -- the chosen alpha (`render::SELECTION_HIGHLIGHT_ALPHA`) is a judgment
  call, not measured against real terminal content on a real display.

### Agent usage / cross-session conflicts / settings screen (wave 5i)

Landed: three small parity items wiring already-ported `labolabo-core`
logic (and one Rust-only addition) into the UI --

- **Agent usage** (`task_workspace.rs`'s `pane_usage`/`format_usage_compact`,
  `app.rs`'s `handle_agent_event`/`refresh_pane_usage`/`apply_pane_usage`):
  a compact `"1.2k tok · $0.08"` label on a pane's tab chip, re-read from
  its transcript (`labolabo_core::transcript_usage::read`) only when that
  pane's hook status transitions to `Idle`/`Ended` -- never polled.
- **Cross-session conflicts** (`git_pane::changed_paths`, `app.rs`'s
  `changed_files_cache`/`task_conflicts`/`compute_task_conflicts`,
  `sidebar.rs`'s ⚠ badge): an orange ⚠ on a Task's sidebar row when another
  Task in the same repo has changed one of the same files, per the
  last-fetched Git status each has cached -- see "Known limitations" above
  for the "only status-fetched Tasks participate" scope decision.
- **Settings screen** (`crate::settings`, `Cmd+,`): an in-window overlay
  (auto-resume toggle, Git-pane-default-visibility toggle, scrollback-lines
  stepper, language picker, and — RC release wave — an "automatically check
  for updates" toggle), persisted through `TaskDatabase` `appState` methods
  (`auto_resume_enabled`/`git_pane_default_visible`/`scrollback_lines`/
  `update_check_enabled` and their `set_*` counterparts). Scrollback itself
  required threading a new
  `max_scrollback: usize` parameter through `labolabo_term`'s `VtBackend::
  new` (both backends) and a new `TermSession::spawn_with_scrollback_options`
  entry point -- see `crates/labolabo-term/README.md`/this crate's own
  "Design" section for that wiring; existing `spawn_with_cwd_options` and
  every pre-existing call site/test are unchanged (they now delegate to the
  new method with `labolabo_term::DEFAULT_MAX_SCROLLBACK`).

Verified locally:

- `cargo build`/`cargo clippy --all-targets -- -D warnings`/`cargo test`/
  `cargo fmt --check` at the workspace root (`default-members`:
  `labolabo-core` + `labolabo-term`) all pass, including a new
  `labolabo-term` integration test (`spawn_with_scrollback_options_caps_
  history_length`, `tests/backend_common.rs`, exercised against the default
  `backend-alacritty`) that floods far more lines than a small explicit
  `max_scrollback` and asserts the backend's reported `scrollback_len`
  lands *exactly* at that cap. This test is also run in CI's separate
  `rust-term-ghostty` job against the real `backend-ghostty-vt` -- an
  earlier version asserting the same exact-cap bound for *that* backend
  failed there (libghostty-vt's pagelist reclaims scrollback in coarse
  page-sized chunks, not synchronously to an exact line count after a small
  burst -- see the test's own doc comment for the real numbers observed).
  The test now only asserts exact capping for alacritty and a weaker
  "accepted, doesn't error, content still readable" check for ghostty-vt --
  an honestly narrower guarantee for that backend, not a second guess.
- Three new `labolabo-core::store::task_database` tests confirm the
  `appState`-backed settings round-trip and default to `None` (not some
  hardcoded value) until first written, and one confirms unparseable
  stored text for `scrollback_lines` degrades to `None` rather than
  erroring.
- `cargo build -p labolabo-app`, `cargo clippy -p labolabo-app --all-targets
  -- -D warnings`, and `cargo test -p labolabo-app` all pass: new unit
  tests cover `format_usage_compact`/`format_compact_count` (small/
  thousands/millions token-count formatting, known- vs. unknown-model cost
  display, the empty-usage/zero-tokens-but-one-turn edge case),
  `git_pane::changed_paths` (staged/unstaged/untracked union, ignored
  entries excluded, unmerged entries *included* unlike `build_changed_items`,
  both sides of a rename), `compute_task_conflicts` (same-repo overlap
  detected, different-repo/never-fetched/unknown-Task-id/empty-Tasks all
  correctly yield no conflicts), and `crate::settings`'s `AppSettings::
  default`/`load` (fresh-database defaults, persisted overrides, per-field
  fallback -- not "any key present skips every default") and
  `adjust_scrollback_lines`'s floor/ceiling clamping.
- `cargo run -p labolabo-app` smoke run (`LABOLABO_RS_DATA_DIR=$(mktemp
  -d)`, ~5 seconds, then killed): no panic/crash output, `tasks.db` created
  under the isolated data dir (confirming `AppSettings::load` ran during
  startup), process exited cleanly on kill.

**Not verified -- no synthetic keyboard/mouse input, on explicit
instruction, same as every wave above:**

- **The settings overlay itself has not been opened/clicked through a real
  window.** `Cmd+,` toggling `settings_open`, the toggle rows' click
  targets, and the -/+ scrollback steppers are each straightforward gpui
  wiring (same shapes already exercised elsewhere in this codebase -- see
  each function's doc comment for which existing pattern it copies), but
  no human has actually opened the panel, clicked a toggle, and confirmed
  the label/checkbox glyph flips and a newly spawned tab picks up the
  change.
- **The tab-chip usage label and sidebar conflict badge have not been seen
  rendered against a real Claude Code session / a real two-Task same-repo
  conflict.** Both are driven by unit-tested pure functions
  (`format_usage_compact`, `compute_task_conflicts`) fed real
  `AgentStatusEvent`/Git-status data in a live run, but no human has driven
  an actual `claude` session to `Idle` and watched the token label appear,
  or opened two worktree Tasks on the same repo and edited the same file in
  both to watch the ⚠ badge appear.
- **`backend-ghostty-vt`'s scrollback-cap wiring was not built or tested on
  this development machine** -- same Zig-toolchain gap as wave 5g's entry
  above (this machine has Zig 0.15.2 only). It *was* built and tested by
  CI's `rust-term-ghostty` job (which does have the 0.16 toolchain), and
  that run is what surfaced the real, coarser-than-alacritty capping
  granularity documented on `spawn_with_scrollback_options_caps_history_
  length`'s doc comment -- so "not tested on this machine" no longer means
  "unverified anywhere," just that this port's own local development loop
  couldn't have caught the exact-cap assumption being wrong without CI.
  What remains genuinely unverified: whether libghostty-vt's pagelist
  *ever* reclaims down toward a tiny configured `max_scrollback` given a
  large enough flood, or whether its practical minimum retained history is
  bounded by something closer to a fixed page size regardless of the
  requested value -- nobody has floods large enough, on that backend, to
  find out yet.

### Wave 5j (terminal-interaction polish: mouse reporting, divider
drag-resize, word/line select)

Landed: three planned polish items, plus two real-machine bug fixes folded
into the same branch --

- **SGR mouse-report forwarding** (`labolabo_term::mouse` -- new
  `MouseTracking`/`MouseMode` types and `VtBackend::mouse_mode`/
  `TermSession::mouse_mode`, mirrored on both backends; `crate::
  mouse_report` -- a pure SGR encoder ported from Ghostty's own
  `mouse_encode.zig`; `app.rs`'s `begin_selection`/`extend_selection`/
  `finish_selection`/`report_mouse_click`/`report_mouse_release`/
  `handle_pane_scroll`): a click/drag/scroll is forwarded to a pane's PTY
  as SGR (DECSET `1006`) escape sequences whenever that pane's own program
  has requested mouse tracking (`1000`/`1002`/`1003`) -- Shift overrides
  this back to local selection for clicks/drags (not scroll), matching
  real Ghostty's own default `mouse-shift-capture` behavior (confirmed by
  reading the vendored source; **not** the Option/Alt convention an
  earlier draft of this wave's brief guessed at). See "Known limitations"
  above for the SGR-only/no-bare-hover/no-horizontal-scroll scope trims.
- **Interactive divider drag-resize** (`labolabo_core::tiling`'s
  `MIN_SPLIT_RATIO`/`MAX_SPLIT_RATIO`/`TileNode::set_ratio`/
  `find_node_mut`/`PaneTilingModel::set_split_ratio`; `task_workspace.rs`'s
  `render_tile` split branch, `DividerDragPayload`; `app.rs`'s
  `update_divider_drag`/`finish_divider_drag`): a thin absolutely-
  positioned handle at a split's boundary, resize cursor + hover highlight,
  dragged via gpui's native drag-and-drop system (`on_drag`/
  `on_drag_move`/`on_drop`, the same mechanism the pre-existing tab-chip
  drag already used) rather than raw mouse-move, so the drag survives the
  cursor briefly outrunning the thin handle's own hitbox during a fast
  flick. `Terminal::resize` is suppressed for every pane in the tree while
  any divider is being dragged (`LaboLaboApp::divider_drag_active`,
  threaded through `render_tile`/`render_leaf`) and applied once, with the
  final size, on drop -- the "間引き" (throttle) the wave's brief asked
  for, done as "finalize once at drag end" rather than a 16ms timer (the
  brief's own wording permits either).
- **Double-click word-select / triple-click line-select**
  (`selection::word_bounds_at`/`line_bounds_at`/`selection_for_click`,
  `crate::selection::WORD_BOUNDARY_CHARS`): a mouse-down's `click_count`
  (gpui's own field) selects the word or line under the cursor instead of
  starting a zero-length click, using Ghostty's own default word-boundary
  character set (confirmed by reading `terminal/selection_codepoints.zig`'s
  `default_word_boundaries` in the vendored source). See "Known
  limitations" above for the single-row/no-drag-resnap scope trims.
- **Bug fix: a mouse-aware alt-screen TUI (e.g. Claude Code's own TUI)
  received arrow-key sequences from a scroll gesture instead of real mouse
  events**, and warned "Scroll wheel is sending arrow keys". Root cause:
  `handle_pane_scroll` converted every alt-screen scroll to cursor keys
  unconditionally, never consulting the program's own mouse tracking.
  Fixed by `handle_pane_scroll`'s new three-way priority (mouse reporting,
  confirmed active via `Terminal::mouse_mode`, always wins; then alt-screen
  + alternate-scroll-mode, DECSET `1007`, itself newly queried via
  `VtBackend::alternate_scroll_active`/`Terminal::alternate_scroll_active`
  rather than assumed always-on; then local scrollback) -- see that
  method's doc comment for the full ordering and the real-Ghostty source
  citations behind each branch.
- **Bug fix: a terminal pane could render stale/wrong-sized content after
  a layout change (e.g. closing the Git side pane) until new PTY output
  arrived.** Root cause, found by an investigation agent reading
  `spawn_redraw_bridge` (`task_workspace.rs`): its post-pacing-sleep
  `drain()` call discarded any `TermEvent::Wakeup` that arrived during the
  16ms `FRAME_INTERVAL` sleep *without* a corresponding `cx.notify()` --
  a resize (whose own snapshot-republish can closely follow the child
  program's own SIGWINCH-triggered redraw) was exactly the kind of event
  prone to landing a second, swallowed `Wakeup` in that window. Fixed by
  removing that discard: anything that arrives during the sleep is simply
  left queued and picked up by the loop's own head (`notify_rx.next()
  .await`, which then resolves immediately) on the next iteration, so every
  real `Wakeup` is now guaranteed an eventual `cx.notify()` call while the
  pacing/coalescing behavior is otherwise unchanged. `labolabo-term`'s own
  resize-publish plumbing (`session.rs`'s `run_worker`) was independently
  confirmed already correct by a new regression test (see below) before
  this UI-layer root cause was found.

Verified locally:

- `cargo build`/`cargo clippy --workspace --all-targets -- -D warnings`/
  `cargo test`/`cargo fmt --check` (workspace root, covering
  `default-members` *and* `-p labolabo-app` together) all pass.
- `cargo build -p labolabo-app`, `cargo clippy -p labolabo-app --all-targets
  -- -D warnings`, and `cargo test -p labolabo-app` (210 tests) all pass in
  isolation too.
- New `labolabo-term` unit tests (`mouse.rs`) and integration tests
  (`tests/backend_common.rs`, exercised against the default
  `backend-alacritty`; also run in CI's `rust-term-ghostty` job against the
  real `backend-ghostty-vt`): `mouse_mode_reflects_decset_1000_1002_1006`
  (Normal -> Button+SGR -> off, via real DECSET sequences from a spawned
  child), `alternate_scroll_defaults_on_and_toggles_via_decset_1007`, and
  `resize_with_existing_content_and_no_further_output_still_republishes`
  (spawns a child, prints content, blocks it in `sleep 5` with zero further
  output, resizes, and confirms a correctly-sized snapshot republishes on
  its own -- direct regression coverage for the redraw-bridge bug above,
  which independently confirmed this crate's own resize-publish plumbing
  was never the actual root cause).
- New `labolabo-app` unit tests: `mouse_report.rs` (17 pure tests --
  Ghostty's `shouldReport`/`buttonCode` gating ported and checked against
  every tracking mode, wheel button-code table, modifier/motion bit
  arithmetic, Shift-overrides-clicks-but-not-scroll), `selection.rs` (15
  new tests -- word-boundary classification against Ghostty's default
  separator set, blank-cell-as-boundary, line-trim-trailing-not-leading,
  click-count dispatch), `grid.rs` (4 new tests -- `ratio_from_drag_
  position`'s midpoint/edges/non-finite-container-length behavior),
  `labolabo-core`'s `tiling.rs` (12 new tests -- `set_ratio` clamping/
  non-finite-input rejection, `find_node_mut` root/leaf/unknown-id lookup,
  `set_split_ratio` end-to-end including a *nested* split and the
  "never bumps `revision`" contract).
- **A genuine end-to-end test spanning both crates**
  (`mouse_report.rs`'s `mouse_scroll_reporting_end_to_end_reaches_the_pty_
  as_sgr_bytes`, Unix-only): spawns a real `labolabo_term::Terminal`, has
  the child enable DECSET `1000`/`1003`/`1006`, polls `Terminal::
  mouse_mode()` (real backend state) until it reports `Any`+SGR, encodes a
  wheel-up press with that *real* queried mode via this crate's own
  `encode_sgr` (the exact call `handle_pane_scroll` makes), writes the
  result through `Terminal::write_input` (same method `handle_pane_scroll`
  calls), and confirms the raw bytes reached the child's stdin unmodified
  by having the child capture them via `stty raw -echo; dd` (bypassing this
  crate's own VT parser entirely, so a mouse-report sequence fed back in as
  *input* -- which no real terminal is meant to interpret -- can't be
  silently swallowed and produce a false pass). Ran it 5 times in a row
  with no flakes.
- `cargo run -p labolabo-app` smoke run (`LABOLABO_RS_DATA_DIR=$(mktemp
  -d)`, ~5 seconds, then killed): no panic/crash output, process exited
  cleanly on kill.

**Not verified -- no synthetic mouse input, on explicit instruction, same
as every wave above:**

- **None of this wave's UI-facing behavior has been driven by a real
  mouse.** The divider handle's cursor icon, hover highlight, drag feel,
  and whether it visually centers correctly on the boundary; double- and
  triple-click actually selecting the expected word/line on screen;
  real mouse-report forwarding into a real running `vim -mouse -mouse=a`/
  `claude` session (rather than the synthetic-DECSET-plus-direct-
  `encode_sgr`-call end-to-end test above, which proves the wiring is
  correct but isn't the same as watching it happen in the actual app
  window) -- none of these have been seen by a human. The redraw-bridge
  fix's *end effect* (a terminal pane staying visually correct after
  toggling the Git pane) is likewise unconfirmed against the real app,
  though the fix directly addresses a concretely-identified defect
  (see above) rather than a guessed-at one.
- **`backend-ghostty-vt`'s new `mouse_mode`/`alternate_scroll_active`
  implementations were not built or tested on this development machine** --
  same Zig-toolchain/linker gap as every earlier wave's ghostty-vt entries
  above (this sandbox's `zig build` fails on missing libSystem symbols
  before it ever reaches this crate's own code, confirmed pre-existing --
  identical failure on a clean `origin/dev` checkout with none of this
  wave's changes applied). Written by directly reading `libghostty-vt`'s
  Rust API source (`Terminal::mode`, the `Mode::X10_MOUSE`/`NORMAL_MOUSE`/
  `BUTTON_MOUSE`/`ANY_MOUSE`/`SGR_MOUSE`/`ALT_SCROLL` constants) and
  mirroring the exact pattern the pre-existing, CI-verified
  `alt_screen_active`/`bracketed_paste` implementations already use on
  that backend -- reasonably low-risk by construction, but CI's
  `rust-term-ghostty` job is what will actually exercise it.
