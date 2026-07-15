//! Pure text-selection geometry and cell-range -> string extraction over a
//! pane's terminal grid.
//!
//! No gpui types appear here on purpose -- same rationale as `grid.rs`:
//! mouse-event handling (in `task_workspace.rs`) converts a window-space
//! pixel position into a `(col, row)` cell via `grid::cell_at` first, and
//! only that plain cell coordinate reaches this module, so the actual
//! selection logic is exercisable by a plain `cargo test` with no gpui
//! `Application`/window.
//!
//! ## What "current snapshot" means for a selection
//!
//! A selection's endpoints are recorded as `(row, col)` cell coordinates
//! *within whichever [`GridSnapshot`] was on hand when the mouse moved* --
//! not a scrollback-stable buffer position with its own persistent identity.
//! [`selected_text`] simply re-reads whatever's currently at those
//! coordinates in the snapshot it's given (which, thanks to
//! `VtBackend::scroll_display`, may itself already be a scrolled-back view
//! showing history -- that's what makes "select text you scrolled back to"
//! work at all). The tradeoff: if the view moves (a scroll, or new PTY
//! output arriving) in between two mouse-move events of the *same* drag, or
//! between finishing a drag and pressing Cmd+C, the (row, col) pair is
//! reinterpreted against whatever is at that position in the *next*
//! snapshot -- which can shift what actually ends up highlighted/copied.
//! This is the simplest class of terminal-emulator selection design (no
//! per-line stable identity threaded through resizes/scrolls/new output);
//! a fuller design would tag each buffer line with a stable id and resolve
//! selection endpoints against that instead. Flagged here as a known,
//! accepted limitation for this wave -- see the crate README.

use labolabo_term::{CellSnapshot, GridSnapshot};

/// One terminal-grid cell, 0-based `(row, col)` within a [`GridSnapshot`]'s
/// current viewport (row 0 = the top on-screen row, regardless of
/// `GridSnapshot::scroll_offset` -- same convention the snapshot itself
/// uses).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct CellPos {
    pub row: u16,
    pub col: u16,
}

/// An in-progress or finished character-based text selection: the cell
/// where the drag started (`anchor`) and the cell currently under the mouse
/// (`cursor`). Either endpoint may be "after" the other in reading order --
/// dragging up-and-left from the start point is exactly as valid as
/// dragging down-and-right -- callers needing reading-order bounds use
/// [`Selection::normalized`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Selection {
    pub anchor: CellPos,
    pub cursor: CellPos,
}

impl Selection {
    /// A zero-length selection anchored (and cursored) at `pos` -- what a
    /// fresh mouse-down starts with, before any drag has extended it.
    pub fn at(pos: CellPos) -> Self {
        Self {
            anchor: pos,
            cursor: pos,
        }
    }

    /// Whether this selection covers zero cells (`anchor == cursor`) -- a
    /// plain click with no drag. Callers treat this the same as "no
    /// selection at all" (see `app::LaboLaboApp::finish_selection`): no
    /// highlight painted, [`selected_text`] returns `""`.
    pub fn is_empty(&self) -> bool {
        self.anchor == self.cursor
    }

    /// `(start, end)` in reading order (top-to-bottom, then left-to-right)
    /// -- whichever of `anchor`/`cursor` comes first.
    pub fn normalized(&self) -> (CellPos, CellPos) {
        let a = (self.anchor.row, self.anchor.col);
        let c = (self.cursor.row, self.cursor.col);
        if a <= c {
            (self.anchor, self.cursor)
        } else {
            (self.cursor, self.anchor)
        }
    }

