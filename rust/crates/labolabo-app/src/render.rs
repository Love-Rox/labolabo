//! Paints one `labolabo_term::GridSnapshot` into a gpui canvas.
//!
//! Pure rendering -- no session/tab-model state lives here, just "snapshot
//! in, `Window` paint calls out". Per-cell `shape_line` granularity (one
//! text-shaping call per non-blank cell) mirrors the `gpui-term-poc` spike's
//! approach (labolabo-spikes) rather than the more idiomatic whole-row
//! batching gpui supports (grouping same-style runs of cells into one
//! `shape_line` call per row). That's fine for this wave's skeleton; the
//! spike's own README flags row-batching as the natural follow-up if
//! per-cell shaping ever shows up as a bottleneck -- not attempted here to
//! keep this first pass small.
//!
//! Unlike the spike (which had to hand-roll an ANSI-color-table ->
//! `gpui::Hsla` mapping, `palette.rs`), `labolabo_term::GridSnapshot`
//! already carries fully-resolved `Rgb` per cell -- no palette table needed
//! here at all.

use gpui::{
    fill, font, point, px, size, App, Bounds, FontStyle, FontWeight, Hsla, Pixels, SharedString,
    TextRun, UnderlineStyle, Window,
};
use labolabo_term::{CursorSnapshot, GridSnapshot, Rgb};

/// Cell width in pixels, matching the `gpui-term-poc` spike's 14pt-Menlo
/// measurement (labolabo-spikes `src/main.rs`). TODO(W5a): hardcoded rather
/// than measured from real font metrics -- fine for a fixed monospace font
/// at a fixed size, revisit if/when font size becomes configurable.
pub const CELL_WIDTH: f32 = 9.0;
/// Cell height in pixels -- see [`CELL_WIDTH`]'s doc comment.
pub const CELL_HEIGHT: f32 = 18.0;
const FONT_SIZE: f32 = 14.0;
const FONT_FAMILY: &str = "Menlo";

fn to_hsla(color: Rgb) -> Hsla {
    gpui::rgb(((color.r as u32) << 16) | ((color.g as u32) << 8) | (color.b as u32)).into()
}

/// Paint `snapshot`'s grid within `bounds`: the base background first, then
/// each cell's own background (only where it differs -- see
/// `CellSnapshot::has_bg`), then non-blank glyphs, then a cursor overlay.
pub fn paint_grid(
    snapshot: &GridSnapshot,
    bounds: Bounds<Pixels>,
    window: &mut Window,
    cx: &mut App,
) {
    window.paint_quad(fill(bounds, to_hsla(snapshot.background)));

    let cols = snapshot.cols as usize;
    if cols == 0 {
        return;
    }

    for (index, cell) in snapshot.cells.iter().enumerate() {
        let row = (index / cols) as f32;
        let col = (index % cols) as f32;
        let x = bounds.origin.x + px(col * CELL_WIDTH);
        let y = bounds.origin.y + px(row * CELL_HEIGHT);

        if cell.has_bg {
            window.paint_quad(fill(
                Bounds::new(point(x, y), size(px(CELL_WIDTH), px(CELL_HEIGHT))),
                to_hsla(cell.bg),
            ));
        }

        if cell.text.is_empty() || cell.text == " " {
            continue;
        }

        let text = SharedString::from(cell.text.clone());
        let mut cell_font = font(FONT_FAMILY);
        if cell.bold {
            cell_font.weight = FontWeight::BOLD;
        }
        if cell.italic {
            cell_font.style = FontStyle::Italic;
        }
        let run = TextRun {
            len: text.len(),
            font: cell_font,
            color: to_hsla(cell.fg),
            background_color: None,
            underline: cell.underline.then(|| UnderlineStyle {
                thickness: px(1.0),
                color: None,
                wavy: false,
            }),
            strikethrough: None,
        };
        let shaped = window
            .text_system()
            .shape_line(text, px(FONT_SIZE), &[run], None);
        let _ = shaped.paint(point(x, y), px(CELL_HEIGHT), window, cx);
    }

    paint_cursor(&snapshot.cursor, bounds, window);
}

/// A translucent block-cursor overlay. TODO(W5a): no caret-style selection
/// (block vs. bar vs. underline) or blink -- future work once a real
/// UI/config layer exists to drive it.
fn paint_cursor(cursor: &CursorSnapshot, bounds: Bounds<Pixels>, window: &mut Window) {
    if !cursor.visible {
        return;
    }
    let x = bounds.origin.x + px(cursor.col as f32 * CELL_WIDTH);
    let y = bounds.origin.y + px(cursor.row as f32 * CELL_HEIGHT);
    window.paint_quad(fill(
        Bounds::new(point(x, y), size(px(CELL_WIDTH), px(CELL_HEIGHT))),
        gpui::hsla(0.0, 0.0, 1.0, 0.35),
    ));
}
