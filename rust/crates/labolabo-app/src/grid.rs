//! Pure geometry: translate a pixel-space window/canvas area into a
//! terminal grid's column/row count, given a fixed cell size.
//!
//! No gpui types appear here on purpose -- both the initial-pane sizing at
//! startup (`app::TerminalApp::viewport_grid_size`) and the per-pane
//! resize-on-layout wiring (`app::render_leaf`'s canvas `prepaint`
//! closure) -> `Terminal::resize` need this math, and keeping it gpui-free
//! makes it unit-testable without spinning up a gpui `Application`/window
//! (which `cargo test` cannot do headlessly in CI).

/// Height in pixels reserved for a pane's tab bar strip, subtracted from its
/// viewport before computing that pane's terminal grid area. Must match
/// `app::render_pane_tab_bar`'s `.h(px(TAB_BAR_HEIGHT))`. Used directly by
/// [`grid_size_for_window`] (the whole-window special case, see its own doc
/// comment); every other pane's own tab bar height is subtracted
/// automatically by gpui's flex layout instead (see `app::render_leaf`'s doc
/// comment).
pub const TAB_BAR_HEIGHT: f32 = 32.0;

/// How many whole `cell_width` x `cell_height` cells fit in an
/// `available_width` x `available_height` pixel area.
///
/// Always at least 1x1 (never 0x0), even for a zero, negative, or
/// sub-cell-size input -- a degenerate measurement (e.g. mid-resize, or a
/// window shorter than the tab bar) must never ask `TermSession::resize` to
/// size a PTY to zero, which `portable-pty` rejects outright.
pub fn grid_size_for_area(
    available_width: f32,
    available_height: f32,
    cell_width: f32,
    cell_height: f32,
) -> (u16, u16) {
    let cols = (available_width / cell_width).floor().max(1.0) as u16;
    let rows = (available_height / cell_height).floor().max(1.0) as u16;
    (cols, rows)
}

/// [`grid_size_for_area`], but starting from a *window* viewport size --
/// `TAB_BAR_HEIGHT` is subtracted from the height first, to get the area
/// actually available to the terminal canvas below the tab bar.
pub fn grid_size_for_window(
    viewport_width: f32,
    viewport_height: f32,
    cell_width: f32,
    cell_height: f32,
) -> (u16, u16) {
    let terminal_height = (viewport_height - TAB_BAR_HEIGHT).max(0.0);
    grid_size_for_area(viewport_width, terminal_height, cell_width, cell_height)
}

/// Which `(col, row)` grid cell a pixel position *local to a pane's canvas*
/// (i.e. already offset by the canvas's own bounds origin -- the caller
/// subtracts `event.position - bounds.origin` before calling this) falls on,
/// given that pane's measured cell size and its current grid dimensions.
///
/// Clamped into `[0, cols-1] x [0, rows-1]` (never out of range, even for a
/// position slightly outside the canvas -- e.g. a fast mouse-drag that
/// briefly reports a coordinate a pixel or two past the last row/column, or
/// starts a selection right as a resize is landing) so callers can index a
/// snapshot's `cells` with the result unconditionally, no separate bounds
/// check needed. Mirrors [`grid_size_for_area`]'s "always in range, never
/// panics" contract.
pub fn cell_at(
    local_x: f32,
    local_y: f32,
    cell_width: f32,
    cell_height: f32,
    cols: u16,
    rows: u16,
) -> (u16, u16) {
    let col = if cell_width > 0.0 {
        (local_x / cell_width).floor().max(0.0) as u32
    } else {
        0
    };
    let row = if cell_height > 0.0 {
        (local_y / cell_height).floor().max(0.0) as u32
    } else {
        0
    };
    let col = col.min(cols.saturating_sub(1) as u32) as u16;
    let row = row.min(rows.saturating_sub(1) as u32) as u16;
    (col, row)
}

