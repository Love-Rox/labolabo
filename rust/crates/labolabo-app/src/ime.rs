//! Pure helpers for IME (input method) composition support: laying out a
//! preedit (marked/composing) string on the terminal grid, and converting
//! between Rust's UTF-8 `String`s and the UTF-16 code-unit ranges gpui's
//! `EntityInputHandler` trait speaks (macOS's `NSTextInputClient`, the
//! protocol it's a thin wrapper over, is UTF-16-native).
//!
//! Neither of these needs an `App`/`Window` runtime, so both are directly
//! unit-testable -- see the tests below. `app::LaboLaboApp`'s
//! `EntityInputHandler` impl and `render::paint_preedit` are the (untested,
//! gpui-dependent) call sites.

use std::ops::Range;

use unicode_width::UnicodeWidthChar;

/// One character of an IME preedit string, laid out at a specific grid
/// column by [`layout_preedit`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PreeditCell {
    pub ch: char,
    /// 0-based column this character's glyph starts at. A wide (e.g. CJK
    /// fullwidth) character occupies this column and the next.
    pub col: u16,
}

/// Lay out `text` (an in-progress IME composition string) starting at the
/// cursor's column `cursor_col` on a `cols`-wide terminal grid, mirroring a
/// real terminal's inline preedit overlay (see the vendored Ghostty
/// source's `renderer/State.zig`, `Preedit.range` -- referenced here as the
/// behavioral spec, not linked against: wide characters occupy two cells,
/// and if the composition would run past the grid's right edge the whole
/// run shifts left just enough to fit, dropping leading characters if the
/// composition itself is wider than the entire grid).
///
/// Returns one [`PreeditCell`] per character actually visible (so a
/// composition wider than the grid yields fewer cells than `text` has
/// characters). Degenerate input (`cols == 0` or empty `text`) yields no
/// cells.
pub fn layout_preedit(text: &str, cursor_col: u16, cols: u16) -> Vec<PreeditCell> {
    if cols == 0 || text.is_empty() {
        return Vec::new();
    }
    let cursor_col = cursor_col.min(cols - 1);

    let widths: Vec<(char, u16)> = text
        .chars()
        .map(|c| (c, UnicodeWidthChar::width(c).unwrap_or(1) as u16))
        .collect();
    let total_width: u16 = widths.iter().map(|(_, w)| *w).sum();
    let available = cols - cursor_col;

    // If the composition is wider than the *entire* grid, drop leading
    // characters (keeping the tail -- the most recently typed part stays
    // visible, matching Ghostty's own edge behavior) until the remainder
    // fits.
    let (start_index, visible_width) = if total_width > cols {
        let mut w = 0u16;
        let mut start = widths.len();
        for (i, &(_, cw)) in widths.iter().enumerate().rev() {
            if w + cw > cols {
                break;
            }
            w += cw;
            start = i;
        }
        (start, w)
    } else {
        (0, total_width)
    };

    // Shift the whole (possibly truncated) run left just enough that it
    // doesn't overflow the right edge.
    let start_col = if visible_width > available {
        cols - visible_width
    } else {
        cursor_col
    };

    let mut cells = Vec::with_capacity(widths.len() - start_index);
    let mut col = start_col;
    for &(ch, w) in &widths[start_index..] {
        cells.push(PreeditCell { ch, col });
        col += w;
    }
    cells
}

/// The length of `s` in UTF-16 code units -- what gpui's
/// `EntityInputHandler` trait measures ranges in (macOS's
/// `NSTextInputClient` protocol it wraps is UTF-16-native).
pub fn utf16_len(s: &str) -> usize {
    s.encode_utf16().count()
}

/// The UTF-8 substring of `s` covered by a UTF-16 code-unit range,
/// clamped to `s`'s actual length. Used by `EntityInputHandler::
/// text_for_range` to answer the platform's "what text is in this range"
/// query against the tracked preedit string.
pub fn utf16_slice(s: &str, range: Range<usize>) -> String {
    let units: Vec<u16> = s.encode_utf16().collect();
    let end = range.end.min(units.len());
    let start = range.start.min(end);
    String::from_utf16_lossy(&units[start..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn narrow_ascii_preedit_starts_at_cursor() {
        let cells = layout_preedit("a", 2, 10);
        assert_eq!(cells, vec![PreeditCell { ch: 'a', col: 2 }]);
    }

    #[test]
    fn wide_character_occupies_its_own_column_but_advances_two() {
        // U+AC00 HANGUL SYLLABLE GA -- a wide character, mirroring Ghostty's
        // own "preedit range covers exact cell width" test.
        let cells = layout_preedit("\u{AC00}", 2, 10);
        assert_eq!(
            cells,
            vec![PreeditCell {
                ch: '\u{AC00}',
                col: 2
            }]
        );
    }

    #[test]
    fn multi_char_ascii_preedit_lays_out_left_to_right() {
        let cells = layout_preedit("ab", 0, 10);
        assert_eq!(
            cells,
            vec![
                PreeditCell { ch: 'a', col: 0 },
                PreeditCell { ch: 'b', col: 1 },
            ]
        );
    }

    #[test]
    fn wide_character_at_the_right_edge_shifts_left_to_fit() {
        // Mirrors Ghostty's "preedit range shifts left at right edge" test:
        // a 2-wide char at the last column (9 of a 10-wide grid) doesn't
        // fit starting there, so it shifts to start one column earlier.
        let cells = layout_preedit("\u{AC00}", 9, 10);
        assert_eq!(
            cells,
            vec![PreeditCell {
                ch: '\u{AC00}',
                col: 8
            }]
        );
    }

    #[test]
    fn composition_wider_than_the_grid_drops_leading_characters() {
        // Three wide (2-cell) characters = 6 cells on a 4-wide grid: only
        // the last two (4 cells) fit, so the first is dropped.
        let text = "\u{3042}\u{3044}\u{3046}"; // あいう
        let cells = layout_preedit(text, 0, 4);
        assert_eq!(
            cells,
            vec![
                PreeditCell {
                    ch: '\u{3044}',
                    col: 0
                },
                PreeditCell {
                    ch: '\u{3046}',
                    col: 2
                },
            ]
        );
    }

    #[test]
    fn zero_width_grid_yields_no_cells() {
        assert_eq!(layout_preedit("a", 0, 0), Vec::new());
    }

    #[test]
    fn empty_text_yields_no_cells() {
        assert_eq!(layout_preedit("", 0, 10), Vec::new());
    }

    #[test]
    fn utf16_len_counts_surrogate_pairs_as_two() {
        assert_eq!(utf16_len("a"), 1);
        assert_eq!(utf16_len("\u{3042}"), 1); // あ: BMP, one unit
        assert_eq!(utf16_len("\u{1F600}"), 2); // 😀: astral, surrogate pair
    }

    #[test]
    fn utf16_slice_round_trips_ascii() {
        assert_eq!(utf16_slice("hello", 1..3), "el");
    }

    #[test]
    fn utf16_slice_handles_bmp_multibyte_characters() {
        assert_eq!(utf16_slice("\u{3042}\u{3044}\u{3046}", 1..2), "\u{3044}");
    }

    #[test]
    fn utf16_slice_clamps_an_out_of_bounds_range() {
        assert_eq!(utf16_slice("hi", 0..100), "hi");
        assert_eq!(utf16_slice("hi", 5..10), "");
    }
}
