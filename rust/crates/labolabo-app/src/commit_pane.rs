//! The commit-history graph pane body (`plans` W6d): a `Commits`-kind tile
//! pane's content, alongside `crate::git_pane`'s `Files`/`Diff` bodies.
//!
//! `labolabo_core::commit_graph::build` already does the hard part -- laying
//! out each [`CommitGraphRow`]'s lanes/edges (see that module's doc
//! comment). This module is just the render step: "曲線不要・直線で可"
//! (`plans` W6d's own brief) simplification is a straight, **all-vertical**
//! lane graph -- every [`Edge`] becomes a 2px vertical bar in its own lane
//! column, plus (only when an edge's lane differs from the row's node lane,
//! i.e. a real branch/merge point) one straight horizontal bar connecting
//! the two columns at the row's mid-height. No diagonals/curves, matching
//! the brief -- this reads like a simplified/schematic commit graph (the
//! same "elbow connector" convention common to minimal graph widgets),
//! trading exact edge angles for zero dependency on arbitrary line-rotation
//! primitives.
//!
//! [`row_segments`]/[`lane_x`]/[`lane_color`] are pure numeric functions
//! (row + geometry constants in, drawing primitives out) -- gpui-independent
//! and unit-tested below, per this wave's quality gate ("コミットグラフ行→
//! 描画プリミティブ列...はユニットテスト"). [`render_commits_pane`] is the
//! only gpui-touching function in this module, turning each [`LaneSegment`]
//! into an absolutely-positioned `div`.

use gpui::{div, prelude::*, px, rgb, AnyElement, Context, SharedString};

use labolabo_core::{CommitGraphRow, EdgeShape};

use crate::app::LaboLaboApp;
use crate::git_pane;
use crate::render::RenderSpec;
use crate::theme;

/// Width of one lane column, in logical pixels.
pub const LANE_WIDTH: f32 = 14.0;
/// Height of one commit row, in logical pixels.
pub const ROW_HEIGHT: f32 = 22.0;
/// Diameter of a commit's node marker.
pub const NODE_SIZE: f32 = 7.0;
/// Thickness of a lane line/connector.
const LINE_THICKNESS: f32 = 2.0;

/// The x-coordinate (row-local, left edge = 0) of `lane`'s center line.
pub fn lane_x(lane: usize) -> f32 {
    LANE_WIDTH * (lane as f32) + LANE_WIDTH / 2.0
}

/// One straight 2px bar to paint for a row's lane graph, in row-local pixel
/// coordinates. Deliberately just two shapes (no diagonal) -- see this
/// module's doc comment.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LaneSegment {
    /// A vertical bar at a fixed x, from `y1` to `y2` (`y1 <= y2`).
    Vertical {
        x: f32,
        y1: f32,
        y2: f32,
        color_lane: usize,
    },
    /// A horizontal bar at a fixed y, from `x1` to `x2` (`x1 <= x2`) --
    /// only ever emitted to bridge a `NodeIn`/`NodeOut` edge whose lane
    /// differs from the row's own node lane (a branch/merge point).
    Horizontal {
        x1: f32,
        x2: f32,
        y: f32,
        color_lane: usize,
    },
}

/// Converts one row's edges (already lane-assigned by
/// `labolabo_core::commit_graph::build`) into straight-line drawing
/// primitives -- pure, gpui-independent (see this module's doc comment).
/// `row.node_lane`'s own node marker is drawn separately by
/// [`render_commits_pane`] (a marker, not a line segment).
pub fn row_segments(row: &CommitGraphRow) -> Vec<LaneSegment> {
    let node_x = lane_x(row.node_lane);
    let mid_y = ROW_HEIGHT / 2.0;
    let mut segments = Vec::new();
    for edge in &row.edges {
        let x = lane_x(edge.lane);
        match edge.shape {
            EdgeShape::Through => segments.push(LaneSegment::Vertical {
                x,
                y1: 0.0,
                y2: ROW_HEIGHT,
                color_lane: edge.color_lane,
            }),
            EdgeShape::NodeIn => {
                segments.push(LaneSegment::Vertical {
                    x,
                    y1: 0.0,
                    y2: mid_y,
                    color_lane: edge.color_lane,
                });
                if edge.lane != row.node_lane {
                    segments.push(LaneSegment::Horizontal {
                        x1: x.min(node_x),
                        x2: x.max(node_x),
                        y: mid_y,
                        color_lane: edge.color_lane,
                    });
                }
            }
            EdgeShape::NodeOut => {
                segments.push(LaneSegment::Vertical {
                    x,
                    y1: mid_y,
                    y2: ROW_HEIGHT,
                    color_lane: edge.color_lane,
                });
                if edge.lane != row.node_lane {
                    segments.push(LaneSegment::Horizontal {
                        x1: x.min(node_x),
                        x2: x.max(node_x),
                        y: mid_y,
                        color_lane: edge.color_lane,
                    });
                }
            }
        }
    }
    segments
}

