//! Paints one `labolabo_term::GridSnapshot` into a gpui canvas.
//!
//! Pure rendering -- no session/tab-model state lives here, just "snapshot
//! and [`RenderSpec`] in, `Window` paint calls out". Per-cell `shape_line`
//! granularity (one text-shaping call per non-blank cell) mirrors the
//! `gpui-term-poc` spike's approach (labolabo-spikes) rather than the more
//! idiomatic whole-row batching gpui supports (grouping same-style runs of
//! cells into one `shape_line` call per row). That's fine for this wave's
//! skeleton; the spike's own README flags row-batching as the natural
//! follow-up if per-cell shaping ever shows up as a bottleneck -- not
//! attempted here to keep this first pass small.
//!
//! Unlike the spike (which had to hand-roll an ANSI-color-table ->
//! `gpui::Hsla` mapping, `palette.rs`), `labolabo_term::GridSnapshot`
//! already carries fully-resolved `Rgb` per cell -- no palette table needed
//! here at all.

use gpui::{
    fill, font, point, px, size, App, Bounds, Font, FontStyle, FontWeight, Hsla, Pixels,
    SharedString, TextRun, UnderlineStyle, Window,
};
use labolabo_term::{CursorSnapshot, GridSnapshot, Rgb};

/// The fallback font family when the user's `font-family` (or an empty
/// config) can't be resolved. Menlo ships with macOS; on Linux, gpui's own
/// fallback font stack kicks in at shape time if Menlo is missing too.
const FALLBACK_FONT_FAMILY: &str = "Menlo";

/// Everything the renderer (and the grid-size math) needs to know about the
/// resolved terminal font: the font itself, its point size, and the
/// *measured* cell box.
#[derive(Clone, Debug)]
pub struct RenderSpec {
    pub font: Font,
    pub font_size: f32,
    /// Measured advance width of one monospace cell, in pixels
    /// (ceil-rounded so adjacent cell backgrounds tile without hairline
    /// gaps).
    pub cell_width: f32,
    /// Measured line height (ascent + descent) of one cell, in pixels
    /// (ceil-rounded, same reason).
    pub cell_height: f32,
}

impl RenderSpec {
    /// Resolve the font from the user's Ghostty `font-family` list (first
    /// available family wins, mirroring "primary family" -- Ghostty's
    /// deeper per-glyph fallback across the *rest* of the list is out of
    /// scope here) and **measure** the cell box with gpui's text system:
    /// shape a reference glyph ("M") and take its advance width and its
    /// line ascent + descent.
    ///
    /// Availability is a case-insensitive match against the platform's
    /// installed family names (`TextSystem::all_font_names` -- gpui 0.2's
    /// only public "does this font exist" signal; `font_id` is private).
    /// This is stricter than Ghostty's own fuzzy font discovery, so a
    /// family Ghostty finds under a slightly different name can fall back
    /// to Menlo here (with a stderr warning saying so).
    ///
    /// The measurement assumes a monospace font (as every terminal does); a
    /// proportional font will render misaligned, same as in any terminal
    /// pointed at one.
    ///
    /// `font_size` is Ghostty's `font-size` in points; on macOS gpui's
    /// logical pixels coincide with AppKit points, so it is used as
    /// `px(font_size)` directly (the platform scale factor is applied by
    /// gpui at raster time). Non-finite or sub-1pt sizes fall back to
    /// Ghostty's own default. Note: gpui 0.2 exposes no public line-gap
    /// metric, so `cell_height` is ascent + descent -- Ghostty itself
    /// additionally adds the font's line gap, so rows here can be slightly
    /// tighter than Ghostty.app's for fonts with a non-zero line gap.
    pub fn resolve(families: &[String], font_size: f32, window: &mut Window) -> Self {
        let font_size = if font_size.is_finite() && font_size >= 1.0 {
            font_size
        } else {
            crate::ghostty_config::default_font_size()
        };

        let installed = window.text_system().all_font_names();
        let mut resolved: Option<Font> = None;
        for family in families {
            let available = installed
                .iter()
                .any(|name| name.eq_ignore_ascii_case(family));
            if available {
                resolved = Some(font(family.clone()));
                break;
            }
            eprintln!(
                "labolabo-app: ghostty font-family \"{family}\" not found; trying next candidate"
            );
        }
        let font_obj = resolved.unwrap_or_else(|| {
            if !families.is_empty() {
                eprintln!(
                    "labolabo-app: no configured font-family could be resolved; \
                     falling back to {FALLBACK_FONT_FAMILY}"
                );
            }
            font(FALLBACK_FONT_FAMILY)
        });

        // Measure one cell by shaping a reference glyph. "M" is the
        // conventional reference; in a monospace font every glyph shares
        // the same advance anyway.
        let text = SharedString::from("M");
        let run = TextRun {
            len: text.len(),
            font: font_obj.clone(),
            color: gpui::white(),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let shaped = window
            .text_system()
            .shape_line(text, px(font_size), &[run], None);
        let cell_width = f32::from(shaped.width).ceil().max(1.0);
        let cell_height = (f32::from(shaped.ascent) + f32::from(shaped.descent))
            .ceil()
            .max(1.0);

        Self {
            font: font_obj,
            font_size,
            cell_width,
            cell_height,
        }
    }
}

fn to_hsla(color: Rgb) -> Hsla {
    gpui::rgb(((color.r as u32) << 16) | ((color.g as u32) << 8) | (color.b as u32)).into()
}

/// Paint `snapshot`'s grid within `bounds`: the base background first, then
/// each cell's own background (only where it differs -- see
/// `CellSnapshot::has_bg`), then non-blank glyphs, then a cursor overlay.
pub fn paint_grid(
    snapshot: &GridSnapshot,
    spec: &RenderSpec,
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
        let x = bounds.origin.x + px(col * spec.cell_width);
        let y = bounds.origin.y + px(row * spec.cell_height);

        if cell.has_bg {
            window.paint_quad(fill(
                Bounds::new(point(x, y), size(px(spec.cell_width), px(spec.cell_height))),
                to_hsla(cell.bg),
            ));
        }

        if cell.text.is_empty() || cell.text == " " {
            continue;
        }

        let text = SharedString::from(cell.text.clone());
        let mut cell_font = spec.font.clone();
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
            .shape_line(text, px(spec.font_size), &[run], None);
        let _ = shaped.paint(point(x, y), px(spec.cell_height), window, cx);
    }

    paint_cursor(&snapshot.cursor, spec, bounds, window);
}

/// A translucent block-cursor overlay. TODO(W5a): no caret-style selection
/// (block vs. bar vs. underline) or blink -- future work once a real
/// UI/config layer exists to drive it.
fn paint_cursor(
    cursor: &CursorSnapshot,
    spec: &RenderSpec,
    bounds: Bounds<Pixels>,
    window: &mut Window,
) {
    if !cursor.visible {
        return;
    }
    let x = bounds.origin.x + px(cursor.col as f32 * spec.cell_width);
    let y = bounds.origin.y + px(cursor.row as f32 * spec.cell_height);
    window.paint_quad(fill(
        Bounds::new(point(x, y), size(px(spec.cell_width), px(spec.cell_height))),
        gpui::hsla(0.0, 0.0, 1.0, 0.35),
    ));
}
