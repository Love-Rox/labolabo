//! 単一行テキスト入力 (第10波 パーソナライズ): タスクの「名前を変更…」と
//! カスタム hex 色の入力欄。
//!
//! ## 設計: アプリ唯一の `EntityInputHandler` に相乗りする
//!
//! gpui にはテキスト入力ウィジェットが無く、テキスト/IME は
//! `EntityInputHandler` を実装した entity へ届く(W5e で端末ペイン向けに
//! `LaboLaboApp` へ実装済み -- `app.rs` の impl doc コメント参照)。この
//! 入力欄は独立した entity にはせず、**開いている間だけ**
//! `LaboLaboApp::active_text_field` が `Some` を返し、既存の
//! `EntityInputHandler` 実装の各メソッドが PTY ではなくこの
//! [`TextFieldState`] へルーティングする -- という形で同じ実装に相乗り
//! する。入力欄は常にモーダルオーバーレイの中にしか存在しない(同時に
//! 端末へタイプできる状態が無い)ので、「アクティブな入力欄があるか」の
//! 一点だけで曖昧さなく振り分けられる。
//!
//! IME(日本語入力)もそのまま動く: `replace_and_mark_text_in_range` が
//! [`TextFieldState::preedit`] を更新し、確定 (`replace_text_in_range`) で
//! `text` に追記される。候補ウィンドウの位置は、この入力欄の描画
//! ([`render_text_field`] 内の `canvas`)が毎フレーム自分の bounds で
//! `Window::handle_input` を呼び直すことで正しくなる -- gpui は同一
//! フレームに複数登録された input handler の**最後の 1 つ**を使う
//! (`Window::next_frame.input_handlers.pop()`)ため、オーバーレイ(ツリー
//! 末尾 = 最後に paint)の登録が端末ペインのものに常に勝つ。
//!
//! ## 編集モデル: 末尾挿入のみ
//!
//! キャレットは常に末尾固定(挿入は append、Backspace は末尾 1 文字削除、
//! ←→ でのキャレット移動は無し)。リネームと 7 桁の hex 入力という用途には
//! 十分で、キャレットを文中に置けるようにするにはテキスト幅の実測
//! (`TextSystem` でのレイアウト)が必要になり、この小さな入力欄には過剰
//! なため意図的に採らない。同じ理由でキャレットの点滅もしない(静的な
//! 1px バー -- `motion.rs` の電力原則にも沿う)。
//!
//! 状態遷移(commit/backspace/preedit)は gpui 非依存の純ロジックで、下の
//! ユニットテストが固定する。

use gpui::{
    canvas, div, prelude::*, px, rgb, ElementInputHandler, FocusHandle, IntoElement, SharedString,
};

use crate::app::LaboLaboApp;
use crate::theme;

/// 開いている入力欄 1 つ分の状態。`LaboLaboApp` 側のオーバーレイ状態
/// (`TaskMenuPhase::Rename`/`ColorPick`、`color_picker::TabColorMenuState`)
/// が値として保持する。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TextFieldState {
    /// 確定済みのテキスト。
    pub text: String,
    /// IME 変換中(preedit)のテキスト。`None` = 変換中でない。
    pub preedit: Option<String>,
}

impl TextFieldState {
    pub fn new(initial: impl Into<String>) -> Self {
        Self {
            text: initial.into(),
            preedit: None,
        }
    }

    /// 確定入力: `text` の末尾へ追記し、進行中の変換をクリアする
    /// (`EntityInputHandler::replace_text_in_range` から呼ばれる --
    /// プレーンな 1 文字タイプも IME の確定もこの 1 経路)。
    pub fn commit(&mut self, text: &str) {
        self.preedit = None;
        self.text.push_str(text);
    }

    /// IME の変換中テキスト更新 (`replace_and_mark_text_in_range`)。空文字
    /// は「変換が消えた」として `None` に潰す(端末側の実装と同じ規約)。
    pub fn set_preedit(&mut self, preedit: Option<String>) {
        self.preedit = preedit.filter(|p| !p.is_empty());
    }

    /// Backspace: 末尾 1 文字(バイトではなく `char`)を削除。変換中は
    /// no-op -- 変換中の Backspace は IME 自身が処理し、`set_preedit` で
    /// 反映される(こちらへは届かない想定だが、届いても壊さない)。
    pub fn backspace(&mut self) {
        if self.preedit.is_some() {
            return;
        }
        self.text.pop();
    }
}

