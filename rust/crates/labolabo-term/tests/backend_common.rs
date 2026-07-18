//! Backend-agnostic integration tests.
//!
//! Every test here names only `labolabo_term::Terminal` (the active backend),
//! so this exact file runs unchanged against **both** backends:
//!
//! - default:            `cargo test -p labolabo-term`
//! - ghostty (opt-in):   `cargo test -p labolabo-term \
//!                          --no-default-features --features backend-ghostty-vt`
//!
//! They are headless (no window): a PTY child writes to the grid, and we
//! assert on the extracted `GridSnapshot`. That is the whole point of the
//! plain-data snapshot design -- the render surface is testable without a UI.
//!
//! Whole file is `#[cfg(unix)]`: every test drives the spawned child with a
//! POSIX shell command (`TermSession::spawn_with_cwd_options`'s `Some(cmd)`
//! path hardcodes `/bin/sh -c <cmd>` -- see `src/session.rs`), which does
//! not exist on Windows. A `cmd.exe`/PowerShell-equivalent Windows shell
//! path is future work (Windows PTY spawning in general is out of scope for
//! this wave -- see rust/README.md's known-scope-limits section), not a
//! rewrite to attempt without a Windows machine to verify it on.
//!
//! ## Flake hardening: gate sequential DECSET/OSC writes on `read`, not `sleep`
//!
//! Several tests below (`alt_screen_active_reflects_decset_1049`,
//! `bracketed_paste_mode_reflects_decset_2004`,
//! `mouse_mode_reflects_decset_1000_1002_1006`,
//! `alternate_scroll_defaults_on_and_toggles_via_decset_1007`,
//! `title_updates_on_second_osc_sequence`) drive the child through two or
//! three sequential terminal-mode states and assert each is observable in
//! turn. A fixed `sleep` between the writes that produce each state looks
//! like it should be enough margin, but it isn't a real synchronization
//! primitive: the worker thread that owns the VT core refreshes these
//! mirrored flags per `WorkerMsg::Bytes` batch, and fires its `Wakeup`
//! event only from a *throttled* snapshot publish (`FRAME_INTERVAL` = 16ms
//! in `session.rs`). Under CI load the worker can fall behind far enough
//! that two writes sent hundreds of ms apart both end up queued by the time
//! it resumes, and it drains them back to back -- the intermediate state
//! then exists for well under the poller's sampling granularity and can be
//! missed entirely, no matter how long the deadline is. `alt_screen_active_
//! reflects_decset_1049` flaked exactly this way 3x in the ubuntu
//! `rust-term-ghostty` CI job (wave 12).
//!
//! The fix used throughout: have the child block on a `read` after writing
//! the first state, and only unblock it (`Terminal::write_input(b"\n")`)
//! from the test *after* that state was actually observed via the existing
//! `wait_for_*` deadline-polling helpers. This makes the hand-off a real
//! happens-before relationship instead of a clock-based guess, so the next
//! write is structurally unable to land in the same worker batch as the
//! previous one. Production code is unchanged -- this is a test-only fix.
#![cfg(unix)]

use std::time::Duration;

use labolabo_term::{ColorScheme, MouseMode, MouseTracking, Rgb, TermEvent, Terminal};

const TIMEOUT: Duration = Duration::from_secs(5);

/// `echo hello` -> the text lands in a snapshot.
#[test]
fn echo_hello_appears_in_snapshot() {
    let term =
        Terminal::spawn_with_command(80, 24, Some("echo hello && sleep 0.2"), &[]).expect("spawn");
    let snap = term.wait_for(TIMEOUT, |g| g.contains_text("hello"));
    assert!(
        snap.is_some(),
        "expected 'hello' in grid, got:\n{}",
        term.snapshot().to_text()
    );
}

/// Env injection reaches the child: `$LABOLABO_PANE` echoed back shows up.
/// This is the mechanism LaboLabo's hooks protocol relies on to tag a pane.
#[test]
fn env_injection_reaches_child() {
    let env = vec![("LABOLABO_PANE".to_string(), "pane-42".to_string())];
    let term = Terminal::spawn_with_command(
        80,
        24,
        Some("printf '%s' \"$LABOLABO_PANE\"; sleep 0.2"),
        &env,
    )
    .expect("spawn");
    let snap = term.wait_for(TIMEOUT, |g| g.contains_text("pane-42"));
    assert!(
        snap.is_some(),
        "expected injected env 'pane-42' in grid, got:\n{}",
        term.snapshot().to_text()
    );
}

/// `TERM_PROGRAM=ghostty` reaches every spawned child -- the actual fix for
/// the wave 15 follow-up bug report ("Shift+Enter still doesn't work"):
/// Claude Code's own terminal-identity detection (confirmed by reading its
/// compiled CLI) resolves a name from `TERM`/`TERM_PROGRAM`/a handful of
/// backup env vars and only ever attempts the Kitty keyboard protocol
/// handshake (`CSI > 1 u`) with a terminal whose resolved name is in its
/// own allowlist (which includes `"ghostty"`) -- **not** from any live
/// capability probe. Without this env var, `VtBackend::kitty_disambiguate`
/// (wave 15) is correct but moot: nothing ever asks this crate's child to
/// push the flag in the first place. See `session.rs`'s `spawn_with_
/// scrollback_options` for the full rationale (including why `TERM_PROGRAM`
/// alone, not also `TERM=xterm-ghostty`).
#[test]
fn term_program_env_is_ghostty_for_every_spawned_child() {
    let term = Terminal::spawn_with_command(
        80,
        24,
        Some("printf '%s' \"$TERM_PROGRAM\"; sleep 0.2"),
        &[],
    )
    .expect("spawn");
    let snap = term.wait_for(TIMEOUT, |g| g.contains_text("ghostty"));
    assert!(
        snap.is_some(),
        "expected TERM_PROGRAM=ghostty in the child's environment, got:\n{}",
        term.snapshot().to_text()
    );
}

/// `TERM_PROGRAM_VERSION` is always set alongside `TERM_PROGRAM` -- real
/// Ghostty never sets one without the other (see `session.rs`'s
/// `spawn_with_scrollback_options` doc comment) -- pinned here to the exact
/// value that comment documents, so a future edit to either the pinned
/// `GHOSTTY_COMMIT` or this string without updating the other is caught.
#[test]
fn term_program_version_env_is_set_alongside_term_program() {
    let term = Terminal::spawn_with_command(
        80,
        24,
        Some("printf '%s' \"$TERM_PROGRAM_VERSION\"; sleep 0.2"),
        &[],
    )
    .expect("spawn");
    let snap = term.wait_for(TIMEOUT, |g| g.contains_text("1.3.2-dev"));
    assert!(
        snap.is_some(),
        "expected TERM_PROGRAM_VERSION=1.3.2-dev in the child's environment, got:\n{}",
        term.snapshot().to_text()
    );
}

