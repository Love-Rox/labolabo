//! `alacritty_terminal`-backed VT core (default; crates.io-only).
//!
//! Unlike the spike (M1-M5), which drove `alacritty_terminal`'s own
//! `tty::EventLoop` (its bundled PTY + background I/O thread), this backend
//! feeds bytes into the parser *directly* -- `vte::ansi::Processor::advance`
//! into a `Term` -- so the PTY layer is `portable-pty`, shared verbatim with
//! the ghostty backend (see the crate README, "PTY unification"). VT response
//! bytes (device-status reports, ...) surface as `Event::PtyWrite` through the
//! `Term`'s `EventListener`, which we forward to the shared PTY writer.

use std::io::Write;

use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Config, Term};
use alacritty_terminal::vte::ansi::{
    Color as AnsiColor, CursorShape, NamedColor, Processor, Rgb as AnsiRgb,
};

use crate::backend::VtBackend;
use crate::session::SharedWriter;
use crate::snapshot::{CellSnapshot, CursorSnapshot, GridSnapshot, Rgb};

/// Forwards `Term`'s VT-response events to the PTY writer.
///
/// `send_event` is called synchronously from `Term`'s VT handler while a byte
/// batch is being parsed (on the worker thread). It must never block --
/// writing to the mutex-guarded PTY writer is fine.
#[derive(Clone)]
struct PtyResponder {
    writer: SharedWriter,
}

impl EventListener for PtyResponder {
    fn send_event(&self, event: Event) {
        if let Event::PtyWrite(text) = event {
            if let Ok(mut w) = self.writer.lock() {
                let _ = w.write_all(text.as_bytes());
            }
        }
    }
}

/// Minimal `Dimensions` for constructing / resizing a `Term`. (Alacritty only
/// ships a `Dimensions` impl under a test-only module; real embedders define
/// their own, as we do here.)
struct GridSize {
    columns: usize,
    screen_lines: usize,
}

impl Dimensions for GridSize {
    fn total_lines(&self) -> usize {
        self.screen_lines
    }
    fn screen_lines(&self) -> usize {
        self.screen_lines
    }
    fn columns(&self) -> usize {
        self.columns
    }
}

pub struct AlacrittyBackend {
    term: Term<PtyResponder>,
    parser: Processor,
    // Tracked here rather than read back from `Term` so snapshot dimensions are
    // always exactly what we asked for.
    cols: u16,
    rows: u16,
}

impl VtBackend for AlacrittyBackend {
    fn new(cols: u16, rows: u16, pty_writer: SharedWriter) -> anyhow::Result<Self> {
        // `scrolling_history: 1000` mirrors the spike's M3 finding (alacritty's
        // 10_000 default measurably hurt steady-state throughput; we don't
        // render scrollback here anyway).
        let config = Config {
            scrolling_history: 1000,
            ..Config::default()
        };
        let size = GridSize {
            columns: cols as usize,
            screen_lines: rows as usize,
        };
        let term = Term::new(config, &size, PtyResponder { writer: pty_writer });
        Ok(Self {
            term,
            parser: Processor::new(),
            cols,
            rows,
        })
    }

    fn feed(&mut self, bytes: &[u8]) {
        self.parser.advance(&mut self.term, bytes);
    }

    fn resize(&mut self, cols: u16, rows: u16) {
        self.term.resize(GridSize {
            columns: cols as usize,
            screen_lines: rows as usize,
        });
        self.cols = cols;
        self.rows = rows;
    }

