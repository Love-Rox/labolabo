//! Pure SGR (DECSET `1006`) mouse-event encoding: turns a normalized mouse
//! action (press/release/motion, button, modifiers, cell position) into the
//! escape-sequence bytes written to a pane's PTY when the running program
//! has requested mouse reporting -- vim/tmux/htop's `-mouse`, or Claude
//! Code's own TUI, via DECSET `1000`/`1002`/`1003` (see
//! `labolabo_term::MouseMode`).
//!
//! No gpui types appear here on purpose -- same rationale as `grid.rs`/
//! `selection.rs`: `task_workspace.rs`'s mouse handlers convert a gpui
//! `MouseButton`/`Modifiers`/window-space position into this module's plain
//! types first, so the actual encoding logic is exercisable by a plain
//! `cargo test`.
//!
//! ## Scope: SGR format only
//!
//! Real Ghostty's own encoder (`src/input/mouse_encode.zig`, confirmed by
//! reading the vendored Ghostty source -- see this crate's README for where
//! that source lives) supports five output formats (X10, UTF-8, SGR, urxvt,
//! SGR-Pixels) selected by the terminal's own negotiated preference. This
//! module only implements SGR (DECSET `1006`) -- effectively every mouse-
//! aware TUI written since SGR's introduction (~2012) requests it alongside
//! a tracking mode, since it removes X10's 223-column/row ceiling, so this
//! covers the realistic common case. [`should_report`]/[`encode_press`]/
//! [`encode_release`] all require [`labolabo_term::MouseMode::sgr`] to be
//! `true`; a program that requests mouse tracking *without* SGR (rare in
//! practice) is out of scope -- its clicks/drags/scroll are left to this
//! app's own local text-selection/scrollback handling, same as before this
//! module existed.
//!
//! ## Button-code table and gating rule, ported from Ghostty
//!
//! The button-code arithmetic (`0`/`1`/`2` for left/middle/right, `64..=67`
//! for the four wheel directions, `+4`/`+8`/`+16` for shift/alt/ctrl, `+32`
//! for a motion event) and the per-tracking-mode gating rule
//! ([`should_report`]) are ported directly from `mouse_encode.zig`'s
//! `buttonCode`/`shouldReport` (confirmed by reading the vendored source),
//! restricted to the SGR-only slice this app needs: unlike Ghostty's own
//! encoder, a *release* here always keeps the button's real identity (SGR's
//! own `M`/`m` terminator already disambiguates press from release, so
//! there is no SGR-specific "legacy release is always button 3" case to
//! port -- that only applies to Ghostty's non-SGR formats, out of scope
//! here).

use labolabo_term::{MouseMode, MouseTracking};

/// A mouse action, mirroring `mouse_encode.zig`'s `Action` enum.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseAction {
    Press,
    Release,
    /// Cursor moved. `button: None` means a plain hover (no button held --
    /// only ever reportable under [`MouseTracking::Any`]); `button: Some`
    /// means a drag (reportable under [`MouseTracking::Button`] and
    /// [`MouseTracking::Any`]).
    Motion,
}

/// A mouse button/wheel-direction identity, mirroring `mouse_encode.zig`'s
/// `Button` enum -- restricted to the variants this app can actually
/// produce (gpui's own `MouseButton` has no navigate-back/forward wheel
/// equivalent, and this app has no need for Ghostty's extra side buttons
/// eight/nine).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseButtonKind {
    Left,
    Middle,
    Right,
    WheelUp,
    WheelDown,
    /// Modeled for API completeness (mirrors Ghostty's own button set, and
    /// is exercised by this module's own tests) but not yet produced by
    /// `task_workspace`/`app.rs`'s wheel handler -- this crate's own
    /// `grid::accumulate_scroll_lines` only accumulates the vertical axis,
    /// so horizontal-scroll forwarding has no accumulated-delta input to
    /// encode from yet (horizontal scroll was unimplemented, not just
    /// unreported, before this module existed too). Future work if a
    /// horizontal-scroll gesture is ever wired up locally.
    #[allow(dead_code)]
    WheelLeft,
    #[allow(dead_code)]
    WheelRight,
}

impl MouseButtonKind {
    /// The SGR button-code base (before modifier/motion bits are added),
    /// ported from `mouse_encode.zig`'s `buttonCode` match arm.
    fn base_code(self) -> u8 {
        match self {
            MouseButtonKind::Left => 0,
            MouseButtonKind::Middle => 1,
            MouseButtonKind::Right => 2,
            MouseButtonKind::WheelUp => 64,
            MouseButtonKind::WheelDown => 65,
            MouseButtonKind::WheelLeft => 66,
            MouseButtonKind::WheelRight => 67,
        }
    }
}