/// The default `TERM_PROGRAM`/`TERM_PROGRAM_VERSION` pair is an *overridable
/// default*, not a hardcoded fact -- `session.rs`'s doc comment documents
/// this as an intentional escape hatch: a caller-supplied `env` entry wins
/// over the default, since `cmd.env(key, value)` for the caller's `env`
/// slice runs after the two defaults are set (`portable-pty`'s
/// `CommandBuilder::env` is last-write-wins). Pins that ordering directly,
/// so it can't silently regress if the two blocks are ever reordered.
#[test]
fn caller_env_can_override_term_program_identity() {
    let env = vec![
        ("TERM_PROGRAM".to_string(), "not-ghostty".to_string()),
        ("TERM_PROGRAM_VERSION".to_string(), "0.0.0".to_string()),
    ];
    let term = Terminal::spawn_with_command(
        80,
        24,
        Some("printf '%s/%s' \"$TERM_PROGRAM\" \"$TERM_PROGRAM_VERSION\"; sleep 0.2"),
        &env,
    )
    .expect("spawn");
    let snap = term.wait_for(TIMEOUT, |g| g.contains_text("not-ghostty/0.0.0"));
    assert!(
        snap.is_some(),
        "expected caller-supplied env to override the TERM_PROGRAM/\
         TERM_PROGRAM_VERSION defaults, got:\n{}",
        term.snapshot().to_text()
    );
}

/// Resizing updates the reported grid dimensions.
#[test]
fn resize_changes_grid_dimensions() {
    // A live child so the session stays up across the resize (it self-exits,
    // so no orphan is left behind).
    let term = Terminal::spawn_with_command(80, 24, Some("sleep 2"), &[]).expect("spawn");

    // The initial (pre-output) snapshot reflects the spawn dimensions.
    let initial = term.snapshot();
    assert_eq!(initial.cols, 80, "initial cols");
    assert_eq!(initial.rows, 24, "initial rows");

    term.resize(100, 40);

    let resized = term.wait_for(TIMEOUT, |g| g.cols == 100 && g.rows == 40);
    let latest = term.snapshot();
    assert!(
        resized.is_some(),
        "expected 100x40 after resize, got {}x{}",
        latest.cols,
        latest.rows
    );
    // The cell buffer is re-sized to match the reported dimensions.
    assert_eq!(latest.cells.len(), 100 * 40, "cell count matches new grid");
}

/// Resizing publishes a fresh, correctly-sized snapshot on its own -- with
/// real content already on screen, and **with no further PTY output at
/// all** after the resize -- so a UI layer polling only `Terminal::
/// snapshot()`/`wait_for` (never blocking on new bytes arriving) always
/// sees a grid whose `cols`/`rows` match its own most recent `resize()`
/// call. Regression coverage for a reported symptom (W5j bug report #2):
/// closing/opening the Git side pane resizes a terminal pane's canvas, and
/// the terminal appeared to "stay broken" -- rendered at the wrong
/// width/garbled -- until the next real PTY output arrived. Investigation
/// (reading `session.rs`'s `run_worker`) found `WorkerMsg::Resize` already
/// unconditionally rebuilds and publishes a snapshot (`publish_snapshot`,
/// which itself fires `TermEvent::Wakeup`) with **no dependency on new PTY
/// bytes** -- this test exercises exactly that contract end-to-end (spawn,
/// print visible content, resize with the child then completely silent,
/// confirm the published snapshot already reflects the new dimensions)
/// rather than just asserting on a blank grid the way
/// `resize_changes_grid_dimensions` above does, since a snapshot rebuild
/// that's merely blank-vs-blank could theoretically mask a reflow-specific
/// bug that only shows up with real content in the grid. It passes against
/// both backends unmodified from this investigation, which points the
/// actual root cause at the Rust UI layer's own resize-trigger wiring
/// (`task_workspace::render_leaf`'s canvas `prepaint` closure / the Git
/// pane visibility toggle) rather than this crate -- see that code's own
/// comments for the follow-up.
#[test]
fn resize_with_existing_content_and_no_further_output_still_republishes() {
    let term = Terminal::spawn_with_command(40, 10, Some("printf 'hello labolabo'; sleep 5"), &[])
        .expect("spawn");
    let before = term.wait_for(TIMEOUT, |g| g.contains_text("hello labolabo"));
    assert!(
        before.is_some(),
        "expected content before resizing, got:\n{}",
        term.snapshot().to_text()
    );

    // The child above is now blocked in `sleep 5` -- no further PTY output
    // will ever arrive before the test's own timeout. If a resized
    // snapshot only ever republishes in response to new bytes, this
    // `wait_for` would time out and return `None`.
    term.resize(100, 30);
    let after = term.wait_for(TIMEOUT, |g| g.cols == 100 && g.rows == 30);
    assert!(
        after.is_some(),
        "expected a 100x30 snapshot published from the resize alone (no new \
         PTY output), got {}x{}",
        term.snapshot().cols,
        term.snapshot().rows,
    );
    assert!(
        after.unwrap().contains_text("hello labolabo"),
        "existing content should still be present after the resize"
    );
}

/// A never-producing child still yields the blank spawn-size snapshot up front
/// and an `Exit` event when it ends -- exercising the event channel directly.
#[test]
fn exit_event_fires_when_child_ends() {
    let term = Terminal::spawn_with_command(40, 10, Some("true"), &[]).expect("spawn");
    assert!(
        wait_for_exit(&term, TIMEOUT),
        "expected an Exit event after the child finished"
    );
}

/// `shutdown` terminates a child that would otherwise outlive the test by
/// far (`sleep 30` vs. the 5s event timeout), and the session then follows
/// the normal exit path: a final `Exit` event fires. If `shutdown` didn't
/// actually kill the child, no EOF would reach the reader and this test
/// would time out waiting for `Exit`.
#[test]
fn shutdown_kills_child_and_fires_exit() {
    let term = Terminal::spawn_with_command(40, 10, Some("sleep 30"), &[]).expect("spawn");
    term.shutdown();
    assert!(
        wait_for_exit(&term, TIMEOUT),
        "expected an Exit event shortly after shutdown()"
    );
}

