//! `libghostty-vt`-backed VT core -- **the intended production backend**.
//!
//! This is the real Ghostty VT engine (the same core the app ultimately wants
//! to render), behind the `backend-ghostty-vt` feature. It is not the default
//! only because building it needs a local Ghostty source tree
//! (`GHOSTTY_SOURCE_DIR`) compiled with Zig 0.16 -- a heavyweight, currently
//! fork-pinned external dependency (see the crate README). The alacritty
//! backend exists to keep CI green without that; the goal here is parity.
//!
//! Distilled from the spike's `ghostty_session.rs` (M6): a `Terminal` plus the
//! reusable `RenderState`/`RowIterator`/`CellIterator` buffers, all owned on
//! the worker thread. `libghostty-vt`'s `Terminal` is `!Send`/`!Sync` and its
//! C API requires the caller to serialize all access -- single-thread
//! ownership on the worker satisfies that for free (see `VtBackend`'s doc).
//!
//! The `'static` lifetimes below are sound because `Terminal::new` /
//! `RenderState::new` / the iterators' `new()` each use `libghostty-vt`'s
//! default (owned) allocator rather than a borrowed one, and the
//! `on_pty_write` callback captures only an owned `Arc` (so its callback
//! lifetime is `'static` too). The `RenderState`/iterators hold no borrow of
//! `Terminal` *at rest* -- they are reusable scratch buffers refreshed per
//! `update()` -- so storing them alongside `Terminal` is not self-referential.

use std::io::Write;

use anyhow::anyhow;
use libghostty_vt::render::{CellIterator, RenderState, RowIterator};
use libghostty_vt::style::{RgbColor, Underline};
use libghostty_vt::{Terminal, TerminalOptions};

use crate::backend::VtBackend;
use crate::session::SharedWriter;
use crate::snapshot::{CellSnapshot, CursorSnapshot, GridSnapshot, Rgb};

/// Cell pixel size handed to `Terminal::resize`. Only affects pixel-based VT
/// queries; the grid itself is driven by cols/rows. Matches the spike's
/// nominal cell metrics.
const CELL_WIDTH_PX: u32 = 9;
const CELL_HEIGHT_PX: u32 = 18;

pub struct GhosttyBackend {
    terminal: Terminal<'static, 'static>,
    render_state: RenderState<'static>,
    row_it: RowIterator<'static>,
    cell_it: CellIterator<'static>,
}

impl VtBackend for GhosttyBackend {
    fn new(cols: u16, rows: u16, pty_writer: SharedWriter) -> anyhow::Result<Self> {
        let mut terminal = Terminal::new(TerminalOptions {
            cols,
            rows,
            max_scrollback: 1000,
        })
        .map_err(|e| anyhow!("libghostty-vt Terminal::new failed: {e:?}"))?;

        // VT response callback: replies to device-status/mode queries so
        // programs that probe the terminal on startup don't hang.
        terminal
            .on_pty_write(move |_t, data| {
                if let Ok(mut w) = pty_writer.lock() {
                    let _ = w.write_all(data);
                }
            })
            .map_err(|e| anyhow!("libghostty-vt on_pty_write failed: {e:?}"))?;

        let render_state =
            RenderState::new().map_err(|e| anyhow!("RenderState::new failed: {e:?}"))?;
        let row_it = RowIterator::new().map_err(|e| anyhow!("RowIterator::new failed: {e:?}"))?;
        let cell_it =
            CellIterator::new().map_err(|e| anyhow!("CellIterator::new failed: {e:?}"))?;

        Ok(Self {
            terminal,
            render_state,
            row_it,
            cell_it,
        })
    }

    fn feed(&mut self, bytes: &[u8]) {
        self.terminal.vt_write(bytes);
    }

    fn resize(&mut self, cols: u16, rows: u16) {
        let _ = self
            .terminal
            .resize(cols, rows, CELL_WIDTH_PX, CELL_HEIGHT_PX);
    }

    fn build_snapshot(&mut self) -> Option<GridSnapshot> {
        let snapshot = self.render_state.update(&self.terminal).ok()?;
        let colors = snapshot.colors().ok()?;
        let cols = snapshot.cols().ok()?;
        let rows = snapshot.rows().ok()?;
        let background = rgb(colors.background);
        let default_fg = rgb(colors.foreground);

        // Read cursor state before we borrow the snapshot for row iteration.
        let cursor = {
            let visible = snapshot.cursor_visible().unwrap_or(false);
            let (col, row) = match snapshot.cursor_viewport() {
                Ok(Some(cv)) => (cv.x, cv.y),
                _ => (0, 0),
            };
            CursorSnapshot { col, row, visible }
        };

        let mut cells = Vec::with_capacity(cols as usize * rows as usize);
        let mut row_iteration = self.row_it.update(&snapshot).ok()?;
        let mut text = String::with_capacity(8);

        while let Some(row) = row_iteration.next() {
            let mut cell_iteration = self.cell_it.update(row).ok()?;
            while let Some(cell) = cell_iteration.next() {
                let explicit_bg = cell.bg_color().ok().flatten();

                if cell.graphemes_len().unwrap_or(0) == 0 {
                    cells.push(CellSnapshot {
                        text: String::new(),
                        fg: default_fg,
                        bg: explicit_bg.map(rgb).unwrap_or(background),
                        has_bg: explicit_bg.is_some(),
                        bold: false,
                        italic: false,
                        underline: false,
                        inverse: false,
                    });
                    continue;
                }

                text.clear();
                let _ = cell.graphemes_utf8(&mut text);

                let mut fg = cell
                    .fg_color()
                    .ok()
                    .flatten()
                    .map(rgb)
                    .unwrap_or(default_fg);
                let mut bg = explicit_bg.map(rgb).unwrap_or(background);
                let mut has_bg = explicit_bg.is_some();
                let mut bold = false;
                let mut italic = false;
                let mut underline = false;
                let mut inverse = false;

                if cell.has_styling().unwrap_or(false) {
                    if let Ok(style) = cell.style() {
                        bold = style.bold;
                        italic = style.italic;
                        underline = !matches!(style.underline, Underline::None);
                        inverse = style.inverse;
                        if inverse {
                            std::mem::swap(&mut fg, &mut bg);
                            has_bg = true;
                        }
                    }
                }

                cells.push(CellSnapshot {
                    text: text.clone(),
                    fg,
                    bg,
                    has_bg,
                    bold,
                    italic,
                    underline,
                    inverse,
                });
            }
        }

        Some(GridSnapshot {
            cols,
            rows,
            background,
            cells,
            cursor,
        })
    }
}

fn rgb(c: RgbColor) -> Rgb {
    Rgb::new(c.r, c.g, c.b)
}
