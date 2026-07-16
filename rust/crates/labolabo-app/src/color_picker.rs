//! 色の選択 UI (第10波 パーソナライズ): プリセット 8 色 + 「なし」+
//! カスタム hex のスウォッチパネルと、それを使う 2 つの対象 --
//!
//! 1. **タスクの色** (`Task::color`): タスク行「…」メニューの
//!    「色を設定…」(`task_menu::TaskMenuPhase::ColorPick`) から。
//! 2. **端末タブの色** (`PaneItem::color`): タブチップの右クリックで開く
//!    ポップオーバー ([`TabColorMenuState`] / [`render_tab_color_overlay`])
//!    から。レイアウト JSON に `color`/`paneColor` キーとして永続化される
//!    (互換性の契約は `labolabo_core::tiling::PanePayload::color` の doc
//!    コメント参照)。
//!
//! どちらの対象でも選択の適用は `LaboLaboApp::pick_color`(いま開いている
//! ピッカーがどちらかを見て振り分ける)に集約されているので、パネルの
//! 描画 ([`render_color_swatch_panel`]) は対象を知らない 1 実装で済む。
//!
//! 純ロジック([`normalize_hex_color`]/[`parse_hex_rgb`])は gpui 非依存で
//! ユニットテスト済み。

use gpui::{
    div, prelude::*, px, rgb, rgba, Animation, AnimationExt, AnyElement, App, Context, IntoElement,
    MouseButton, MouseDownEvent, Pixels, Point, SharedString, Window,
};
use rust_i18n::t;

use labolabo_core::PaneId;

use crate::app::LaboLaboApp;
use crate::motion;
use crate::task_menu::clamp_popover_origin;
use crate::text_field::{render_text_field, TextFieldState};
use crate::theme;

/// プリセット 8 色 (指示のパレットそのまま: LaboLabo ライム / ブルー /
/// グリーン / イエロー / オレンジ / レッド / パープル / グレー)。
/// 永続化形式 (`#rrggbb` 小文字) と描画用 `u32` の対で持つ。
pub const PRESET_COLORS: [(&str, u32); 8] = [
    ("#d0ff00", 0xd0ff00), // LaboLabo ライム (theme::BRAND と同値)
    ("#5e9eff", 0x5e9eff), // ブルー (theme::ACCENT と同値)
    ("#30d158", 0x30d158), // グリーン
    ("#ffd60a", 0xffd60a), // イエロー
    ("#ff9f0a", 0xff9f0a), // オレンジ
    ("#ff6b6b", 0xff6b6b), // レッド
    ("#bf5af2", 0xbf5af2), // パープル
    ("#8e8e93", 0x8e8e93), // グレー
];

/// ユーザー入力の hex 文字列を永続化形式へ正規化する: 前後空白と先頭 `#`
/// の有無を許容し、6 桁の 16 進数のみ受け付けて小文字 `#rrggbb` にする。
/// 3 桁ショートハンド (`#fff`) やアルファ付き 8 桁は受け付けない(受理
/// 形式は保存形式と 1:1 のシンプルさを優先)。
pub fn normalize_hex_color(input: &str) -> Option<String> {
    let trimmed = input.trim();
    let hex = trimmed.strip_prefix('#').unwrap_or(trimmed);
    if hex.len() != 6 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    Some(format!("#{}", hex.to_ascii_lowercase()))
}

/// 永続化された `#rrggbb` 文字列を描画用の `0xRRGGBB` へ。不正な文字列
/// (手編集・将来の別形式)は `None` = 「色なしとして描画」に落とす --
/// この crate 系の「不正な永続データは静かにデフォルトへ」の姿勢。
pub fn parse_hex_rgb(color: &str) -> Option<u32> {
    let hex = color.strip_prefix('#')?;
    if hex.len() != 6 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    u32::from_str_radix(hex, 16).ok()
}

/// 右クリックされたタブチップの色ポップオーバーの全状態
/// (`LaboLaboApp::tab_color_menu`)。`hex_field` はカスタム色のテキスト
/// 入力(開いている間 `LaboLaboApp::active_text_field` がこれを指す)。
#[derive(Debug, Clone, PartialEq)]
pub struct TabColorMenuState {
    pub task_id: String,
    pub pane_id: PaneId,
    /// 右クリック位置(ウィンドウ座標) -- ポップオーバーのアンカー。
    pub anchor: Point<Pixels>,
    pub hex_field: TextFieldState,
    /// 直前の「適用」が不正な hex で失敗したか。入力が変わるとクリア
    /// (`LaboLaboApp::clear_hex_error`)。
    pub hex_error: bool,
}

