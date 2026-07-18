//! Pure translation from a gpui key event to the bytes a real terminal
//! expects on its PTY input, for the keys that must **never** reach the
//! platform's text-input/IME machinery.
//!
//! `gpui::Keystroke`/`Modifiers` are plain data (no `App`/`Window` runtime
//! needed to construct one), so this is directly unit-testable -- see the
//! tests below, which build `Keystroke` values by hand.
//!
//! ## IME design decision (see `app::LaboLaboApp`'s `EntityInputHandler` impl)
//!
//! Once a pane has gpui's IME/text-input handler wired up
//! (`Window::handle_input`, called from the focused pane's canvas paint --
//! see `task_workspace::render_leaf`), plain printable text -- including
//! space -- is deliberately *not* handled in this function; it arrives
//! exclusively via `EntityInputHandler::replace_text_in_range` (a committed
//! character, or an IME composition's final confirmed text) or
//! `replace_and_mark_text_in_range` (an in-progress composition's preedit).
//! Handling such a keystroke in both places would either double-send the
//! character, or -- worse -- pre-empt IME composition entirely (writing the
//! *unconverted* Roman letter to the PTY before the IME ever gets a chance
//! to turn it into a composed character).
//!
//! **Correction (wave 15 follow-up) to an earlier version of this comment,
//! which claimed gpui's platform backends pre-empt `on_key_down` for every
//! `key_char`-carrying keystroke, routing it to the input handler *instead
//! of* the raw `KeyDownEvent` -- that is not what the real macOS backend
//! does, and the earlier framing led directly to an incorrect hypothesis
//! about why Shift+Enter wasn't working (it was never a key-routing
//! problem; see `rust/README.md`'s "Wave 15 followup" for the real root
//! cause).** Verified by reading gpui `0.2.2`'s actual dispatch, not
//! assumed: `platform/mac/events.rs`'s `parse_keystroke` sets `key_char` to
//! `Some(..)` for Enter/Tab/Space **unconditionally** on the *named* key
//! matched (`Some(ENTER_KEY) => { key_char = Some("\n".into()); "enter" }`,
//! wholly independent of which modifiers are held), so a Shift+Enter
//! keystroke on macOS is `{ key: "enter", key_char: Some("\n"), modifiers:
//! { shift: true, .. } }` -- it *does* carry a `key_char`. And
//! `platform/mac/window.rs`'s `handle_key_event` (the actual routing
//! decision, in the real crate, not this module's own paraphrase of it)
//! calls `run_callback` -- which is what ultimately reaches `div::
//! on_key_down`'s registered listener, `app::LaboLaboApp::key_down` here --
//! **first**, for every keystroke that isn't mid-IME-composition, key_char
//! or not; the platform input context (`NSTextInputContext`) is consulted
//! only as a *fallback*, when `on_key_down`'s handler didn't claim the
//! event (no `cx.stop_propagation()`). So this function does not need to
//! -- and must not try to -- avoid handling `key_char`-carrying named keys
//! like Enter/Tab/Backspace/Escape at all: what actually keeps a plain
//! letter from double-sending is that *this function itself* returns
//! `None` for it (no `cx.stop_propagation()`, so gpui's dispatch falls
//! through to the input context on its own), not any pre-routing gpui does
//! based on `key_char`'s presence. (X11/Wayland/Windows were not re-audited
//! this wave -- their own platform files, listed near the end of this
//! comment's revision history in git blame, were not re-read against this
//! corrected macOS understanding; treat their exact dispatch order as
//! unverified rather than assumed identical.)
//!
//! `app::LaboLaboApp::key_down` calls `cx.stop_propagation()` whenever this
//! function returns `Some(..)`, which is what prevents gpui from *also*
//! forwarding an already-handled keystroke (e.g. Ctrl-A, which macOS's
//! default Cocoa key bindings would otherwise also route to
//! `doCommandBySelector:`) to the input handler.
//!
//! TODO(W5a): only the keys this wave's brief calls for are mapped (Enter/
//! Backspace/Tab/Escape/arrows, and a bare Ctrl-<letter>).
//! Delete/Home/End/PageUp/PageDown/function keys and modifier combinations
//! beyond a lone Ctrl are future work.
//!
//! ## Kitty keyboard protocol (W15, extended W17)
//!
//! Claude Code's own TUI relies on the Kitty keyboard protocol's
//! "disambiguate escape codes" progressive-enhancement flag (`CSI > 1 u`,
//! <https://sw.kovidgoyal.net/kitty/keyboard-protocol/>) to tell Shift+Enter
//! (insert a newline) apart from a plain Enter (submit) -- both keys, in
//! the legacy protocol below, always send the exact same `\r` regardless of
//! modifiers, so a program with no other channel has no way to distinguish
//! them. Once the running program has requested that flag (queried via
//! `labolabo_term::TermSession::kitty_disambiguate`, threaded in from
//! `app::LaboLaboApp::key_down`), `kitty_disambiguated_bytes` below
//! re-encodes three families of keystroke as `CSI <n>;<modifier> u` instead
//! of their legacy bytes:
//!
//! - A **modifier-carrying** Enter/Tab (`CSI 13;<m> u` / `CSI 9;<m> u`).
//!   The spec's own documented exception reads narrower than it first
//!   looks: Enter/Tab (and Backspace, which this module still doesn't bind
//!   any modifier combination for -- see below) "still generate the same
//!   bytes as in legacy mode", but that exemption only covers a *fully
//!   unmodified* press -- if it covered every modifier combination,
//!   Shift+Enter could never be told apart from a plain Enter, which is the
//!   entire feature this section exists for. A held modifier always routes
//!   through `CSI u` here; unmodified never does.
//! - **Escape**, unconditionally (`CSI 27 u`, or `CSI 27;<m> u` with a
//!   modifier held) -- unlike Enter/Tab, Escape has *no* legacy-byte
//!   exemption at all in the spec: "Turning on this flag will cause the
//!   terminal to report the Esc, alt+key, ctrl+key, ctrl+alt+key,
//!   shift+alt+key keys using `CSI u` sequences instead of legacy ones." An
//!   earlier version of this comment claimed Escape was "left untouched
//!   entirely (out of scope)"; that was a misreading of the exemption
//!   paragraph above, which names only Enter/Tab/Backspace, never Escape.
//! - **Ctrl+<letter>**, optionally combined with Alt (`CSI <codepoint>;<m>
//!   u`, `codepoint` being the lowercase letter's Unicode value -- e.g.
//!   ctrl+c is `CSI 99;5 u`) -- replacing the legacy C0 control byte
//!   (`keystroke_to_bytes`'s own Ctrl+<letter> match arm further down,
//!   still the `kitty_disambiguate == false` fallback) with the form the
//!   same spec paragraph above calls for.
//!
//! Backspace is deliberately left sending its plain legacy byte
//! unconditionally, modifiers or not -- nothing in this app currently binds
//! a modified Backspace to anything, so there is no ambiguity to resolve
//! for it yet (unlike Enter/Tab/Escape/Ctrl+letter above, all of which
//! either this app's own bindings or Claude Code's TUI reading them already
//! rely on being told apart).
//!
//! Modifier-carrying **arrow keys** (`CSI 1;<m> {A,B,C,D}`) are handled
//! separately, directly in the plain `match` below rather than
//! `kitty_disambiguated_bytes` -- and *unconditionally*, regardless of
//! `kitty_disambiguate`. This isn't a Kitty-specific enhancement at all:
//! it's the legacy xterm modified-cursor-key encoding every VT100-ish
//! terminal already expects, disambiguate flag or not (the Kitty spec's own
//! functional-key table lists arrows in this same legacy-compatible
//! `CSI 1;<m> <letter>` form, not the `CSI u` unicode-key-code form the
//! three bullets above use). Before this wave, this module sent every
//! arrow key's plain unmodified form even with a modifier held, silently
//! dropping Shift/Ctrl/Alt on every arrow keystroke in every mode.