/// Keyboard modifiers held during a mouse event -- the SGR-relevant subset
/// of gpui's own `Modifiers` (which also carries `platform`/`function`,
/// neither of which the SGR mouse protocol encodes).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct MouseMods {
    pub shift: bool,
    pub alt: bool,
    pub ctrl: bool,
}

/// Whether an event with this `action`/`button` should be reported at all
/// under `tracking` -- ported from `mouse_encode.zig`'s `shouldReport`,
/// restricted to the four tracking modes `labolabo_term::MouseTracking`
/// models (format-independent in Ghostty's own version too, so nothing is
/// lost by dropping the `opts.format` parameter Ghostty's version also
/// takes).
pub fn should_report(
    tracking: MouseTracking,
    action: MouseAction,
    button: Option<MouseButtonKind>,
) -> bool {
    match tracking {
        MouseTracking::Off => false,
        // X10 only reports button presses of left/middle/right.
        MouseTracking::X10 => {
            action == MouseAction::Press
                && matches!(
                    button,
                    Some(MouseButtonKind::Left | MouseButtonKind::Middle | MouseButtonKind::Right)
                )
        }
        // Normal mode reports press/release, never motion.
        MouseTracking::Normal => action != MouseAction::Motion,
        // Button-event tracking requires an active button (covers press/
        // release always, and motion only while dragging).
        MouseTracking::Button => button.is_some(),
        // Any-event tracking reports everything, including a bare hover.
        MouseTracking::Any => true,
    }
}

/// Encodes one SGR mouse event: `ESC [ < Cb ; Cx ; Cy M` for press/motion,
/// `ESC [ < Cb ; Cx ; Cy m` for release (the trailing letter is how SGR
/// tells press/motion apart from release -- ported from `mouse_encode.zig`'s
/// `.sgr` writer arm). `col`/`row` are 0-based grid cells (this crate's own
/// convention throughout, e.g. `crate::selection::CellPos`); SGR itself is
/// 1-indexed, so `+1` is applied here, once, so no caller needs to know
/// that.
///
/// Returns `None` (nothing to write) when [`should_report`] says this event
/// shouldn't be reported under `tracking` -- callers that already checked
/// `should_report` themselves get a harmless double-check; callers that
/// didn't still get correct gating for free, so this is the one function
/// most call sites actually need.
pub fn encode_sgr(
    tracking: MouseTracking,
    action: MouseAction,
    button: Option<MouseButtonKind>,
    mods: MouseMods,
    col: u16,
    row: u16,
) -> Option<Vec<u8>> {
    if !should_report(tracking, action, button) {
        return None;
    }

    // A null button (motion with nothing pressed, only reachable under
    // `Any` tracking -- `should_report` above already gated everything
    // else out) encodes as code `3`, matching `mouse_encode.zig`'s
    // "Null button means motion with no pressed button" case.
    let mut code: u16 = match button {
        Some(b) => u16::from(b.base_code()),
        None => 3,
    };
    if mods.shift {
        code += 4;
    }
    if mods.alt {
        code += 8;
    }
    if mods.ctrl {
        code += 16;
    }
    if action == MouseAction::Motion {
        code += 32;
    }

    let terminator = if action == MouseAction::Release {
        'm'
    } else {
        'M'
    };
    Some(
        format!(
            "\x1b[<{code};{};{}{terminator}",
            col as u32 + 1,
            row as u32 + 1
        )
        .into_bytes(),
    )
}

/// Whether `mouse_mode` plus the live `shift_held` state means a mouse
/// click/drag should be SGR-encoded and forwarded to the pane's PTY right
/// now, instead of driving this app's own local text-selection behavior.
/// Requires both an active tracking mode *and* SGR (DECSET `1006`) -- see
/// this module's doc comment for why SGR-less tracking is out of scope --
/// and Shift held forces local selection regardless of tracking, matching
/// real Ghostty's own default `mouse-shift-capture` behavior (confirmed by
/// reading the vendored Ghostty source's `Surface.zig`:
/// `mouseButtonCallback` gates its mouse-report branch on
/// `!(self.mouse.mods.shift and !self.mouseShiftCapture(false))`, and
/// `mouse-shift-capture`'s own doc comment says its default value `false`
/// means "the shift key is not sent with the mouse protocol and will
/// extend the selection" -- i.e. Shift always wins locally unless the
/// *user* explicitly reconfigures that default, which this app doesn't
/// expose a setting for, so Shift always wins here).
pub fn is_click_reporting_active(mouse_mode: MouseMode, shift_held: bool) -> bool {
    mouse_mode.tracking.is_active() && mouse_mode.sgr && !shift_held
}

