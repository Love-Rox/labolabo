//! A backend-independent, stateful scanner for OSC `52` (clipboard-set)
//! escape sequences: `ESC ] 52 ; <selection> ; <base64> BEL` (or the same
//! with an ST terminator, `ESC \` instead of a bare BEL).
//!
//! ## Why this crate parses OSC 52 itself, instead of reusing a backend
//!
//! Every other piece of terminal state this crate mirrors for its caller
//! (title, bracketed paste, mouse mode, ...) is read straight off the active
//! [`crate::backend::VtBackend`] -- each backend's own VT core already
//! tracks it. OSC 52 is the one exception: neither backend's public API
//! surfaces it at all. `libghostty-vt`'s `Terminal` has no clipboard
//! callback or getter (only `on_title_changed`/`on_pty_write`/
//! `on_device_attributes`) -- confirmed by reading its bindings, which do
//! advertise OSC 52 support in a Device Attributes response
//! (`DeviceAttributesFeature::CLIPBOARD`) but expose no way to read the
//! *payload* back out, even via its lower-level standalone `ghostty_osc_*`
//! parser (its `OscCommandData` enum has a data-extraction case for the
//! window-title string, and nothing for `CLIPBOARD_CONTENTS`).
//! `alacritty_terminal` does parse it internally (`Term::clipboard_store`,
//! `vendor/alacritty_terminal/src/term/mod.rs`) but only reports it through
//! an `Event::ClipboardStore` on its `EventListener` -- a mechanism neither
//! backend currently wires up for this event, and one that wouldn't help the
//! ghostty backend anyway (the two backends need to behave identically).
//!
//! So this module scans the *raw PTY byte stream* directly -- the same bytes
//! [`crate::session::run_worker`] already hands to `VtBackend::feed` --
//! independently of whichever VT core is compiled in. It is a small,
//! special-purpose OSC recognizer, not a general escape-sequence parser: it
//! only tracks enough state to find `ESC ] ... (BEL|ESC \)` spans, and only
//! acts on ones whose payload starts with `52;`. Everything else (other OSC
//! numbers, CSI sequences, plain text, ...) passes through as state
//! transitions that produce no output, exactly as if this scanner weren't
//! watching at all -- it never mutates or drops bytes, only observes them.
//!
//! ## Write-only, deliberately
//!
//! OSC 52 also carries a *read* request (`Pd == "?"`, "tell me the current
//! clipboard contents"). This scanner recognizes and silently discards that
//! case -- it is never surfaced to a caller, and this crate never writes a
//! reply back to the PTY. Answering it would let the child program read
//! whatever was last on the system clipboard, a real information leak this
//! crate does not opt into (matching Ghostty's own default of not answering
//! OSC 52 reads either). See [`Osc52Scanner::feed`]'s doc comment.

use base64::engine::general_purpose::STANDARD as Base64;
use base64::Engine as _;

const ESC: u8 = 0x1B;
const BEL: u8 = 0x07;

/// Payload length (in encoded bytes, before base64 decoding) above which an
/// in-progress OSC sequence is abandoned rather than buffered further --
/// defense against a runaway/malicious child flooding this scanner's memory
/// with an OSC sequence that never terminates. `1 MiB` is generous for any
/// legitimate clipboard payload (VT100-era terminals that originated OSC 52
/// assumed far less) while still bounding worst-case memory to something
/// negligible for a single terminal pane.
const MAX_PAYLOAD_LEN: usize = 1024 * 1024;

