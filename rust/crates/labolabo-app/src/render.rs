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

use crate::ime;
use crate::selection::Selection;
use crate::theme;

/// Selection highlight tint -- the same accent hue as the focused-pane
/// border (`task_workspace::FOCUS_BORDER_COLOR`, [`theme::ACCENT`]) so
/// "this pane is focused" and "this is your selection" read as one visual
/// family, alpha'd down so the glyph painted on top (see [`paint_grid`])
/// stays legible. This is the one place `crate::theme` reaches into
/// `render.rs`'s otherwise-out-of-scope terminal-cell painting (`plans/013`'s
/// module doc comment calls this out as the deliberate exception) --
/// everything else here (cell/cursor/preedit colors) stays exactly what the
/// user's Ghostty config resolves to.
const SELECTION_HIGHLIGHT_RGB: Rgb = Rgb::new(
    ((theme::ACCENT >> 16) & 0xff) as u8,
    ((theme::ACCENT >> 8) & 0xff) as u8,
    (theme::ACCENT & 0xff) as u8,
);
const SELECTION_HIGHLIGHT_ALPHA: f32 = 0.35;

/// The fallback font families (first installed one wins, same availability
/// probe as the user's own `font-family` list -- see [`RenderSpec::resolve`])
/// when the user's `font-family` (or an empty config) can't be resolved.
///
/// Per-OS because there is no cross-platform monospace family name: Menlo
/// ships with macOS but not Linux, and the CSS-style generic `"monospace"`
/// doesn't resolve on Linux either -- gpui's cosmic-text backend matches
/// family names *literally* against the font database's face families
/// (`load_family` in gpui's `platform/linux/text_system.rs`), and
/// "monospace" is a fontconfig *alias*, not a face family. Nor can we lean
/// on gpui's own shape-time fallback stack: its first monospace entry
/// (`.ZedMono` = "Lilex") ships with Zed, not with this app, so on a stock
/// Linux desktop that stack bottoms out in a *proportional* face
/// (Ubuntu/Cantarell/Noto Sans/DejaVu Sans). The candidates below cover the
/// default monospace of every mainstream distro family: DejaVu Sans Mono
/// (Debian/Ubuntu baseline, near-universal), Noto Sans Mono (GNOME/Fedora),
/// Liberation Mono (RHEL/Fedora), Ubuntu Mono (Ubuntu desktop). If none is
/// installed the first candidate is still handed to gpui, whose stack picks
/// *some* renderable face at shape time (text stays readable, grid pitch
/// still comes from our own "M" measurement below).
#[cfg(target_os = "macos")]
const FALLBACK_FONT_FAMILIES: &[&str] = &["Menlo"];
#[cfg(not(target_os = "macos"))]
const FALLBACK_FONT_FAMILIES: &[&str] = &[
    "DejaVu Sans Mono",
    "Noto Sans Mono",
    "Liberation Mono",
    "Ubuntu Mono",
];