/// Accumulate a scroll-wheel/trackpad pixel delta into whole terminal lines
/// to scroll by.
///
/// `pending` carries the fractional remainder between calls (mutated in
/// place, one instance per pane) so a slow trackpad gesture -- many small
/// sub-cell-height pixel deltas -- still eventually produces whole-line
/// scroll steps instead of silently rounding each individual event to zero;
/// mirrors real Ghostty's own `pending_scroll_y` accumulator
/// (`Surface.scrollCallback` in the vendored Ghostty source). Callers pass
/// gpui's `ScrollDelta::pixel_delta(line_height)` (which already unifies the
/// traditional line-wheel and precision-trackpad cases into one pixel
/// value, treating one "line" tick as worth exactly one `cell_height`) as
/// `delta_y_px`.
///
/// Returns the (possibly zero) whole number of lines to scroll -- sign
/// convention matches `labolabo_term::backend::VtBackend::scroll_display`
/// directly (positive = up/into history), since gpui forwards the raw
/// platform scroll delta unchanged and that raw value already carries the
/// same "positive = up" convention real Ghostty's own apprt layer
/// normalizes to (see `VtBackend::scroll_display`'s doc comment for the
/// full chain of reasoning) -- callers can feed this straight into
/// `Terminal::scroll` with no sign flip.
///
/// A non-finite or non-positive `cell_height` yields `0` every time (no
/// division by zero/garbage) without touching `pending`.
pub fn accumulate_scroll_lines(pending: &mut f32, delta_y_px: f32, cell_height: f32) -> i64 {
    if !cell_height.is_finite() || cell_height <= 0.0 || !delta_y_px.is_finite() {
        return 0;
    }
    let total = *pending + delta_y_px / cell_height;
    let whole = total.trunc();
    *pending = total - whole;
    whole as i64
}