/// Parser state, advanced one PTY byte at a time. Persists across calls to
/// [`Osc52Scanner::feed`] so a sequence split across two PTY reads (a near
/// certainty for anything but the smallest payloads, given `READ_BUF_SIZE`
/// and how TTYs chunk output) is still recognized correctly.
#[derive(Default)]
enum State {
    /// Not inside any escape sequence. The overwhelmingly common state --
    /// most PTY output is plain text or CSI sequences (cursor movement,
    /// SGR color, ...) this scanner doesn't care about.
    #[default]
    Ground,
    /// Just saw a bare `ESC` byte. The only two transitions that matter:
    /// `]` starts an OSC sequence; anything else (including another `ESC`,
    /// which simply stays here) means this wasn't the start of one.
    Esc,
    /// Inside an OSC sequence's payload, collecting bytes until a
    /// terminator. `Vec<u8>` is the payload collected so far (everything
    /// after `ESC ]`, before any terminator).
    Osc(Vec<u8>),
    /// Inside an OSC payload, just saw an `ESC` that might be the first
    /// half of an ST terminator (`ESC \`). If the next byte is `\`, the
    /// sequence is complete; otherwise this `ESC` did *not* terminate the
    /// OSC (real terminals treat a bare `ESC` inside an OSC string as
    /// aborting it), so the pending payload is discarded and this byte is
    /// reprocessed as if freshly seen in [`State::Esc`] -- correctly
    /// recognizing a new `ESC ]` that immediately follows an unterminated
    /// one, with no byte lost.
    OscEsc(Vec<u8>),
    /// Same as [`State::Osc`], but the payload already exceeded
    /// [`MAX_PAYLOAD_LEN`] -- further bytes are discarded (not buffered) so
    /// memory stays bounded, while still tracking enough state to find the
    /// terminator and resynchronize afterward.
    OscOverflow,
    /// [`State::OscOverflow`]'s analogue of [`State::OscEsc`].
    OscOverflowEsc,
}

/// What [`State::Esc`] (a bare `ESC` byte) transitions to on the next byte --
/// shared by [`State::Esc`] itself and by [`State::OscEsc`]/
/// [`State::OscOverflowEsc`]'s "not a valid ST, reprocess this byte as if we
/// were freshly in `Esc`" fallback (the `ESC` that put us in `OscEsc` has
/// already been consumed; this byte is the one *after* it, exactly what
/// `State::Esc` itself expects next).
fn esc_transition(byte: u8) -> State {
    match byte {
        b']' => State::Osc(Vec::new()),
        ESC => State::Esc,
        _ => State::Ground,
    }
}

/// A stateful OSC 52 recognizer -- see the module doc comment for the full
/// rationale. Cheap to construct (`Default`); one instance lives for the
/// lifetime of a `TermSession`'s worker thread ([`crate::session::
/// run_worker`]), fed every batch of raw PTY bytes alongside (not instead
/// of) the active `VtBackend::feed`.
#[derive(Default)]
pub(crate) struct Osc52Scanner {
    state: State,
}

impl Osc52Scanner {
    /// Advance the scanner by `bytes` (a raw PTY read -- may start or end
    /// mid-sequence; state persists to the next call). `on_clipboard_set` is
    /// invoked, in order, once per complete `OSC 52 ; c ; <base64>` (or
    /// empty-selection `OSC 52 ; ; <base64>`) sequence found, with the
    /// base64 payload already decoded to text (invalid UTF-8 in the decoded
    /// bytes is replaced with U+FFFD via lossy conversion rather than
    /// dropping the whole payload -- see [`String::from_utf8_lossy`]).
    /// Called zero times if `bytes` contains no such sequence -- the common
    /// case for almost every PTY read.
    ///
    /// Every other case is recognized and silently ignored (`on_clipboard_
    /// set` not called), matching real terminals' typical leniency toward
    /// malformed control sequences rather than surfacing parse errors that
    /// have no sensible caller-facing representation:
    ///
    /// - A selection other than `c` or empty (e.g. `p`, the primary
    ///   selection) -- this crate only ever writes the system clipboard.
    /// - A `?` payload -- a clipboard *read* request. Deliberately never
    ///   answered; see the module doc comment.
    /// - A payload that fails to base64-decode.
    /// - Any other OSC number (title, hyperlink, ...) -- collected and
    ///   discarded once its terminator is found, same cost as a no-op.
    /// - A sequence whose buffered payload exceeds [`MAX_PAYLOAD_LEN`] --
    ///   discarded once its terminator is found; the scanner resynchronizes
    ///   cleanly and keeps recognizing subsequent sequences.
    pub(crate) fn feed(&mut self, bytes: &[u8], mut on_clipboard_set: impl FnMut(String)) {
        for &byte in bytes {
            self.step(byte, &mut on_clipboard_set);
        }
    }