/// Everything the renderer (and the grid-size math) needs to know about the
/// resolved terminal font: the font itself, its point size, and the
/// *measured* cell box.
#[derive(Clone, Debug)]
pub struct RenderSpec {
    pub font: Font,
    pub font_size: f32,
    /// Measured advance width of one monospace cell, in logical pixels,
    /// snapped to a whole device pixel (see [`RenderSpec::resolve`] and
    /// [`round_to_device_pixels`]) so adjacent cell backgrounds tile without
    /// hairline gaps.
    pub cell_width: f32,
    /// Measured line height (ascent + descent) of one cell, in logical
    /// pixels, snapped the same way.
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
    /// to [`FALLBACK_FONT_FAMILIES`] here (with a stderr warning saying so).
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
    ///
    /// The measured advance/height are snapped to whole **device** pixels
    /// by rounding, mirroring Ghostty's own metrics derivation
    /// (`src/font/Metrics.zig`: `cell_width = @round(face_width)` in device
    /// pixels, with a comment explaining that rounding tracks the font's
    /// authorial intent better than ceiling). Ceiling in *logical* pixels
    /// -- what this function originally did -- inflates the grid pitch by
    /// up to almost a full logical pixel: MonaspiceKr Nerd Font Mono at
    /// 13pt has an advance of 8.06px, which `ceil` turns into a 9px pitch
    /// (+12% letter spacing, visibly "airy" text -- reported on-device)
    /// where Ghostty renders an 8px pitch (16.12 device px rounds to 16).
    pub fn resolve(families: &[String], font_size: f32, window: &mut Window) -> Self {
        let font_size = if font_size.is_finite() && font_size >= 1.0 {
            font_size
        } else {
            crate::ghostty_config::default_font_size()
        };

        let installed = window.text_system().all_font_names();
        let is_installed = |family: &str| {
            installed
                .iter()
                .any(|name| name.eq_ignore_ascii_case(family))
        };
        let mut resolved: Option<Font> = None;
        for family in families {
            if is_installed(family) {
                resolved = Some(font(family.clone()));
                break;
            }
            eprintln!(
                "labolabo-app: ghostty font-family \"{family}\" not found; trying next candidate"
            );
        }
        let font_obj = resolved.unwrap_or_else(|| {
            // Same installed-check as the user's own list, over the per-OS
            // candidates (see FALLBACK_FONT_FAMILIES' doc comment); an
            // uninstalled first candidate is still a workable last resort
            // (gpui substitutes at shape time).
            let fallback = FALLBACK_FONT_FAMILIES
                .iter()
                .copied()
                .find(|family| is_installed(family))
                .unwrap_or(FALLBACK_FONT_FAMILIES[0]);
            if !families.is_empty() {
                eprintln!(
                    "labolabo-app: no configured font-family could be resolved; \
                     falling back to {fallback}"
                );
            }
            font(fallback)
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
        let scale_factor = window.scale_factor();
        let cell_width = round_to_device_pixels(f32::from(shaped.width), scale_factor);
        let cell_height = round_to_device_pixels(
            f32::from(shaped.ascent) + f32::from(shaped.descent),
            scale_factor,
        );

        Self {
            font: font_obj,
            font_size,
            cell_width,
            cell_height,
        }
    }
}

/// Snap a logical-pixel measurement to the nearest whole **device** pixel
/// (Ghostty's `@round(face_width)` semantics -- see [`RenderSpec::resolve`]'s
/// doc comment), clamped to at least one device pixel so a degenerate
/// measurement can never produce a zero-width/height cell.
fn round_to_device_pixels(logical: f32, scale_factor: f32) -> f32 {
    let scale = if scale_factor.is_finite() && scale_factor > 0.0 {
        scale_factor
    } else {
        1.0
    };
    (logical * scale).round().max(1.0) / scale
}

fn to_hsla(color: Rgb) -> Hsla {
    to_hsla_with_alpha(color, 1.0)
}

fn to_hsla_with_alpha(color: Rgb, alpha: f32) -> Hsla {
    let mut hsla: Hsla =
        gpui::rgb(((color.r as u32) << 16) | ((color.g as u32) << 8) | (color.b as u32)).into();
    hsla.a = alpha;
    hsla
}

/// Paint `snapshot`'s grid within `bounds` in two passes: first **every**
/// cell's background and selection highlight (base background, then per-cell
/// `CellSnapshot::has_bg`, then a translucent highlight over any cell
/// [`Selection::contains`]s -- see `crate::selection`'s module doc comment
/// for what a selection's coordinates mean against a possibly-scrolled
/// `snapshot`), and only then **every** glyph, then a cursor overlay.
///
/// The two-pass split is load-bearing, not style: a glyph may extend past
/// its own cell's rectangle -- a double-width (CJK) glyph spans two cells
/// but is painted once from its leading cell, and even a single-width glyph
/// can overhang by a fraction of a pixel when the font's advance rounds
/// down to the cell pitch. With the old single interleaved loop, the *next*
/// cell's background/selection quad painted **after** that glyph and
/// clipped its right half -- most visibly as "the right half of every
/// selected full-width character disappears" (the wide glyph's trailing
/// spacer cell is part of the selection, so its highlight quad landed on
/// top of the glyph; reported on-device with Japanese text).
///
/// `selection: None` paints no highlight at all -- the common case (most
/// panes have no active selection most of the time).
pub fn paint_grid(
    snapshot: &GridSnapshot,
    spec: &RenderSpec,
    selection: Option<&Selection>,
    bounds: Bounds<Pixels>,
    window: &mut Window,
    cx: &mut App,
) {
    window.paint_quad(fill(bounds, to_hsla(snapshot.background)));

    let cols = snapshot.cols as usize;
    if cols == 0 {
        return;
    }

    // Pass 1: backgrounds + selection highlight, for every cell.
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

        if selection.is_some_and(|s| s.contains(row as u16, col as u16)) {
            window.paint_quad(fill(
                Bounds::new(point(x, y), size(px(spec.cell_width), px(spec.cell_height))),
                to_hsla_with_alpha(SELECTION_HIGHLIGHT_RGB, SELECTION_HIGHLIGHT_ALPHA),
            ));
        }
    }

    // Pass 2: glyphs, over all backgrounds/highlights.
    for (index, cell) in snapshot.cells.iter().enumerate() {
        let row = (index / cols) as f32;
        let col = (index % cols) as f32;
        let x = bounds.origin.x + px(col * spec.cell_width);
        let y = bounds.origin.y + px(row * spec.cell_height);

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

/// A translucent block-cursor overlay, tinted by the session's configured
/// cursor color (`ColorScheme::cursor` -- see `CursorSnapshot::color`'s doc
/// comment) when one is set, at the same alpha as the original hardcoded
/// white so an unconfigured cursor renders exactly as it did before color
/// configuration existed. TODO(W5a): no caret-style selection (block vs.
/// bar vs. underline) or blink -- future work once a real UI/config layer
/// exists to drive it.
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
    const CURSOR_ALPHA: f32 = 0.35;
    let color = cursor
        .color
        .map(|c| to_hsla_with_alpha(c, CURSOR_ALPHA))
        .unwrap_or_else(|| gpui::hsla(0.0, 0.0, 1.0, CURSOR_ALPHA));
    window.paint_quad(fill(
        Bounds::new(point(x, y), size(px(spec.cell_width), px(spec.cell_height))),
        color,
    ));
}

/// Paint an in-progress IME composition (preedit/marked text) inline over
/// the cursor's row, underlined -- the standard terminal convention (see
/// the vendored Ghostty source's `Surface.preeditCallback`/`renderer/
/// State.zig`, referenced as the behavioral spec this mirrors). Column
/// layout (including the wide-character and right-edge-shift handling) is
/// `ime::layout_preedit`'s job -- this function only shapes and paints each
/// laid-out character. A no-op for empty `text` (nothing being composed).
pub fn paint_preedit(
    text: &str,
    cursor: &CursorSnapshot,
    cols: u16,
    spec: &RenderSpec,
    bounds: Bounds<Pixels>,
    window: &mut Window,
    cx: &mut App,
) {
    if text.is_empty() {
        return;
    }
    let y = bounds.origin.y + px(cursor.row as f32 * spec.cell_height);
    let cells = ime::layout_preedit(text, cursor.col, cols);

    // Glyphs first, with no per-run underline: a per-glyph underline only
    // spans that glyph's own advance, which is narrower than the cell pitch
    // (full-width glyphs especially), so the composition underline rendered
    // as one dash per character instead of one continuous line (reported
    // on-device with Japanese IME input). The underline is painted below as
    // explicit quads over each contiguous cell span instead.
    for cell in &cells {
        let x = bounds.origin.x + px(cell.col as f32 * spec.cell_width);
        let text = SharedString::from(cell.ch.to_string());
        let run = TextRun {
            len: text.len(),
            font: spec.font.clone(),
            color: gpui::white(),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let shaped = window
            .text_system()
            .shape_line(text, px(spec.font_size), &[run], None);
        let _ = shaped.paint(point(x, y), px(spec.cell_height), window, cx);
    }

    // One continuous underline quad per contiguous run of preedit cells
    // (layout_preedit can split the preedit when it shifts/wraps at the
    // right edge, so spans are derived from the actual cell columns, each
    // extended by its character's own cell width).
    let underline_y = y + px(spec.cell_height - 2.0);
    for (start_col, end_col) in preedit_underline_spans(&cells) {
        let x = bounds.origin.x + px(start_col as f32 * spec.cell_width);
        let width = px((end_col - start_col) as f32 * spec.cell_width);
        window.paint_quad(fill(
            Bounds::new(point(x, underline_y), size(width, px(1.0))),
            gpui::white(),
        ));
    }
}

/// Collapses laid-out preedit cells into contiguous `[start_col, end_col)`
/// column spans (each cell covering its character's full 1- or 2-column
/// width), so the composition underline can be painted as one unbroken quad
/// per span rather than one dash per glyph.
fn preedit_underline_spans(cells: &[ime::PreeditCell]) -> Vec<(u16, u16)> {
    use unicode_width::UnicodeWidthChar;

    let mut spans: Vec<(u16, u16)> = Vec::new();
    for cell in cells {
        let width = cell.ch.width().unwrap_or(1).max(1) as u16;
        let end = cell.col.saturating_add(width);
        match spans.last_mut() {
            Some((_, last_end)) if *last_end == cell.col => *last_end = end,
            _ => spans.push((cell.col, end)),
        }
    }
    spans
}

#[cfg(test)]
mod tests {
    use super::round_to_device_pixels;

    #[test]
    fn rounds_in_device_pixels_not_logical() {
        // The reported case: MonaspiceKr Nerd Font Mono @13pt = 8.06px
        // advance on a 2x display. 16.12 device px rounds to 16 = 8.0
        // logical -- the old logical-pixel ceil gave 9.0 (+12% pitch).
        assert_eq!(round_to_device_pixels(8.06, 2.0), 8.0);
        // On a 1x display the same measurement rounds to 8.0 as well.
        assert_eq!(round_to_device_pixels(8.06, 1.0), 8.0);
        // A half-device-pixel fraction rounds up: 7.8 * 2 = 15.6 -> 16.
        assert_eq!(round_to_device_pixels(7.8, 2.0), 8.0);
        // ...but a smaller fraction rounds down: 7.7 * 2 = 15.4 -> 15 -> 7.5.
        assert_eq!(round_to_device_pixels(7.7, 2.0), 7.5);
    }

    #[test]
    fn degenerate_inputs_clamp_to_one_device_pixel() {
        assert_eq!(round_to_device_pixels(0.0, 2.0), 0.5);
        assert_eq!(round_to_device_pixels(-3.0, 2.0), 0.5);
    }

    #[test]
    fn bogus_scale_factor_falls_back_to_one() {
        assert_eq!(round_to_device_pixels(8.06, 0.0), 8.0);
        assert_eq!(round_to_device_pixels(8.06, f32::NAN), 8.0);
    }
}

#[cfg(test)]
mod preedit_span_tests {
    use super::preedit_underline_spans;
    use crate::ime::PreeditCell;

    fn cell(ch: char, col: u16) -> PreeditCell {
        PreeditCell { ch, col }
    }

    #[test]
    fn ascii_run_is_one_span() {
        let cells = vec![cell('a', 5), cell('b', 6), cell('c', 7)];
        assert_eq!(preedit_underline_spans(&cells), vec![(5, 8)]);
    }

    #[test]
    fn fullwidth_run_is_one_span_covering_both_columns_per_char() {
        // Each CJK char occupies 2 columns; the span must be continuous.
        let cells = vec![cell('あ', 4), cell('い', 6), cell('う', 8)];
        assert_eq!(preedit_underline_spans(&cells), vec![(4, 10)]);
    }

    #[test]
    fn mixed_width_run_is_one_span() {
        let cells = vec![cell('a', 0), cell('あ', 1), cell('b', 3)];
        assert_eq!(preedit_underline_spans(&cells), vec![(0, 4)]);
    }

    #[test]
    fn discontinuous_cells_split_into_separate_spans() {
        // e.g. layout wrapped/shifted at the right edge.
        let cells = vec![cell('a', 78), cell('b', 79), cell('c', 0)];
        assert_eq!(preedit_underline_spans(&cells), vec![(78, 80), (0, 1)]);
    }

    #[test]
    fn empty_input_produces_no_spans() {
        assert_eq!(preedit_underline_spans(&[]), Vec::<(u16, u16)>::new());
    }
}