use gpui::{Keystroke, Modifiers};

/// Translate one key-down event into the bytes to write to the PTY, or
/// `None` if this keystroke has no *direct* terminal-input meaning here --
/// either because it's a bare modifier key with no `key_char`, a Cmd/Super
/// combination (reserved for application-level shortcuts), or plain
/// printable text/space, which (see this module's doc comment) is now
/// handled exclusively via the platform's text-input/IME machinery instead.
///
/// `kitty_disambiguate` is the focused pane's live `TermSession::
/// kitty_disambiguate()` reading (`false` for any caller with no real PTY
/// behind it, e.g. the app's own text-field input routing -- see this
/// module's "Kitty keyboard protocol" doc section above) -- when `true`,
/// Escape, a modifier-carrying Enter/Tab, or a Ctrl+<letter> combination is
/// re-encoded as a Kitty `CSI u` sequence instead of its plain legacy byte
/// or C0 code (see that doc section for the exact three cases). Modifier-
/// carrying arrow keys are re-encoded too, but unconditionally -- see
/// `cursor_key_bytes`, called from the `match` below independent of this
/// parameter.
pub fn keystroke_to_bytes(keystroke: &Keystroke, kitty_disambiguate: bool) -> Option<Vec<u8>> {
    // Cmd (macOS) / Super (Linux/Windows) combinations are reserved for
    // application-level shortcuts (tab switching, quit, ...), never sent to
    // the terminal.
    if keystroke.modifiers.platform {
        return None;
    }

    if kitty_disambiguate {
        if let Some(bytes) = kitty_disambiguated_bytes(keystroke) {
            return Some(bytes);
        }
    }

    match keystroke.key.as_str() {
        "enter" => return Some(vec![b'\r']),
        "backspace" => return Some(vec![0x7f]),
        "tab" => return Some(vec![b'\t']),
        "escape" => return Some(vec![0x1b]),
        "up" => return Some(cursor_key_bytes('A', &keystroke.modifiers)),
        "down" => return Some(cursor_key_bytes('B', &keystroke.modifiers)),
        "right" => return Some(cursor_key_bytes('C', &keystroke.modifiers)),
        "left" => return Some(cursor_key_bytes('D', &keystroke.modifiers)),
        _ => {}
    }

    // Ctrl-<letter> -> the corresponding C0 control code (Ctrl-A = 0x01,
    // ..., Ctrl-Z = 0x1a) -- the minimal Ctrl support this wave asks for.
    // gpui reports `key` as the lowercase, unshifted key even while Ctrl is
    // held, so no case-folding of a shifted key is needed here.
    if keystroke.modifiers.control && !keystroke.modifiers.alt {
        if let Some(letter) = single_ascii_letter(&keystroke.key) {
            let code = letter.to_ascii_lowercase() as u8 - b'a' + 1;
            return Some(vec![code]);
        }
    }

    // Anything else -- plain printable text, space, Alt-modified accented
    // characters, ... -- is deliberately left to the input handler (see
    // this module's doc comment).
    None
}

