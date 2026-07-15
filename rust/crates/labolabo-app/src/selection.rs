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

use labolabo_term::GridSnapshot;

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
}
