//! Pure encoding of a clipboard paste (Cmd+V) into the bytes written to a
//! pane's PTY.
//!
//! Like `keys.rs`'s key-to-bytes translation, this needs no `App`/`Window`
//! runtime to exercise, so it's directly unit-testable -- see the tests
//! below.
//!
//! Two conventions a real terminal follows that a raw clipboard string
//! doesn't already satisfy:
//!
//! - **Newlines.** A pasted multi-line string typically arrives with the
//!   platform's/editor's own line-ending convention (`"\n"`, or `"\r\n"` on
//!   Windows-authored text). A terminal's Enter key sends a bare `"\r"`, and
//!   programs reading raw/cbreak-mode input (readline, vim, ...) expect
//!   exactly that for a pasted line break too -- so every line ending is
//!   normalized to `"\r"` before it reaches the PTY.
//! - **Bracketed paste** (DECSET `2004`). When the foreground program has
//!   opted in (`Terminal::bracketed_paste`, `labolabo-term`'s mode-query
//!   API), the pasted bytes are wrapped in `ESC[200~...ESC[201~` so the
//!   program can tell "this arrived via paste" from "the user typed this"
//!   (readline/shells use it to paste literally instead of interpreting
//!   each character, e.g. auto-indent-on-newline).
//!
//! Before either transform, unsafe control bytes are stripped from the
//! clipboard text (see [`strip_unsafe_control_bytes`]) -- in particular,
//! `ESC` (`0x1b`), which is what a malicious/corrupted clipboard payload
//! would need to embed a literal `ESC[201~` bracketed-paste end marker (or
//! any other escape sequence) to break out of the paste and have the
//! terminal interpret the rest of the payload as directly-typed input. This
//! mirrors the "unsafe control bytes" stripping `libghostty-vt`'s own
//! `paste::encode` documents (this crate doesn't depend on that function --
//! it's ghostty-backend-only -- so the same behavior is reimplemented here,
//! backend-independent).

/// The bracketed-paste start/end markers (DECSET `2004`).
const BRACKETED_PASTE_START: &[u8] = b"\x1b[200~";
const BRACKETED_PASTE_END: &[u8] = b"\x1b[201~";

/// Encode a clipboard string as the bytes to write to a pane's PTY for a
/// paste: unsafe control bytes stripped, line endings normalized to `"\r"`,
/// and -- when `bracketed` is true (the pane's `Terminal::bracketed_paste()`
/// at the moment of the paste) -- wrapped in `ESC[200~...ESC[201~`.
pub fn encode_paste(text: &str, bracketed: bool) -> Vec<u8> {
    let sanitized = strip_unsafe_control_bytes(text);
    let normalized = normalize_paste_newlines(&sanitized);
    if bracketed {
        let mut out = Vec::with_capacity(
            normalized.len() + BRACKETED_PASTE_START.len() + BRACKETED_PASTE_END.len(),
        );
        out.extend_from_slice(BRACKETED_PASTE_START);
        out.extend_from_slice(normalized.as_bytes());
        out.extend_from_slice(BRACKETED_PASTE_END);
        out
    } else {
        normalized.into_bytes()
    }
}

/// Normalize every line ending in `text` to a bare `"\r"` (a real terminal's
/// Enter key convention): `"\r\n"` collapses to a single `"\r"`, and a lone
/// `"\n"` becomes `"\r"` too. A lone `"\r"` already present is left alone.
pub fn normalize_paste_newlines(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\r' => {
                // "\r\n" -> "\r" (consume the paired "\n"); a lone "\r"
                // passes through unchanged either way.
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                out.push('\r');
            }
            '\n' => out.push('\r'),
            other => out.push(other),
        }
    }
    out
}

/// Strip ASCII control bytes from `text` that could be used to inject
/// terminal escape sequences -- every C0 control character except tab
/// (`\t`, preserved verbatim) and the newline characters
/// [`normalize_paste_newlines`] handles, plus DEL (`0x7f`). In particular
/// this removes every `ESC` (`0x1b`), which is what would otherwise let a
/// crafted clipboard payload embed a literal bracketed-paste end marker
/// (`ESC[201~`) or any other escape sequence within pasted text.
fn strip_unsafe_control_bytes(text: &str) -> String {
    text.chars()
        .filter(|&c| {
            let is_stripped_control = matches!(c, '\u{0}'..='\u{1f}' | '\u{7f}');
            let is_kept = matches!(c, '\t' | '\n' | '\r');
            !is_stripped_control || is_kept
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crlf_collapses_to_cr() {
        assert_eq!(normalize_paste_newlines("a\r\nb"), "a\rb");
    }

    #[test]
    fn lone_lf_becomes_cr() {
        assert_eq!(normalize_paste_newlines("a\nb\nc"), "a\rb\rc");
    }

    #[test]
    fn lone_cr_is_left_alone() {
        assert_eq!(normalize_paste_newlines("a\rb"), "a\rb");
    }

    #[test]
    fn mixed_line_endings_all_normalize() {
        assert_eq!(normalize_paste_newlines("a\r\nb\nc\rd"), "a\rb\rc\rd");
    }

    #[test]
    fn text_with_no_newlines_is_unchanged() {
        assert_eq!(normalize_paste_newlines("hello world"), "hello world");
    }

    #[test]
    fn tab_is_preserved() {
        assert_eq!(normalize_paste_newlines("a\tb"), "a\tb");
    }

    #[test]
    fn escape_and_control_bytes_are_stripped() {
        let text = "a\u{1b}[31mb\u{0}c\u{7f}d";
        assert_eq!(strip_unsafe_control_bytes(text), "a[31mbcd");
    }

    #[test]
    fn unbracketed_paste_is_just_the_normalized_bytes() {
        let bytes = encode_paste("echo hi\ndone", false);
        assert_eq!(bytes, b"echo hi\rdone");
    }

    #[test]
    fn bracketed_paste_wraps_with_start_and_end_markers() {
        let bytes = encode_paste("echo hi", true);
        assert_eq!(bytes, b"\x1b[200~echo hi\x1b[201~");
    }

    #[test]
    fn bracketed_paste_strips_embedded_end_marker_before_wrapping() {
        // A malicious/corrupted clipboard payload embedding the literal end
        // marker must not be able to break out of the wrapper early -- the
        // leading ESC of any such embedded sequence is stripped first.
        let text = "safe\x1b[201~ls -la\x1b[200~more";
        let bytes = encode_paste(text, true);
        let as_str = String::from_utf8(bytes).unwrap();
        assert_eq!(as_str, "\x1b[200~safe[201~ls -la[200~more\x1b[201~");
        // Exactly one real start marker and one real end marker (the
        // wrapper's own), nowhere else.
        assert_eq!(as_str.matches("\x1b[200~").count(), 1);
        assert_eq!(as_str.matches("\x1b[201~").count(), 1);
    }
}