/// Kitty keyboard protocol `CSI u` encoding for the three keystroke
/// families this module disambiguates -- see this module's "Kitty keyboard
/// protocol" doc section for the full rationale behind each:
///
/// - Escape, always (`CSI 27 u` / `CSI 27;<m> u`).
/// - A **modifier-carrying** Enter/Tab (`CSI 13;<m> u` / `CSI 9;<m> u`).
/// - Ctrl+<letter>, optionally combined with Alt (`CSI <codepoint>;<m> u`).
///
/// Returns `None` for every other keystroke this crate doesn't re-encode
/// here: any key besides those three families, or an Enter/Tab held with
/// no modifier at all (the spec's own documented exception -- these still
/// fall through to their plain legacy byte in the caller's match below,
/// unchanged from before this function existed).
fn kitty_disambiguated_bytes(keystroke: &Keystroke) -> Option<Vec<u8>> {
    // Escape has no legacy-byte exemption at all (unlike Enter/Tab below) --
    // sent as `CSI 27u`, or `CSI 27;<m>u` once a modifier is held, in both
    // cases regardless of whether the key otherwise carries a `key_char`.
    if keystroke.key == "escape" {
        return Some(match kitty_modifier(&keystroke.modifiers) {
            Some(m) => format!("\x1b[27;{m}u").into_bytes(),
            None => b"\x1b[27u".to_vec(),
        });
    }

    // Key codes from the Kitty keyboard protocol's functional-key table
    // (https://sw.kovidgoyal.net/kitty/keyboard-protocol/#functional-key-definitions).
    if let Some(key_code) = match keystroke.key.as_str() {
        "enter" => Some(13),
        "tab" => Some(9),
        _ => None,
    } {
        let modifier = kitty_modifier(&keystroke.modifiers)?;
        return Some(format!("\x1b[{key_code};{modifier}u").into_bytes());
    }

    // Ctrl+<letter>, optionally combined with Alt -- both `ctrl+key` and
    // `ctrl+alt+key` are named in the same spec paragraph that documents
    // this whole `CSI u` re-encoding (quoted above). Bare Alt+<letter> and
    // Shift+Alt+<letter>, also named there, are -- like Backspace above --
    // left to the input handler unchanged: out of scope, since nothing in
    // this app currently depends on disambiguating them.
    if keystroke.modifiers.control {
        if let Some(letter) = single_ascii_letter(&keystroke.key) {
            let codepoint = letter.to_ascii_lowercase() as u32;
            let modifier = kitty_modifier(&keystroke.modifiers)?;
            return Some(format!("\x1b[{codepoint};{modifier}u").into_bytes());
        }
    }

    None
}

