//! Pure tile-tree focus logic: which pane gets keyboard focus after a
//! structural change, and how "focus the next/previous pane" and "select tab
//! N" resolve against a [`PaneTilingModel`].
//!
//! Deliberately gpui-free (only `labolabo_core::tiling` types) so it's
//! unit-testable without a gpui `Application`/window, per this wave's
//! quality gate ("木操作→UI状態の純ロジック部分... はユニットテストを付ける
//! （gpui非依存に切り出す）").
//!
//! ## The focus invariant
//!
//! [`crate::app::TerminalApp`] tracks focus as a single `PaneId` (not a
//! `NodeId`): the tab that currently has keyboard focus. It is always both
//! (a) present in the tree and (b) the *selected* tab of the leaf (tab
//! group) that owns it -- i.e. it doubly identifies "which pane has focus"
//! and "which of that pane's tabs is active", since a leaf's active tab is
//! exactly its `selected_pane()`. `PaneId`s are stable across tree mutations
//! (split/move/collapse shuffle `PaneItem`s and `TileNode` identities around
//! without ever re-minting a `PaneId` for an existing tab -- see
//! `tiling.rs`'s doc comment), which is what makes tracking focus by
//! `PaneId` -- rather than by the leaf's `NodeId`, which is *not* stable
//! across a split (the leaf being split keeps its id but stops being a leaf)
//! -- both simple and robust.
//!
//! The caller (`TerminalApp`) is responsible for maintaining the invariant
//! (updating the tracked `PaneId` on every focus-affecting mutation); the
//! functions here are pure queries/resolutions against a model + a
//! caller-supplied anchor, they don't themselves hold any state.

use labolabo_core::{PaneId, PaneTilingModel};

/// Resolves which pane should receive focus after closing a pane that had
/// focus, given `close`'s own return value (the pane that became
/// front-facing in the closed pane's leaf/collapsed region, if any).
///
/// `close` returns `None` in two cases the caller must distinguish before
/// calling this: (1) the closed pane was the tree's only pane (a no-op,
/// `close` didn't touch the tree at all -- the caller must detect this itself
/// beforehand, e.g. to shut down and quit rather than resolve a new focus);
/// (2) closing a leaf collapsed its parent into a *sibling that was itself a
/// split* (not a plain leaf), so there is no single front-facing pane to
/// reveal -- a real, reachable case in any tree with 3+ panes. This function
/// only needs to handle case (2): it falls back to the tree's first leaf (DFS
/// order, `TileNode::leaves()`)'s *currently selected* tab, a simple,
/// deterministic choice, over trying to find the "nearest" leaf (which
/// `close`'s `Option<PaneId>` return value doesn't carry enough information
/// to do without changing that method's signature).
///
/// Deliberately `leaves().first()`'s `selected_pane()`, not
/// `PaneTilingModel::panes().first()`: `panes()` flattens *every* tab in DFS
/// order, so its first element is that leaf's tab at index 0 -- not
/// necessarily the one currently on screen, if that leaf's `selected_index`
/// happens to be nonzero. Focusing a hidden tab would violate the focus
/// invariant (see this module's doc comment): the returned pane must always
/// be its leaf's active tab.
pub(crate) fn resolve_close_focus(
    model: &PaneTilingModel,
    revealed: Option<PaneId>,
) -> Option<PaneId> {
    revealed.or_else(|| {
        model
            .root
            .leaves()
            .first()
            .and_then(|leaf| leaf.selected_pane())
            .map(|p| p.id)
    })
}