/// The split `ratio` (first child's fraction) a divider drag implies:
/// `local_pos` (the drag cursor's position along the drag axis, relative to
/// the split container's own origin) divided by `container_len` (that
/// container's own pixel extent along the same axis -- width for a
/// row/horizontal split's left-right divider, height for a column/vertical
/// split's up-down divider).
///
/// Deliberately performs **no clamping and no zero-guard** of its own: a
/// degenerate `container_len` (`0.0`, negative, or non-finite -- reachable
/// momentarily mid-drag if a concurrent window resize collapses the split
/// container to zero size) produces `NaN`/`±inf` here, same as any other
/// float division would, and is left for the one caller
/// (`labolabo_core::TileNode::set_ratio`, via `PaneTilingModel::
/// set_split_ratio`) that already has to reject a non-finite ratio anyway
/// (leaving the previous ratio untouched) to be the single place that
/// safety net lives, rather than duplicating it here.
pub fn ratio_from_drag_position(local_pos: f32, container_len: f32) -> f64 {
    (local_pos / container_len) as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_multiple_fits_perfectly() {
        assert_eq!(grid_size_for_area(90.0, 180.0, 9.0, 18.0), (10, 10));
    }

    #[test]
    fn leftover_pixels_are_truncated_not_rounded() {
        // 95 / 9 = 10.55 -> 10 cols, not rounded up to 11.
        assert_eq!(grid_size_for_area(95.0, 188.0, 9.0, 18.0), (10, 10));
    }

    #[test]
    fn degenerate_area_still_reports_at_least_one_cell() {
        assert_eq!(grid_size_for_area(0.0, 0.0, 9.0, 18.0), (1, 1));
        assert_eq!(grid_size_for_area(-5.0, -5.0, 9.0, 18.0), (1, 1));
        assert_eq!(grid_size_for_area(4.0, 4.0, 9.0, 18.0), (1, 1));
    }

    #[test]
    fn window_size_subtracts_tab_bar_height() {
        // 200 - 32 = 168; 168 / 18 = 9.33 -> 9 rows.
        assert_eq!(grid_size_for_window(90.0, 200.0, 9.0, 18.0), (10, 9));
    }

    #[test]
    fn window_shorter_than_tab_bar_still_reports_one_row() {
        assert_eq!(grid_size_for_window(90.0, 10.0, 9.0, 18.0), (10, 1));
    }

    #[test]
    fn window_size_matches_a_realistic_default() {
        // The app's INITIAL_WIDTH/HEIGHT (main.rs): 900x600.
        assert_eq!(grid_size_for_window(900.0, 600.0, 9.0, 18.0), (100, 31));
    }

    #[test]
    fn cell_at_exact_cell_boundary() {
        assert_eq!(cell_at(0.0, 0.0, 9.0, 18.0, 80, 24), (0, 0));
        assert_eq!(cell_at(9.0, 18.0, 9.0, 18.0, 80, 24), (1, 1));
        assert_eq!(cell_at(44.9, 89.9, 9.0, 18.0, 80, 24), (4, 4));
    }

    #[test]
    fn cell_at_clamps_negative_position_to_zero() {
        assert_eq!(cell_at(-50.0, -50.0, 9.0, 18.0, 80, 24), (0, 0));
    }

    #[test]
    fn cell_at_clamps_past_the_last_row_or_column() {
        // 80 cols x 24 rows -> last valid indices are (79, 23). A position
        // far past the grid (a drag that slipped outside the canvas) must
        // still resolve in-range, not panic or wrap.
        assert_eq!(cell_at(10_000.0, 10_000.0, 9.0, 18.0, 80, 24), (79, 23));
    }

    #[test]
    fn cell_at_degenerate_cell_size_never_divides_by_zero() {
        assert_eq!(cell_at(10.0, 10.0, 0.0, 0.0, 80, 24), (0, 0));
    }

    #[test]
    fn accumulate_scroll_lines_needs_a_full_cell_height_of_pixels() {
        let mut pending = 0.0;
        // Half a cell height: not enough for a whole line yet.
        assert_eq!(accumulate_scroll_lines(&mut pending, 9.0, 18.0), 0);
        assert!((pending - 0.5).abs() < f32::EPSILON);
        // The other half arrives in a later event -> exactly one line, and
        // the fractional remainder resets to (near) zero.
        assert_eq!(accumulate_scroll_lines(&mut pending, 9.0, 18.0), 1);
        assert!(pending.abs() < 1e-5);
    }

    #[test]
    fn accumulate_scroll_lines_carries_remainder_across_many_small_deltas() {
        // Ten trackpad micro-deltas of 2px each = 20px = a bit over one
        // 18px cell -- exactly one line should eventually fall out, not
        // zero (each individual 2px delta alone would round to 0).
        let mut pending = 0.0;
        let mut total_lines = 0i64;
        for _ in 0..10 {
            total_lines += accumulate_scroll_lines(&mut pending, 2.0, 18.0);
        }
        assert_eq!(total_lines, 1);
    }

    #[test]
    fn accumulate_scroll_lines_negative_delta_scrolls_the_other_way() {
        let mut pending = 0.0;
        assert_eq!(accumulate_scroll_lines(&mut pending, -36.0, 18.0), -2);
    }

    #[test]
    fn accumulate_scroll_lines_degenerate_cell_height_is_a_safe_no_op() {
        let mut pending = 0.3;
        assert_eq!(accumulate_scroll_lines(&mut pending, 100.0, 0.0), 0);
        assert_eq!(accumulate_scroll_lines(&mut pending, 100.0, f32::NAN), 0);
        assert_eq!(accumulate_scroll_lines(&mut pending, f32::NAN, 18.0), 0);
        // `pending` is untouched by the degenerate calls above.
        assert!((pending - 0.3).abs() < f32::EPSILON);
    }

    // MARK: - ratio_from_drag_position (divider drag-resize)

    #[test]
    fn ratio_from_drag_position_midpoint() {
        let ratio = ratio_from_drag_position(50.0, 100.0);
        assert!((ratio - 0.5).abs() < 1e-9);
    }

    #[test]
    fn ratio_from_drag_position_at_either_edge() {
        assert!((ratio_from_drag_position(0.0, 100.0) - 0.0).abs() < 1e-9);
        assert!((ratio_from_drag_position(100.0, 100.0) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn ratio_from_drag_position_past_the_edges_is_not_clamped_here() {
        // Clamping is the caller's (`TileNode::set_ratio`'s) job -- this
        // function is a plain division.
        assert!(ratio_from_drag_position(-10.0, 100.0) < 0.0);
        assert!(ratio_from_drag_position(150.0, 100.0) > 1.0);
    }

    #[test]
    fn ratio_from_drag_position_zero_container_len_is_non_finite_not_a_panic() {
        assert!(ratio_from_drag_position(0.0, 0.0).is_nan());
        assert!(ratio_from_drag_position(10.0, 0.0).is_infinite());
    }
}