    /// Whether cell `(row, col)` falls within this selection's normalized
    /// range. Character-based (like every desktop terminal's default
    /// selection mode -- not line/box mode), inclusive of both endpoints.
    /// An empty selection contains no cells.
    pub fn contains(&self, row: u16, col: u16) -> bool {
        if self.is_empty() {
            return false;
        }
        let (start, end) = self.normalized();
        if row < start.row || row > end.row {
            return false;
        }
        if start.row == end.row {
            col >= start.col && col <= end.col
        } else if row == start.row {
            col >= start.col
        } else if row == end.row {
            col <= end.col
        } else {
            true
        }
    }
}

/// Extract `selection`'s covered text from `snapshot`: character-based,
/// spanning [`Selection::normalized`]'s row range; each row's trailing
/// blank cells are trimmed (mirroring `GridSnapshot::row_text`'s own
/// convention), and rows are joined with `"\n"`. An empty
/// ([`Selection::is_empty`]) or fully out-of-range selection yields `""`.
///
/// Pure function of plain data -- no gpui, no backend -- so a scrolled-back
/// `snapshot` (whatever `VtBackend::scroll_display` produced) is extracted
/// exactly the same way as a live one; see this module's doc comment for
/// what "current snapshot" means across a scroll/resize mid-drag.
pub fn selected_text(snapshot: &GridSnapshot, selection: &Selection) -> String {
    if selection.is_empty() || snapshot.rows == 0 || snapshot.cols == 0 {
        return String::new();
    }
    let (start, end) = selection.normalized();
    let last_row = snapshot.rows - 1;
    let last_col = snapshot.cols - 1;
    if start.row > last_row {
        return String::new();
    }
    let end_row = end.row.min(last_row);

    let mut lines = Vec::with_capacity((end_row - start.row) as usize + 1);
    for row in start.row..=end_row {
        let col_start = (if row == start.row { start.col } else { 0 }).min(last_col);
        let col_end = (if row == end.row { end.col } else { last_col }).min(last_col);
        lines.push(row_range_text(snapshot, row, col_start, col_end));
    }
    lines.join("\n")
}

/// `row`'s text from `col_start` to `col_end` (both inclusive; callers
/// guarantee `col_start <= col_end < snapshot.cols` and `row <
/// snapshot.rows`), blank cells rendered as spaces and trailing whitespace
/// trimmed -- the same convention as `GridSnapshot::row_text`, just
/// column-bounded.
fn row_range_text(snapshot: &GridSnapshot, row: u16, col_start: u16, col_end: u16) -> String {
    if row >= snapshot.rows || col_start > col_end {
        return String::new();
    }
    let cols = snapshot.cols as usize;
    let row_base = row as usize * cols;
    let start = (row_base + col_start as usize).min(snapshot.cells.len());
    let end = (row_base + col_end as usize + 1).min(snapshot.cells.len());
    let mut out = String::new();
    for cell in &snapshot.cells[start..end] {
        if cell.text.is_empty() {
            out.push(' ');
        } else {
            out.push_str(&cell.text);
        }
    }
    out.trim_end().to_string()
}

// MARK: - double-click word selection / triple-click line selection (W5j #3)

/// Ghostty's default word-boundary character set for double-click word
/// selection: whitespace, quotes, common bracket-pair punctuation, and a
/// handful of separator symbols. Confirmed by reading the vendored Ghostty
/// source's `terminal/selection_codepoints.zig` (`default_word_boundaries`,
/// the built-in default backing `selection-word-chars` when unconfigured):
///
/// ```text
/// pub const default_word_boundaries = [_]u21{
///     0, ' ', '\t', '\'', '"', '│', '`', '|', ':', ';', ',',
///     '(', ')', '[', ']', '{', '}', '<', '>', '$',
/// };
/// ```
///
/// (the `\u{2502}` below is `│`, U+2502 BOX DRAWINGS LIGHT VERTICAL -- the
/// character `tmux`/box-drawn UIs use for pane borders, included so
/// double-clicking a word up against one doesn't pull the border glyph into
/// the selection).
pub const WORD_BOUNDARY_CHARS: &str = "\t '\"\u{2502}`|:;,()[]{}<>$";