impl TabColorMenuState {
    pub fn new(
        task_id: impl Into<String>,
        pane_id: PaneId,
        anchor: Point<Pixels>,
        current: Option<String>,
    ) -> Self {
        Self {
            task_id: task_id.into(),
            pane_id,
            anchor,
            // 現在色をプリフィル -- 微調整(1 桁だけ変える等)がしやすい。
            hex_field: TextFieldState::new(current.unwrap_or_default()),
            hex_error: false,
        }
    }
}

/// スウォッチ 1 つの辺長。
const SWATCH_SIZE: f32 = 22.0;
/// パネルの幅(スウォッチ 4 列 + 余白が収まる値)。
pub const PANEL_WIDTH: f32 = 208.0;

/// プリセット 8 色のグリッド + 「なし」+ カスタム hex 行。選択は
/// `LaboLaboApp::pick_color` / `LaboLaboApp::apply_custom_hex_color` に
/// 集約(モジュール doc コメント)。`current` は現在の色(スウォッチの
/// 選択リング表示にだけ使う)。
pub fn render_color_swatch_panel(
    id_prefix: &'static str,
    current: Option<&str>,
    hex_field: &TextFieldState,
    hex_error: bool,
    focus_handle: &gpui::FocusHandle,
    cx: &mut Context<LaboLaboApp>,
) -> impl IntoElement {
    let mut grid = div().flex().flex_row().flex_wrap().gap_2().px_2();
    for (hex, rgb_value) in PRESET_COLORS {
        let is_current = current == Some(hex);
        grid = grid.child(
            div()
                .id(SharedString::from(format!(
                    "{id_prefix}-swatch-{}",
                    &hex[1..]
                )))
                .w(px(SWATCH_SIZE))
                .h(px(SWATCH_SIZE))
                .rounded_sm()
                .bg(rgb(rgb_value))
                .border_2()
                .border_color(rgb(if is_current {
                    theme::text::PRIMARY
                } else {
                    theme::surface::STROKE
                }))
                .hover(|el| el.border_color(rgb(theme::text::SECONDARY)))
                .active(|el| el.opacity(0.8))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _: &MouseDownEvent, _window, cx| {
                        this.pick_color(Some(hex.to_string()), cx);
                    }),
                ),
        );
    }

    // 「なし」(色を外す)。
    let none_row = div()
        .id(SharedString::from(format!("{id_prefix}-color-none")))
        .mx_2()
        .px_2()
        .py_1()
        .rounded_sm()
        .text_size(px(theme::font_size::LABEL))
        .text_color(rgb(theme::text::PRIMARY))
        .bg(rgb(theme::surface::RAISED))
        .border_1()
        .border_color(rgb(if current.is_none() {
            theme::text::SECONDARY
        } else {
            theme::surface::STROKE
        }))
        .hover(|el| el.bg(rgb(theme::surface::ACTIVE)))
        .active(|el| el.opacity(0.8))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _: &MouseDownEvent, _window, cx| {
                this.pick_color(None, cx);
            }),
        )
        .child(t!("color.none").to_string());

    // カスタム hex: 入力欄 + 適用ボタン。
    let hex_row = div()
        .flex()
        .flex_row()
        .items_center()
        .gap_2()
        .px_2()
        .child(render_text_field(
            SharedString::from(format!("{id_prefix}-hex-field")),
            hex_field,
            SharedString::from("#rrggbb"),
            focus_handle,
            cx,
        ))
        .child(
            div()
                .id(SharedString::from(format!("{id_prefix}-hex-apply")))
                .px_2()
                .py_1()
                .flex_shrink_0()
                .rounded_sm()
                .bg(rgb(theme::surface::RAISED))
                .text_size(px(theme::font_size::LABEL))
                .text_color(rgb(theme::text::PRIMARY))
                .hover(|el| el.bg(rgb(theme::surface::ACTIVE)))
                .active(|el| el.opacity(0.8))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _: &MouseDownEvent, _window, cx| {
                        this.apply_custom_hex_color(cx);
                    }),
                )
                .child(t!("color.custom_apply").to_string()),
        );

    let mut panel = div()
        .flex()
        .flex_col()
        .gap_2()
        .py_2()
        .child(grid)
        .child(none_row)
        .child(
            div()
                .px_2()
                .text_size(px(theme::font_size::CAPTION))
                .text_color(rgb(theme::text::MUTED))
                .child(t!("color.custom_label").to_string()),
        )
        .child(hex_row);

    if hex_error {
        panel = panel.child(
            div()
                .px_2()
                .text_size(px(theme::font_size::CAPTION))
                .text_color(rgb(theme::status::CONFLICT))
                .child(t!("color.invalid_hex").to_string()),
        );
    }
    panel
}