/// A small fixed palette, cycling by `color_lane % palette.len()` -- stable
/// coloring per lane (mirrors `commit_graph::build`'s own "kept == `lane`
/// for stable coloring" doc comment on [`labolabo_core::Edge::color_lane`]),
/// distinct enough at a glance without pulling in a full HSL-generation
/// scheme for what's usually 1-3 simultaneously-open lanes.
const LANE_PALETTE: [u32; 6] = [
    theme::ACCENT,
    theme::status::RUNNING,
    theme::status::STARTING,
    theme::diff::DEL,
    0xbf5af2, // violet
    0x64d2ff, // cyan
];

pub fn lane_color(lane: usize) -> u32 {
    LANE_PALETTE[lane % LANE_PALETTE.len()]
}

/// `Some(epoch_secs)` -> `"YYYY-MM-DD"` (UTC -- a pure, deterministic
/// formatting choice so this stays unit-testable without a timezone
/// dependency; local-time display is a nice-to-have left for later). `None`
/// (or an out-of-range epoch value) renders as an empty string rather than
/// panicking.
fn format_commit_date(epoch_secs: Option<i64>) -> String {
    let Some(secs) = epoch_secs else {
        return String::new();
    };
    match chrono::DateTime::from_timestamp(secs, 0) {
        Some(dt) => dt.format("%Y-%m-%d").to_string(),
        None => String::new(),
    }
}

/// Renders `task_id`'s commit-graph tile body: one row per
/// [`CommitGraphRow`], each a lane-graph column (drawn from
/// [`row_segments`] + a node marker) followed by the short hash, subject,
/// and date. `rows` is `GitPaneState::commits` -- the same per-Task Git
/// state the `Files`/`Diff` tile bodies (`crate::git_pane::
/// render_file_list`/`render_detail`) already read, so this pane updates on
/// the same refresh/watch cadence as everything else Git-related for this
/// Task (see `crate::git_pane`'s module doc comment).
pub fn render_commits_pane(
    task_id: &str,
    rows: &[CommitGraphRow],
    spec: &RenderSpec,
    _cx: &mut Context<LaboLaboApp>,
) -> AnyElement {
    let _ = task_id; // no click-through affordance yet (rows aren't selectable this wave)
    if rows.is_empty() {
        return git_pane::placeholder("No commits");
    }

    let max_lane = rows
        .iter()
        .flat_map(|row| {
            row.edges
                .iter()
                .map(|e| e.lane)
                .chain(std::iter::once(row.node_lane))
        })
        .max()
        .unwrap_or(0);
    let graph_width = LANE_WIDTH * (max_lane as f32 + 1.0);

    let mut col = div().flex().flex_col().overflow_hidden();
    for row in rows {
        col = col.child(render_commit_row(row, graph_width, spec));
    }
    col.into_any_element()
}

// `spec.font.family` rather than a hardcoded `"Menlo"` literal -- see
// `git_pane::render_diff_line`'s doc comment (same reasoning: Menlo is
// macOS-only, and gpui's generic cross-platform font fallback stack isn't
// guaranteed to be monospace).
fn render_commit_row(row: &CommitGraphRow, graph_width: f32, spec: &RenderSpec) -> AnyElement {
    let mut graph = div()
        .relative()
        .flex_shrink_0()
        .w(px(graph_width))
        .h(px(ROW_HEIGHT));
    for segment in row_segments(row) {
        graph = graph.child(render_segment(segment));
    }
    let node_x = lane_x(row.node_lane);
    graph = graph.child(
        div()
            .absolute()
            .left(px(node_x - NODE_SIZE / 2.0))
            .top(px(ROW_HEIGHT / 2.0 - NODE_SIZE / 2.0))
            .w(px(NODE_SIZE))
            .h(px(NODE_SIZE))
            .rounded_full()
            .bg(rgb(lane_color(row.node_lane))),
    );

    let subject: SharedString = row.commit.subject.clone().into();
    let hash: SharedString = row.commit.hash.clone().into();
    let date: SharedString = format_commit_date(row.commit.date).into();

    div()
        .flex()
        .flex_row()
        .items_center()
        .gap_2()
        .px_2()
        .h(px(ROW_HEIGHT))
        .flex_shrink_0()
        .text_size(px(11.0))
        .child(graph)
        .child(
            div()
                .flex_shrink_0()
                .font_family(spec.font.family.clone())
                .text_color(rgb(theme::text::MUTED))
                .child(hash),
        )
        .child(
            div()
                .flex_1()
                .overflow_hidden()
                .text_color(rgb(theme::text::PRIMARY))
                .child(subject),
        )
        .child(
            div()
                .flex_shrink_0()
                .font_family(spec.font.family.clone())
                .text_size(px(theme::font_size::CAPTION))
                .text_color(rgb(theme::text::SECONDARY))
                .child(date),
        )
        .into_any_element()
}