/// Whether `cell` counts as a word-boundary cell for [`word_bounds_at`]'s
/// purposes: its text is exactly one [`WORD_BOUNDARY_CHARS`] character, or
/// it's blank. A blank cell is treated the same as a space (itself already
/// in [`WORD_BOUNDARY_CHARS`]) -- mirrors how [`row_range_text`] already
/// renders a blank cell as a space when extracting text, so "boundary-ness"
/// and "text extraction" agree on what a blank cell means.
fn is_boundary_cell(cell: &CellSnapshot) -> bool {
    cell.text.is_empty() || WORD_BOUNDARY_CHARS.contains(cell.text.as_str())
}

/// The word under `pos` in `snapshot`, as a `(start, end)` cell range on
/// `pos.row` -- ported from real Ghostty's own `Screen.selectWord`
/// algorithm (confirmed by reading the vendored source, `terminal/
/// Screen.zig`): classify the clicked cell as "boundary" or "not a
/// boundary" ([`is_boundary_cell`]), then extend left and right while
/// neighboring cells share that same classification. Double-clicking a
/// space (or any [`WORD_BOUNDARY_CHARS`] character) therefore selects the
/// contiguous run of boundary characters it's part of, exactly as
/// double-clicking a word selects the contiguous run of non-boundary
/// characters it's part of -- both are "select the word" from the user's
/// point of view.
///
/// Out-of-range input (`pos` past the snapshot's bounds, or an empty grid)
/// returns `(pos, pos)` -- a harmless zero-length range, same fallback
/// shape [`selected_text`] already uses for an out-of-range selection.
///
/// **Scope**: single-row only -- unlike Ghostty's own algorithm, this does
/// not cross a soft-wrapped line boundary to continue a word onto the next
/// row. A word selection landing exactly at the last/first column of a
/// wrapped line stops there rather than continuing onto the wrapped
/// continuation. Documented, accepted limitation for this wave (see the
/// crate README).
pub fn word_bounds_at(snapshot: &GridSnapshot, pos: CellPos) -> (CellPos, CellPos) {
    if snapshot.cols == 0 || pos.row >= snapshot.rows || pos.col >= snapshot.cols {
        return (pos, pos);
    }
    let cols = snapshot.cols;
    let idx = |col: u16| pos.row as usize * cols as usize + col as usize;
    let Some(start_cell) = snapshot.cells.get(idx(pos.col)) else {
        return (pos, pos);
    };
    let expect_boundary = is_boundary_cell(start_cell);

    let mut start_col = pos.col;
    while start_col > 0 {
        let candidate = start_col - 1;
        let Some(cell) = snapshot.cells.get(idx(candidate)) else {
            break;
        };
        if is_boundary_cell(cell) != expect_boundary {
            break;
        }
        start_col = candidate;
    }

    let mut end_col = pos.col;
    while end_col + 1 < cols {
        let candidate = end_col + 1;
        let Some(cell) = snapshot.cells.get(idx(candidate)) else {
            break;
        };
        if is_boundary_cell(cell) != expect_boundary {
            break;
        }
        end_col = candidate;
    }

    (
        CellPos {
            row: pos.row,
            col: start_col,
        },
        CellPos {
            row: pos.row,
            col: end_col,
        },
    )
}

