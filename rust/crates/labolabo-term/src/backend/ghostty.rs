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
use libghostty_vt::screen::Screen;
use libghostty_vt::style::{RgbColor, Underline};
use libghostty_vt::terminal::{Mode, ScrollViewport};
use libghostty_vt::{Terminal, TerminalOptions};

use crate::backend::VtBackend;
use crate::color::ColorScheme;
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
    fn new(
        cols: u16,
        rows: u16,
        pty_writer: SharedWriter,
        colors: &ColorScheme,
    ) -> anyhow::Result<Self> {
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

        // Seed the VT core's default colors from `colors`.
        //
        // Foreground and background are always set *together*, never as
        // `None` -- libghostty-vt's own `RenderState.update` (`terminal/
        // render.zig`) only resolves the effective bg/fg pair when *both*
        // are set at the Terminal level:
        //
        //     bg_fg: {
        //         // Background/foreground can be unset initially which
        //         // would depend on "default" background/foreground. The
        //         // expected use case of Terminal is that the caller set
        //         // their own configured defaults on load so this doesn't
        //         // happen.
        //         const bg = t.colors.background.get() orelse break :bg_fg;
        //         const fg = t.colors.foreground.get() orelse break :bg_fg;
        //         ...
        //     }
        //
        // Leaving either one `None` makes that block bail *before updating
        // either color*, so a `ColorScheme` that only configures one of the
        // two would otherwise silently apply neither (caught by this crate's
        // own `backend_common` integration tests). We satisfy the "caller
        // sets both" contract by always passing concrete values, falling
        // back to this crate's own default constants -- the same ones the
        // alacritty backend falls back to -- for whichever side is
        // unconfigured, so both backends resolve to an identical default
        // fg/bg regardless of what `colors` does or doesn't set.
        terminal
            .set_default_fg_color(Some(to_rgb_color(
                colors.foreground.unwrap_or(Rgb::DEFAULT_FG),
            )))
            .map_err(|e| anyhow!("libghostty-vt set_default_fg_color failed: {e:?}"))?;
        terminal
            .set_default_bg_color(Some(to_rgb_color(colors.background.unwrap_or(Rgb::BLACK))))
            .map_err(|e| anyhow!("libghostty-vt set_default_bg_color failed: {e:?}"))?;
        // Cursor color has no such pairing requirement (`RenderState.update`
        // reads it unconditionally: `self.colors.cursor = t.colors.cursor.
        // get();`), so `None` is passed straight through and simply leaves
        // it unset, matching libghostty-vt's own documented behavior
        // ("Passing None clears the default, leaving the color unset").
        terminal
            .set_default_cursor_color(colors.cursor.map(to_rgb_color))
            .map_err(|e| anyhow!("libghostty-vt set_default_cursor_color failed: {e:?}"))?;
        if !colors.palette.is_empty() {
            // Start from libghostty-vt's own built-in default (its doc
            // comment recommends exactly this pattern) so unconfigured
            // indices keep their real Ghostty-default color rather than
            // this crate's own approximation of it.
            let base = terminal
                .default_color_palette()
                .map_err(|e| anyhow!("libghostty-vt default_color_palette failed: {e:?}"))?;
            let mut base_rgb = [Rgb::BLACK; 256];
            for (slot, color) in base_rgb.iter_mut().zip(base.iter()) {
                *slot = rgb(*color);
            }
            let resolved = colors.apply_palette(base_rgb);
            let mut resolved_ffi = [RgbColor::default(); 256];
            for (slot, color) in resolved_ffi.iter_mut().zip(resolved.iter()) {
                *slot = to_rgb_color(*color);
            }
            terminal
                .set_default_color_palette(Some(resolved_ffi))
                .map_err(|e| anyhow!("libghostty-vt set_default_color_palette failed: {e:?}"))?;
        }

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

        // `scrollbar()` reports `{ total, offset, len }` in libghostty-vt's
        // own "row space" (its doc comment: `offset` is "into the total
        // area that the viewport is at", `0` = the very top of scrollback --
        // the *opposite* end from this crate's own `GridSnapshot::
        // scroll_offset` convention, which is `0` at the live tail, matching
        // `VtBackend::scroll_display`'s doc comment). Re-based here so
        // nothing above this backend ever has to know libghostty-vt's
        // convention differs from alacritty's:
        //
        //   scrollback_len = total - len        (the max distance from the
        //                                         live tail, i.e. the same
        //                                         quantity alacritty's
        //                                         `Grid::history_size()`
        //                                         reports)
        //   scroll_offset  = scrollback_len - offset
        //
        // `terminal.scrollbar()`'s own doc comment warns it "may be
        // expensive to calculate depending on where the viewport is" --
        // acceptable here since `build_snapshot` is already throttled to
        // `FRAME_INTERVAL` (~60fps) by `session.rs`, not called per PTY
        // byte.
        let (scroll_offset, scrollback_len) = match self.terminal.scrollbar() {
            Ok(bar) => {
                let scrollback_len = (bar.total as usize).saturating_sub(bar.len as usize);
                let scroll_offset = scrollback_len.saturating_sub(bar.offset as usize);
                (scroll_offset, scrollback_len)
            }
            Err(_) => (0, 0),
        };

        // Read cursor state before we borrow the snapshot for row iteration.
        let cursor = {
            let visible = snapshot.cursor_visible().unwrap_or(false);
            let (col, row) = match snapshot.cursor_viewport() {
                Ok(Some(cv)) => (cv.x, cv.y),
                _ => (0, 0),
            };
            // The *effective* cursor color -- our configured default, or a
            // live OSC-12 override if the running program set one -- unlike
            // the alacritty backend, which has no OSC-12 tracking and so
            // always reports the configured default verbatim.
            let color = snapshot.cursor_color().ok().flatten().map(rgb);
            CursorSnapshot {
                col,
                row,
                visible,
                color,
            }
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
            scroll_offset,
            scrollback_len,
        })
    }

    fn bracketed_paste(&self) -> bool {
        self.terminal.mode(Mode::BRACKETED_PASTE).unwrap_or(false)
    }

    fn scroll_display(&mut self, delta_lines: i64) {
        // libghostty-vt's `ScrollViewport::Delta` convention is "up is
        // negative" -- the *opposite* of this trait method's own convention
        // (positive = up/into history, matching alacritty's native
        // behavior; see `VtBackend::scroll_display`'s doc comment) -- so the
        // sign is flipped here, once, and nowhere else needs to know.
        //
        // Clamped to a *symmetric* `isize` range (not simply `isize::MIN
        // ..=isize::MAX`) before negating: negating `isize::MIN` itself
        // would overflow. Realistic per-event deltas are at most a few
        // dozen lines either way, so this bound is never actually reached
        // in practice -- it only guards the theoretical extreme.
        let bound = isize::MAX as i64;
        let delta = delta_lines.clamp(-bound, bound) as isize;
        self.terminal.scroll_viewport(ScrollViewport::Delta(-delta));
    }

    fn scroll_to_bottom(&mut self) {
        self.terminal.scroll_viewport(ScrollViewport::Bottom);
    }

    fn alt_screen_active(&self) -> bool {
        matches!(self.terminal.active_screen(), Ok(Screen::Alternate))
    }
}

fn rgb(c: RgbColor) -> Rgb {
    Rgb::new(c.r, c.g, c.b)
}

fn to_rgb_color(c: Rgb) -> RgbColor {
    RgbColor {
        r: c.r,
        g: c.g,
        b: c.b,
    }
}