/// The pane to focus when moving focus to the next (`forward = true`) or
/// previous (`forward = false`) pane, cycling through the tree's leaves in
/// DFS order (`TileNode::leaves()`) with wraparound. This is the "simplest
/// to implement" option the wave's brief explicitly allows in place of true
/// geometric (left/right/up/down) adjacency, which would need each leaf's
/// on-screen rectangle -- not something this gpui-independent module has
/// access to (or, arguably, should: geometric adjacency is a rendering-layer
/// concern).
///
/// Returns `None` if `anchor` isn't found in the tree (shouldn't happen given
/// the focus invariant, but the caller should treat it as "no-op" rather than
/// panic). Returns `Some(anchor)` unchanged when the tree has exactly one
/// leaf (wraparound of a single element).
pub(crate) fn adjacent_pane(
    model: &PaneTilingModel,
    anchor: PaneId,
    forward: bool,
) -> Option<PaneId> {
    let leaves = model.root.leaves();
    let current = model.root.find_leaf(anchor)?;
    let idx = leaves.iter().position(|leaf| leaf.id == current.id)?;
    let len = leaves.len();
    let next_idx = if forward {
        (idx + 1) % len
    } else {
        (idx + len - 1) % len
    };
    leaves[next_idx].selected_pane().map(|p| p.id)
}

