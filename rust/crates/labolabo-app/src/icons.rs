//! LaboLabo's SVG icon system (第13波b §2 -- モダン化第2弾).
//!
//! Previously every "icon" in the sidebar/tab bar/Git pane was a plain
//! Unicode glyph (⚠ ∅ ⎇ ± ▾ × …) drawn as a text child -- cheap, but
//! font-dependent (glyph shape/weight varies by platform font) and visually
//! inconsistent with itself (a triangle from one font family next to a
//! circle-slash from another). This module replaces that whole family with
//! a small, hand-authored SVG set: 16px viewBox, 1.5px stroke, round
//! caps/joins, single color (`fill="#000"`/`stroke="#000"` in the source
//! files -- see the paragraph below for why the literal color there is
//! irrelevant).
//!
//! ## Why the SVG source color doesn't matter
//!
//! gpui 0.2's `svg()` element (`elements/svg.rs`) renders through `resvg`
//! into a `Pixmap`, then keeps **only the alpha channel** as a coverage mask
//! (`svg_renderer.rs`'s `SvgRenderer::render`: `pixmap.pixels().iter().map(|p|
//! p.alpha())`), which `Window::paint_svg` then tints with whatever color
//! `Styled::text_color` set on the element. So every icon here is authored
//! with an arbitrary opaque fill/stroke (`#000`) purely as "coverage" --
//! the actual on-screen color always comes from the caller's `.text_color(..)`
//! (typically a `crate::theme` token), which is how these stay `currentColor`-
//! style single-tone icons that track the surrounding text color for free.
//! No emoji, no multi-color icons (project policy, `plans` 第13波b brief).
//!
//! ## Embedding
//!
//! [`Assets`] is a minimal [`gpui::AssetSource`] -- a `match` over the exact
//! asset path strings [`Icon::asset_path`] returns, each arm an
//! `include_bytes!` of one file under `../icons/`. This (rather than a
//! `rust-embed`-style directory-walking crate) is the "実 API を確認して
//! 最小構成で" the brief asks for: one new trait impl, zero new
//! dependencies, and every path is a compile-time-checked string literal (a
//! typo in `asset_path` would fail to compile, not silently 404 at
//! runtime). Wired in via `Application::new().with_assets(icons::Assets)`
//! in `main.rs`.
use std::borrow::Cow;

use gpui::{px, rgb, AssetSource, IntoElement, SharedString, Styled, Svg};

/// One glyph in LaboLabo's icon set. See this module's doc comment for the
/// rendering model (single-tone, tinted via `.text_color(..)`) and
/// [`Icon::asset_path`] for the name -> asset mapping this enum exists to
/// make exhaustive and unit-testable (`plans` 第13波b brief's "アイコン名→
/// アセット解決はユニットテスト" quality gate).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    /// "+" -- new Task (attached), tab-bar "add tab".
    Plus,
    /// "⎇" replacement -- worktree Task kind marker, Git pane branch bar.
    Branch,
    /// `sidebar::kind_marker`'s attached-Task counterpart to [`Icon::Branch`]
    /// (`plans` 第16波follow-up: replaces a plain filled dot, which visually
    /// doubled up with the unified status/color dot right next to it --
    /// see that function's doc comment).
    Folder,
    /// "⚠" replacement -- sidebar cross-session conflict badge.
    Warning,
    /// "∅" replacement -- sidebar missing-worktree badge.
    NotFound,
    /// "▾"/"▸" replacement -- archived-section disclosure chevron. Points
    /// right by default; rotate 90° (see `chevron_element`) for "expanded".
    Chevron,
    /// "×" replacement -- tab close, banner dismiss, Git pane close.
    Close,
    /// "⋯" replacement -- sidebar Task row "…" menu button.
    More,
    /// "▤" replacement -- Git tile-pane button (changed files).
    Files,
    /// "±" replacement -- Git tile-pane button (diff).
    Diff,
    /// "⧖" replacement -- Git tile-pane button (commit history).
    History,
    /// "▦" replacement -- Git pane "open as tile" button.
    Grid,
    /// New (第13波b §4): banner leading icon (update/import banners).
    Info,
    /// New (第13波b §3): empty-workspace illustration icon.
    Window,
    /// New (第13波b §1/§3): "clean" (no changes) -- the titlebar pill's
    /// changed-count segment when it's zero, and the Git pane's "変更なし"
    /// empty state (same icon, same meaning, both places).
    Check,
}

