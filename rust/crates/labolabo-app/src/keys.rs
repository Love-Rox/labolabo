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

use gpui::Keystroke;

/// Translate one key-down event into the bytes to write to the PTY, or
/// `None` if this keystroke has no *direct* terminal-input meaning here --
/// either because it's a bare modifier key with no `key_char`, a Cmd/Super
/// combination (reserved for application-level shortcuts), or plain
/// printable text/space, which (see this module's doc comment) is now
/// handled exclusively via the platform's text-input/IME machinery instead.
pub fn keystroke_to_bytes(keystroke: &Keystroke) -> Option<Vec<u8>> {
    // Cmd (macOS) / Super (Linux/Windows) combinations are reserved for
    // application-level shortcuts (tab switching, quit, ...), never sent to
    // the terminal.
    if keystroke.modifiers.platform {
        return None;
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
        assert_eq!(keystroke_to_bytes(&ks), None);
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
        assert_eq!(keystroke_to_bytes(&ks), None);
    }

    #[test]
    fn space_is_left_to_the_input_handler() {
        let ks = keystroke("space", Some(" "), Modifiers::none());
        assert_eq!(keystroke_to_bytes(&ks), None);
    }

    #[test]
    fn enter_backspace_tab_escape() {
        assert_eq!(
            keystroke_to_bytes(&keystroke("enter", None, Modifiers::none())),
            Some(vec![b'\r'])
        );
        assert_eq!(
            keystroke_to_bytes(&keystroke("backspace", None, Modifiers::none())),
            Some(vec![0x7f])
        );
        assert_eq!(
            keystroke_to_bytes(&keystroke("tab", None, Modifiers::none())),
            Some(vec![b'\t'])
        );
        assert_eq!(
            keystroke_to_bytes(&keystroke("escape", None, Modifiers::none())),
            Some(vec![0x1b])
        );
    }

    #[test]
    fn arrow_keys_send_csi_sequences() {
        assert_eq!(
            keystroke_to_bytes(&keystroke("up", None, Modifiers::none())),
            Some(b"\x1b[A".to_vec())
        );
        assert_eq!(
            keystroke_to_bytes(&keystroke("down", None, Modifiers::none())),
            Some(b"\x1b[B".to_vec())
        );
        assert_eq!(
            keystroke_to_bytes(&keystroke("right", None, Modifiers::none())),
            Some(b"\x1b[C".to_vec())
        );
        assert_eq!(
            keystroke_to_bytes(&keystroke("left", None, Modifiers::none())),
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
            keystroke_to_bytes(&keystroke("a", Some("a"), ctrl)),
            Some(vec![0x01])
        );
        assert_eq!(
            keystroke_to_bytes(&keystroke("c", Some("c"), ctrl)),
            Some(vec![0x03])
        );
        assert_eq!(
            keystroke_to_bytes(&keystroke("z", Some("z"), ctrl)),
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
        assert_eq!(keystroke_to_bytes(&ks), None);
    }

    #[test]
    fn cmd_combination_is_not_forwarded_to_the_terminal() {
        let cmd = Modifiers {
            platform: true,
            ..Modifiers::none()
        };
        assert_eq!(keystroke_to_bytes(&keystroke("t", Some("t"), cmd)), None);
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
        assert_eq!(keystroke_to_bytes(&ks), None);
    }
}
