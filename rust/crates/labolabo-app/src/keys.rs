//! Pure translation from a gpui key event to the bytes a real terminal
//! expects on its PTY input.
//!
//! `gpui::Keystroke`/`Modifiers` are plain data (no `App`/`Window` runtime
//! needed to construct one), so this is directly unit-testable -- see the
//! tests below, which build `Keystroke` values by hand.
//!
//! TODO(W5a): no IME support. gpui's `EntityInputHandler` (composition,
//! marked/underlined text, CJK input methods) is not wired up -- this module
//! only handles single dispatched `KeyDownEvent`s. Composed input (e.g. a
//! Japanese IME) will not work until that lands.
//!
//! TODO(W5a): only the keys this wave's brief calls for are mapped
//! (printable text, Enter/Backspace/Tab/Escape/arrows, and a bare
//! Ctrl-<letter>). Delete/Home/End/PageUp/PageDown/function keys and
//! modifier combinations beyond a lone Ctrl are future work.

use gpui::Keystroke;

/// Translate one key-down event into the bytes to write to the PTY, or
/// `None` if this keystroke has no terminal-input meaning (a bare modifier
/// key with no `key_char`, or a Cmd/Super combination -- reserved for
/// application-level shortcuts, not terminal input).
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
        "space" => return Some(vec![b' ']),
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

    // Fall back to whatever gpui says was actually typed -- printable text,
    // including non-ASCII (accented letters, symbols typed via a
    // dead-key-free layout, ...).
    keystroke.key_char.as_deref().map(|s| s.as_bytes().to_vec())
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
    fn printable_character_forwards_key_char() {
        let ks = keystroke("a", Some("a"), Modifiers::none());
        assert_eq!(keystroke_to_bytes(&ks), Some(b"a".to_vec()));
    }

    #[test]
    fn shifted_character_forwards_the_shifted_key_char() {
        let ks = keystroke(
            "a",
            Some("A"),
            Modifiers {
                shift: true,
                ..Modifiers::none()
            },
        );
        assert_eq!(keystroke_to_bytes(&ks), Some(b"A".to_vec()));
    }

    #[test]
    fn enter_backspace_tab_escape_space() {
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
        assert_eq!(
            keystroke_to_bytes(&keystroke("space", Some(" "), Modifiers::none())),
            Some(vec![b' '])
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
        // Falls back to whatever key_char gpui reports for the combination
        // (e.g. option-a on macOS types "\u{e5}"), not a bare control code.
        let ks = keystroke("a", Some("\u{e5}"), ctrl_alt);
        assert_eq!(keystroke_to_bytes(&ks), Some("\u{e5}".as_bytes().to_vec()));
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