impl Icon {
    /// The exact [`gpui::AssetSource::load`] path this icon resolves
    /// through -- also [`Assets::load`]'s match key, so a mismatch between
    /// the two would show up immediately as a failing icon (never silently
    /// render nothing) rather than only at test time.
    pub const fn asset_path(self) -> &'static str {
        match self {
            Icon::Plus => "icons/plus.svg",
            Icon::Branch => "icons/branch.svg",
            Icon::Folder => "icons/folder.svg",
            Icon::Warning => "icons/warning.svg",
            Icon::NotFound => "icons/not-found.svg",
            Icon::Chevron => "icons/chevron.svg",
            Icon::Close => "icons/close.svg",
            Icon::More => "icons/more.svg",
            Icon::Files => "icons/files.svg",
            Icon::Diff => "icons/diff.svg",
            Icon::History => "icons/history.svg",
            Icon::Grid => "icons/grid.svg",
            Icon::Info => "icons/info.svg",
            Icon::Window => "icons/window.svg",
            Icon::Check => "icons/check.svg",
        }
    }

    /// All variants -- used by this module's own tests (every icon resolves
    /// through [`Assets`]) and available to callers that want to iterate
    /// the whole set. No non-test caller does today (every use site names
    /// an exact variant), hence the `allow` -- a real, documented part of
    /// this type's API, not dead code to delete.
    #[allow(dead_code)]
    pub const ALL: [Icon; 15] = [
        Icon::Plus,
        Icon::Branch,
        Icon::Folder,
        Icon::Warning,
        Icon::NotFound,
        Icon::Chevron,
        Icon::Close,
        Icon::More,
        Icon::Files,
        Icon::Diff,
        Icon::History,
        Icon::Grid,
        Icon::Info,
        Icon::Window,
        Icon::Check,
    ];
}

/// Builds one icon at `size` (both dimensions -- every icon here is a
/// square viewBox), `.flex_shrink_0()` so it never gets squeezed by a
/// crowded flex row (the same failure mode a text glyph never had, since
/// text doesn't shrink below its line box either). Callers still need to
/// chain `.text_color(..)` themselves -- this helper doesn't default one,
/// since the right color is always contextual (a `theme::status::CONFLICT`
/// badge vs. a `theme::text::SECONDARY` toolbar icon are both "just an
/// icon" as far as this function is concerned).
pub fn icon(name: Icon, size: f32) -> Svg {
    gpui::svg()
        .path(SharedString::from(name.asset_path()))
        .w(px(size))
        .h(px(size))
        .flex_shrink_0()
}

/// [`Icon::Chevron`] pre-rotated 90° clockwise (`gpui::Transformation::
/// rotate`) -- the archived-section header's "expanded" state ("▾" in the
/// old glyph scheme). The un-rotated icon (0°) is "collapsed" ("▸"). A
/// rotation transform (rather than a second SVG file) keeps the disclosure
/// chevron a single asset, matching how a CSS `rotate()` transform is the
/// conventional way to implement this exact affordance on the web.
pub fn chevron_element(size: f32, expanded: bool) -> Svg {
    let base = icon(Icon::Chevron, size);
    if expanded {
        base.with_transformation(gpui::Transformation::rotate(gpui::radians(
            std::f32::consts::FRAC_PI_2,
        )))
    } else {
        base
    }
}

/// Convenience: [`icon`] plus `.text_color(rgb(color))` in one call, for the
/// overwhelmingly common case of "an icon tinted a single theme color."
/// Callers that need a non-`rgb` color source (rare) can call [`icon`]
/// directly.
pub fn icon_colored(name: Icon, size: f32, color: u32) -> impl IntoElement {
    icon(name, size).text_color(rgb(color))
}

