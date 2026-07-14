//! Pure geometry: translate a pixel-space window/canvas area into a
//! terminal grid's column/row count, given a fixed cell size.
//!
//! No gpui types appear here on purpose -- window-resize -> `TermSession::
//! resize` wiring (`app::TerminalApp::handle_window_resized`) needs this
//! math, and keeping it gpui-free makes it unit-testable without spinning
//! up a gpui `Application`/window (which `cargo test` cannot do headlessly
//! in CI).

/// Height in pixels reserved for the tab bar strip at the top of the
/// window, subtracted from the viewport before computing the terminal
/// grid's own area. Must match `app::TerminalApp::render_tab_bar`'s
/// `.h(px(TAB_BAR_HEIGHT))`.
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
}