fn render_segment(segment: LaneSegment) -> AnyElement {
    match segment {
        LaneSegment::Vertical {
            x,
            y1,
            y2,
            color_lane,
        } => div()
            .absolute()
            .left(px(x - LINE_THICKNESS / 2.0))
            .top(px(y1))
            .w(px(LINE_THICKNESS))
            .h(px((y2 - y1).max(0.0)))
            .bg(rgb(lane_color(color_lane)))
            .into_any_element(),
        LaneSegment::Horizontal {
            x1,
            x2,
            y,
            color_lane,
        } => div()
            .absolute()
            .left(px(x1))
            .top(px(y - LINE_THICKNESS / 2.0))
            .w(px((x2 - x1).max(0.0)))
            .h(px(LINE_THICKNESS))
            .bg(rgb(lane_color(color_lane)))
            .into_any_element(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use labolabo_core::{Commit, Edge};

    fn row(node_lane: usize, edges: Vec<Edge>) -> CommitGraphRow {
        CommitGraphRow {
            id: 0,
            commit: Commit {
                hash: "abc1234".to_string(),
                subject: "subject".to_string(),
                author: "Alice".to_string(),
                date: Some(1_700_000_000),
                refs: String::new(),
            },
            node_lane,
            edges,
        }
    }

    fn edge(shape: EdgeShape, lane: usize) -> Edge {
        Edge {
            shape,
            lane,
            color_lane: lane,
        }
    }

    // MARK: - lane_x

    #[test]
    fn lane_x_centers_each_lane_column() {
        assert_eq!(lane_x(0), LANE_WIDTH / 2.0);
        assert_eq!(lane_x(1), LANE_WIDTH + LANE_WIDTH / 2.0);
        assert_eq!(lane_x(2), 2.0 * LANE_WIDTH + LANE_WIDTH / 2.0);
    }

    // MARK: - lane_color

    #[test]
    fn lane_color_is_stable_per_lane_and_cycles_through_the_palette() {
        assert_eq!(lane_color(0), lane_color(0));
        assert_eq!(lane_color(0), lane_color(LANE_PALETTE.len()));
        assert_ne!(lane_color(0), lane_color(1));
    }

    // MARK: - row_segments

    #[test]
    fn a_leaf_commit_with_no_edges_has_no_segments() {
        // Root commit / first-ever row: no children above, no parents below.
        assert!(row_segments(&row(0, Vec::new())).is_empty());
    }

    #[test]
    fn linear_history_row_is_two_vertical_segments_in_the_same_lane_no_horizontal() {
        // A middle-of-history commit on a straight line: one child entering
        // from above (NodeIn) and one parent leaving below (NodeOut), both
        // in the node's own lane -- no branch/merge, so no horizontal
        // connector.
        let r = row(
            0,
            vec![edge(EdgeShape::NodeIn, 0), edge(EdgeShape::NodeOut, 0)],
        );
        let segments = row_segments(&r);
        assert_eq!(segments.len(), 2);
        assert!(segments
            .iter()
            .all(|s| matches!(s, LaneSegment::Vertical { .. })));
        assert!(segments.contains(&LaneSegment::Vertical {
            x: lane_x(0),
            y1: 0.0,
            y2: ROW_HEIGHT / 2.0,
            color_lane: 0,
        }));
        assert!(segments.contains(&LaneSegment::Vertical {
            x: lane_x(0),
            y1: ROW_HEIGHT / 2.0,
            y2: ROW_HEIGHT,
            color_lane: 0,
        }));
    }

    #[test]
    fn a_passing_lane_is_a_single_full_height_vertical_segment() {
        let r = row(1, vec![edge(EdgeShape::Through, 0)]);
        let segments = row_segments(&r);
        assert_eq!(
            segments,
            vec![LaneSegment::Vertical {
                x: lane_x(0),
                y1: 0.0,
                y2: ROW_HEIGHT,
                color_lane: 0,
            }]
        );
    }

    #[test]
    fn a_merge_commits_second_parent_gets_a_vertical_and_a_connecting_horizontal() {
        // Merge commit: node in lane 0, second parent opens/reuses lane 1 --
        // that edge's lane (1) differs from the node's lane (0), so it
        // should get both its own vertical stub *and* a horizontal bridge
        // back to the node column.
        let r = row(
            0,
            vec![edge(EdgeShape::NodeOut, 0), edge(EdgeShape::NodeOut, 1)],
        );
        let segments = row_segments(&r);
        assert_eq!(segments.len(), 3);
        assert!(segments.contains(&LaneSegment::Vertical {
            x: lane_x(0),
            y1: ROW_HEIGHT / 2.0,
            y2: ROW_HEIGHT,
            color_lane: 0,
        }));
        assert!(segments.contains(&LaneSegment::Vertical {
            x: lane_x(1),
            y1: ROW_HEIGHT / 2.0,
            y2: ROW_HEIGHT,
            color_lane: 1,
        }));
        assert!(segments.contains(&LaneSegment::Horizontal {
            x1: lane_x(0),
            x2: lane_x(1),
            y: ROW_HEIGHT / 2.0,
            color_lane: 1,
        }));
    }

    // MARK: - format_commit_date

    #[test]
    fn format_commit_date_renders_utc_calendar_date() {
        assert_eq!(format_commit_date(Some(1_700_000_000)), "2023-11-14");
    }

    #[test]
    fn format_commit_date_of_none_is_empty() {
        assert_eq!(format_commit_date(None), "");
    }
}