/// A minimal [`AssetSource`] serving exactly the SVGs [`Icon::asset_path`]
/// names, each `include_bytes!`-embedded at compile time (no filesystem
/// access at runtime, so this works identically from a packaged app bundle
/// with no `icons/` directory alongside it). See this module's doc comment
/// for why a hand-written `match` was chosen over a directory-embedding
/// crate.
pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
        let bytes: &'static [u8] = match path {
            "icons/plus.svg" => include_bytes!("../icons/plus.svg"),
            "icons/branch.svg" => include_bytes!("../icons/branch.svg"),
            "icons/folder.svg" => include_bytes!("../icons/folder.svg"),
            "icons/warning.svg" => include_bytes!("../icons/warning.svg"),
            "icons/not-found.svg" => include_bytes!("../icons/not-found.svg"),
            "icons/chevron.svg" => include_bytes!("../icons/chevron.svg"),
            "icons/close.svg" => include_bytes!("../icons/close.svg"),
            "icons/more.svg" => include_bytes!("../icons/more.svg"),
            "icons/files.svg" => include_bytes!("../icons/files.svg"),
            "icons/diff.svg" => include_bytes!("../icons/diff.svg"),
            "icons/history.svg" => include_bytes!("../icons/history.svg"),
            "icons/grid.svg" => include_bytes!("../icons/grid.svg"),
            "icons/info.svg" => include_bytes!("../icons/info.svg"),
            "icons/window.svg" => include_bytes!("../icons/window.svg"),
            "icons/check.svg" => include_bytes!("../icons/check.svg"),
            _ => return Ok(None),
        };
        Ok(Some(Cow::Borrowed(bytes)))
    }

    fn list(&self, _path: &str) -> gpui::Result<Vec<SharedString>> {
        // No caller enumerates the icon set at runtime (every use site names
        // an exact `Icon` variant) -- an empty list is a truthful answer,
        // not a stub.
        Ok(Vec::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every [`Icon`] variant's [`Icon::asset_path`] must resolve through
    /// [`Assets::load`] to real, non-empty bytes -- the "アイコン名→
    /// アセット解決" gate from the wave's brief. A variant added to
    /// [`Icon`] without a matching `Assets::load` arm (or vice versa) fails
    /// here immediately instead of silently rendering a blank icon at
    /// runtime.
    #[test]
    fn every_icon_variant_resolves_to_non_empty_bytes() {
        let assets = Assets;
        for icon in Icon::ALL {
            let path = icon.asset_path();
            let bytes = assets
                .load(path)
                .unwrap_or_else(|err| panic!("Assets::load({path:?}) errored: {err}"))
                .unwrap_or_else(|| panic!("Assets::load({path:?}) resolved to nothing"));
            assert!(!bytes.is_empty(), "{path:?} resolved to empty bytes");
        }
    }

    /// Every embedded file is at least well-formed enough to be an SVG
    /// document (starts with `<svg`) -- catches an accidental wrong-file
    /// `include_bytes!` (e.g. pointing at a `.png`) that the byte-count
    /// check above wouldn't.
    #[test]
    fn every_icon_asset_looks_like_svg() {
        let assets = Assets;
        for icon in Icon::ALL {
            let path = icon.asset_path();
            let bytes = assets.load(path).unwrap().unwrap();
            let text = std::str::from_utf8(&bytes).expect("SVG source must be valid UTF-8");
            assert!(
                text.trim_start().starts_with("<svg"),
                "{path:?} doesn't look like an SVG document: {text:?}"
            );
        }
    }

    /// Every [`Icon`] variant maps to a distinct path, and every path lives
    /// under the `icons/` prefix [`Assets::load`]'s `match` (and every
    /// caller's mental model of "where do icons live") assumes.
    #[test]
    fn asset_paths_are_unique_and_namespaced() {
        let paths: Vec<&str> = Icon::ALL.iter().map(|icon| icon.asset_path()).collect();
        let mut seen = std::collections::HashSet::new();
        for path in &paths {
            assert!(path.starts_with("icons/"), "{path:?} not under icons/");
            assert!(seen.insert(*path), "duplicate asset path: {path:?}");
        }
    }

    /// An unknown path is a normal "not found" (`Ok(None)`), not an error --
    /// matches [`AssetSource`]'s contract (`()`'s own impl returns
    /// `Ok(None)` unconditionally) and lets gpui's own error path (rather
    /// than a panic here) handle a genuinely-missing asset.
    #[test]
    fn unknown_path_resolves_to_none_not_an_error() {
        let assets = Assets;
        assert!(assets.load("icons/does-not-exist.svg").unwrap().is_none());
        assert!(assets.list("icons/").unwrap().is_empty());
    }
}