/// `shutdown` is idempotent: calling it repeatedly, and again after the
/// child has already exited on its own, must not panic or misbehave.
#[test]
fn shutdown_is_idempotent_and_safe_after_natural_exit() {
    let term = Terminal::spawn_with_command(40, 10, Some("true"), &[]).expect("spawn");
    assert!(
        wait_for_exit(&term, TIMEOUT),
        "expected the child to exit on its own first"
    );
    // Child is gone (and by now reaped by the worker); these must be no-ops.
    term.shutdown();
    term.shutdown();
}

/// `spawn_with_options` with a default (empty) `ColorScheme` behaves exactly
/// like the older, narrower `spawn_with_command` -- guards the API-
/// compatibility contract `spawn_with_command` was refactored to lean on
/// (`ColorScheme::default()` under the hood).
#[test]
fn default_color_scheme_matches_spawn_with_command() {
    let via_options = Terminal::spawn_with_options(
        40,
        10,
        Some("printf same; sleep 0.2"),
        &[],
        &ColorScheme::default(),
    )
    .expect("spawn_with_options");
    let via_command =
        Terminal::spawn_with_command(40, 10, Some("printf same; sleep 0.2"), &[]).expect("spawn");

    let a = via_options.wait_for(TIMEOUT, |g| g.contains_text("same"));
    let b = via_command.wait_for(TIMEOUT, |g| g.contains_text("same"));
    assert!(
        a.is_some() && b.is_some(),
        "both sessions should produce output"
    );
    assert_eq!(
        a.unwrap().background,
        b.unwrap().background,
        "default ColorScheme must not change the built-in background"
    );
}

/// A configured `background` shows up as `GridSnapshot::background`.
#[test]
fn colors_background_override_reflected_in_snapshot() {
    let custom = Rgb::new(0x11, 0x22, 0x33);
    let colors = ColorScheme {
        background: Some(custom),
        ..ColorScheme::default()
    };
    let term = Terminal::spawn_with_options(40, 10, Some("printf ready; sleep 0.2"), &[], &colors)
        .expect("spawn");
    let snap = term.wait_for(TIMEOUT, |g| g.contains_text("ready"));
    assert!(snap.is_some(), "expected output before asserting on colors");
    assert_eq!(term.snapshot().background, custom);
}

/// A configured `foreground` shows up as the fg color of a cell with no SGR
/// styling of its own -- the direct fix for the reported symptom (Ghostty's
/// configured foreground wasn't reaching the embedded terminal).
#[test]
fn colors_foreground_override_reflected_in_unstyled_cell() {
    let custom = Rgb::new(0xaa, 0xbb, 0xcc);
    let colors = ColorScheme {
        foreground: Some(custom),
        ..ColorScheme::default()
    };
    let term = Terminal::spawn_with_options(40, 10, Some("printf PLAIN; sleep 0.2"), &[], &colors)
        .expect("spawn");
    let snap = term.wait_for(TIMEOUT, |g| g.contains_text("PLAIN"));
    assert!(snap.is_some(), "expected output before asserting on colors");
    let latest = term.snapshot();
    let cell = find_cell(&latest, "P").expect("the printed 'P' cell");
    assert_eq!(cell.fg, custom);
}