/// Whether `mouse_mode` means a scroll-wheel/trackpad event should be
/// SGR-encoded and forwarded to the pane's PTY right now, instead of
/// driving this app's own scrollback/alternate-scroll behavior.
///
/// Unlike [`is_click_reporting_active`], **not** overridden by Shift --
/// confirmed by reading real Ghostty's own `Surface.scrollCallback`
/// (vendored source): its mouse-report branch (`if self.isMouseReporting()
/// { ... self.mouseReport(...) }`) is unconditioned on
/// `self.mouse.mods.shift`/`mouseShiftCapture`, unlike its click/drag path
/// (`mouseButtonCallback`), which does check it. Requires SGR (`1006`) --
/// see this module's doc comment for why SGR-less tracking is out of
/// scope.
pub fn is_scroll_reporting_active(mouse_mode: MouseMode) -> bool {
    mouse_mode.tracking.is_active() && mouse_mode.sgr
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mode(tracking: MouseTracking, sgr: bool) -> MouseMode {
        MouseMode { tracking, sgr }
    }

    // MARK: - is_click_reporting_active / is_scroll_reporting_active

    #[test]
    fn click_reporting_requires_active_tracking_and_sgr() {
        assert!(!is_click_reporting_active(MouseMode::OFF, false));
        assert!(!is_click_reporting_active(
            mode(MouseTracking::Normal, false),
            false
        ));
        assert!(is_click_reporting_active(
            mode(MouseTracking::Normal, true),
            false
        ));
    }

    #[test]
    fn click_reporting_is_overridden_by_shift() {
        let active = mode(MouseTracking::Any, true);
        assert!(is_click_reporting_active(active, false));
        assert!(
            !is_click_reporting_active(active, true),
            "Shift held should force local selection even though tracking+SGR are active"
        );
    }

    #[test]
    fn scroll_reporting_requires_active_tracking_and_sgr() {
        assert!(!is_scroll_reporting_active(MouseMode::OFF));
        assert!(!is_scroll_reporting_active(mode(
            MouseTracking::Button,
            false
        )));
        assert!(is_scroll_reporting_active(mode(
            MouseTracking::Button,
            true
        )));
    }

    #[test]
    fn scroll_reporting_is_not_overridden_by_shift() {
        // `is_scroll_reporting_active` takes no `shift_held` parameter at
        // all -- this test exists mainly to pin the API shape (a caller
        // cannot accidentally gate scroll on Shift the way click/drag is
        // gated), and doubles as a readable contrast with
        // `click_reporting_is_overridden_by_shift` above.
        assert!(is_scroll_reporting_active(mode(MouseTracking::Any, true)));
    }

    // MARK: - should_report

    #[test]
    fn off_never_reports() {
        for action in [
            MouseAction::Press,
            MouseAction::Release,
            MouseAction::Motion,
        ] {
            assert!(!should_report(
                MouseTracking::Off,
                action,
                Some(MouseButtonKind::Left)
            ));
        }
    }

    #[test]
    fn x10_reports_only_left_middle_right_press() {
        for button in [
            MouseButtonKind::Left,
            MouseButtonKind::Middle,
            MouseButtonKind::Right,
        ] {
            assert!(should_report(
                MouseTracking::X10,
                MouseAction::Press,
                Some(button)
            ));
        }
        assert!(!should_report(
            MouseTracking::X10,
            MouseAction::Release,
            Some(MouseButtonKind::Left)
        ));
        assert!(!should_report(
            MouseTracking::X10,
            MouseAction::Motion,
            Some(MouseButtonKind::Left)
        ));
        assert!(!should_report(
            MouseTracking::X10,
            MouseAction::Press,
            Some(MouseButtonKind::WheelUp)
        ));
        assert!(!should_report(MouseTracking::X10, MouseAction::Press, None));
    }

    #[test]
    fn normal_reports_press_and_release_not_motion() {
        assert!(should_report(
            MouseTracking::Normal,
            MouseAction::Press,
            Some(MouseButtonKind::Left)
        ));
        assert!(should_report(
            MouseTracking::Normal,
            MouseAction::Release,
            Some(MouseButtonKind::Left)
        ));
        assert!(!should_report(
            MouseTracking::Normal,
            MouseAction::Motion,
            Some(MouseButtonKind::Left)
        ));
    }

    #[test]
    fn button_tracking_requires_a_button_for_every_action() {
        for action in [
            MouseAction::Press,
            MouseAction::Release,
            MouseAction::Motion,
        ] {
            assert!(should_report(
                MouseTracking::Button,
                action,
                Some(MouseButtonKind::Left)
            ));
            assert!(!should_report(MouseTracking::Button, action, None));
        }
    }

    #[test]
    fn any_tracking_reports_everything_including_bare_hover() {
        for action in [
            MouseAction::Press,
            MouseAction::Release,
            MouseAction::Motion,
        ] {
            assert!(should_report(
                MouseTracking::Any,
                action,
                Some(MouseButtonKind::Left)
            ));
        }
        assert!(should_report(MouseTracking::Any, MouseAction::Motion, None));
    }

    // MARK: - encode_sgr

    #[test]
    fn left_press_no_modifiers() {
        let bytes = encode_sgr(
            MouseTracking::Normal,
            MouseAction::Press,
            Some(MouseButtonKind::Left),
            MouseMods::default(),
            0,
            0,
        )
        .expect("normal tracking reports a press");
        assert_eq!(bytes, b"\x1b[<0;1;1M");
    }

    #[test]
    fn release_keeps_button_identity_and_uses_lowercase_terminator() {
        let bytes = encode_sgr(
            MouseTracking::Normal,
            MouseAction::Release,
            Some(MouseButtonKind::Right),
            MouseMods::default(),
            4,
            5,
        )
        .expect("normal tracking reports a release");
        assert_eq!(bytes, b"\x1b[<2;5;6m");
    }

    #[test]
    fn wheel_button_codes_match_ghostty() {
        let cases = [
            (MouseButtonKind::WheelUp, 64u16),
            (MouseButtonKind::WheelDown, 65),
            (MouseButtonKind::WheelLeft, 66),
            (MouseButtonKind::WheelRight, 67),
        ];
        for (button, code) in cases {
            let bytes = encode_sgr(
                MouseTracking::Any,
                MouseAction::Press,
                Some(button),
                MouseMods::default(),
                0,
                0,
            )
            .unwrap();
            assert_eq!(bytes, format!("\x1b[<{code};1;1M").into_bytes());
        }
    }

    #[test]
    fn modifiers_add_their_bits() {
        let bytes = encode_sgr(
            MouseTracking::Any,
            MouseAction::Press,
            Some(MouseButtonKind::Left),
            MouseMods {
                shift: true,
                alt: true,
                ctrl: true,
            },
            2,
            3,
        )
        .unwrap();
        // 0 (left) + 4 (shift) + 8 (alt) + 16 (ctrl) = 28.
        assert_eq!(bytes, b"\x1b[<28;3;4M");
    }

    #[test]
    fn motion_adds_the_32_bit() {
        let bytes = encode_sgr(
            MouseTracking::Button,
            MouseAction::Motion,
            Some(MouseButtonKind::Left),
            MouseMods::default(),
            0,
            0,
        )
        .unwrap();
        assert_eq!(bytes, b"\x1b[<32;1;1M");
    }

    #[test]
    fn hover_motion_with_no_button_encodes_as_code_3() {
        let bytes = encode_sgr(
            MouseTracking::Any,
            MouseAction::Motion,
            None,
            MouseMods::default(),
            1,
            2,
        )
        .unwrap();
        // Base 3 (null button) + 32 (motion) = 35.
        assert_eq!(bytes, b"\x1b[<35;2;3M");
    }

    #[test]
    fn coordinates_are_one_indexed() {
        let bytes = encode_sgr(
            MouseTracking::Normal,
            MouseAction::Press,
            Some(MouseButtonKind::Left),
            MouseMods::default(),
            79,
            23,
        )
        .unwrap();
        assert_eq!(bytes, b"\x1b[<0;80;24M");
    }

    #[test]
    fn gating_suppresses_output_even_without_a_separate_should_report_check() {
        // Motion under Normal tracking never reports -- callers that skip
        // their own `should_report` check still get correct behavior.
        assert!(encode_sgr(
            MouseTracking::Normal,
            MouseAction::Motion,
            Some(MouseButtonKind::Left),
            MouseMods::default(),
            0,
            0,
        )
        .is_none());
        assert!(encode_sgr(
            MouseTracking::Off,
            MouseAction::Press,
            Some(MouseButtonKind::Left),
            MouseMods::default(),
            0,
            0
        )
        .is_none());
    }

    // MARK: - end-to-end: real PTY mouse-mode detection -> real SGR
    // encoding -> bytes actually reaching the child (W5j bug report #1)

    /// A mouse-aware alt-screen TUI (e.g. Claude Code's own TUI) was
    /// receiving arrow-key sequences from a scroll gesture instead of real
    /// mouse events, because its own DECSET `1000`/`1003` mouse tracking
    /// was never consulted before `app::LaboLaboApp::handle_pane_scroll`
    /// fell into its alt-screen branch (fixed by that method's new
    /// priority order -- see its doc comment). Full headless verification
    /// of the actual gpui wheel handler isn't practical (no gpui test
    /// harness in this codebase), so this instead proves the two pieces
    /// `handle_pane_scroll` composes really do work together end-to-end,
    /// against a **real** spawned PTY on whichever backend this test
    /// binary was built against (`cargo test -p labolabo-app` / `--features
    /// backend-ghostty-vt`):
    ///
    /// 1. A real child enables DECSET `1000`, `1003`, then `1006` --
    ///    `Terminal::mouse_mode()` (real backend state, not a mock)
    ///    eventually reports `Any` tracking with `sgr: true`.
    /// 2. [`encode_sgr`], called with that *real* queried `MouseMode` --
    ///    the exact same call `handle_pane_scroll` makes for a wheel-up
    ///    scroll -- produces the expected SGR bytes.
    /// 3. Those bytes, written via `Terminal::write_input` (the same
    ///    method `handle_pane_scroll` calls), are captured **raw** by the
    ///    child (`stty raw -echo` + `dd`, bypassing this crate's own VT
    ///    parser entirely -- an SGR mouse-*report* sequence fed back in as
    ///    *input* is not something any real terminal is meant to
    ///    interpret, so reading it back through our own snapshot could
    ///    silently swallow/reinterpret it and produce a false pass) and
    ///    confirmed byte-for-byte.
    #[cfg(unix)]
    #[test]
    fn mouse_scroll_reporting_end_to_end_reaches_the_pty_as_sgr_bytes() {
        use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

        use labolabo_term::Terminal;

        const TIMEOUT: Duration = Duration::from_secs(5);

        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos() as u64;
        let capture_path = std::env::temp_dir().join(format!(
            "labolabo-mouse-report-capture-{}-{nanos:x}",
            std::process::id()
        ));

        // The expected bytes are computed up front so the child script
        // knows exactly how many raw bytes to capture.
        let expected = encode_sgr(
            MouseTracking::Any,
            MouseAction::Press,
            Some(MouseButtonKind::WheelUp),
            MouseMods::default(),
            0,
            0,
        )
        .expect("Any tracking + SGR always reports a wheel press");

        let script = format!(
            "printf '\\033[?1000h\\033[?1003h\\033[?1006h'; printf READY; \
             stty raw -echo; dd bs=1 count={} of='{}' 2>/dev/null",
            expected.len(),
            capture_path.display(),
        );
        let term = Terminal::spawn_with_command(80, 24, Some(&script), &[]).expect("spawn");
        let ready = term.wait_for(TIMEOUT, |g| g.contains_text("READY"));
        assert!(
            ready.is_some(),
            "expected the child to finish its DECSET setup, got:\n{}",
            term.snapshot().to_text()
        );

        // Poll for the mode flags to refresh -- same "no dedicated event,
        // just poll the plain flag" shape as labolabo-term's own
        // `wait_for_mouse_mode` (this crate has no such event either).
        let deadline = Instant::now() + TIMEOUT;
        let mode = loop {
            let mode = term.mouse_mode();
            if mode.tracking == MouseTracking::Any && mode.sgr {
                break mode;
            }
            assert!(
                Instant::now() < deadline,
                "expected DECSET 1000/1003/1006 to land within the timeout, last mode: {mode:?}"
            );
            std::thread::sleep(Duration::from_millis(20));
        };

        // The exact call `app::LaboLaboApp::handle_pane_scroll` makes for
        // a wheel-up scroll: encode with the *real*, just-queried mode.
        let bytes = encode_sgr(
            mode.tracking,
            MouseAction::Press,
            Some(MouseButtonKind::WheelUp),
            MouseMods::default(),
            0,
            0,
        )
        .expect("Any tracking + SGR should report a wheel press");
        assert_eq!(bytes, expected);

        term.write_input(&bytes);

        // Give `dd` a moment to read and flush, then read back exactly
        // what reached the child's stdin, bypassing this crate's own VT
        // parser entirely.
        std::thread::sleep(Duration::from_millis(500));
        let captured = std::fs::read(&capture_path).unwrap_or_default();
        let _ = std::fs::remove_file(&capture_path);
        assert_eq!(
            captured, bytes,
            "expected the raw SGR bytes to reach the child's stdin unmodified"
        );
    }
}
