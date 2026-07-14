//! Plain-data, UI-independent render snapshot shared by every backend.
//!
//! Nothing in here touches a backend's FFI/VT types: a `GridSnapshot` is an
//! ordinary owned value (trivially `Send`/`Clone`) that the worker thread
//! extracts once per redraw and hands across a channel to whichever thread
//! renders. That deliberate design (a plain snapshot rather than a shared,
//! externally-locked VT handle) is what lets the two backends expose the
//! *same* rendering surface, and lets rendering live entirely in the future
//! `labolabo-ui` without depending on this crate's backend internals. See
//! the spike's `ghostty_session.rs` module doc for the original rationale.

/// A resolved 8-bit-per-channel RGB color. No palette lookups left to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    pub const BLACK: Rgb = Rgb { r: 0, g: 0, b: 0 };
    /// A light gray -- the fallback default foreground when the VT core
    /// reports the "default foreground" named color.
    pub const DEFAULT_FG: Rgb = Rgb {
        r: 229,
        g: 229,
        b: 229,
    };

    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }
}

/// One cell's worth of already-resolved rendering data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CellSnapshot {
    /// UTF-8 grapheme cluster for this cell; empty means blank. This is a
    /// full grapheme cluster (not a single `char`) so multi-codepoint
    /// graphemes render correctly.
    pub text: String,
    pub fg: Rgb,
    /// Only meaningful when `has_bg` is true -- otherwise the grid's base
    /// `GridSnapshot::background` already covers this cell. When `inverse`
    /// was set, `fg`/`bg` are already swapped here and `has_bg` is forced on.
    pub bg: Rgb,
    pub has_bg: bool,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    /// Whether the source cell had the inverse/reverse-video attribute. The
    /// `fg`/`bg` above already reflect the swap; this is kept for callers
    /// that want to know (e.g. cursor-under-inverse handling).
    pub inverse: bool,
}

impl CellSnapshot {
    /// A blank (empty, default-colored) cell.
    pub fn blank() -> Self {
        Self {
            text: String::new(),
            fg: Rgb::DEFAULT_FG,
            bg: Rgb::BLACK,
            has_bg: false,
            bold: false,
            italic: false,
            underline: false,
            inverse: false,
        }
    }
}

/// Cursor position (0-based, viewport-relative), visibility, and color.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CursorSnapshot {
    pub col: u16,
    pub row: u16,
    pub visible: bool,
    /// The cursor's effective color (the session's configured default --
    /// see `ColorScheme::cursor` -- or a live OSC-12 override, backend
    /// permitting). `None` means no color is configured; callers that paint
    /// a cursor overlay should fall back to their own default in that case.
    pub color: Option<Rgb>,
}

/// A fully-extracted, plain-data render of one terminal grid at some point in
/// time. Cheap to `Arc`-share across threads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GridSnapshot {
    pub cols: u16,
    pub rows: u16,
    pub background: Rgb,
    /// Row-major, `cols * rows` entries.
    pub cells: Vec<CellSnapshot>,
    pub cursor: CursorSnapshot,
}

impl GridSnapshot {
    /// A blank grid to render before the first real snapshot arrives.
    pub fn blank(cols: u16, rows: u16) -> Self {
        Self {
            cols,
            rows,
            background: Rgb::BLACK,
            cells: vec![CellSnapshot::blank(); cols as usize * rows as usize],
            cursor: CursorSnapshot {
                col: 0,
                row: 0,
                visible: true,
                color: None,
            },
        }
    }

    /// The visible text of one row (0-based), with blank cells rendered as
    /// spaces and trailing whitespace trimmed. Out-of-range rows yield "".
    pub fn row_text(&self, row: u16) -> String {
        if row >= self.rows {
            return String::new();
        }
        let cols = self.cols as usize;
        let start = row as usize * cols;
        let end = start + cols;
        let mut out = String::new();
        for cell in &self.cells[start..end.min(self.cells.len())] {
            if cell.text.is_empty() {
                out.push(' ');
            } else {
                out.push_str(&cell.text);
            }
        }
        out.trim_end().to_string()
    }

    /// The whole grid rendered as text (`row_text` for every row, joined by
    /// newlines). Handy for asserting that some output reached the screen.
    pub fn to_text(&self) -> String {
        (0..self.rows)
            .map(|r| self.row_text(r))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Whether `needle` appears anywhere in the rendered grid text.
    pub fn contains_text(&self, needle: &str) -> bool {
        self.to_text().contains(needle)
    }
}
