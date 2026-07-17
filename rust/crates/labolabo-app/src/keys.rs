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
//! see `task_workspace::render_leaf`), the platform (macOS's
//! `NSTextInputContext`, X11/Wayland's IBus/fcitx bridge) takes over
//! **every** keystroke that carries a `key_char` and no
//! Ctrl/Alt/Cmd modifier: it either self-inserts the character (calling
//! `EntityInputHandler::replace_text_in_range`) or starts an IME
//! composition (`replace_and_mark_text_in_range`). This is true on macOS,
//! X11, *and* Wayland (traced through gpui's own platform backends: all
//! three route plain/shift-only key_char keystrokes through the input
//! handler once one is registered, not through the raw `KeyDownEvent`).
//!
//! That means this function must **not** also handle those keystrokes --
//! doing so would either double-send the character (once here, once via the
//! input handler) or, worse, pre-empt IME composition entirely (writing the
//! *unconverted* Roman letter to the PTY before the IME ever gets a chance
//! to turn it into a composed character). So `keystroke_to_bytes` is
//! intentionally narrow: it only covers keys that must always be handled
//! directly, because they either carry no `key_char` at all (Enter,
//! Backspace, Tab, Escape, arrows) or must never be treated as text
//! (Ctrl-<letter>). Plain printable text -- including space -- is *not*
//! handled here anymore; it arrives exclusively via
//! `EntityInputHandler::replace_text_in_range`.
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
//! ## Kitty keyboard protocol (W15)
//!
//! Claude Code's own TUI relies on the Kitty keyboard protocol's
//! "disambiguate escape codes" progressive-enhancement flag (`CSI > 1 u`,
//! <https://sw.kovidgoyal.net/kitty/keyboard-protocol/>) to tell Shift+Enter
//! (insert a newline) apart from a plain Enter (submit) -- both keys, in
//! the legacy protocol below, always send the exact same `\r` regardless of
//! modifiers, so a program with no other channel has no way to distinguish
//! them. Once the running program has requested that flag (queried via
//! `labolabo_term::TermSession::kitty_disambiguate`, threaded in from
//! `app::LaboLaboApp::key_down`), a **modifier-carrying** Enter/Tab is
//! re-encoded as `CSI <code>;<modifier> u` instead -- see
//! `kitty_disambiguated_bytes` below.
//!
//! Deliberately narrow, matching the Kitty spec's own documented exception:
//! an *unmodified* Enter/Tab still sends its plain legacy byte even with
//! disambiguate on ("this is to allow the user to type and execute
//! commands in the shell such as `reset` after a program that sets this
//! mode crashes without clearing it" -- the spec's own words), so this
//! change is invisible to every other keystroke this module handles, and
//! Backspace/Escape are left untouched entirely (out of scope -- nothing
//! in this app currently binds a modified Backspace/Escape, so there is
//! no ambiguity to resolve for them yet).

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
/// module's "Kitty keyboard protocol" doc section above) -- when `true`, a
/// modifier-carrying Enter/Tab is re-encoded as a Kitty `CSI u` sequence
/// instead of its plain legacy byte.
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
        "up" => return Some(b"\x1b[A".to_vec()),
        "down" => return Some(b"\x1b[B".to_vec()),
        "right" => return Some(b"\x1b[C".to_vec()),
        "left" => return Some(b"\x1b[D".to_vec()),
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

/// Kitty keyboard protocol `CSI u` encoding for a **modifier-carrying**
/// Enter/Tab -- see this module's "Kitty keyboard protocol" doc section.
///
/// Returns `None` for every keystroke this crate doesn't re-encode: any key
/// other than Enter/Tab, or an Enter/Tab held with no modifier at all (the
/// spec's own documented exception -- these still fall through to their
/// plain legacy byte in the caller's match below, unchanged from before
/// this function existed).
fn kitty_disambiguated_bytes(keystroke: &Keystroke) -> Option<Vec<u8>> {
    // Key codes from the Kitty keyboard protocol's functional-key table
    // (https://sw.kovidgoyal.net/kitty/keyboard-protocol/#functional-key-definitions).
    let key_code = match keystroke.key.as_str() {
        "enter" => 13,
        "tab" => 9,
        _ => return None,
    };

    let modifier = kitty_modifier(&keystroke.modifiers)?;
    Some(format!("\x1b[{key_code};{modifier}u").into_bytes())
}

/// The Kitty keyboard protocol's modifier encoding: `1 + sum of active
/// modifier bits` (shift `1`, alt `2`, ctrl `4`; super/hyper/meta/caps-lock/
/// num-lock are not tracked by `gpui::Modifiers` and so never contribute
/// here -- see <https://sw.kovidgoyal.net/kitty/keyboard-protocol/#modifiers>).
///
/// `None` when no modifier is held at all -- the caller's signal to leave
/// the keystroke to its plain legacy encoding instead (see
/// `kitty_disambiguated_bytes`'s doc comment: an unmodified Enter/Tab is
/// the protocol's own documented exception, not something this function
/// ever needs to represent as `1`, the "no modifiers" value a full `CSI u`
/// sequence would otherwise carry).
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
    fn kitty_on_other_keys_unaffected() {
        // Only Enter/Tab are in scope -- every other key this module
        // handles (backspace/escape/arrows/ctrl-letter) is byte-for-byte
        // unchanged whether the flag is on or off.
        assert_eq!(
            keystroke_to_bytes(&keystroke("backspace", None, Modifiers::none()), true),
            Some(vec![0x7f])
        );
        assert_eq!(
            keystroke_to_bytes(&keystroke("escape", None, Modifiers::none()), true),
            Some(vec![0x1b])
        );
        assert_eq!(
            keystroke_to_bytes(&keystroke("up", None, Modifiers::none()), true),
            Some(b"\x1b[A".to_vec())
        );
        let ctrl = Modifiers {
            control: true,
            ..Modifiers::none()
        };
        assert_eq!(
            keystroke_to_bytes(&keystroke("a", Some("a"), ctrl), true),
            Some(vec![0x01])
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