/// The `index`-th (0-based) tab's [`PaneId`] in `anchor`'s tab group (leaf),
/// for a keyboard tab-select shortcut (Cmd+1..9). `None` if `anchor` isn't
/// found, or if the leaf has no tab at `index` (e.g. Cmd+5 on a 2-tab group).
pub(crate) fn nth_tab(model: &PaneTilingModel, anchor: PaneId, index: usize) -> Option<PaneId> {
    let leaf = model.root.find_leaf(anchor)?;
    leaf.panes.get(index).map(|p| p.id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use labolabo_core::{PaneItem, PaneKind, TileOrientation};

    /// A single-terminal-pane model, the app's actual initial layout.
    fn single_pane_model() -> PaneTilingModel {
        PaneTilingModel::new(labolabo_core::TileNode::leaf(PaneItem::new(
            PaneKind::Terminal,
            "t1",
        )))
    }

    // MARK: - resolve_close_focus

    #[test]
    fn resolve_close_focus_uses_revealed_when_present() {
        let model = single_pane_model();
        let revealed = model.panes()[0].id;
        assert_eq!(resolve_close_focus(&model, Some(revealed)), Some(revealed));
    }

    #[test]
    fn resolve_close_focus_falls_back_to_first_leafs_selected_tab() {
        let mut model = single_pane_model();
        let first_id = model.panes()[0].id;
        model.add_pane(PaneItem::new(PaneKind::Files, "f"));
        assert_eq!(model.panes()[0].id, first_id, "sanity: still DFS-first");

        assert_eq!(resolve_close_focus(&model, None), Some(first_id));
    }

    #[test]
    fn resolve_close_focus_fallback_uses_the_selected_tab_not_index_zero() {
        // Regression test: the first leaf in DFS order has 2 tabs, and its
        // *second* tab is the one currently selected (on screen). The
        // fallback must resolve to that selected tab, not blindly to
        // `panes()[0]` (which would be the first leaf's tab at index 0 --
        // hidden here -- and would violate the focus invariant of always
        // pointing at a leaf's active tab).
        let mut model = single_pane_model();
        let t1 = model.panes()[0].id;
        model.add_tab(t1, PaneItem::new(PaneKind::Terminal, "t2"));
        let t2 = model
            .root
            .find_leaf(t1)
            .unwrap()
            .selected_pane()
            .unwrap()
            .id;
        assert_ne!(t2, t1, "sanity: the new tab is selected, not t1");
        assert_eq!(
            model.panes()[0].id,
            t1,
            "sanity: panes() DFS order still starts with t1 (index 0), not the selected t2"
        );

        assert_eq!(resolve_close_focus(&model, None), Some(t2));
    }

    // MARK: - adjacent_pane

    /// Builds a 3-leaf tree: split the root leaf right (A|B), then split B's
    /// leaf down (A | (B / C)). DFS leaf order is A, B, C.
    fn three_pane_model() -> (PaneTilingModel, PaneId, PaneId, PaneId) {
        let mut model = single_pane_model();
        let a = model.panes()[0].id;
        model.split(
            a,
            TileOrientation::Horizontal,
            PaneItem::new(PaneKind::Files, "b"),
        );
        let b = model
            .panes()
            .iter()
            .find(|p| p.kind == PaneKind::Files)
            .unwrap()
            .id;
        model.split(
            b,
            TileOrientation::Vertical,
            PaneItem::new(PaneKind::Diff, "c"),
        );
        let c = model
            .panes()
            .iter()
            .find(|p| p.kind == PaneKind::Diff)
            .unwrap()
            .id;
        (model, a, b, c)
    }

    #[test]
    fn adjacent_pane_forward_cycles_dfs_order_with_wraparound() {
        let (model, a, b, c) = three_pane_model();
        assert_eq!(adjacent_pane(&model, a, true), Some(b));
        assert_eq!(adjacent_pane(&model, b, true), Some(c));
        assert_eq!(
            adjacent_pane(&model, c, true),
            Some(a),
            "wraps back to the first leaf"
        );
    }

    #[test]
    fn adjacent_pane_backward_cycles_dfs_order_with_wraparound() {
        let (model, a, b, c) = three_pane_model();
        assert_eq!(
            adjacent_pane(&model, a, false),
            Some(c),
            "wraps back to the last leaf"
        );
        assert_eq!(adjacent_pane(&model, c, false), Some(b));
        assert_eq!(adjacent_pane(&model, b, false), Some(a));
    }

    #[test]
    fn adjacent_pane_single_leaf_returns_itself() {
        let model = single_pane_model();
        let only = model.panes()[0].id;
        assert_eq!(adjacent_pane(&model, only, true), Some(only));
        assert_eq!(adjacent_pane(&model, only, false), Some(only));
    }

    #[test]
    fn adjacent_pane_targets_the_leafs_selected_tab_not_its_first_tab() {
        let (mut model, a, b, _c) = three_pane_model();
        // Give leaf A a second tab and select it, so A's "active tab" is no
        // longer its first pane.
        model.add_tab(a, PaneItem::new(PaneKind::Terminal, "a2"));
        let a2 = model.root.find_leaf(a).unwrap().selected_pane().unwrap().id;
        assert_ne!(a2, a);

        // Moving focus backward from B (DFS order A, B, C) should land on
        // whichever tab is *currently selected* in A -- a2, not a.
        assert_eq!(adjacent_pane(&model, b, false), Some(a2));
    }

    #[test]
    fn adjacent_pane_unknown_anchor_returns_none() {
        let model = single_pane_model();
        // A PaneId that never existed in this tree: use a pane id minted for
        // a *different* model, which is guaranteed not to collide with any
        // id `model` handed out (ids are process-global and monotonic).
        let other = single_pane_model().panes()[0].id;
        assert_eq!(adjacent_pane(&model, other, true), None);
    }

    // MARK: - nth_tab

    #[test]
    fn nth_tab_returns_pane_at_index_within_anchors_group() {
        let mut model = single_pane_model();
        let t1 = model.panes()[0].id;
        model.add_tab(t1, PaneItem::new(PaneKind::Terminal, "t2"));
        model.add_tab(t1, PaneItem::new(PaneKind::Terminal, "t3"));
        let group = model.root.find_leaf(t1).unwrap();
        let (id0, id1, id2) = (group.panes[0].id, group.panes[1].id, group.panes[2].id);

        assert_eq!(nth_tab(&model, t1, 0), Some(id0));
        assert_eq!(nth_tab(&model, t1, 1), Some(id1));
        assert_eq!(nth_tab(&model, t1, 2), Some(id2));
    }

    #[test]
    fn nth_tab_out_of_range_returns_none() {
        let model = single_pane_model();
        let t1 = model.panes()[0].id;
        assert_eq!(
            nth_tab(&model, t1, 1),
            None,
            "only one tab exists (index 0)"
        );
    }

    #[test]
    fn nth_tab_unknown_anchor_returns_none() {
        let model = single_pane_model();
        let other = single_pane_model().panes()[0].id;
        assert_eq!(nth_tab(&model, other, 0), None);
    }
}