/// タブ色ポップオーバー(`app.tab_color_menu()` が `Some` のときだけ
/// `Some`)。`task_menu::render_menu_popover` と同じ「暗幕 + アンカー位置に
/// クランプしたパネル + 1 段フェード」構成。
pub fn render_tab_color_overlay(
    app: &LaboLaboApp,
    window: &Window,
    cx: &mut Context<LaboLaboApp>,
) -> Option<AnyElement> {
    let state = app.tab_color_menu()?.clone();
    let current = app.tab_color(&state.task_id, state.pane_id);

    let panel = div()
        .flex()
        .flex_col()
        .w(px(PANEL_WIDTH))
        .rounded(px(theme::radius::OVERLAY))
        .bg(rgb(theme::surface::RAISED))
        .border_1()
        .border_color(rgb(theme::surface::STROKE))
        .shadow(theme::shadow::overlay())
        .on_mouse_down(MouseButton::Left, |_event, _window, cx: &mut App| {
            cx.stop_propagation();
        })
        .child(
            div()
                .px_2()
                .pt_2()
                .text_size(px(theme::font_size::CAPTION))
                .text_color(rgb(theme::text::MUTED))
                .child(t!("color.tab_menu_title").to_string()),
        )
        .child(render_color_swatch_panel(
            "tab-color",
            current.as_deref(),
            &state.hex_field,
            state.hex_error,
            &app.focus_handle().clone(),
            cx,
        ));

    // 高さの見積り: ヘッダ + スウォッチ 2 行 + なし + ラベル + hex 行 +
    // (エラー行)。クランプ用のおおよそで足りる(`task_menu` の
    // `estimated_height` と同じ精度感)。
    let estimated_height = 190.0 + if state.hex_error { 18.0 } else { 0.0 };
    let origin = clamp_popover_origin(
        state.anchor,
        gpui::size(px(PANEL_WIDTH), px(estimated_height)),
        window.viewport_size(),
    );

    let positioned = div().absolute().left(origin.x).top(origin.y).child(panel);

    Some(
        div()
            .absolute()
            .inset_0()
            .bg(rgba(theme::OVERLAY_SCRIM))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _: &MouseDownEvent, _window, cx| {
                    this.close_tab_color_menu(cx);
                }),
            )
            .child(positioned)
            .with_animation(
                "tab-color-backdrop-enter",
                Animation::new(motion::OVERLAY_ENTER).with_easing(motion::ease_out_strong()),
                |el, t| el.opacity(t),
            )
            .into_any_element(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    // MARK: - normalize_hex_color

    #[test]
    fn normalize_accepts_hash_prefixed_and_bare_six_digit_hex() {
        assert_eq!(
            normalize_hex_color("#D0FF00").as_deref(),
            Some("#d0ff00"),
            "uppercase is lowercased"
        );
        assert_eq!(normalize_hex_color("5e9eff").as_deref(), Some("#5e9eff"));
        assert_eq!(
            normalize_hex_color("  #30d158  ").as_deref(),
            Some("#30d158"),
            "surrounding whitespace is tolerated"
        );
    }

    #[test]
    fn normalize_rejects_everything_else() {
        for bad in [
            "",
            "#",
            "#fff",
            "fff",
            "#d0ff0",
            "#d0ff000",
            "#d0ff0g",
            "red",
            "#d0 f00",
            "#d0ff00aa",
        ] {
            assert_eq!(normalize_hex_color(bad), None, "should reject {bad:?}");
        }
    }

    /// 正規化した文字列はそのままパースできる(保存形式と描画の整合)。
    #[test]
    fn normalized_output_always_parses() {
        let normalized = normalize_hex_color("#D0FF00").unwrap();
        assert_eq!(parse_hex_rgb(&normalized), Some(0xd0ff00));
    }

    // MARK: - parse_hex_rgb

    #[test]
    fn parse_hex_rgb_decodes_the_persisted_format() {
        assert_eq!(parse_hex_rgb("#d0ff00"), Some(0xd0ff00));
        assert_eq!(parse_hex_rgb("#8e8e93"), Some(0x8e8e93));
        assert_eq!(parse_hex_rgb("#000000"), Some(0x000000));
    }

    #[test]
    fn parse_hex_rgb_degrades_to_none_on_foreign_text() {
        for bad in ["", "d0ff00", "#fff", "#zzzzzz", "not-a-color", "#d0ff00aa"] {
            assert_eq!(parse_hex_rgb(bad), None, "should reject {bad:?}");
        }
    }

    /// プリセット表の 2 表現(`#rrggbb` 文字列と `u32`)が食い違っていない
    /// ことを機械的に固定する。
    #[test]
    fn preset_table_hex_strings_match_their_u32_values() {
        for (hex, value) in PRESET_COLORS {
            assert_eq!(parse_hex_rgb(hex), Some(value), "{hex} mismatch");
            assert_eq!(
                normalize_hex_color(hex).as_deref(),
                Some(hex),
                "{hex} should already be in normalized form"
            );
        }
        assert_eq!(PRESET_COLORS[0].1, theme::BRAND, "先頭は LaboLabo ライム");
    }
}