/// `row`'s content span as a `(start, end)` cell range for triple-click
/// line selection: from column `0` to the row's last non-blank column, or
/// `(0, 0)` if the row is entirely blank. Trimming trailing blanks (rather
/// than always spanning the full grid width) keeps a triple-click from
/// visually highlighting a wall of empty cells out to the right edge on a
/// mostly-empty line -- the same "don't include trailing blanks" instinct
/// [`selected_text`]'s own `row_range_text` already applies when
/// *extracting* text, just also applied to what gets *highlighted*.
///
/// Out-of-range input (`row` past the snapshot's bounds, or an empty grid)
/// returns `(pos, pos)` at column `0` of that row -- the same harmless
/// zero-length-range shape [`word_bounds_at`] uses.
pub fn line_bounds_at(snapshot: &GridSnapshot, row: u16) -> (CellPos, CellPos) {
    let start = CellPos { row, col: 0 };
    if snapshot.cols == 0 || row >= snapshot.rows {
        return (start, start);
    }
    let cols = snapshot.cols;
    let row_base = row as usize * cols as usize;
    let mut last_nonblank: u16 = 0;
    for col in 0..cols {
        if let Some(cell) = snapshot.cells.get(row_base + col as usize) {
            if !cell.text.is_empty() {
                last_nonblank = col;
            }
        }
    }
    (
        start,
        CellPos {
            row,
            col: last_nonblank,
        },
    )
}

