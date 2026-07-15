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
#![cfg(unix)]

use std::time::Duration;

use labolabo_term::{ColorScheme, Rgb, TermEvent, Terminal};

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

/// DECSET `2004` (bracketed paste) toggles `Terminal::bracketed_paste()`:
/// off before the child runs, on once it enables it, off again once it
/// disables it. This is the mode-query API `labolabo-app`'s Cmd+V paste
/// handler uses to decide whether to wrap pasted text in
/// `ESC[200~...ESC[201~`.
#[test]
fn bracketed_paste_mode_reflects_decset_2004() {
    let term = Terminal::spawn_with_command(
        80,
        24,
        Some(r#"printf '\033[?2004h'; sleep 0.3; printf '\033[?2004l'; sleep 0.3"#),
        &[],
    )
    .expect("spawn");
    assert!(
        !term.bracketed_paste(),
        "bracketed paste should be off before the child runs"
    );
    assert!(
        wait_for_bracketed_paste(&term, TIMEOUT, true),
        "expected bracketed paste to turn on after DECSET 2004h"
    );
    assert!(
        wait_for_bracketed_paste(&term, TIMEOUT, false),
        "expected bracketed paste to turn back off after DECSET 2004l"
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

/// Entering the alternate screen (the mode `vim`/`less`/`htop` use) is
/// visible via `Terminal::alt_screen_active()`, and leaving it clears the
/// flag again -- the signal `labolabo-app`'s wheel handler uses to decide
/// whether to scroll this crate's own viewport or send cursor keys instead.
#[test]
fn alt_screen_active_reflects_decset_1049() {
    let term = Terminal::spawn_with_command(
        40,
        10,
        Some(r#"printf '\033[?1049h'; sleep 0.3; printf '\033[?1049l'; sleep 0.3"#),
        &[],
    )
    .expect("spawn");
    assert!(
        !term.alt_screen_active(),
        "alt screen should be off before the child runs"
    );
    assert!(
        wait_for_alt_screen(&term, TIMEOUT, true),
        "expected alt screen to turn on after DECSET 1049h"
    );
    assert!(
        wait_for_alt_screen(&term, TIMEOUT, false),
        "expected alt screen to turn back off after DECSET 1049l"
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