    fn step(&mut self, byte: u8, on_clipboard_set: &mut impl FnMut(String)) {
        let state = std::mem::take(&mut self.state);
        self.state = match state {
            State::Ground => {
                if byte == ESC {
                    State::Esc
                } else {
                    State::Ground
                }
            }
            State::Esc => esc_transition(byte),
            State::Osc(mut buf) => {
                if byte == BEL {
                    Self::finish(&buf, on_clipboard_set);
                    State::Ground
                } else if byte == ESC {
                    State::OscEsc(buf)
                } else if buf.len() >= MAX_PAYLOAD_LEN {
                    State::OscOverflow
                } else {
                    buf.push(byte);
                    State::Osc(buf)
                }
            }
            State::OscEsc(buf) => {
                if byte == b'\\' {
                    Self::finish(&buf, on_clipboard_set);
                    State::Ground
                } else {
                    esc_transition(byte)
                }
            }
            State::OscOverflow => {
                if byte == BEL {
                    State::Ground
                } else if byte == ESC {
                    State::OscOverflowEsc
                } else {
                    State::OscOverflow
                }
            }
            State::OscOverflowEsc => {
                if byte == b'\\' {
                    State::Ground
                } else {
                    // Same "not a valid ST, reprocess this byte as if
                    // freshly in `Esc`" fallback as `State::OscEsc` above --
                    // a fresh `ESC ]` here correctly starts buffering again
                    // (the new sequence hasn't overflowed anything yet).
                    esc_transition(byte)
                }
            }
        };
    }

    /// A terminator was found -- `buf` is the complete OSC payload (after
    /// `ESC ]`, before the terminator). Parse and decode it as an OSC 52
    /// clipboard-set, invoking `on_clipboard_set` if (and only if) it is
    /// one this scanner accepts (see [`Self::feed`]'s doc comment for every
    /// rejection case).
    fn finish(buf: &[u8], on_clipboard_set: &mut impl FnMut(String)) {
        if let Some(text) = decode_clipboard_set(buf) {
            on_clipboard_set(text);
        }
    }
}

