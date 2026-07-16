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
use std::sync::{Arc, Mutex};

use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Config, Term, TermMode};
use alacritty_terminal::vte::ansi::{
    Color as AnsiColor, CursorShape, NamedColor, Processor, Rgb as AnsiRgb,
};

use crate::backend::VtBackend;
use crate::color::ColorScheme;
use crate::mouse::{MouseMode, MouseTracking};
use crate::session::SharedWriter;
use crate::snapshot::{CellSnapshot, CursorSnapshot, GridSnapshot, Rgb};

/// Forwards `Term`'s VT-response events to the PTY writer, and mirrors its
/// title-change events into a shared slot `AlacrittyBackend::title` reads
/// back (see that method's doc comment for why: `Term` has no public title
/// getter, unlike `libghostty-vt`'s `Terminal::title()`).
///
/// `send_event` is called synchronously from `Term`'s VT handler while a byte
/// batch is being parsed (on the worker thread). It must never block --
/// writing to the mutex-guarded PTY writer / title slot is fine.
#[derive(Clone)]
struct PtyResponder {
    writer: SharedWriter,
    title: Arc<Mutex<Option<String>>>,
}

impl EventListener for PtyResponder {
    fn send_event(&self, event: Event) {
        match event {
            Event::PtyWrite(text) => {
                if let Ok(mut w) = self.writer.lock() {
                    let _ = w.write_all(text.as_bytes());
                }
            }
            Event::Title(title) => {
                if let Ok(mut slot) = self.title.lock() {
                    *slot = Some(title);
                }
            }
            // OSC sets an *empty* title (or a title pop restoring `None` off
            // the stack, see `Term::pop_title`) -- `Term`'s own convention
            // (`set_title`) is to fire `ResetTitle` rather than
            // `Event::Title(String::new())` for either case.
            Event::ResetTitle => {
                if let Ok(mut slot) = self.title.lock() {
                    *slot = None;
                }
            }
            _ => {}
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
    // Resolved coloring: the built-in ANSI_16 + xterm cube/grayscale table
    // with any `ColorScheme::palette` overrides already applied (see
    // `base_palette`/`ColorScheme::apply_palette`), plus the configured
    // default fg/bg/cursor (falling back to the previous hardcoded
    // defaults when unset). Alacritty's `Term` has no notion of a
    // caller-supplied default-color theme -- unlike libghostty-vt, which
    // tracks this natively -- so this backend resolves every `AnsiColor`
    // through these fields itself, replacing what used to be free
    // functions over hardcoded constants.
    palette: [Rgb; 256],
    default_fg: Rgb,
    default_bg: Rgb,
    // `None` (the default) preserves the pre-ColorScheme behavior: the
    // `NamedColor::Cursor` VT color and `CursorSnapshot::color` both fall
    // back to `default_fg`, and no color is reported for the rendered
    // cursor overlay either. Alacritty's `Term` doesn't track a live OSC-12
    // cursor-color override, so unlike fg/bg this is always exactly the
    // configured default, never a per-session override.
    default_cursor: Option<Rgb>,
    // Mirrors `PtyResponder::title`'s slot (same `Arc`, cloned at
    // construction) -- see `VtBackend::title`'s doc comment for why this
    // backend needs its own event-driven cache rather than a direct getter.
    title: Arc<Mutex<Option<String>>>,
}

impl VtBackend for AlacrittyBackend {
    fn new(
        cols: u16,
        rows: u16,
        pty_writer: SharedWriter,
        colors: &ColorScheme,
        max_scrollback: usize,
    ) -> anyhow::Result<Self> {
        // The spike's M3 finding motivated the original `1000` default
        // (alacritty's own `10_000` default measurably hurt steady-state
        // throughput; we don't render scrollback here anyway) -- now
        // caller-configurable (`labolabo-app`'s settings screen), still
        // defaulting to `1000` (`crate::session::DEFAULT_MAX_SCROLLBACK`).
        let config = Config {
            scrolling_history: max_scrollback,
            ..Config::default()
        };
        let size = GridSize {
            columns: cols as usize,
            screen_lines: rows as usize,
        };
        let title = Arc::new(Mutex::new(None));
        let term = Term::new(
            config,
            &size,
            PtyResponder {
                writer: pty_writer,
                title: title.clone(),
            },
        );
        let palette = colors.apply_palette(base_palette());
        Ok(Self {
            term,
            parser: Processor::new(),
            cols,
            rows,
            palette,
            default_fg: colors.foreground.unwrap_or(Rgb::DEFAULT_FG),
            default_bg: colors.background.unwrap_or(Rgb::BLACK),
            default_cursor: colors.cursor,
            title,
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
        let background = self.ansi_to_rgb(AnsiColor::Named(NamedColor::Background), false);

        let mut cells = vec![CellSnapshot::blank(); cols as usize * rows as usize];
        let content = self.term.renderable_content();
        let display_offset = content.display_offset;
        let scrollback_len = self.term.grid().history_size();

        let cursor = {
            let c = content.cursor;
            CursorSnapshot {
                col: c.point.column.0 as u16,
                row: c.point.line.0.max(0) as u16,
                visible: !matches!(c.shape, CursorShape::Hidden),
                color: self.default_cursor,
            }
        };

        // `display_iter` yields absolute grid lines in
        // `[-(display_offset), -(display_offset) + rows - 1]` (see
        // `Grid::display_iter`'s doc comment upstream) -- i.e. viewport row
        // 0 is always at `line == -display_offset`, not `line == 0`, once
        // scrolled back. `+ display_offset` re-bases that back to a plain
        // `0..rows` viewport row, so this loop (and the `GridSnapshot` it
        // builds) never has to know about the absolute/scrollback
        // coordinate space at all -- unchanged from before scrolling
        // existed when `display_offset == 0`.
        for indexed in content.display_iter {
            let row = indexed.point.line.0 + display_offset as i32;
            if row < 0 {
                continue;
            }
            let row = row as usize;
            let col = indexed.point.column.0;
            if row >= rows as usize || col >= cols as usize {
                continue;
            }
            let cell = indexed.cell;
            let idx = row * cols as usize + col;

            let inverse = cell.flags.contains(Flags::INVERSE);
            let mut fg = self.ansi_to_rgb(cell.fg, true);
            let mut bg = self.ansi_to_rgb(cell.bg, false);
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
            scroll_offset: display_offset,
            scrollback_len,
        })
    }

    fn bracketed_paste(&self) -> bool {
        self.term.mode().contains(TermMode::BRACKETED_PASTE)
    }

    fn scroll_display(&mut self, delta_lines: i64) {
        // Alacritty's own `Scroll::Delta` already matches this trait
        // method's sign convention (positive = up/into history) directly --
        // see `VtBackend::scroll_display`'s doc comment -- so this is a
        // straight passthrough, just clamped into `i32`'s range (realistic
        // per-event deltas are at most a few dozen lines; `Grid::
        // scroll_display` itself clamps the *result* into `[0,
        // history_size()]` regardless of how large a delta is passed in).
        let delta = delta_lines.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32;
        self.term.scroll_display(Scroll::Delta(delta));
    }

    fn scroll_to_bottom(&mut self) {
        self.term.scroll_display(Scroll::Bottom);
    }

    fn alt_screen_active(&self) -> bool {
        self.term.mode().contains(TermMode::ALT_SCREEN)
    }

    fn alternate_scroll_active(&self) -> bool {
        self.term.mode().contains(TermMode::ALTERNATE_SCROLL)
    }

    fn mouse_mode(&self) -> MouseMode {
        let mode = self.term.mode();
        // The three tracking bits below are mutually exclusive (confirmed
        // by reading `alacritty_terminal`'s `set_private_mode`: "Mouse
        // protocols are mutually exclusive" -- it clears the whole
        // `TermMode::MOUSE_MODE` group before inserting the newly-set bit),
        // checked most-specific first for the same reasoning as the
        // ghostty backend's `mouse_mode`.
        //
        // DECSET `9` (X10) has no dedicated `TermMode` bit at all: the
        // vendored `vte::ansi::PrivateMode::from(u16)` mapping this crate
        // depends on (confirmed by reading its source) has no
        // `NamedPrivateMode` variant for mode `9`, so `alacritty_terminal`
        // silently ignores it -- this backend can never report
        // `MouseTracking::X10` (see `MouseTracking::X10`'s own doc comment
        // for the same limitation, and `crate::backend::ghostty
        // ::GhosttyBackend::mouse_mode` for the backend that *does* track
        // it).
        let tracking = if mode.contains(TermMode::MOUSE_MOTION) {
            MouseTracking::Any
        } else if mode.contains(TermMode::MOUSE_DRAG) {
            MouseTracking::Button
        } else if mode.contains(TermMode::MOUSE_REPORT_CLICK) {
            MouseTracking::Normal
        } else {
            MouseTracking::Off
        };
        let sgr = mode.contains(TermMode::SGR_MOUSE);
        MouseMode { tracking, sgr }
    }

    fn title(&self) -> Option<String> {
        // Populated by `PtyResponder::send_event` (`Event::Title`/
        // `Event::ResetTitle`) as `Term` parses OSC 0/2 -- see
        // `VtBackend::title`'s doc comment for why this backend needs an
        // event-driven cache rather than a direct getter.
        match self.title.lock() {
            Ok(t) => t.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }
}

impl AlacrittyBackend {
    fn ansi_to_rgb(&self, color: AnsiColor, is_foreground: bool) -> Rgb {
        match color {
            AnsiColor::Spec(rgb) => rgb_to_rgb(rgb),
            AnsiColor::Indexed(index) => self.palette[index as usize],
            AnsiColor::Named(named) => self.named_to_rgb(named, is_foreground),
        }
    }

    fn named_to_rgb(&self, named: NamedColor, is_foreground: bool) -> Rgb {
        match named {
            NamedColor::Foreground | NamedColor::BrightForeground => self.default_fg,
            NamedColor::Background => self.default_bg,
            NamedColor::Cursor => self.default_cursor.unwrap_or(self.default_fg),
            _ => {
                let code = named as usize;
                if code < 16 {
                    self.palette[code]
                } else if is_foreground {
                    self.default_fg
                } else {
                    self.default_bg
                }
            }
        }
    }
}

// --- palette: ANSI color -> Rgb -------------------------------------------
//
// A plain 16-color ANSI table plus the standard xterm 6x6x6 cube and grayscale
// ramp for 256-color mode -- ported from the spike's `palette.rs`. It doesn't
// track any particular terminal theme (that's `ColorScheme::apply_palette`'s
// job, layered on top in `AlacrittyBackend::new`), but is close enough for
// `ls --color` on its own.

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

fn rgb_to_rgb(rgb: AnsiRgb) -> Rgb {
    Rgb::new(rgb.r, rgb.g, rgb.b)
}

/// The built-in 256-color table before any `ColorScheme::palette` overrides:
/// `ANSI_16` for indices 0-15, then the standard xterm 6x6x6 color cube
/// (16-231) and 24-step grayscale ramp (232-255) -- the same formula
/// Ghostty's own built-in default palette uses
/// (`terminal/color.zig`'s `default`), so overlaying a partial user
/// `palette` (e.g. just the base 16) on top of this produces the same
/// result as it would on Ghostty's own default.
fn base_palette() -> [Rgb; 256] {
    let mut table = [Rgb::BLACK; 256];
    for (index, &(r, g, b)) in ANSI_16.iter().enumerate() {
        table[index] = Rgb::new(r, g, b);
    }
    let scale = |v: usize| if v == 0 { 0 } else { (v * 40 + 55) as u8 };
    for cube in 0..216usize {
        let r = cube / 36;
        let g = (cube / 6) % 6;
        let b = cube % 6;
        table[16 + cube] = Rgb::new(scale(r), scale(g), scale(b));
    }
    for gray in 0..24usize {
        let level = (8 + gray * 10).min(255) as u8;
        table[232 + gray] = Rgb::new(level, level, level);
    }
    table
}
