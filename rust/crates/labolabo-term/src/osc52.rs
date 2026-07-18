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

    // MARK: - CAN/SUB and 8-bit C1 handling
    //
    // Real xterm-family terminals treat CAN (0x18) and SUB (0x1A) as
    // aborting whatever OSC/DCS/... string is in progress. This scanner
    // deliberately does *not* special-case either byte (see `State::Osc`'s
    // `step` match: neither appears as a distinct arm, so both are just
    // buffered like any other payload byte, or silently dropped once
    // `OscOverflow` is reached). The tests below -- plus the fuzz/soak test
    // further down -- exist to pin down whether that difference from real
    // VT behavior can ever cost this scanner a *later*, well-formed OSC 52
    // it should have found (the failure mode a stuck/wedged scanner would
    // produce). It cannot: every state this module's `step` function can be
    // in reacts to a fresh `ESC` byte *before* any other per-state check
    // (including the `OscOverflow` length gate), and `esc_transition`
    // always turns a following `]` into a brand new, empty `State::Osc`
    // buffer -- so a literal `ESC ] 52;...` occurring anywhere in the byte
    // stream, no matter what preceded it (including an in-flight CAN/SUB-
    // laden unterminated OSC), is always recognized. These tests encode
    // that invariant as regression coverage; the fuzz test below stress-
    // tests it at scale instead of just these handful of hand-picked
    // shapes.

    #[test]
    fn can_byte_inside_an_unterminated_osc_does_not_block_the_next_real_osc52() {
        // "ESC ] 0 ; <CAN byte, never a terminator>", no BEL/ST at all --
        // then a real OSC 52 starts immediately after. A real xterm would
        // have aborted the OSC 0 the instant it saw CAN; this scanner
        // instead just keeps buffering CAN as an ordinary payload byte
        // until the *next* `ESC` -- which is exactly the `ESC` that starts
        // the real OSC 52 below, so it's still found correctly.
        let mut bytes = b"\x1b]0;".to_vec();
        bytes.push(0x18); // CAN
        bytes.extend_from_slice(b"more bogus title text, still never terminated");
        bytes.extend(osc52_bel("c", "after an embedded CAN"));
        assert_eq!(
            scan_whole(&bytes),
            vec!["after an embedded CAN".to_string()]
        );
    }

    #[test]
    fn sub_byte_inside_an_unterminated_osc_does_not_block_the_next_real_osc52() {
        let mut bytes = b"\x1b]8;;http://example.invalid/".to_vec();
        bytes.push(0x1A); // SUB
        bytes.extend_from_slice(b"still no terminator");
        bytes.extend(osc52_st("c", "after an embedded SUB"));
        assert_eq!(
            scan_whole(&bytes),
            vec!["after an embedded SUB".to_string()]
        );
    }

    #[test]
    fn can_and_sub_bytes_do_not_desync_a_run_of_several_real_osc52s() {
        // A denser version of the two tests above: CAN/SUB scattered both
        // inside and between several genuine OSC 52 sequences, none of
        // which should be lost or corrupted by the stray bytes.
        let mut bytes = Vec::new();
        bytes.push(0x18);
        bytes.extend(osc52_bel("c", "first"));
        bytes.push(0x1A);
        bytes.extend_from_slice(b"\x1b]133;A"); // unterminated shell-integration OSC
        bytes.push(0x18);
        bytes.extend(osc52_st("c", "second"));
        bytes.push(0x1A);
        bytes.push(0x18);
        bytes.extend(osc52_bel("c", "third"));
        assert_eq!(
            scan_whole(&bytes),
            vec![
                "first".to_string(),
                "second".to_string(),
                "third".to_string()
            ]
        );
    }

    #[test]
    fn c1_8bit_osc_introducer_is_ignored_and_does_not_desync_a_following_real_osc52() {
        // 0x9D is the single-byte C1 form of the OSC introducer (`ESC ]`'s
        // 8-bit equivalent). This scanner only recognizes the 7-bit `ESC ]`
        // form, so a bare 0x9D followed by what *looks* like a clipboard-set
        // body must be ignored outright (never fires, and the "52;c;...''
        // text is simply Ground-state noise) -- then a real 7-bit OSC 52
        // right after must still be found.
        let mut bytes = vec![0x9D];
        bytes.extend_from_slice(b"52;c;aGVsbG8=\x07"); // would decode to "hello" if 0x9D counted
        bytes.extend(osc52_bel("c", "after a C1 OSC introducer"));
        let found = scan_whole(&bytes);
        assert_eq!(found, vec!["after a C1 OSC introducer".to_string()]);
    }

    #[test]
    fn c1_8bit_st_terminator_is_ignored_inside_a_payload_and_scanner_recovers() {
        // 0x9C is the single-byte C1 form of ST (`ESC \`'s 8-bit
        // equivalent). Embedded inside an otherwise well-formed OSC 52
        // payload, it must *not* terminate the sequence -- this scanner
        // only recognizes the 7-bit `ESC \` (or bare BEL) forms. The 0x9C
        // byte ends up as literal (invalid-base64) payload content, so this
        // one sequence correctly fails to decode -- but the real BEL later
        // in the same buffered payload still finds it, and the scanner
        // still recovers cleanly for whatever comes after.
        let mut bytes = b"\x1b]52;c;aGVsbG8=".to_vec();
        bytes.push(0x9C); // inert -- not a recognized terminator
        bytes.extend_from_slice(b"aGVsbG8=\x07"); // now-corrupted payload, BEL-terminated
        bytes.extend(osc52_bel("c", "recovered after a C1 ST byte"));
        let found = scan_whole(&bytes);
        assert_eq!(found, vec!["recovered after a C1 ST byte".to_string()]);
    }

    // MARK: - Overflow-boundary self-healing
    //
    // `State::Osc`'s `step` arm checks `byte == ESC` *before* the
    // `buf.len() >= MAX_PAYLOAD_LEN` overflow gate, so an `ESC` byte is
    // never one of the bytes silently discarded by that gate, regardless of
    // how long the in-progress buffer already is. These tests place a real
    // OSC 52 sequence's introducer at (and just around) the exact byte
    // offset where an in-progress non-52 OSC would overflow, to pin that
    // property down empirically rather than relying only on reading the
    // match arms.

    #[test]
    fn real_osc52_starting_exactly_at_the_overflow_boundary_is_still_found() {
        for pad in [
            MAX_PAYLOAD_LEN - 1,
            MAX_PAYLOAD_LEN,
            MAX_PAYLOAD_LEN + 1,
            MAX_PAYLOAD_LEN + 2,
        ] {
            let mut bytes = b"\x1b]0;".to_vec();
            bytes.extend(std::iter::repeat_n(b'x', pad));
            bytes.extend(osc52_bel("c", "found past the overflow boundary"));
            assert_eq!(
                scan_whole(&bytes),
                vec!["found past the overflow boundary".to_string()],
                "pad={pad}"
            );
        }
    }

    // MARK: - Fuzz/soak: megabytes of adversarial noise around known-good
    // markers
    //
    // No `rand` dependency -- a tiny, self-contained deterministic PRNG
    // (fixed seeds) keeps this reproducible across runs/CI machines while
    // still exercising far more shapes than the hand-written tests above.
    mod fuzz {
        use super::*;

        /// A dependency-free xorshift64* PRNG. Not cryptographic -- only
        /// used to generate test fixtures.
        struct Rng(u64);

        impl Rng {
            fn new(seed: u64) -> Self {
                Self(seed | 1) // avoid the all-zero fixed point
            }

            fn next_u64(&mut self) -> u64 {
                let mut x = self.0;
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                self.0 = x;
                x.wrapping_mul(0x2545_F491_4F6C_DD1D)
            }

            fn next_u8(&mut self) -> u8 {
                (self.next_u64() & 0xFF) as u8
            }

            /// Uniform-ish over `[0, bound)`. `0` if `bound == 0`.
            fn below(&mut self, bound: usize) -> usize {
                if bound == 0 {
                    return 0;
                }
                (self.next_u64() % bound as u64) as usize
            }
        }

        fn push_random_bytes(rng: &mut Rng, out: &mut Vec<u8>, n: usize) {
            for _ in 0..n {
                out.push(rng.next_u8());
            }
        }

        /// A non-52 OSC left deliberately unterminated -- exercises the
        /// long-buffering path without ever looking like a real
        /// clipboard-set (`52` is never used as the OSC number here).
        fn push_unterminated_osc(rng: &mut Rng, out: &mut Vec<u8>) {
            const NUMBERS: [&str; 8] = ["0", "1", "2", "4", "7", "8", "9", "133"];
            out.push(ESC);
            out.push(b']');
            out.extend_from_slice(NUMBERS[rng.below(NUMBERS.len())].as_bytes());
            out.push(b';');
            let len = rng.below(64);
            push_random_bytes(rng, out, len);
            // deliberately no terminator
        }

        /// A CSI fragment (`ESC [ <params> <final>`), fully valid and
        /// self-terminating -- ordinary noise a shell prompt or `ls
        /// --color` produces constantly.
        fn push_csi_fragment(rng: &mut Rng, out: &mut Vec<u8>) {
            out.push(ESC);
            out.push(b'[');
            let len = rng.below(6);
            for _ in 0..len {
                out.push(b'0' + (rng.next_u8() % 10));
            }
            const FINALS: &[u8] = b"mHJKABCD";
            out.push(FINALS[rng.below(FINALS.len())]);
        }

        /// A byte that is never `ESC` -- used inside DCS/APC payload
        /// filler below so each helper's own `ESC \` terminator stays
        /// unambiguous (a coincidental embedded `ESC` is exercised plenty
        /// by `push_unterminated_osc` and the raw random-byte filler
        /// instead).
        fn non_esc_byte(rng: &mut Rng) -> u8 {
            let mut b = rng.next_u8();
            while b == ESC {
                b = rng.next_u8();
            }
            b
        }

        /// A DCS fragment (`ESC P ... ESC \`) -- a sequence family this
        /// scanner doesn't special-case at all, collected as Ground-state
        /// noise until its own terminator closes it.
        fn push_dcs_fragment(rng: &mut Rng, out: &mut Vec<u8>) {
            out.push(ESC);
            out.push(b'P');
            let len = rng.below(32);
            for _ in 0..len {
                out.push(non_esc_byte(rng));
            }
            out.push(ESC);
            out.push(b'\\');
        }

        /// An APC fragment (`ESC _ ... ESC \`) -- same shape as DCS, a
        /// different introducer.
        fn push_apc_fragment(rng: &mut Rng, out: &mut Vec<u8>) {
            out.push(ESC);
            out.push(b'_');
            let len = rng.below(32);
            for _ in 0..len {
                out.push(non_esc_byte(rng));
            }
            out.push(ESC);
            out.push(b'\\');
        }

        /// A lone CAN (0x18) or SUB (0x1A) -- see this module's `mod
        /// tests` doc comment block above for why these are deliberately
        /// inert here.
        fn push_can_or_sub(rng: &mut Rng, out: &mut Vec<u8>) {
            out.push(if rng.next_u8() & 1 == 0 { 0x18 } else { 0x1A });
        }

        /// A bare 8-bit C1 OSC (0x9D) or ST (0x9C) byte -- never
        /// recognized by this scanner (only the 7-bit `ESC ]`/`ESC \`
        /// forms are).
        fn push_c1_osc_or_st(rng: &mut Rng, out: &mut Vec<u8>) {
            out.push(if rng.next_u8() & 1 == 0 { 0x9D } else { 0x9C });
        }

        /// A genuine OSC 52 clipboard-set, BEL- or ST-terminated (picked
        /// by `rng`), with a payload unique enough (a marker index plus
        /// Japanese text) that it can never collide with anything the
        /// noise generators above could produce by chance.
        fn push_real_osc52(rng: &mut Rng, out: &mut Vec<u8>, marker: usize) -> String {
            let payload = format!("fuzz-marker-{marker}-日本語ペイロード-☃");
            let bytes = if rng.next_u8() & 1 == 0 {
                osc52_st("c", &payload)
            } else {
                osc52_bel("c", &payload)
            };
            out.extend_from_slice(&bytes);
            payload
        }

        /// Feeds `bytes` to a fresh scanner in randomly sized chunks
        /// (1..=4096 bytes each -- the worst case for a scanner that must
        /// resume mid-sequence across `feed` calls), returning every
        /// payload reported, in order.
        fn scan_in_random_chunks(rng: &mut Rng, bytes: &[u8]) -> Vec<String> {
            let mut scanner = Osc52Scanner::default();
            let mut out = Vec::new();
            let mut pos = 0;
            while pos < bytes.len() {
                let remaining = bytes.len() - pos;
                let chunk = 1 + rng.below(remaining.min(4096));
                scanner.feed(&bytes[pos..pos + chunk], |text| out.push(text));
                pos += chunk;
            }
            out
        }

        /// Builds a stream of `noise_actions` random noise fragments
        /// (random bytes, unterminated OSCs, CSI/DCS/APC fragments,
        /// CAN/SUB, 8-bit C1 OSC/ST) with `marker_count` genuine OSC 52
        /// sequences evenly interleaved throughout -- not all front- or
        /// back-loaded -- returning `(stream, expected_payloads_in_order)`.
        /// Guarantees exactly `marker_count` markers regardless of how the
        /// randomness elsewhere falls (a trailing loop places any that
        /// weren't reached by a checkpoint).
        fn build_fuzz_stream(
            rng: &mut Rng,
            noise_actions: usize,
            marker_count: usize,
        ) -> (Vec<u8>, Vec<String>) {
            let mut out = Vec::new();
            let mut expected = Vec::new();
            let mut placed = 0usize;
            let interval = (noise_actions / marker_count.max(1)).max(1);
            for step in 0..noise_actions {
                match rng.below(7) {
                    0 => {
                        let n = 1 + rng.below(96);
                        push_random_bytes(rng, &mut out, n);
                    }
                    1 => push_unterminated_osc(rng, &mut out),
                    2 => push_csi_fragment(rng, &mut out),
                    3 => push_dcs_fragment(rng, &mut out),
                    4 => push_apc_fragment(rng, &mut out),
                    5 => push_can_or_sub(rng, &mut out),
                    _ => push_c1_osc_or_st(rng, &mut out),
                }
                if placed < marker_count && step % interval == interval - 1 {
                    expected.push(push_real_osc52(rng, &mut out, placed));
                    placed += 1;
                }
            }
            while placed < marker_count {
                expected.push(push_real_osc52(rng, &mut out, placed));
                placed += 1;
            }
            (out, expected)
        }

        #[test]
        fn fuzz_soak_finds_every_embedded_osc52_amid_megabytes_of_adversarial_noise() {
            // Several independent seeds, each generating a several-MiB
            // adversarial stream -- fixed seeds, so this is a reproducible
            // regression test, not a flaky property test. Chunk sizes for
            // feeding the scanner are themselves randomized (see
            // `scan_in_random_chunks`), so a failure here is deterministic
            // for its seed but the exact byte-batching that triggered it
            // varies run to run only if the seed list below changes.
            let mut failures = Vec::new();
            for seed in [0x5EED_0001_u64, 0x5EED_0002, 0x5EED_0003] {
                let mut rng = Rng::new(seed);
                let (stream, expected) = build_fuzz_stream(&mut rng, 150_000, 1_500);
                let found = scan_in_random_chunks(&mut rng, &stream);
                if found != expected {
                    // Report a diff instead of the full (huge) vectors --
                    // first mismatching index plus counts, enough to
                    // diagnose without dumping megabytes of test output.
                    let first_mismatch = found
                        .iter()
                        .zip(expected.iter())
                        .position(|(a, b)| a != b)
                        .unwrap_or(found.len().min(expected.len()));
                    failures.push(format!(
                        "seed {seed:#x}: stream len {} bytes, found {} of {} expected markers, first mismatch at index {first_mismatch}",
                        stream.len(),
                        found.len(),
                        expected.len()
                    ));
                }
            }
            assert!(
                failures.is_empty(),
                "scanner lost/misordered embedded OSC 52 sequences amid adversarial noise:\n{}",
                failures.join("\n")
            );
        }

        #[test]
        fn fuzz_soak_survives_can_sub_heavy_noise_without_losing_markers() {
            // A second run biased heavily toward CAN/SUB and C1 8-bit
            // OSC/ST noise specifically (the two behaviors this scanner
            // deliberately doesn't special-case, per this module's `mod
            // tests` doc comment) -- the general-purpose fuzz above mixes
            // these in at only ~2/7 of noise steps; this run makes them the
            // overwhelming majority, maximizing the chance of surfacing any
            // "next real OSC 52 gets lost" desync specifically attributable
            // to them.
            let mut rng = Rng::new(0xCA5B_0001);
            let mut out = Vec::new();
            let mut expected = Vec::new();
            let noise_actions = 400_000;
            let marker_count = 1_000;
            let interval = noise_actions / marker_count;
            let mut placed = 0usize;
            for step in 0..noise_actions {
                if rng.below(10) < 8 {
                    push_can_or_sub(&mut rng, &mut out);
                    push_c1_osc_or_st(&mut rng, &mut out);
                } else {
                    push_unterminated_osc(&mut rng, &mut out);
                }
                if placed < marker_count && step % interval == interval - 1 {
                    expected.push(push_real_osc52(&mut rng, &mut out, placed));
                    placed += 1;
                }
            }
            while placed < marker_count {
                expected.push(push_real_osc52(&mut rng, &mut out, placed));
                placed += 1;
            }
            let found = scan_in_random_chunks(&mut rng, &out);
            assert_eq!(
                found,
                expected,
                "CAN/SUB/C1-heavy noise (stream len {}) lost or reordered an embedded OSC 52 -- \
                 this would confirm the scanner *can* get stuck on this class of input",
                out.len()
            );
        }
    }
}