/// Parse a complete OSC payload (everything between `ESC ]` and its
/// terminator) as `52;<selection>;<data>`, returning the decoded clipboard
/// text if `selection` is `c` or empty and `data` is valid base64 -- `None`
/// for every other case (see [`Osc52Scanner::feed`]'s doc comment).
fn decode_clipboard_set(buf: &[u8]) -> Option<String> {
    let rest = buf.strip_prefix(b"52;")?;
    let sep = rest.iter().position(|&b| b == b';')?;
    let selection = &rest[..sep];
    let payload = &rest[sep + 1..];
    if !(selection.is_empty() || selection == b"c") {
        return None;
    }
    // A bare `?` is a clipboard *read* request -- never answered. See the
    // module doc comment.
    if payload == b"?" {
        return None;
    }
    let decoded = Base64.decode(payload).ok()?;
    Some(String::from_utf8_lossy(&decoded).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Feed `bytes` in one call and collect every payload the scanner
    /// reports, in order.
    fn scan_whole(bytes: &[u8]) -> Vec<String> {
        let mut scanner = Osc52Scanner::default();
        let mut out = Vec::new();
        scanner.feed(bytes, |text| out.push(text));
        out
    }

    /// Feed `bytes` one byte at a time (worst case for a scanner that must
    /// resume mid-sequence across calls) and collect every payload reported.
    fn scan_byte_by_byte(bytes: &[u8]) -> Vec<String> {
        let mut scanner = Osc52Scanner::default();
        let mut out = Vec::new();
        for &b in bytes {
            scanner.feed(&[b], |text| out.push(text));
        }
        out
    }

    fn osc52_bel(selection: &str, text: &str) -> Vec<u8> {
        let encoded = Base64.encode(text);
        format!("\x1b]52;{selection};{encoded}\x07").into_bytes()
    }

    fn osc52_st(selection: &str, text: &str) -> Vec<u8> {
        let encoded = Base64.encode(text);
        format!("\x1b]52;{selection};{encoded}\x1b\\").into_bytes()
    }

    #[test]
    fn bel_terminated_sequence_decodes_in_one_chunk() {
        let bytes = osc52_bel("c", "hello");
        assert_eq!(scan_whole(&bytes), vec!["hello".to_string()]);
    }

    #[test]
    fn st_terminated_sequence_decodes() {
        let bytes = osc52_st("c", "hello via ST");
        assert_eq!(scan_whole(&bytes), vec!["hello via ST".to_string()]);
    }

    #[test]
    fn empty_selection_is_accepted_same_as_c() {
        let bytes = osc52_bel("", "no selection char");
        assert_eq!(scan_whole(&bytes), vec!["no selection char".to_string()]);
    }

    #[test]
    fn split_byte_by_byte_across_many_feed_calls_still_decodes() {
        let bytes = osc52_bel("c", "split across every byte");
        assert_eq!(
            scan_byte_by_byte(&bytes),
            vec!["split across every byte".to_string()]
        );
    }

    #[test]
    fn split_mid_sequence_across_two_feed_calls_still_decodes() {
        let bytes = osc52_bel("c", "split in half");
        let mid = bytes.len() / 2;
        let mut scanner = Osc52Scanner::default();
        let mut out = Vec::new();
        scanner.feed(&bytes[..mid], |text| out.push(text));
        assert!(
            out.is_empty(),
            "should not fire before the terminator arrives"
        );
        scanner.feed(&bytes[mid..], |text| out.push(text));
        assert_eq!(out, vec!["split in half".to_string()]);
    }

    #[test]
    fn other_osc_sequences_are_ignored_but_do_not_break_state() {
        // OSC 0 (title) immediately followed by a real OSC 52 -- the title
        // sequence must produce no callback, and must not desync the scanner
        // for the OSC 52 that follows it.
        let mut bytes = b"\x1b]0;My Title\x07".to_vec();
        bytes.extend(osc52_bel("c", "after a title"));
        assert_eq!(scan_whole(&bytes), vec!["after a title".to_string()]);
    }

    #[test]
    fn non_c_selection_is_ignored() {
        let bytes = osc52_bel("p", "primary selection, not clipboard");
        assert!(scan_whole(&bytes).is_empty());
    }

    #[test]
    fn clipboard_read_request_is_never_reported() {
        let bytes = b"\x1b]52;c;?\x07".to_vec();
        assert!(scan_whole(&bytes).is_empty());
    }

    #[test]
    fn read_request_does_not_desync_a_following_set() {
        let mut bytes = b"\x1b]52;c;?\x07".to_vec();
        bytes.extend(osc52_bel("c", "a real set after a read request"));
        assert_eq!(
            scan_whole(&bytes),
            vec!["a real set after a read request".to_string()]
        );
    }

    #[test]
    fn invalid_base64_payload_is_ignored() {
        let bytes = b"\x1b]52;c;not valid base64 !!\x07".to_vec();
        assert!(scan_whole(&bytes).is_empty());
    }

    #[test]
    fn oversized_payload_is_discarded_and_scanner_recovers() {
        let huge = "x".repeat(MAX_PAYLOAD_LEN * 2);
        let mut bytes = osc52_bel("c", &huge);
        // Followed by a normal, small OSC 52 -- proves the scanner
        // resynchronizes after discarding the oversized one rather than
        // getting stuck.
        bytes.extend(osc52_bel("c", "small payload after overflow"));
        assert_eq!(
            scan_whole(&bytes),
            vec!["small payload after overflow".to_string()]
        );
    }

    #[test]
    fn multiple_sequences_in_one_chunk_report_in_order() {
        let mut bytes = osc52_bel("c", "first");
        bytes.extend(osc52_bel("c", "second"));
        assert_eq!(
            scan_whole(&bytes),
            vec!["first".to_string(), "second".to_string()]
        );
    }

    #[test]
    fn japanese_utf8_payload_round_trips() {
        let bytes = osc52_bel("c", "こんにちは、世界");
        assert_eq!(scan_whole(&bytes), vec!["こんにちは、世界".to_string()]);
    }

    #[test]
    fn unterminated_osc_followed_by_fresh_esc_bracket_still_parses() {
        // "ESC ] 0 ; bogus" with no terminator at all, then a stray ESC that
        // is *not* followed by `\` (so not a valid ST) but instead starts a
        // brand new `ESC ]` sequence -- real terminals treat the bare ESC as
        // aborting the first OSC, and the new introducer should be
        // recognized cleanly.
        let mut bytes = b"\x1b]0;bogus, never terminated".to_vec();
        bytes.push(ESC); // aborts the OSC 0 above (not `\`, so not ST)
        bytes.extend(osc52_bel("c", "recovered after abort"));
        assert_eq!(
            scan_whole(&bytes),
            vec!["recovered after abort".to_string()]
        );
    }

    #[test]
    fn plain_text_and_unrelated_csi_sequences_produce_no_callbacks() {
        let bytes = b"hello \x1b[31mred\x1b[0m world\r\n".to_vec();
        assert!(scan_whole(&bytes).is_empty());
    }
}