/// 入力欄の描画。`state` の表示に加え、`canvas` の paint で毎フレーム
/// `Window::handle_input` を自分の bounds で登録し直す(モジュール doc
/// コメントの「最後の登録が勝つ」設計)。呼び出し側はこの欄が見えている
/// 間、`LaboLaboApp::active_text_field` が同じ `state` を指していることを
/// 保証する(でなければタイプしても何も起きない)。
pub fn render_text_field(
    id: impl Into<SharedString>,
    state: &TextFieldState,
    placeholder: SharedString,
    focus_handle: &FocusHandle,
    cx: &mut Context<LaboLaboApp>,
) -> impl IntoElement {
    let entity = cx.entity();
    let focus_handle = focus_handle.clone();
    let input_registrar = canvas(
        |_bounds, _window, _cx| {},
        move |bounds, _prepaint, window, cx| {
            window.handle_input(&focus_handle, ElementInputHandler::new(bounds, entity), cx);
        },
    )
    .absolute()
    .size_full();

    let show_placeholder = state.text.is_empty() && state.preedit.is_none();

    let mut field = div()
        .id(id.into())
        .relative()
        .flex()
        .flex_row()
        .items_center()
        .h(px(26.0))
        .px_2()
        .w_full()
        .overflow_hidden()
        .rounded_sm()
        .bg(rgb(theme::surface::SUNKEN))
        .border_1()
        .border_color(rgb(theme::BRAND_DIM))
        .text_size(px(theme::font_size::LABEL))
        .child(input_registrar);

    if show_placeholder {
        field = field.child(div().text_color(rgb(theme::text::MUTED)).child(placeholder));
    } else {
        if !state.text.is_empty() {
            field = field.child(
                div()
                    .text_color(rgb(theme::text::PRIMARY))
                    .child(SharedString::from(state.text.clone())),
            );
        }
        if let Some(preedit) = &state.preedit {
            // 変換中テキストは下線 + ACCENT で「未確定」を示す(端末側の
            // preedit オーバーレイ `render::paint_preedit` と同じ意味付け)。
            field = field.child(
                div()
                    .text_color(rgb(theme::ACCENT))
                    .underline()
                    .child(SharedString::from(preedit.clone())),
            );
        }
    }

    // 静的キャレット(常に末尾)。
    field.child(
        div()
            .w(px(1.0))
            .h(px(14.0))
            .flex_shrink_0()
            .bg(rgb(theme::BRAND)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commit_appends_at_the_end() {
        let mut f = TextFieldState::new("ab");
        f.commit("c");
        assert_eq!(f.text, "abc");
        assert!(f.preedit.is_none());
    }

    #[test]
    fn commit_clears_an_in_progress_composition() {
        // IME 確定の実シーケンス: preedit "にほんご" → commit "日本語"。
        let mut f = TextFieldState::new("");
        f.set_preedit(Some("にほんご".to_string()));
        assert!(f.preedit.is_some());
        f.commit("日本語");
        assert_eq!(f.text, "日本語");
        assert_eq!(f.preedit, None);
    }

    #[test]
    fn backspace_removes_one_character_not_one_byte() {
        let mut f = TextFieldState::new("あab");
        f.backspace();
        assert_eq!(f.text, "あa");
        f.backspace();
        f.backspace();
        assert_eq!(f.text, "");
        // 空でもう一度押しても panic しない。
        f.backspace();
        assert_eq!(f.text, "");
    }

    #[test]
    fn backspace_is_a_no_op_while_composing() {
        let mut f = TextFieldState::new("x");
        f.set_preedit(Some("か".to_string()));
        f.backspace();
        assert_eq!(f.text, "x");
        assert_eq!(f.preedit.as_deref(), Some("か"));
    }

    #[test]
    fn empty_preedit_collapses_to_none() {
        let mut f = TextFieldState::new("x");
        f.set_preedit(Some("か".to_string()));
        f.set_preedit(Some(String::new()));
        assert_eq!(f.preedit, None);
    }

    #[test]
    fn preedit_updates_replace_the_previous_composition() {
        let mut f = TextFieldState::new("");
        f.set_preedit(Some("に".to_string()));
        f.set_preedit(Some("にほ".to_string()));
        assert_eq!(f.preedit.as_deref(), Some("にほ"));
        assert_eq!(f.text, "");
    }
}