/// The legacy xterm modified-cursor-key encoding for one arrow key: its
/// plain `CSI <letter>` when unmodified (byte-for-byte unchanged from
/// before this wave), or `CSI 1;<modifier> <letter>` once any modifier is
/// held -- see this module's "Kitty keyboard protocol" doc section for why
/// this applies unconditionally, independent of `kitty_disambiguate`.
/// `letter` is one of `A`/`B`/`C`/`D` (up/down/right/left).
fn cursor_key_bytes(letter: char, modifiers: &Modifiers) -> Vec<u8> {
    match kitty_modifier(modifiers) {
        None => format!("\x1b[{letter}").into_bytes(),
        Some(m) => format!("\x1b[1;{m}{letter}").into_bytes(),
    }
}

/// The modifier-bit formula the Kitty keyboard protocol's `CSI u` encoding
/// uses: `1 + sum of active modifier bits` (shift `1`, alt `2`, ctrl `4`;
/// super/hyper/meta/caps-lock/num-lock are not tracked by `gpui::Modifiers`
/// and so never contribute here -- see
/// <https://sw.kovidgoyal.net/kitty/keyboard-protocol/#modifiers>). Shared
/// by `kitty_disambiguated_bytes` (gated on `kitty_disambiguate`) and
/// `cursor_key_bytes` (unconditional -- see this module's "Kitty keyboard
/// protocol" doc section for why modified arrow keys use this same
/// modifier field despite not being a Kitty-specific encoding at all).
///
/// `None` when no modifier is held at all -- each caller's signal to leave
/// the keystroke to its plain legacy encoding instead: for
/// `kitty_disambiguated_bytes`'s Enter/Tab case, this is the protocol's own
/// documented exception; for `cursor_key_bytes`, it's simply that an
/// unmodified arrow key's plain `CSI <letter>` carries no modifier field at
/// all.
fn kitty_modifier(modifiers: &Modifiers) -> Option<u8> {
    if !(modifiers.shift || modifiers.alt || modifiers.control) {
        return None;
    }
    let mut value: u8 = 1;
    if modifiers.shift {
        value += 1;
    }
    if modifiers.alt {
        value += 2;
    }
    if modifiers.control {
        value += 4;
    }
    Some(value)
}