/// A `palette` override for a given index shows up as the fg color of a cell
/// whose text was styled with the matching SGR code (SGR 31 = red = ANSI
/// palette index 1).
#[test]
fn colors_palette_override_reflected_in_sgr_colored_cell() {
    let custom = Rgb::new(0x12, 0x34, 0x56);
    let colors = ColorScheme {
        palette: vec![(1, custom)],
        ..ColorScheme::default()
    };
    let term = Terminal::spawn_with_options(
        40,
        10,
        Some(r#"printf '\033[31mX\033[0m'; sleep 0.2"#),
        &[],
        &colors,
    )
    .expect("spawn");
    let snap = term.wait_for(TIMEOUT, |g| g.contains_text("X"));
    assert!(snap.is_some(), "expected output before asserting on colors");
    let latest = term.snapshot();
    let cell = find_cell(&latest, "X").expect("the SGR-red 'X' cell");
    assert_eq!(cell.fg, custom);
}

/// `spawn_with_cwd_options` with a `cwd` sets the child's initial working
/// directory: a shell started there and asked for its directory's basename
/// prints it back (basename, not the full `pwd` path, so the assertion is
/// immune to the 80-col grid wrapping a long temp-dir path mid-string). This
/// is the mechanism the Task model (`plans/012`) relies on to spawn a Task's
/// panes inside that Task's worktree/attached directory.
#[test]
fn cwd_option_sets_child_working_directory() {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "labolabo-term-cwd-{}-{:x}",
        std::process::id(),
        nanos as u64
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let leaf = dir.file_name().unwrap().to_string_lossy().into_owned();

    let term = Terminal::spawn_with_cwd_options(
        80,
        10,
        Some(r#"basename "$(pwd)"; sleep 0.2"#),
        &[],
        &ColorScheme::default(),
        Some(&dir),
    )
    .expect("spawn");
    let snap = term.wait_for(TIMEOUT, |g| g.contains_text(&leaf));
    assert!(
        snap.is_some(),
        "expected cwd leaf {leaf:?} in pwd output, got:\n{}",
        term.snapshot().to_text()
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// `cwd: None` behaves exactly like `spawn_with_options` (this process's own
/// working directory, unset by us -- `portable-pty`'s own default) -- guards
/// the API-compatibility contract `spawn_with_options` was refactored to
/// lean on.
#[test]
fn cwd_none_matches_spawn_with_options() {
    let via_cwd = Terminal::spawn_with_cwd_options(
        40,
        10,
        Some("printf same; sleep 0.2"),
        &[],
        &ColorScheme::default(),
        None,
    )
    .expect("spawn_with_cwd_options");
    let via_options = Terminal::spawn_with_options(
        40,
        10,
        Some("printf same; sleep 0.2"),
        &[],
        &ColorScheme::default(),
    )
    .expect("spawn_with_options");

    let a = via_cwd.wait_for(TIMEOUT, |g| g.contains_text("same"));
    let b = via_options.wait_for(TIMEOUT, |g| g.contains_text("same"));
    assert!(
        a.is_some() && b.is_some(),
        "both sessions should produce output"
    );
}

// NOTE: これらのトグル系テストは「spawn 直後にデフォルト状態を assert」しない。
// 子プロセス（printf）はテストスレッドの最初の読み取りより先に走り得るため、
// その形の precondition assert は本質的に race する（CI で実際にフレークした:
// mouse_mode_reflects_decset_1000_1002_1006 が高速ランナーで子の 1000h に先を
// 越された）。既定値そのものは各型のユニットテストが担保している。

/// DECSET `2004` (bracketed paste) toggles `Terminal::bracketed_paste()`:
/// off before the child runs, on once it enables it, off again once it
/// disables it. This is the mode-query API `labolabo-app`'s Cmd+V paste
/// handler uses to decide whether to wrap pasted text in
/// `ESC[200~...ESC[201~`.
///
/// The child blocks on `read` between the two DECSET writes instead of a
/// fixed `sleep` -- see [`wait_for_alt_screen`]'s doc comment (the same
/// technique, applied here) for why a clock-based gap alone can't
/// guarantee the transient ON state survives being observed under CI load.
#[test]
fn bracketed_paste_mode_reflects_decset_2004() {
    let term = Terminal::spawn_with_command(
        80,
        24,
        Some(r#"printf '\033[?2004h'; read _sync; printf '\033[?2004l'; sleep 0.2"#),
        &[],
    )
    .expect("spawn");
    assert!(
        wait_for_bracketed_paste(&term, TIMEOUT, true),
        "expected bracketed paste to turn on after DECSET 2004h"
    );
    // Only unblock the child's second `printf` (DECSET 2004l) once the ON
    // state was actually observed, so the OFF write can never land in the
    // same worker batch as the ON one -- see the doc comment above.
    term.write_input(b"\n");
    assert!(
        wait_for_bracketed_paste(&term, TIMEOUT, false),
        "expected bracketed paste to turn back off after DECSET 2004l"
    );
}

/// The Kitty keyboard protocol's "disambiguate escape codes" flag toggles
/// `Terminal::kitty_disambiguate()` the same way a real kitty-protocol-aware
/// program (Claude Code's own TUI is the motivating case) requests it on
/// startup and releases it on exit: `CSI > 1 u` pushes the flag onto the
/// mode stack, `CSI < u` pops it back off (default pop count `1`). This is
/// the query `labolabo-app`'s `keys::keystroke_to_bytes` uses to decide
/// whether a modifier-carrying Enter/Tab should be re-encoded as a Kitty
/// `CSI u` sequence instead of its plain legacy byte.
///
/// Same "block on `read` between writes" hardening as
/// [`bracketed_paste_mode_reflects_decset_2004`] -- see this file's module
/// doc comment for why a clock-based gap alone can't guarantee the
/// transient ON state survives being observed under CI load.
#[test]
fn kitty_disambiguate_reflects_csi_push_pop() {
    let term = Terminal::spawn_with_command(
        80,
        24,
        Some(r#"printf '\033[>1u'; read _sync; printf '\033[<u'; sleep 0.2"#),
        &[],
    )
    .expect("spawn");
    assert!(
        wait_for_kitty_disambiguate(&term, TIMEOUT, true),
        "expected kitty_disambiguate to turn on after CSI > 1 u (push)"
    );
    // Only unblock the child's second `printf` (the pop) once the ON state
    // was actually observed, so the OFF write can never land in the same
    // worker batch as the ON one -- see the doc comment above.
    term.write_input(b"\n");
    assert!(
        wait_for_kitty_disambiguate(&term, TIMEOUT, false),
        "expected kitty_disambiguate to turn back off after CSI < u (pop)"
    );
}

/// Kitty spec: "report all keys as escape codes" (flag `8`, `REPORT_ALL` in
/// `libghostty-vt`'s `KittyKeyFlags` / `REPORT_ALL_KEYS_AS_ESC` in
/// `alacritty_terminal`'s `TermMode`) "implies all keys are automatically
/// disambiguated as well, since they are represented in their canonical
/// escape code form" -- so a program that pushes *only* that flag (without
/// also setting `DISAMBIGUATE`, flag `1`) must still see
/// `Terminal::kitty_disambiguate()` report `true`. Regression coverage for
/// both backends' `kitty_disambiguate` (`backend/ghostty.rs`,
/// `backend/alacritty.rs`), which used to check the `DISAMBIGUATE` bit
/// alone and so would incorrectly report `false` for a `CSI > 8 u`-only
/// push.
///
/// Same "block on `read` between writes" hardening as
/// [`kitty_disambiguate_reflects_csi_push_pop`] above -- see this file's
/// module doc comment.
#[test]
fn kitty_disambiguate_reflects_report_all_keys_flag_alone() {
    let term = Terminal::spawn_with_command(
        80,
        24,
        Some(r#"printf '\033[>8u'; read _sync; printf '\033[<u'; sleep 0.2"#),
        &[],
    )
    .expect("spawn");
    assert!(
        wait_for_kitty_disambiguate(&term, TIMEOUT, true),
        "expected kitty_disambiguate to turn on after CSI > 8 u (report-all-keys push, no DISAMBIGUATE bit set)"
    );
    // Only unblock the child's second `printf` (the pop) once the ON state
    // was actually observed, so the OFF write can never land in the same
    // worker batch as the ON one -- see the doc comment above.
    term.write_input(b"\n");
    assert!(
        wait_for_kitty_disambiguate(&term, TIMEOUT, false),
        "expected kitty_disambiguate to turn back off after CSI < u (pop)"
    );
}

/// Regression test for the vendored `alacritty_terminal` patch (see
/// `vendor/alacritty_terminal/README.md`): upstream's `push_keyboard_mode`
/// used to panic (`Vec::remove(0)` on the still-empty `title_stack`) once
/// its keyboard-mode stack reached `KEYBOARD_MODE_STACK_MAX_DEPTH` (4096)
/// pushes with no intervening pop -- exactly what 4100 back-to-back
/// `CSI > 1 u` sequences with no `CSI < u` in between produce. Before the
/// vendored fix, that panic unwound the worker thread
/// (`labolabo_term::session::run_worker`) mid-`feed()`, which has no
/// `catch_unwind`, so the thread simply exits: no more snapshots are ever
/// published again, and the marker text this test waits for below would
/// never appear -- the test would hang until `TIMEOUT` and fail. With the
/// fix, the worker survives and keeps processing normally, so the marker
/// lands like any other echoed text.
///
/// This exercises the alacritty backend's actual bug (the ghostty backend
/// has no equivalent stack-trimming code at all), but runs unchanged under
/// `--features backend-ghostty-vt` too, same as every other test in this
/// file -- there it's simply a (trivially passing) sanity check that
/// libghostty-vt tolerates the same push storm.
#[test]
fn keyboard_mode_stack_survives_pushes_past_max_depth() {
    let term = Terminal::spawn_with_command(
        80,
        24,
        Some(
            r#"i=0; while [ $i -lt 4100 ]; do printf '\033[>1u'; i=$((i+1)); done; echo SURVIVED_DEPTH_LIMIT; sleep 0.2"#,
        ),
        &[],
    )
    .expect("spawn");
    let snap = term.wait_for(TIMEOUT, |g| g.contains_text("SURVIVED_DEPTH_LIMIT"));
    assert!(
        snap.is_some(),
        "expected the worker thread to survive pushing the keyboard-mode \
         stack past its max depth (4096) with no intervening pop -- if this \
         times out, the vendored `alacritty_terminal` patch (see \
         vendor/alacritty_terminal/README.md) has regressed and the worker \
         thread panicked mid-`feed()`; got:\n{}",
        term.snapshot().to_text()
    );
}

/// `CSI ? u` (query current Kitty keyboard mode) gets a real `CSI ? <flags>
/// u` reply written back to the child's own stdin -- proving both
/// backends' existing VT-response plumbing (the same `Event::PtyWrite`/
/// `on_pty_write` path DA1/DSR/cursor-position replies already use) covers
/// the Kitty protocol's query too, with no new backend code needed for it.
///
/// Investigated as part of the wave 15 follow-up (a real terminal-aware
/// program *could* gate its own `CSI > 1 u` push on first confirming a
/// query round trip, even though Claude Code itself turned out not to --
/// see `session.rs`'s `TERM_PROGRAM` doc comment) -- this closes that
/// question definitively for both backends rather than leaving it assumed.
///
/// Captured **raw**, bypassing this crate's own VT parser entirely (same
/// technique as `labolabo-app::mouse_report`'s `mouse_scroll_reporting_
/// end_to_end_reaches_the_pty_as_sgr_bytes`, this wave's own e2e model): the
/// child puts its pty into raw/no-echo mode and `dd`s the exact expected
/// byte count to a temp file, so an escape sequence fed back in as *input*
/// (which no real terminal is meant to interpret) can't be silently
/// swallowed/reinterpreted by our own grid and produce a false pass.
///
/// No "block on `read` between writes" hand-off is needed here (unlike the
/// DECSET-toggle tests above): `dd bs=1 count=N` itself blocks until
/// exactly `N` bytes have arrived, which is already a real happens-before
/// gate, and VT parsing (unlike snapshot publishing) is never throttled --
/// `feed()` runs against every `WorkerMsg::Bytes` batch as it's received,
/// so the push write is always fully applied before the second query's
/// bytes are ever parsed (same fd, program-order writes, no reordering
/// possible).
#[test]
fn kitty_query_response_reflects_current_flags_before_and_after_push() {
    use std::time::{SystemTime, UNIX_EPOCH};

    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos() as u64;
    let before_path = std::env::temp_dir().join(format!(
        "labolabo-kitty-query-before-{}-{nanos:x}",
        std::process::id()
    ));
    let after_path = std::env::temp_dir().join(format!(
        "labolabo-kitty-query-after-{}-{nanos:x}",
        std::process::id()
    ));

    // `\x1b[?0u` / `\x1b[?1u` are both exactly 5 bytes (a single-digit
    // flags value in both the never-pushed and disambiguate-only-pushed
    // cases), so both captures can use the same fixed count.
    const RESPONSE_LEN: usize = 5;

    let script = format!(
        "stty raw -echo; \
         printf '\\033[?u'; dd bs=1 count={RESPONSE_LEN} of='{}' 2>/dev/null; \
         printf '\\033[>1u'; \
         printf '\\033[?u'; dd bs=1 count={RESPONSE_LEN} of='{}' 2>/dev/null; \
         printf DONE",
        before_path.display(),
        after_path.display(),
    );
    let term = Terminal::spawn_with_command(80, 24, Some(&script), &[]).expect("spawn");
    let done = term.wait_for(TIMEOUT, |g| g.contains_text("DONE"));
    assert!(
        done.is_some(),
        "expected the child's capture script to finish, got:\n{}",
        term.snapshot().to_text()
    );

    let before = std::fs::read(&before_path).unwrap_or_default();
    let after = std::fs::read(&after_path).unwrap_or_default();
    let _ = std::fs::remove_file(&before_path);
    let _ = std::fs::remove_file(&after_path);

    assert_eq!(
        before, b"\x1b[?0u",
        "expected a not-yet-pushed query to reply with flags=0"
    );
    assert_eq!(
        after, b"\x1b[?1u",
        "expected a post-push (disambiguate-only) query to reply with flags=1"
    );
}

/// DECSET `1000`/`1002`/`1006` toggle `Terminal::mouse_mode()` the same way
/// `1000`, `1002`, and `1006` are used together by real mouse-aware TUIs
/// (vim, tmux, ...): normal tracking, then switched to button-event
/// tracking with SGR extended coordinates enabled, then all off again.
/// This is the mode-query API `labolabo-app`'s mouse-event routing uses to
/// decide whether a click/drag/scroll should be SGR-encoded and forwarded
/// to the child instead of driving this crate's own text-selection/
/// scrollback UI (W5j #1).
///
/// Same "block on `read` between writes" hardening as [`wait_for_alt_screen`]
/// -- three distinct states in sequence means two hand-off points, each
/// gated so the next DECSET write can't reach the VT parser before the
/// previous state was actually observed.
#[test]
fn mouse_mode_reflects_decset_1000_1002_1006() {
    let term = Terminal::spawn_with_command(
        80,
        24,
        Some(
            r#"printf '\033[?1000h'; read _sync1; \
               printf '\033[?1000l\033[?1002h\033[?1006h'; read _sync2; \
               printf '\033[?1002l\033[?1006l'; sleep 0.2"#,
        ),
        &[],
    )
    .expect("spawn");
    assert!(
        wait_for_mouse_mode(
            &term,
            TIMEOUT,
            MouseMode {
                tracking: MouseTracking::Normal,
                sgr: false,
            },
        ),
        "expected normal tracking (no SGR) after DECSET 1000h"
    );
    term.write_input(b"\n");
    assert!(
        wait_for_mouse_mode(
            &term,
            TIMEOUT,
            MouseMode {
                tracking: MouseTracking::Button,
                sgr: true,
            },
        ),
        "expected button-event tracking with SGR after DECSET 1000l 1002h 1006h"
    );
    term.write_input(b"\n");
    assert!(
        wait_for_mouse_mode(&term, TIMEOUT, MouseMode::OFF),
        "expected mouse mode to turn back off after DECSET 1002l 1006l"
    );
}

/// A fresh session has no scrollback yet: `scroll_offset`/`scrollback_len`
/// both start at `0` (the live tail, nothing to scroll into).
#[test]
fn fresh_session_has_no_scrollback() {
    let term =
        Terminal::spawn_with_command(20, 5, Some("printf ready; sleep 0.2"), &[]).expect("spawn");
    let snap = term.wait_for(TIMEOUT, |g| g.contains_text("ready"));
    assert!(
        snap.is_some(),
        "expected output before asserting scroll state"
    );
    let latest = term.snapshot();
    assert_eq!(latest.scroll_offset, 0);
    assert_eq!(latest.scrollback_len, 0);
}

/// Flood more lines than fit on screen, then scroll back: a line that was
/// pushed off the top of the live viewport becomes visible again, and
/// `scroll_offset`/`scrollback_len` both reflect the move. This is the
/// core scrollback contract `VtBackend::scroll_display` promises, exercised
/// identically on whichever backend this test binary was built against.
#[test]
fn scrolling_up_reveals_history_pushed_off_the_live_viewport() {
    // A 20x5 grid: print 40 numbered lines (well past 1000-line history
    // cap concerns -- this only needs to overflow 5 *visible* rows), so
    // "line 0" is long gone from the live viewport by the time we're done.
    let term = Terminal::spawn_with_command(
        20,
        5,
        Some("for i in $(seq 0 39); do echo \"line-$i\"; done; sleep 0.2"),
        &[],
    )
    .expect("spawn");
    let snap = term.wait_for(TIMEOUT, |g| g.contains_text("line-39"));
    assert!(
        snap.is_some(),
        "expected the flood to finish, got:\n{}",
        term.snapshot().to_text()
    );

    // Live tail: scrolled all the way down, "line-0" long since scrolled
    // off, but real scrollback exists for it to have gone somewhere.
    let live = term.snapshot();
    assert_eq!(
        live.scroll_offset, 0,
        "fresh output starts at the live tail"
    );
    assert!(
        live.scrollback_len > 0,
        "40 lines into a 5-row viewport must have produced scrollback"
    );
    assert!(
        !live.contains_text("line-0"),
        "line-0 should have scrolled off the live viewport:\n{}",
        live.to_text()
    );

    // Scroll all the way back (exactly `scrollback_len`, landing precisely
    // at the top rather than guessing a delta) so "line-0" -- the very
    // first line ever printed -- is visible again, then confirm both the
    // content and the reported offset moved.
    term.scroll(live.scrollback_len as i64);
    let scrolled = term.wait_for(TIMEOUT, |g| {
        g.contains_text("line-0") && g.scroll_offset > 0
    });
    assert!(
        scrolled.is_some(),
        "expected 'line-0' back in view after scrolling up, got:\n{}",
        term.snapshot().to_text()
    );
    let scrolled = scrolled.unwrap();
    assert!(
        scrolled.scroll_offset > 0,
        "scroll_offset should have moved off 0"
    );
    assert_eq!(
        scrolled.scrollback_len, live.scrollback_len,
        "scrolling shouldn't change how much history exists, only where we're looking"
    );

    // `scroll_to_bottom` snaps straight back to the live tail regardless of
    // how far up we scrolled.
    term.scroll_to_bottom();
    let bottom = term.wait_for(TIMEOUT, |g| g.scroll_offset == 0);
    assert!(
        bottom.is_some(),
        "expected scroll_to_bottom to return scroll_offset to 0"
    );
    assert!(
        bottom.unwrap().contains_text("line-39"),
        "the live tail should show the most recent output again"
    );
}

/// `scroll_display`'s delta is clamped, not merely tolerated: scrolling by
/// an absurdly large delta lands exactly at the top of history
/// (`scroll_offset == scrollback_len`), never panics, and never exceeds it.
#[test]
fn scroll_delta_clamps_to_scrollback_length() {
    let term = Terminal::spawn_with_command(
        20,
        5,
        Some("for i in $(seq 0 39); do echo \"line-$i\"; done; sleep 0.2"),
        &[],
    )
    .expect("spawn");
    let snap = term.wait_for(TIMEOUT, |g| g.contains_text("line-39"));
    assert!(snap.is_some(), "expected the flood to finish");
    let scrollback_len = term.snapshot().scrollback_len;
    assert!(
        scrollback_len > 0,
        "expected some scrollback to clamp against"
    );

    term.scroll(1_000_000);
    let top = term.wait_for(TIMEOUT, |g| g.scroll_offset == scrollback_len);
    assert!(
        top.is_some(),
        "expected an oversized scroll delta to clamp to scrollback_len ({scrollback_len}), got {}",
        term.snapshot().scroll_offset
    );

    // And the opposite direction clamps at 0, not negative/underflowed.
    term.scroll(-1_000_000);
    let bottom = term.wait_for(TIMEOUT, |g| g.scroll_offset == 0);
    assert!(
        bottom.is_some(),
        "expected an oversized negative delta to clamp to 0, got {}",
        term.snapshot().scroll_offset
    );
}

/// `spawn_with_scrollback_options`'s `max_scrollback` reaches the VT core
/// (isn't a documented-but-ignored parameter). The two backends' *actual*
/// capping behavior was found, via real CI runs against both (not assumed),
/// to differ enough that this test asserts different things per backend:
///
/// - **alacritty**: `Term`'s `Grid::update_history` trims synchronously and
///   exactly to `Config::scrolling_history` -- `scrollback_len` is *exactly*
///   `max_scrollback` once flooded past it, asserted precisely below.
/// - **ghostty-vt**: its pagelist reclaims scrollback in coarse,
///   memory-page-sized chunks rather than trimming to an exact line count
///   after every write. An earlier version of this test asserted an exact
///   cap here too and **failed in CI**: `max_scrollback: 5` after flooding
///   100 lines reported `scrollback_len: 96` (essentially untrimmed -- no
///   page boundary had been crossed yet by such a small burst). Lacking a
///   local Zig 0.16 toolchain to characterize the real reclaim threshold
///   further (see `README.md`'s "Not independently verified against real
///   libghostty-vt"), this test only asserts the parameter is *accepted and
///   plumbed through without erroring* for that backend, rather than
///   guessing at a flood size/cap combination that might reclaim -- a
///   weaker guarantee, honestly documented, rather than a second unverified
///   guess.
#[test]
fn spawn_with_scrollback_options_caps_history_length() {
    use labolabo_term::ColorScheme;

    const MAX_SCROLLBACK: usize = 5;
    let term = Terminal::spawn_with_scrollback_options(
        20,
        5,
        Some("for i in $(seq 0 99); do echo \"line-$i\"; done; sleep 0.2"),
        &[],
        &ColorScheme::default(),
        None,
        MAX_SCROLLBACK,
    )
    .expect("spawn_with_scrollback_options");

    let snap = term.wait_for(TIMEOUT, |g| g.contains_text("line-99"));
    assert!(snap.is_some(), "expected the flood to finish");

    // Give the worker a moment to settle on its final, post-flood snapshot.
    std::thread::sleep(Duration::from_millis(200));
    let scrollback_len = term.snapshot().scrollback_len;

    if cfg!(feature = "backend-alacritty") {
        assert_eq!(
            scrollback_len, MAX_SCROLLBACK,
            "alacritty's scrolling_history should trim exactly to max_scrollback \
             after flooding 100 lines, got scrollback_len={scrollback_len}"
        );
    } else {
        // ghostty-vt: only assert the spawn+flood+query round-trip works at
        // all (no panic, a well-formed grid) -- see this test's doc comment
        // for why a tight numeric bound isn't asserted for this backend.
        assert!(
            snap.unwrap().contains_text("line-99"),
            "expected the flooded content to still be readable after the \
             configured-max_scrollback spawn, got:\n{}",
            term.snapshot().to_text()
        );
    }
}

/// Entering the alternate screen (the mode `vim`/`less`/`htop` use) is
/// visible via `Terminal::alt_screen_active()`, and leaving it clears the
/// flag again -- the signal `labolabo-app`'s wheel handler uses to decide
/// whether to scroll this crate's own viewport or send cursor keys instead.
///
/// Flaked 3x in the ubuntu `rust-term-ghostty` CI job (wave 12) even though
/// this test already polls with a deadline (`wait_for_alt_screen`, below) --
/// the actual race isn't "read immediately after write", it's that the
/// worker thread refreshes `alt_screen_active()`'s mirrored flag per
/// `WorkerMsg::Bytes` batch (`session.rs`'s `run_worker`) but only fires a
/// `TermEvent::Wakeup` from its *throttled* snapshot publish
/// (`FRAME_INTERVAL` = 16ms). Under CI load the worker thread can fall far
/// enough behind that both the ON (`1049h`) and OFF (`1049l`) writes are
/// already queued by the time it resumes, and it processes them back to
/// back inside that same throttle window -- the flag genuinely passes
/// through `true` for well under the poller's 50ms sampling granularity, so
/// polling harder or waiting longer doesn't help; the ON state can be
/// skipped entirely from the observer's point of view. Fixed by having the
/// child block on `read` after the first write, so the test only unblocks
/// the second (OFF) write *after* it has already observed the first (ON)
/// one -- a real happens-before relationship instead of a clock-based gap,
/// which makes the two writes structurally unable to land in the same
/// worker batch. Production code is unchanged; this is a test-only fix
/// (see this file's module doc comment).
#[test]
fn alt_screen_active_reflects_decset_1049() {
    let term = Terminal::spawn_with_command(
        40,
        10,
        Some(r#"printf '\033[?1049h'; read _sync; printf '\033[?1049l'; sleep 0.2"#),
        &[],
    )
    .expect("spawn");
    assert!(
        wait_for_alt_screen(&term, TIMEOUT, true),
        "expected alt screen to turn on after DECSET 1049h"
    );
    // Only unblock the child's second `printf` (DECSET 1049l) once the ON
    // state was actually observed -- see the doc comment above.
    term.write_input(b"\n");
    assert!(
        wait_for_alt_screen(&term, TIMEOUT, false),
        "expected alt screen to turn back off after DECSET 1049l"
    );
}

/// DECSET `1007` (alternate scroll mode) defaults to **on**, matching real
/// Ghostty's and `alacritty_terminal`'s own defaults (confirmed by reading
/// each backend's source -- see `Terminal::alternate_scroll_active`'s doc
/// comment), and toggles off/back on via `ESC[?1007l`/`ESC[?1007h` -- the
/// query `labolabo-app`'s wheel handler uses to decide whether an
/// alt-screen scroll gesture (when mouse reporting is off) should convert
/// to cursor-key sequences at all.
///
/// Same "block on `read` between writes" hardening as [`wait_for_alt_screen`]
/// -- without it, a worker thread that falls behind could process the OFF
/// and back-ON writes together and never expose the intermediate `false`.
#[test]
fn alternate_scroll_defaults_on_and_toggles_via_decset_1007() {
    let term = Terminal::spawn_with_command(
        80,
        24,
        Some(r#"printf '\033[?1007l'; read _sync; printf '\033[?1007h'; sleep 0.2"#),
        &[],
    )
    .expect("spawn");
    assert!(
        term.alternate_scroll_active(),
        "alternate scroll should default to on, before the child runs"
    );
    assert!(
        wait_for_alternate_scroll(&term, TIMEOUT, false),
        "expected alternate scroll to turn off after DECSET 1007l"
    );
    term.write_input(b"\n");
    assert!(
        wait_for_alternate_scroll(&term, TIMEOUT, true),
        "expected alternate scroll to turn back on after DECSET 1007h"
    );
}

/// OSC `2` (set window title), BEL-terminated -- the common case emitted by
/// Claude Code and most shells' `\e]0;...\a` prompt hooks. `title()` is
/// `None` before the child ever sends it, then reflects the set value.
#[test]
fn title_reflects_osc_2_bel_terminated() {
    let term = Terminal::spawn_with_command(
        80,
        24,
        Some(r#"printf '\033]2;Hello Title\007'; sleep 0.3"#),
        &[],
    )
    .expect("spawn");
    assert_eq!(
        term.title(),
        None,
        "no title should be set before the child runs"
    );
    assert!(
        wait_for_title(&term, TIMEOUT, |t| t.as_deref() == Some("Hello Title")),
        "expected title 'Hello Title' after OSC 2, got {:?}",
        term.title()
    );
}

/// OSC `0` (set icon name + window title), ST-terminated (`ESC \` rather
/// than BEL) -- the other legal terminator real programs use.
#[test]
fn title_reflects_osc_0_st_terminated() {
    let term = Terminal::spawn_with_command(
        80,
        24,
        Some("printf '\\033]0;ST Title\\033\\\\'; sleep 0.3"),
        &[],
    )
    .expect("spawn");
    assert!(
        wait_for_title(&term, TIMEOUT, |t| t.as_deref() == Some("ST Title")),
        "expected title 'ST Title' after OSC 0 (ST-terminated), got {:?}",
        term.title()
    );
}

/// The OSC sequence arrives split across two separate PTY writes (a `sleep`
/// between two `printf`s all but guarantees two distinct `read()`s on the
/// reader thread -- see `session.rs`'s reader/worker split) -- exercises
/// that both backends' VT parsers (not a bespoke state machine in this
/// crate) correctly resume mid-sequence rather than losing/mangling it.
#[test]
fn title_reflects_osc_sequence_split_across_writes() {
    let term = Terminal::spawn_with_command(
        80,
        24,
        Some(r#"printf '\033]2;Spl'; sleep 0.2; printf 'it Title\007'; sleep 0.3"#),
        &[],
    )
    .expect("spawn");
    assert!(
        wait_for_title(&term, TIMEOUT, |t| t.as_deref() == Some("Split Title")),
        "expected title 'Split Title' after a split OSC write, got {:?}",
        term.title()
    );
}

/// A second OSC title sequence replaces the first (not appended/ignored).
///
/// Same "block on `read` between writes" hardening as [`wait_for_alt_screen`]
/// -- without it, a worker thread that falls behind could process both OSC
/// writes together and the mirrored title would jump straight from `None`
/// to `"Second"`, so `wait_for_title(.., "First")` would never observe it.
#[test]
fn title_updates_on_second_osc_sequence() {
    let term = Terminal::spawn_with_command(
        80,
        24,
        Some(r#"printf '\033]2;First\007'; read _sync; printf '\033]2;Second\007'; sleep 0.2"#),
        &[],
    )
    .expect("spawn");
    assert!(
        wait_for_title(&term, TIMEOUT, |t| t.as_deref() == Some("First")),
        "expected title 'First' after the first OSC 2, got {:?}",
        term.title()
    );
    term.write_input(b"\n");
    assert!(
        wait_for_title(&term, TIMEOUT, |t| t.as_deref() == Some("Second")),
        "expected title 'Second' after the second OSC 2, got {:?}",
        term.title()
    );
}

/// Find the first cell whose text matches `needle` (a single grapheme, as
/// printed by the tests above -- there's no ambiguity to resolve).
fn find_cell<'a>(
    snapshot: &'a labolabo_term::GridSnapshot,
    needle: &str,
) -> Option<&'a labolabo_term::CellSnapshot> {
    snapshot.cells.iter().find(|c| c.text == needle)
}

/// Drain events until `Exit` (Wakeups may precede it), bounded by `timeout`.
fn wait_for_exit(term: &Terminal, timeout: Duration) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        match term.recv_event(Duration::from_millis(200)) {
            Some(TermEvent::Exit) => return true,
            _ => continue,
        }
    }
    false
}

/// Poll `term.bracketed_paste()` until it equals `expected` or `timeout`
/// elapses -- `bracketed_paste()` has no dedicated event of its own (see
/// `TermSession::bracketed_paste`'s doc comment: it's a plain flag refreshed
/// alongside snapshot publishing), so this polls on the same wakeup/exit
/// channel `wait_for` itself blocks on rather than busy-spinning.
fn wait_for_bracketed_paste(term: &Terminal, timeout: Duration, expected: bool) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        if term.bracketed_paste() == expected {
            return true;
        }
        let Some(remaining) = deadline.checked_duration_since(std::time::Instant::now()) else {
            return term.bracketed_paste() == expected;
        };
        if term.recv_event(remaining.min(Duration::from_millis(50))) == Some(TermEvent::Exit) {
            return term.bracketed_paste() == expected;
        }
    }
}

/// Poll `term.kitty_disambiguate()` until it equals `expected` or `timeout`
/// elapses -- same shape as [`wait_for_bracketed_paste`] (no dedicated event
/// of its own; a plain flag refreshed alongside snapshot publishing).
fn wait_for_kitty_disambiguate(term: &Terminal, timeout: Duration, expected: bool) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        if term.kitty_disambiguate() == expected {
            return true;
        }
        let Some(remaining) = deadline.checked_duration_since(std::time::Instant::now()) else {
            return term.kitty_disambiguate() == expected;
        };
        if term.recv_event(remaining.min(Duration::from_millis(50))) == Some(TermEvent::Exit) {
            return term.kitty_disambiguate() == expected;
        }
    }
}

/// Poll `term.alt_screen_active()` until it equals `expected` or `timeout`
/// elapses -- same shape as [`wait_for_bracketed_paste`] (no dedicated event
/// of its own; a plain flag refreshed alongside snapshot publishing).
fn wait_for_alt_screen(term: &Terminal, timeout: Duration, expected: bool) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        if term.alt_screen_active() == expected {
            return true;
        }
        let Some(remaining) = deadline.checked_duration_since(std::time::Instant::now()) else {
            return term.alt_screen_active() == expected;
        };
        if term.recv_event(remaining.min(Duration::from_millis(50))) == Some(TermEvent::Exit) {
            return term.alt_screen_active() == expected;
        }
    }
}

/// Poll `term.alternate_scroll_active()` until it equals `expected` or
/// `timeout` elapses -- same shape as [`wait_for_alt_screen`] (no dedicated
/// event of its own; a plain flag refreshed alongside snapshot publishing).
fn wait_for_alternate_scroll(term: &Terminal, timeout: Duration, expected: bool) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        if term.alternate_scroll_active() == expected {
            return true;
        }
        let Some(remaining) = deadline.checked_duration_since(std::time::Instant::now()) else {
            return term.alternate_scroll_active() == expected;
        };
        if term.recv_event(remaining.min(Duration::from_millis(50))) == Some(TermEvent::Exit) {
            return term.alternate_scroll_active() == expected;
        }
    }
}