/// The [`Selection`] a mouse-down with `click_count` should produce at
/// `pos`: a single click (`click_count <= 1`) is a plain zero-length click
/// (unchanged, pre-existing behavior -- [`crate::app::LaboLaboApp::
/// begin_selection`]'s drag then extends it as before); a double-click
/// (`click_count == 2`) selects the word under `pos` ([`word_bounds_at`]);
/// a triple-click or more (`click_count >= 3`) selects `pos.row`'s whole
/// content span ([`line_bounds_at`]) -- matching every desktop terminal's
/// click-count convention (and macOS's own text-view convention more
/// broadly), and mirroring real Ghostty's own default click-count-to-
/// behavior mapping (`SelectionGesture.default_behaviors`: cell, word,
/// line -- confirmed by reading the vendored source).
///
/// **Scope**: continuing to *drag* after a double/triple-click extends the
/// selection cell-by-cell (this crate's existing `extend_selection`
/// behavior, unchanged), not word-by-word/line-by-line the way Ghostty's
/// own drag gesture does. Documented, accepted simplification for this
/// wave -- the word/line *classification* logic itself
/// ([`word_bounds_at`]/[`line_bounds_at`]) is what's ported and tested;
/// drag-time re-snapping is future work (see the crate README).
pub fn selection_for_click(snapshot: &GridSnapshot, pos: CellPos, click_count: usize) -> Selection {
    match click_count {
        0 | 1 => Selection::at(pos),
        2 => {
            let (start, end) = word_bounds_at(snapshot, pos);
            Selection {
                anchor: start,
                cursor: end,
            }
        }
        _ => {
            let (start, end) = line_bounds_at(snapshot, pos.row);
            Selection {
                anchor: start,
                cursor: end,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use labolabo_term::{CellSnapshot, CursorSnapshot, Rgb};

    /// Build a `GridSnapshot` where each row is exactly `text` (padded with
    /// blank cells to `cols`, truncated if longer) -- enough for these
    /// pure-logic tests without spinning up a real `Terminal`.
    fn snapshot_from_rows(rows: &[&str], cols: u16) -> GridSnapshot {
        let mut cells = Vec::with_capacity(rows.len() * cols as usize);
        for row in rows {
            let chars: Vec<char> = row.chars().collect();
            for i in 0..cols as usize {
                let mut cell = CellSnapshot::blank();
                if let Some(&ch) = chars.get(i) {
                    if ch != ' ' {
                        cell.text = ch.to_string();
                    }
                }
                cells.push(cell);
            }
        }
        GridSnapshot {
            cols,
            rows: rows.len() as u16,
            background: Rgb::BLACK,
            cells,
            cursor: CursorSnapshot {
                col: 0,
                row: 0,
                visible: true,
                color: None,
            },
            scroll_offset: 0,
            scrollback_len: 0,
        }
    }

    #[test]
    fn is_empty_when_anchor_equals_cursor() {
        let pos = CellPos { row: 1, col: 2 };
        assert!(Selection::at(pos).is_empty());
        let dragged = Selection {
            anchor: pos,
            cursor: CellPos { row: 1, col: 3 },
        };
        assert!(!dragged.is_empty());
    }

    #[test]
    fn normalized_orders_regardless_of_drag_direction() {
        let forward = Selection {
            anchor: CellPos { row: 0, col: 0 },
            cursor: CellPos { row: 2, col: 5 },
        };
        let backward = Selection {
            anchor: CellPos { row: 2, col: 5 },
            cursor: CellPos { row: 0, col: 0 },
        };
        assert_eq!(forward.normalized(), backward.normalized());
        assert_eq!(
            forward.normalized(),
            (CellPos { row: 0, col: 0 }, CellPos { row: 2, col: 5 })
        );
    }

    #[test]
    fn contains_single_row_is_column_bounded() {
        let sel = Selection {
            anchor: CellPos { row: 1, col: 2 },
            cursor: CellPos { row: 1, col: 5 },
        };
        assert!(!sel.contains(1, 1));
        assert!(sel.contains(1, 2));
        assert!(sel.contains(1, 5));
        assert!(!sel.contains(1, 6));
        assert!(!sel.contains(0, 3));
        assert!(!sel.contains(2, 3));
    }

    #[test]
    fn contains_multi_row_middle_rows_are_fully_selected() {
        let sel = Selection {
            anchor: CellPos { row: 0, col: 5 },
            cursor: CellPos { row: 2, col: 2 },
        };
        // First row: from col 5 to the end.
        assert!(!sel.contains(0, 4));
        assert!(sel.contains(0, 5));
        assert!(sel.contains(0, 79));
        // Middle row: fully selected, any column.
        assert!(sel.contains(1, 0));
        assert!(sel.contains(1, 79));
        // Last row: from the start up to col 2.
        assert!(sel.contains(2, 0));
        assert!(sel.contains(2, 2));
        assert!(!sel.contains(2, 3));
    }

    #[test]
    fn empty_selection_contains_nothing() {
        let sel = Selection::at(CellPos { row: 3, col: 3 });
        assert!(!sel.contains(3, 3));
    }

    #[test]
    fn selected_text_empty_selection_is_empty_string() {
        let snap = snapshot_from_rows(&["hello world"], 20);
        let sel = Selection::at(CellPos { row: 0, col: 2 });
        assert_eq!(selected_text(&snap, &sel), "");
    }

    #[test]
    fn selected_text_single_row_substring() {
        let snap = snapshot_from_rows(&["hello world"], 20);
        // "hello" -- columns 0..=4.
        let sel = Selection {
            anchor: CellPos { row: 0, col: 0 },
            cursor: CellPos { row: 0, col: 4 },
        };
        assert_eq!(selected_text(&snap, &sel), "hello");
        // "world" -- columns 6..=10.
        let sel = Selection {
            anchor: CellPos { row: 0, col: 6 },
            cursor: CellPos { row: 0, col: 10 },
        };
        assert_eq!(selected_text(&snap, &sel), "world");
    }

    #[test]
    fn selected_text_trims_trailing_blanks_per_row() {
        let snap = snapshot_from_rows(&["hi"], 20);
        let sel = Selection {
            anchor: CellPos { row: 0, col: 0 },
            cursor: CellPos { row: 0, col: 19 },
        };
        assert_eq!(selected_text(&snap, &sel), "hi");
    }

    #[test]
    fn selected_text_multi_row_joins_with_newline() {
        let snap = snapshot_from_rows(&["first line", "second line", "third"], 20);
        let sel = Selection {
            anchor: CellPos { row: 0, col: 6 }, // "line" of "first line"
            cursor: CellPos { row: 2, col: 2 }, // "thi" of "third"
        };
        assert_eq!(selected_text(&snap, &sel), "line\nsecond line\nthi");
    }

    #[test]
    fn selected_text_works_backward_dragged_selection() {
        // Same selection as the multi-row test, but anchor/cursor swapped
        // (dragged from bottom-right up to top-left) -- must extract
        // identically, since `normalized()` handles the direction.
        let snap = snapshot_from_rows(&["first line", "second line", "third"], 20);
        let sel = Selection {
            anchor: CellPos { row: 2, col: 2 },
            cursor: CellPos { row: 0, col: 6 },
        };
        assert_eq!(selected_text(&snap, &sel), "line\nsecond line\nthi");
    }

    #[test]
    fn selected_text_out_of_range_row_clamps_instead_of_panicking() {
        let snap = snapshot_from_rows(&["only row"], 20);
        // `end.row` well past the grid -- must clamp to the last real row,
        // not panic or index out of bounds (e.g. a selection surviving a
        // resize that shrank the grid -- see this module's doc comment).
        let sel = Selection {
            anchor: CellPos { row: 0, col: 0 },
            cursor: CellPos { row: 50, col: 3 },
        };
        assert_eq!(selected_text(&snap, &sel), "only row");
    }

    #[test]
    fn selected_text_start_row_beyond_grid_is_empty() {
        let snap = snapshot_from_rows(&["only row"], 20);
        let sel = Selection {
            anchor: CellPos { row: 10, col: 0 },
            cursor: CellPos { row: 12, col: 3 },
        };
        assert_eq!(selected_text(&snap, &sel), "");
    }

    #[test]
    fn selected_text_out_of_range_column_clamps() {
        let snap = snapshot_from_rows(&["hi"], 20);
        let sel = Selection {
            anchor: CellPos { row: 0, col: 0 },
            cursor: CellPos { row: 0, col: 500 },
        };
        assert_eq!(selected_text(&snap, &sel), "hi");
    }

    // MARK: - word_bounds_at (double-click)

    #[test]
    fn word_bounds_at_selects_the_clicked_word_only() {
        let snap = snapshot_from_rows(&["hello world"], 20);
        // Click inside "world" (columns 6..=10).
        let (start, end) = word_bounds_at(&snap, CellPos { row: 0, col: 8 });
        assert_eq!(start, CellPos { row: 0, col: 6 });
        assert_eq!(end, CellPos { row: 0, col: 10 });
    }

    #[test]
    fn word_bounds_at_click_on_first_or_last_letter_still_spans_the_whole_word() {
        let snap = snapshot_from_rows(&["hello world"], 20);
        let (start, end) = word_bounds_at(&snap, CellPos { row: 0, col: 0 });
        assert_eq!((start.col, end.col), (0, 4));
        let (start, end) = word_bounds_at(&snap, CellPos { row: 0, col: 10 });
        assert_eq!((start.col, end.col), (6, 10));
    }

    #[test]
    fn word_bounds_at_click_on_a_boundary_char_selects_the_boundary_run() {
        // "a::b" -- clicking the first colon selects both colons (the
        // contiguous run of boundary characters), not the letters.
        let snap = snapshot_from_rows(&["a::b"], 10);
        let (start, end) = word_bounds_at(&snap, CellPos { row: 0, col: 1 });
        assert_eq!((start.col, end.col), (1, 2));
    }

    #[test]
    fn word_bounds_at_click_on_blank_cell_selects_the_run_of_blanks() {
        let snap = snapshot_from_rows(&["a   b"], 10);
        let (start, end) = word_bounds_at(&snap, CellPos { row: 0, col: 2 });
        assert_eq!((start.col, end.col), (1, 3));
    }

    #[test]
    fn word_bounds_at_stops_at_default_ghostty_separators() {
        // Each of Ghostty's default `selection-word-chars` should itself
        // act as a one-character-wide boundary between two words either
        // side of it.
        let snap = snapshot_from_rows(&["foo(bar)"], 10);
        let (start, end) = word_bounds_at(&snap, CellPos { row: 0, col: 0 });
        assert_eq!((start.col, end.col), (0, 2), "\"foo\"");
        let (start, end) = word_bounds_at(&snap, CellPos { row: 0, col: 4 });
        assert_eq!((start.col, end.col), (4, 6), "\"bar\"");
    }

    #[test]
    fn word_bounds_at_single_word_fills_the_whole_row() {
        let snap = snapshot_from_rows(&["hello"], 5);
        let (start, end) = word_bounds_at(&snap, CellPos { row: 0, col: 2 });
        assert_eq!((start.col, end.col), (0, 4));
    }

    #[test]
    fn word_bounds_at_out_of_range_position_is_a_harmless_zero_length_range() {
        let snap = snapshot_from_rows(&["hi"], 10);
        let pos = CellPos { row: 5, col: 0 };
        assert_eq!(word_bounds_at(&snap, pos), (pos, pos));
        let pos = CellPos { row: 0, col: 50 };
        assert_eq!(word_bounds_at(&snap, pos), (pos, pos));
    }

    // MARK: - line_bounds_at (triple-click)

    #[test]
    fn line_bounds_at_trims_trailing_blanks_not_the_full_row_width() {
        let snap = snapshot_from_rows(&["hi"], 20);
        let (start, end) = line_bounds_at(&snap, 0);
        assert_eq!(start, CellPos { row: 0, col: 0 });
        assert_eq!(end, CellPos { row: 0, col: 1 });
    }

    #[test]
    fn line_bounds_at_leading_blanks_are_kept_only_trailing_are_trimmed() {
        let snap = snapshot_from_rows(&["  hi"], 20);
        let (start, end) = line_bounds_at(&snap, 0);
        assert_eq!(start.col, 0, "leading blanks stay in the selection");
        assert_eq!(end.col, 3);
    }

    #[test]
    fn line_bounds_at_entirely_blank_row_is_a_single_cell_at_column_zero() {
        let snap = snapshot_from_rows(&[""], 20);
        let (start, end) = line_bounds_at(&snap, 0);
        assert_eq!(start, CellPos { row: 0, col: 0 });
        assert_eq!(end, CellPos { row: 0, col: 0 });
    }

    #[test]
    fn line_bounds_at_out_of_range_row_is_a_harmless_zero_length_range() {
        let snap = snapshot_from_rows(&["hi"], 10);
        let (start, end) = line_bounds_at(&snap, 9);
        assert_eq!(start, CellPos { row: 9, col: 0 });
        assert_eq!(end, CellPos { row: 9, col: 0 });
    }

    // MARK: - selection_for_click (click-count dispatch)

    #[test]
    fn selection_for_click_single_click_is_a_zero_length_selection() {
        let snap = snapshot_from_rows(&["hello world"], 20);
        let pos = CellPos { row: 0, col: 3 };
        let sel = selection_for_click(&snap, pos, 1);
        assert!(sel.is_empty());
        assert_eq!(sel.anchor, pos);
        let sel = selection_for_click(&snap, pos, 0);
        assert!(
            sel.is_empty(),
            "click_count 0 behaves like a plain click too"
        );
    }

    #[test]
    fn selection_for_click_double_click_selects_the_word() {
        let snap = snapshot_from_rows(&["hello world"], 20);
        let sel = selection_for_click(&snap, CellPos { row: 0, col: 8 }, 2);
        assert_eq!(selected_text(&snap, &sel), "world");
    }

    #[test]
    fn selection_for_click_triple_click_selects_the_line() {
        let snap = snapshot_from_rows(&["hello world"], 20);
        let sel = selection_for_click(&snap, CellPos { row: 0, col: 8 }, 3);
        assert_eq!(selected_text(&snap, &sel), "hello world");
    }

    #[test]
    fn selection_for_click_click_count_above_three_still_selects_the_line() {
        let snap = snapshot_from_rows(&["hello world"], 20);
        let sel = selection_for_click(&snap, CellPos { row: 0, col: 8 }, 4);
        assert_eq!(selected_text(&snap, &sel), "hello world");
    }
}