    fn build_snapshot(&mut self) -> Option<GridSnapshot> {
        let cols = self.cols;
        let rows = self.rows;
        let background = ansi_to_rgb(AnsiColor::Named(NamedColor::Background), false);

        let mut cells = vec![CellSnapshot::blank(); cols as usize * rows as usize];
        let content = self.term.renderable_content();

        let cursor = {
            let c = content.cursor;
            CursorSnapshot {
                col: c.point.column.0 as u16,
                row: c.point.line.0.max(0) as u16,
                visible: !matches!(c.shape, CursorShape::Hidden),
            }
        };

        for indexed in content.display_iter {
            let line = indexed.point.line.0;
            if line < 0 {
                // Scrollback above the viewport; we only render the live screen.
                continue;
            }
            let row = line as usize;
            let col = indexed.point.column.0;
            if row >= rows as usize || col >= cols as usize {
                continue;
            }
            let cell = indexed.cell;
            let idx = row * cols as usize + col;

            let inverse = cell.flags.contains(Flags::INVERSE);
            let mut fg = ansi_to_rgb(cell.fg, true);
            let mut bg = ansi_to_rgb(cell.bg, false);
            let mut has_bg = bg != background;
            if inverse {
                std::mem::swap(&mut fg, &mut bg);
                has_bg = true;
            }

            let text = if cell.c == ' ' || cell.c == '\0' {
                String::new()
            } else {
                cell.c.to_string()
            };

            cells[idx] = CellSnapshot {
                text,
                fg,
                bg,
                has_bg,
                bold: cell.flags.contains(Flags::BOLD),
                italic: cell.flags.contains(Flags::ITALIC),
                underline: cell.flags.intersects(Flags::ALL_UNDERLINES),
                inverse,
            };
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

// --- palette: ANSI color -> Rgb -------------------------------------------
//
// A plain 16-color ANSI table plus the standard xterm 6x6x6 cube and grayscale
// ramp for 256-color mode -- ported from the spike's `palette.rs`. It doesn't
// track any particular terminal theme, but is close enough for `ls --color`.

const ANSI_16: [(u8, u8, u8); 16] = [
    (0x00, 0x00, 0x00),
    (0xcd, 0x00, 0x00),
    (0x00, 0xcd, 0x00),
    (0xcd, 0xcd, 0x00),
    (0x00, 0x00, 0xee),
    (0xcd, 0x00, 0xcd),
    (0x00, 0xcd, 0xcd),
    (0xe5, 0xe5, 0xe5),
    (0x7f, 0x7f, 0x7f),
    (0xff, 0x00, 0x00),
    (0x00, 0xff, 0x00),
    (0xff, 0xff, 0x00),
    (0x5c, 0x5c, 0xff),
    (0xff, 0x00, 0xff),
    (0x00, 0xff, 0xff),
    (0xff, 0xff, 0xff),
];

fn ansi_to_rgb(color: AnsiColor, is_foreground: bool) -> Rgb {
    match color {
        AnsiColor::Spec(rgb) => rgb_to_rgb(rgb),
        AnsiColor::Indexed(index) => indexed_to_rgb(index),
        AnsiColor::Named(named) => named_to_rgb(named, is_foreground),
    }
}

fn rgb_to_rgb(rgb: AnsiRgb) -> Rgb {
    Rgb::new(rgb.r, rgb.g, rgb.b)
}

fn ansi16(index: usize) -> Rgb {
    let (r, g, b) = ANSI_16[index];
    Rgb::new(r, g, b)
}

fn named_to_rgb(named: NamedColor, is_foreground: bool) -> Rgb {
    match named {
        NamedColor::Foreground | NamedColor::BrightForeground => Rgb::DEFAULT_FG,
        NamedColor::Background => Rgb::BLACK,
        NamedColor::Cursor => Rgb::DEFAULT_FG,
        _ => {
            let code = named as usize;
            if code < 16 {
                ansi16(code)
            } else if is_foreground {
                Rgb::DEFAULT_FG
            } else {
                Rgb::BLACK
            }
        }
    }
}

fn indexed_to_rgb(index: u8) -> Rgb {
    let index = index as usize;
    if index < 16 {
        return ansi16(index);
    }
    if index < 232 {
        let cube = index - 16;
        let r = cube / 36;
        let g = (cube / 6) % 6;
        let b = cube % 6;
        let scale = |v: usize| if v == 0 { 0 } else { (v * 40 + 55) as u8 };
        return Rgb::new(scale(r), scale(g), scale(b));
    }
    let level = 8 + (index - 232) as u16 * 10;
    let level = level.min(255) as u8;
    Rgb::new(level, level, level)
}