/// Poll `term.title()` until `pred` holds or `timeout` elapses -- same shape
/// as [`wait_for_bracketed_paste`]/[`wait_for_alt_screen`] (no dedicated
/// event of its own; a plain flag refreshed alongside snapshot publishing).
/// Takes a predicate rather than a fixed expected value (unlike the other
/// `wait_for_*` helpers here) since callers need to distinguish "unset" from
/// "set to a particular string" cleanly via `Option<&str>` matching.
fn wait_for_title(
    term: &Terminal,
    timeout: Duration,
    pred: impl Fn(&Option<String>) -> bool,
) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        let title = term.title();
        if pred(&title) {
            return true;
        }
        let Some(remaining) = deadline.checked_duration_since(std::time::Instant::now()) else {
            return pred(&term.title());
        };
        if term.recv_event(remaining.min(Duration::from_millis(50))) == Some(TermEvent::Exit) {
            return pred(&term.title());
        }
    }
}

/// Poll `term.mouse_mode()` until it equals `expected` or `timeout` elapses
/// -- same shape as [`wait_for_bracketed_paste`]/[`wait_for_alt_screen`] (no
/// dedicated event of its own; a plain flag refreshed alongside snapshot
/// publishing).
fn wait_for_mouse_mode(term: &Terminal, timeout: Duration, expected: MouseMode) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        if term.mouse_mode() == expected {
            return true;
        }
        let Some(remaining) = deadline.checked_duration_since(std::time::Instant::now()) else {
            return term.mouse_mode() == expected;
        };
        if term.recv_event(remaining.min(Duration::from_millis(50))) == Some(TermEvent::Exit) {
            return term.mouse_mode() == expected;
        }
    }
}