/// `Some(letter)` iff `key` is exactly one ASCII letter (gpui's `key` field
/// for a plain letter keystroke, e.g. `"a"`, not `"a1"` or a named key like
/// `"tab"`).
fn single_ascii_letter(key: &str) -> Option<char> {
    let mut chars = key.chars();
    let first = chars.next()?;
    if chars.next().is_none() && first.is_ascii_alphabetic() {
        Some(first)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::Modifiers;

    fn keystroke(key: &str, key_char: Option<&str>, modifiers: Modifiers) -> Keystroke {
        Keystroke {
            modifiers,
            key: key.to_string(),
            key_char: key_char.map(|s| s.to_string()),
        }
    }

    #[test]
    fn printable_character_is_left_to_the_input_handler() {
        // No key_char-carrying, unmodified (or shift-only) keystroke is
        // handled here anymore -- see this module's doc comment: it must
        // reach the platform's IME/text-input machinery instead, both so a
        // composition can start and so it isn't double-sent once one does.
        let ks = keystroke("a", Some("a"), Modifiers::none());
        assert_eq!(keystroke_to_bytes(&ks, false), None);
    }

    #[test]
    fn shifted_character_is_left_to_the_input_handler() {
        let ks = keystroke(
            "a",
            Some("A"),
            Modifiers {
                shift: true,
                ..Modifiers::none()
            },
        );
        assert_eq!(keystroke_to_bytes(&ks, false), None);
    }

    #[test]
    fn space_is_left_to_the_input_handler() {
        let ks = keystroke("space", Some(" "), Modifiers::none());
        assert_eq!(keystroke_to_bytes(&ks, false), None);
    }

    #[test]
    fn enter_backspace_tab_escape() {
        assert_eq!(
            keystroke_to_bytes(&keystroke("enter", None, Modifiers::none()), false),
            Some(vec![b'\r'])
        );
        assert_eq!(
            keystroke_to_bytes(&keystroke("backspace", None, Modifiers::none()), false),
            Some(vec![0x7f])
        );
        assert_eq!(
            keystroke_to_bytes(&keystroke("tab", None, Modifiers::none()), false),
            Some(vec![b'\t'])
        );
        assert_eq!(
            keystroke_to_bytes(&keystroke("escape", None, Modifiers::none()), false),
            Some(vec![0x1b])
        );
    }

    #[test]
    fn arrow_keys_send_csi_sequences() {
        assert_eq!(
            keystroke_to_bytes(&keystroke("up", None, Modifiers::none()), false),
            Some(b"\x1b[A".to_vec())
        );
        assert_eq!(
            keystroke_to_bytes(&keystroke("down", None, Modifiers::none()), false),
            Some(b"\x1b[B".to_vec())
        );
        assert_eq!(
            keystroke_to_bytes(&keystroke("right", None, Modifiers::none()), false),
            Some(b"\x1b[C".to_vec())
        );
        assert_eq!(
            keystroke_to_bytes(&keystroke("left", None, Modifiers::none()), false),
            Some(b"\x1b[D".to_vec())
        );
    }

    #[test]
    fn ctrl_letter_sends_control_code() {
        let ctrl = Modifiers {
            control: true,
            ..Modifiers::none()
        };
        assert_eq!(
            keystroke_to_bytes(&keystroke("a", Some("a"), ctrl), false),
            Some(vec![0x01])
        );
        assert_eq!(
            keystroke_to_bytes(&keystroke("c", Some("c"), ctrl), false),
            Some(vec![0x03])
        );
        assert_eq!(
            keystroke_to_bytes(&keystroke("z", Some("z"), ctrl), false),
            Some(vec![0x1a])
        );
    }

    #[test]
    fn ctrl_alt_letter_is_not_treated_as_a_control_code() {
        let ctrl_alt = Modifiers {
            control: true,
            alt: true,
            ..Modifiers::none()
        };
        // Not a bare control code, and (like any other key_char-carrying
        // keystroke this function doesn't special-case) left to the input
        // handler rather than falling back to key_char here -- e.g.
        // option-a on macOS types "\u{e5}" via the same text-input path a
        // plain letter would.
        let ks = keystroke("a", Some("\u{e5}"), ctrl_alt);
        assert_eq!(keystroke_to_bytes(&ks, false), None);
    }

    #[test]
    fn cmd_combination_is_not_forwarded_to_the_terminal() {
        let cmd = Modifiers {
            platform: true,
            ..Modifiers::none()
        };
        assert_eq!(
            keystroke_to_bytes(&keystroke("t", Some("t"), cmd), false),
            None
        );
    }

    #[test]
    fn bare_modifier_with_no_key_char_produces_nothing() {
        let ks = keystroke(
            "shift",
            None,
            Modifiers {
                shift: true,
                ..Modifiers::none()
            },
        );
        assert_eq!(keystroke_to_bytes(&ks, false), None);
    }

    // -- Kitty keyboard protocol (`kitty_disambiguate: true`) --------------

    #[test]
    fn kitty_off_leaves_every_keystroke_unchanged() {
        // With the flag off (the default -- no program has requested it),
        // behavior must be byte-for-byte identical to every test above,
        // even for a keystroke `kitty_disambiguated_bytes` *would* re-encode
        // if asked. Spot-checked here with Shift+Enter, which is exactly
        // the keystroke this feature exists to change when the flag is on.
        let shift_enter = keystroke(
            "enter",
            None,
            Modifiers {
                shift: true,
                ..Modifiers::none()
            },
        );
        assert_eq!(
            keystroke_to_bytes(&shift_enter, false),
            Some(vec![b'\r']),
            "shift+enter must still be plain \\r when kitty_disambiguate is off"
        );
    }

    #[test]
    fn kitty_on_unmodified_enter_and_tab_stay_legacy() {
        // The Kitty spec's own documented exception: even with disambiguate
        // on, an unmodified Enter/Tab keeps sending its plain legacy byte
        // (so `reset<Enter>` still works in a shell after a crashed program
        // leaves the mode enabled) -- see this module's doc comment.
        assert_eq!(
            keystroke_to_bytes(&keystroke("enter", None, Modifiers::none()), true),
            Some(vec![b'\r'])
        );
        assert_eq!(
            keystroke_to_bytes(&keystroke("tab", None, Modifiers::none()), true),
            Some(vec![b'\t'])
        );
    }

    #[test]
    fn kitty_on_shift_enter_sends_csi_u() {
        // The motivating case: Claude Code's TUI distinguishes Shift+Enter
        // (insert a newline) from a plain Enter (submit) via exactly this
        // sequence once it has pushed the disambiguate flag.
        let shift_enter = keystroke(
            "enter",
            None,
            Modifiers {
                shift: true,
                ..Modifiers::none()
            },
        );
        assert_eq!(
            keystroke_to_bytes(&shift_enter, true),
            Some(b"\x1b[13;2u".to_vec())
        );
    }

    /// Regression coverage for the wave 15 follow-up's key-routing
    /// hypothesis (see this module's doc comment's "Correction" note):
    /// real macOS keystrokes for Enter carry `key_char: Some("\n")`
    /// **unconditionally** (verified by reading gpui `0.2.2`'s
    /// `platform/mac/events.rs`), not `None` the way every other test in
    /// this file constructs it -- this function must be indifferent to
    /// that, since it never inspects `key_char` for a named key match at
    /// all. Exercised with the exact keystroke shape a real Shift+Enter
    /// keypress produces on macOS, both with the flag off (legacy `\r`,
    /// matching this file's pre-wave-15 behavior) and on (the Kitty `CSI
    /// u` encoding) -- both must be identical to the `key_char: None`
    /// versions above.
    #[test]
    fn real_macos_enter_key_char_does_not_change_the_result() {
        let shift_enter_real_key_char = keystroke(
            "enter",
            Some("\n"),
            Modifiers {
                shift: true,
                ..Modifiers::none()
            },
        );
        assert_eq!(
            keystroke_to_bytes(&shift_enter_real_key_char, false),
            Some(vec![b'\r']),
            "kitty off: identical to the key_char: None case"
        );
        assert_eq!(
            keystroke_to_bytes(&shift_enter_real_key_char, true),
            Some(b"\x1b[13;2u".to_vec()),
            "kitty on: identical to the key_char: None case"
        );

        let plain_enter_real_key_char = keystroke("enter", Some("\n"), Modifiers::none());
        assert_eq!(
            keystroke_to_bytes(&plain_enter_real_key_char, true),
            Some(vec![b'\r']),
            "kitty on, unmodified: still legacy \\r regardless of key_char"
        );
    }

    #[test]
    fn kitty_on_alt_and_ctrl_enter_send_csi_u_with_correct_modifier() {
        let alt_enter = keystroke(
            "enter",
            None,
            Modifiers {
                alt: true,
                ..Modifiers::none()
            },
        );
        assert_eq!(
            keystroke_to_bytes(&alt_enter, true),
            Some(b"\x1b[13;3u".to_vec()),
            "alt+enter: modifier = 1 + alt(2) = 3"
        );

        let ctrl_enter = keystroke(
            "enter",
            None,
            Modifiers {
                control: true,
                ..Modifiers::none()
            },
        );
        assert_eq!(
            keystroke_to_bytes(&ctrl_enter, true),
            Some(b"\x1b[13;5u".to_vec()),
            "ctrl+enter: modifier = 1 + ctrl(4) = 5"
        );

        let ctrl_shift_alt_enter = keystroke(
            "enter",
            None,
            Modifiers {
                shift: true,
                alt: true,
                control: true,
                ..Modifiers::none()
            },
        );
        assert_eq!(
            keystroke_to_bytes(&ctrl_shift_alt_enter, true),
            Some(b"\x1b[13;8u".to_vec()),
            "ctrl+alt+shift+enter: modifier = 1 + shift(1) + alt(2) + ctrl(4) = 8"
        );
    }

    #[test]
    fn kitty_on_shift_tab_sends_csi_u() {
        // Shift+Tab ("backtab") -- Claude Code's TUI uses it to cycle modes;
        // legacy `\t` alone can't be told apart from a plain Tab.
        let shift_tab = keystroke(
            "tab",
            None,
            Modifiers {
                shift: true,
                ..Modifiers::none()
            },
        );
        assert_eq!(
            keystroke_to_bytes(&shift_tab, true),
            Some(b"\x1b[9;2u".to_vec())
        );
    }

    #[test]
    fn kitty_on_cmd_combination_still_not_forwarded() {
        // The Cmd/Super early-return above `kitty_disambiguated_bytes` in
        // `keystroke_to_bytes` still wins even when the flag is on --
        // Cmd+Enter (if bound to anything) must never reach the terminal.
        let cmd_enter = keystroke(
            "enter",
            None,
            Modifiers {
                platform: true,
                ..Modifiers::none()
            },
        );
        assert_eq!(keystroke_to_bytes(&cmd_enter, true), None);
    }

    #[test]
    fn kitty_on_backspace_and_unmodified_arrows_unaffected() {
        // Backspace (any modifiers -- out of scope, see the module doc
        // comment) and an *unmodified* arrow key (handled by
        // `cursor_key_bytes`, not `kitty_disambiguated_bytes` -- see below)
        // are the only keys this module handles that stay byte-for-byte
        // unchanged whether the flag is on or off. Escape and Ctrl+<letter>
        // are *not* in this set any more -- see
        // `kitty_on_escape_unmodified_sends_csi_27u` and
        // `kitty_on_ctrl_letter_sends_csi_u_with_codepoint` below.
        assert_eq!(
            keystroke_to_bytes(&keystroke("backspace", None, Modifiers::none()), true),
            Some(vec![0x7f])
        );
        assert_eq!(
            keystroke_to_bytes(&keystroke("up", None, Modifiers::none()), true),
            Some(b"\x1b[A".to_vec())
        );
    }

    // -- Kitty keyboard protocol (`kitty_disambiguate: true`) -- Escape ----

    #[test]
    fn kitty_on_escape_unmodified_sends_csi_27u() {
        // Unlike Enter/Tab, Escape has no legacy-byte exemption at all once
        // disambiguate is on -- see the module doc comment.
        assert_eq!(
            keystroke_to_bytes(&keystroke("escape", None, Modifiers::none()), true),
            Some(b"\x1b[27u".to_vec())
        );
    }

    #[test]
    fn kitty_on_escape_modified_sends_csi_27_with_modifier() {
        let shift_escape = keystroke(
            "escape",
            None,
            Modifiers {
                shift: true,
                ..Modifiers::none()
            },
        );
        assert_eq!(
            keystroke_to_bytes(&shift_escape, true),
            Some(b"\x1b[27;2u".to_vec())
        );

        let ctrl_escape = keystroke(
            "escape",
            None,
            Modifiers {
                control: true,
                ..Modifiers::none()
            },
        );
        assert_eq!(
            keystroke_to_bytes(&ctrl_escape, true),
            Some(b"\x1b[27;5u".to_vec())
        );
    }

    #[test]
    fn kitty_off_escape_stays_legacy_even_with_modifiers() {
        // With the flag off, this module doesn't disambiguate Escape at all
        // -- unchanged from before this wave.
        let shift_escape = keystroke(
            "escape",
            None,
            Modifiers {
                shift: true,
                ..Modifiers::none()
            },
        );
        assert_eq!(keystroke_to_bytes(&shift_escape, false), Some(vec![0x1b]));
    }

    // -- Kitty keyboard protocol (`kitty_disambiguate: true`) -- Ctrl+letter

    #[test]
    fn kitty_on_ctrl_letter_sends_csi_u_with_codepoint() {
        let ctrl = Modifiers {
            control: true,
            ..Modifiers::none()
        };
        assert_eq!(
            keystroke_to_bytes(&keystroke("c", Some("c"), ctrl), true),
            Some(b"\x1b[99;5u".to_vec()),
            "ctrl+c: codepoint 99 ('c'), modifier = 1 + ctrl(4) = 5"
        );
    }

    #[test]
    fn kitty_on_ctrl_alt_letter_sends_csi_u_with_combined_modifier() {
        let ctrl_alt = Modifiers {
            control: true,
            alt: true,
            ..Modifiers::none()
        };
        assert_eq!(
            keystroke_to_bytes(&keystroke("c", Some("\u{e7}"), ctrl_alt), true),
            Some(b"\x1b[99;7u".to_vec()),
            "ctrl+alt+c: modifier = 1 + alt(2) + ctrl(4) = 7"
        );
    }

    #[test]
    fn kitty_on_ctrl_shift_letter_sends_csi_u_with_combined_modifier() {
        // gpui reports `key` as the lowercase, unshifted key even while
        // Shift is held alongside Ctrl (see `single_ascii_letter`'s use
        // above, and the doc comment on the plain Ctrl+<letter> match arm
        // further down) -- so this is exactly the same codepoint as the
        // bare-Ctrl case above, just with the shift bit folded into the
        // modifier field too.
        let ctrl_shift = Modifiers {
            control: true,
            shift: true,
            ..Modifiers::none()
        };
        assert_eq!(
            keystroke_to_bytes(&keystroke("c", Some("C"), ctrl_shift), true),
            Some(b"\x1b[99;6u".to_vec()),
            "ctrl+shift+c: modifier = 1 + shift(1) + ctrl(4) = 6"
        );
    }

    #[test]
    fn kitty_off_ctrl_letter_stays_legacy_c0() {
        // Unchanged from before this wave -- the C0 fallback in
        // `keystroke_to_bytes` still owns this case when the flag is off.
        let ctrl = Modifiers {
            control: true,
            ..Modifiers::none()
        };
        assert_eq!(
            keystroke_to_bytes(&keystroke("c", Some("c"), ctrl), false),
            Some(vec![0x03])
        );
    }

    // -- Modified arrow keys (unconditional -- not gated on kitty_disambiguate)

    #[test]
    fn modified_arrow_keys_send_csi_1_modifier_letter() {
        // xterm's legacy modified-cursor-key encoding -- applies regardless
        // of the Kitty flag (see the module doc comment); exercised here
        // with `kitty_disambiguate: false` specifically, since this is the
        // regression this wave fixes: before it, every arrow keystroke sent
        // its bare unmodified form even with a modifier held.
        let shift = Modifiers {
            shift: true,
            ..Modifiers::none()
        };
        assert_eq!(
            keystroke_to_bytes(&keystroke("up", None, shift), false),
            Some(b"\x1b[1;2A".to_vec())
        );
        let ctrl = Modifiers {
            control: true,
            ..Modifiers::none()
        };
        assert_eq!(
            keystroke_to_bytes(&keystroke("down", None, ctrl), false),
            Some(b"\x1b[1;5B".to_vec())
        );
        let alt = Modifiers {
            alt: true,
            ..Modifiers::none()
        };
        assert_eq!(
            keystroke_to_bytes(&keystroke("right", None, alt), false),
            Some(b"\x1b[1;3C".to_vec())
        );
        let ctrl_shift = Modifiers {
            control: true,
            shift: true,
            ..Modifiers::none()
        };
        assert_eq!(
            keystroke_to_bytes(&keystroke("left", None, ctrl_shift), false),
            Some(b"\x1b[1;6D".to_vec())
        );
    }

    #[test]
    fn modified_arrow_keys_identical_with_kitty_disambiguate_on() {
        // Not a Kitty-specific enhancement -- identical whether the flag is
        // on or off (see `cursor_key_bytes`'s doc comment).
        let shift = Modifiers {
            shift: true,
            ..Modifiers::none()
        };
        assert_eq!(
            keystroke_to_bytes(&keystroke("up", None, shift), true),
            Some(b"\x1b[1;2A".to_vec())
        );
    }

    #[test]
    fn kitty_modifier_matches_spec_formula() {
        assert_eq!(kitty_modifier(&Modifiers::none()), None);
        assert_eq!(
            kitty_modifier(&Modifiers {
                shift: true,
                ..Modifiers::none()
            }),
            Some(2)
        );
        assert_eq!(
            kitty_modifier(&Modifiers {
                alt: true,
                ..Modifiers::none()
            }),
            Some(3)
        );
        assert_eq!(
            kitty_modifier(&Modifiers {
                control: true,
                ..Modifiers::none()
            }),
            Some(5)
        );
        assert_eq!(
            kitty_modifier(&Modifiers {
                shift: true,
                alt: true,
                control: true,
                ..Modifiers::none()
            }),
            Some(8)
        );
    }
}
