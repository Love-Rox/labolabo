//! タスク行の「…」メニューと削除確認オーバーレイ (wave 6c §2)。
//!
//! サイドバーのタスク行ホバーで出る「…」アイコンボタン（`sidebar.rs`）が
//! [`TaskMenuState`] を開き、ここが 3 つの相を描く:
//!
//! 1. **Menu**: クリック位置に出す小さなポップオーバー。「IDE で開く」
//!    （macOS のみ、インストール済みエディタの列挙 + Finder で表示 --
//!    `crate::ide_open`）/「アーカイブ」/「削除…」。Zed の ContextMenu
//!    (GPL) は参照せず、settings.rs のオーバーレイパターン（開いている間
//!    だけ要素が存在する）を流用した自前の最小実装。
//! 2. **ConfirmDelete**: 削除の確認モーダル（settings.rs と同じ中央
//!    パネル + 薄暗幕）。attached 型は「登録を解除します。ディレクトリの
//!    ファイルには触れません」（DB からの削除のみ）、worktree 型は
//!    「worktree を削除しますか？」+「ブランチも削除」チェックボックス
//!    （既定 off）。実行結果（未コミット変更による拒否等）はこの
//!    オーバーレイ内に表示する。
//! 3. **Notice**: worktree は消えたがブランチ削除だけ失敗した、等の
//!    後日談表示。
//!
//! 相遷移（[`TaskMenuState`] のメソッド群）は gpui 非依存の純ロジックで、
//! 下のユニットテストが状態機械を固定する。git の実処理は
//! `crate::task_lifecycle`、フローの配線は `app.rs`
//! （`execute_delete_task` / `finish_worktree_delete`）。
//!
//! ## クリック伝播の設計
//!
//! バックドロップ（全面）に「閉じる」ハンドラ、パネル自身に
//! `stop_propagation` だけのハンドラを置く。gpui のマウスイベントは
//! 深い要素から順に bubble するので、行ハンドラ → パネル（伝播停止）→
//! バックドロップ（届かない）の順になり、パネル内クリックでメニューが
//! 閉じることはない。

use gpui::{
    div, prelude::*, px, rgb, rgba, Animation, AnimationExt, AnyElement, App, Context, IntoElement,
    MouseButton, MouseDownEvent, Pixels, Point, SharedString, Size, Window,
};
use rust_i18n::t;

use labolabo_core::{Task, TaskKind};

use crate::app::LaboLaboApp;
use crate::color_picker;
use crate::motion;
use crate::path_abbrev;
use crate::text_field::{render_text_field, TextFieldState};
use crate::theme;

/// ポップオーバーの幅。
const MENU_WIDTH: f32 = 240.0;
/// ポップオーバーの 1 行分の高さ（クランプ用の見積り）。
const MENU_ROW_HEIGHT: f32 = 26.0;
/// 確認モーダルの幅（settings.rs の PANEL_WIDTH と同じ）。
const CONFIRM_WIDTH: f32 = 420.0;
const OVERLAY_BG: u32 = theme::OVERLAY_SCRIM;

/// worktree 型タスクの削除に必要な git 情報（メニューを開いた時点で
/// スナップショット -- 削除完了後もタスク本体を引き直さずに表示できる）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeInfo {
    pub branch: String,
    pub path: String,
    pub repo_root: String,
}

/// メニューの相。遷移は [`TaskMenuState`] のメソッド経由のみ。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskMenuPhase {
    /// 項目一覧のポップオーバー。
    Menu,
    /// 「名前を変更…」の小モーダル (第10波)。`field` は開いた時点の
    /// タイトルをプリフィルした単一行入力（開いている間
    /// `LaboLaboApp::active_text_field` がこれを指し、タイプ/IME が
    /// ここへルーティングされる -- `text_field.rs` のモジュール doc
    /// コメント参照）。
    Rename { field: TextFieldState },
    /// 「色を設定…」のスウォッチポップオーバー (第10波、
    /// `color_picker::render_color_swatch_panel`)。`hex_field` はカスタム
    /// hex 入力、`hex_error` は直前の「適用」が不正な hex で失敗したか。
    ColorPick {
        hex_field: TextFieldState,
        hex_error: bool,
    },
    /// 削除確認モーダル。`in_flight` の間はキャンセル/再実行/チェック変更
    /// 不可、`error` は直近の失敗（拒否）メッセージ。
    ConfirmDelete {
        delete_branch: bool,
        in_flight: bool,
        error: Option<String>,
    },
    /// 完了後の後日談（ブランチ削除失敗など）。
    Notice { message: String },
}

/// 開いているタスクメニューの全状態（`LaboLaboApp::task_menu`）。
#[derive(Debug, Clone, PartialEq)]
pub struct TaskMenuState {
    pub task_id: String,
    pub title: String,
    /// `Some` = worktree 型。`None` = attached 型（実ディレクトリには
    /// 絶対に触れない削除フローになる）。
    pub worktree: Option<WorktreeInfo>,
    /// メニューを開いた時点で作業ディレクトリが見つからなかったか
    /// （`LaboLaboApp::is_task_missing` のスナップショット、第8波c）。
    /// worktree 型の削除確認文言を分岐させる（[`render_confirm_modal`]）のと、
    /// `execute_delete_task` が `crate::task_lifecycle::
    /// remove_worktree_and_maybe_branch`（通常の `git worktree remove`）
    /// ではなく `prune_missing_worktree_and_maybe_branch`（ディレクトリが
    /// 無い前提で `git worktree prune` から始める経路）を選ぶのに使う。
    /// attached 型はどちらにせよ実ディレクトリに触れないため無関係。
    pub missing: bool,
    /// 「…」ボタンのクリック位置（ウィンドウ座標）。ポップオーバーの
    /// アンカー。
    pub anchor: Point<Pixels>,
    pub phase: TaskMenuPhase,
}

impl TaskMenuState {
    pub fn new(task: &Task, anchor: Point<Pixels>, missing: bool) -> Self {
        let worktree = match &task.kind {
            TaskKind::Worktree { branch, path, .. } => Some(WorktreeInfo {
                branch: branch.clone(),
                path: path.clone(),
                repo_root: task.repo_root.clone(),
            }),
            TaskKind::Attached { .. } => None,
        };
        Self {
            task_id: task.id.clone(),
            title: task.title.clone(),
            worktree,
            missing,
            anchor,
            phase: TaskMenuPhase::Menu,
        }
    }

    /// Menu → Rename（現在タイトルをプリフィル）。他の相では no-op。
    pub fn request_rename(&mut self) {
        if self.phase == TaskMenuPhase::Menu {
            self.phase = TaskMenuPhase::Rename {
                field: TextFieldState::new(self.title.clone()),
            };
        }
    }

    /// Menu → ColorPick（hex 入力は現在のカスタム色をプリフィル --
    /// 1 桁だけ変える微調整がしやすい）。他の相では no-op。
    pub fn request_color_pick(&mut self, current_color: Option<String>) {
        if self.phase == TaskMenuPhase::Menu {
            self.phase = TaskMenuPhase::ColorPick {
                hex_field: TextFieldState::new(current_color.unwrap_or_default()),
                hex_error: false,
            };
        }
    }

    /// Rename 相の入力の確定値: 前後空白を落としたタイトル。空(=確定
    /// 不可)なら `None`。Rename 相以外も `None`。
    pub fn rename_result(&self) -> Option<String> {
        let TaskMenuPhase::Rename { field } = &self.phase else {
            return None;
        };
        let trimmed = field.text.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    }

    /// Menu → ConfirmDelete（既定: ブランチ削除 off）。他の相では no-op。
    pub fn request_delete(&mut self) {
        if self.phase == TaskMenuPhase::Menu {
            self.phase = TaskMenuPhase::ConfirmDelete {
                delete_branch: false,
                in_flight: false,
                error: None,
            };
        }
    }

    /// 「ブランチも削除」のトグル。worktree 型の ConfirmDelete で、実行中
    /// でないときのみ。
    pub fn toggle_delete_branch(&mut self) {
        if self.worktree.is_none() {
            return;
        }
        if let TaskMenuPhase::ConfirmDelete {
            delete_branch,
            in_flight: false,
            ..
        } = &mut self.phase
        {
            *delete_branch = !*delete_branch;
        }
    }

    /// 実行開始。ConfirmDelete かつ未実行のときだけ `true`（in_flight を
    /// 立て、前回のエラーをクリア）。二度押し・他相からの呼び出しは `false`。
    pub fn begin_execution(&mut self) -> bool {
        if let TaskMenuPhase::ConfirmDelete {
            in_flight: in_flight @ false,
            error,
            ..
        } = &mut self.phase
        {
            *in_flight = true;
            *error = None;
            true
        } else {
            false
        }
    }

    /// 実行失敗（未コミット変更による拒否等）: in_flight を下ろして
    /// メッセージを確認モーダル内に表示する（タスクは残っている）。
    pub fn fail(&mut self, message: String) {
        if let TaskMenuPhase::ConfirmDelete {
            in_flight, error, ..
        } = &mut self.phase
        {
            *in_flight = false;
            *error = Some(message);
        }
    }

    /// 完了後の後日談へ（worktree は消えたがブランチ削除は失敗、等）。
    pub fn show_notice(&mut self, message: String) {
        self.phase = TaskMenuPhase::Notice { message };
    }

    /// 「ブランチも削除」が要求されているか。
    pub fn delete_branch_requested(&self) -> bool {
        matches!(
            self.phase,
            TaskMenuPhase::ConfirmDelete {
                delete_branch: true,
                ..
            }
        )
    }

    /// バックドロップクリック等で閉じてよいか（git 実行中は閉じない --
    /// 完了/失敗の表示先が消えてしまうため）。
    pub fn can_dismiss(&self) -> bool {
        !matches!(
            self.phase,
            TaskMenuPhase::ConfirmDelete {
                in_flight: true,
                ..
            }
        )
    }
}

/// ポップオーバーの左上位置: アンカー基準で、ビューポートからはみ出す分を
/// 内側へクランプする純関数。
pub fn clamp_popover_origin(
    anchor: Point<Pixels>,
    panel: Size<Pixels>,
    viewport: Size<Pixels>,
) -> Point<Pixels> {
    let clamp = |pos: f32, extent: f32, limit: f32| -> f32 {
        let max = (limit - extent).max(0.0);
        pos.clamp(0.0, max)
    };
    Point {
        x: px(clamp(
            f32::from(anchor.x),
            f32::from(panel.width),
            f32::from(viewport.width),
        )),
        y: px(clamp(
            f32::from(anchor.y),
            f32::from(panel.height),
            f32::from(viewport.height),
        )),
    }
}

// MARK: - 描画

/// タスクメニュー（`app.task_menu()` が `Some` のときだけ `Some`）。
/// `app.rs` の `Render` がルートツリー末尾へ `.children(..)` で足す。
pub fn render_task_menu_overlay(
    app: &LaboLaboApp,
    window: &Window,
    cx: &mut Context<LaboLaboApp>,
) -> Option<AnyElement> {
    let state = app.task_menu()?.clone();
    let element = match &state.phase {
        TaskMenuPhase::Menu => render_menu_popover(app, &state, window, cx),
        TaskMenuPhase::Rename { field } => render_rename_modal(app, &state, field, cx),
        TaskMenuPhase::ColorPick {
            hex_field,
            hex_error,
        } => render_color_pick_popover(app, &state, hex_field, *hex_error, window, cx),
        TaskMenuPhase::ConfirmDelete {
            delete_branch,
            in_flight,
            error,
        } => render_confirm_modal(&state, *delete_branch, *in_flight, error.as_deref(), cx),
        TaskMenuPhase::Notice { message } => render_notice_modal(message, cx),
    };
    Some(element)
}

fn render_menu_popover(
    app: &LaboLaboApp,
    state: &TaskMenuState,
    window: &Window,
    cx: &mut Context<LaboLaboApp>,
) -> AnyElement {
    let task_id = state.task_id.clone();

    let mut panel = div()
        .flex()
        .flex_col()
        .py_1()
        .w(px(MENU_WIDTH))
        .rounded(px(theme::radius::OVERLAY))
        .bg(rgb(theme::surface::RAISED))
        .border_1()
        .border_color(rgb(theme::surface::STROKE))
        .shadow(theme::shadow::overlay())
        // パネル内クリックはバックドロップの「閉じる」まで届かせない
        // （module doc コメントのクリック伝播設計）。
        .on_mouse_down(MouseButton::Left, |_event, _window, cx: &mut App| {
            cx.stop_propagation();
        });

    let mut row_count: usize = 0;

    // IDE で開く（macOS のみ -- 非 macOS では項目自体を出さない）。
    #[cfg(target_os = "macos")]
    {
        panel = panel.child(
            div()
                .px_2()
                .py_1()
                .text_size(px(theme::font_size::CAPTION))
                .text_color(rgb(theme::text::MUTED))
                .child(t!("task.menu.ide_section_title").to_string()),
        );
        row_count += 1;
        for editor in app.installed_editors() {
            let editor_task_id = task_id.clone();
            let bundle_id = editor.bundle_id;
            panel = panel.child(menu_row(
                format!("task-menu-editor-{bundle_id}"),
                editor.name.into(),
                false,
                cx.listener(move |this, _: &MouseDownEvent, _window, cx| {
                    this.open_task_in_editor(&editor_task_id, bundle_id, cx);
                }),
            ));
            row_count += 1;
        }
        let finder_task_id = task_id.clone();
        panel = panel.child(menu_row(
            "task-menu-finder".to_string(),
            t!("task.menu.reveal_in_finder").to_string().into(),
            false,
            cx.listener(move |this, _: &MouseDownEvent, _window, cx| {
                this.reveal_task_in_finder(&finder_task_id, cx);
            }),
        ));
        row_count += 1;
        panel = panel.child(menu_separator());
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app; // installed_editors は macOS 分岐でのみ参照する
    }

    // 名前を変更… / 色を設定… (第10波 パーソナライズ)。
    panel = panel.child(menu_row(
        "task-menu-rename".to_string(),
        t!("task.menu.rename").to_string().into(),
        false,
        cx.listener(|this, _: &MouseDownEvent, _window, cx| {
            this.request_rename_task(cx);
        }),
    ));
    row_count += 1;
    panel = panel.child(menu_row(
        "task-menu-color".to_string(),
        t!("task.menu.set_color").to_string().into(),
        false,
        cx.listener(|this, _: &MouseDownEvent, _window, cx| {
            this.request_task_color_pick(cx);
        }),
    ));
    row_count += 1;

    let archive_task_id = task_id.clone();
    panel = panel.child(menu_row(
        "task-menu-archive".to_string(),
        t!("task.menu.archive").to_string().into(),
        false,
        cx.listener(move |this, _: &MouseDownEvent, window, cx| {
            this.archive_task(&archive_task_id, window, cx);
        }),
    ));
    row_count += 1;
    panel = panel.child(menu_row(
        "task-menu-delete".to_string(),
        t!("task.menu.delete").to_string().into(),
        true,
        cx.listener(move |this, _: &MouseDownEvent, _window, cx| {
            this.request_delete_task(cx);
        }),
    ));
    row_count += 1;

    let estimated_height = row_count as f32 * MENU_ROW_HEIGHT + 12.0;
    let origin = clamp_popover_origin(
        state.anchor,
        gpui::size(px(MENU_WIDTH), px(estimated_height)),
        window.viewport_size(),
    );

    let panel = div().absolute().left(origin.x).top(origin.y).child(panel);

    // `plans` 第8波a §5: 「…」メニューも設定/確認モーダルと同じ暗幕へ
    // 統一する(以前は透明でメニュー外クリックのみで閉じていた)。暗幕と
    // ポップオーバーを同じ 1 段のフェードで一緒に表示する(`settings.rs`
    // の `render_settings_overlay` と同じ、親 1 箇所だけに opacity
    // アニメーションを掛ける理由 -- 親子両方に掛けると `t * t` で合成され
    // 背景に遅れて見えてしまう)。
    div()
        .absolute()
        .inset_0()
        .bg(rgba(OVERLAY_BG))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _: &MouseDownEvent, _window, cx| {
                this.close_task_menu(cx);
            }),
        )
        .child(panel)
        .with_animation(
            "task-menu-backdrop-enter",
            Animation::new(motion::OVERLAY_ENTER).with_easing(motion::ease_out_strong()),
            |el, t| el.opacity(t),
        )
        .into_any_element()
}

/// 「名前を変更…」モーダル (第10波): タイトル + 単一行入力 + キャンセル/
/// 変更ボタン。Enter でも確定できる（`LaboLaboApp::key_down` の
/// text-input ゲート -- `confirm_text_input` 経由で同じ
/// `commit_rename_task` に落ちる）。入力が空(空白のみ)の間は「変更」を
/// 出さない -- `TaskMenuState::rename_result` の `None` と同じ判定。
fn render_rename_modal(
    app: &LaboLaboApp,
    state: &TaskMenuState,
    field: &TextFieldState,
    cx: &mut Context<LaboLaboApp>,
) -> AnyElement {
    let mut panel = div()
        .flex()
        .flex_col()
        .gap_3()
        .w(px(CONFIRM_WIDTH))
        .p_4()
        .rounded(px(theme::radius::OVERLAY))
        .bg(rgb(theme::surface::ROOT))
        .border_1()
        .border_color(rgb(theme::surface::STROKE))
        .shadow(theme::shadow::overlay())
        .on_mouse_down(MouseButton::Left, |_event, _window, cx: &mut App| {
            cx.stop_propagation();
        })
        .child(
            div()
                .text_size(px(15.0))
                .text_color(rgb(theme::text::PRIMARY))
                .child(t!("task.menu.rename_title", title = state.title).to_string()),
        )
        .child(render_text_field(
            "task-rename-field",
            field,
            t!("task.menu.rename_placeholder").to_string().into(),
            app.focus_handle(),
            cx,
        ));

    let mut buttons = div()
        .flex()
        .flex_row()
        .justify_end()
        .gap_2()
        .child(dialog_button(
            "task-rename-cancel",
            t!("task.menu.confirm.cancel").to_string().into(),
            false,
            cx.listener(|this, _: &MouseDownEvent, _window, cx| {
                this.close_task_menu(cx);
            }),
        ));
    if state.rename_result().is_some() {
        buttons = buttons.child(dialog_button(
            "task-rename-confirm",
            t!("task.menu.rename_confirm").to_string().into(),
            false,
            cx.listener(|this, _: &MouseDownEvent, _window, cx| {
                this.commit_rename_task(cx);
            }),
        ));
    }
    panel = panel.child(buttons);

    centered_backdrop(panel.into_any_element(), "task-rename-enter", cx)
}

/// 「色を設定…」ポップオーバー (第10波): メニューと同じアンカー位置に、
/// プリセットスウォッチ + なし + カスタム hex
/// (`color_picker::render_color_swatch_panel`)。
fn render_color_pick_popover(
    app: &LaboLaboApp,
    state: &TaskMenuState,
    hex_field: &TextFieldState,
    hex_error: bool,
    window: &Window,
    cx: &mut Context<LaboLaboApp>,
) -> AnyElement {
    let current = app
        .tasks()
        .iter()
        .find(|t| t.id == state.task_id)
        .and_then(|t| t.color.clone());

    let panel = div()
        .flex()
        .flex_col()
        .w(px(color_picker::PANEL_WIDTH))
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
                .child(t!("task.menu.set_color_title", title = state.title).to_string()),
        )
        .child(color_picker::render_color_swatch_panel(
            "task-color",
            current.as_deref(),
            hex_field,
            hex_error,
            app.focus_handle(),
            cx,
        ));

    let estimated_height = 190.0 + if hex_error { 18.0 } else { 0.0 };
    let origin = clamp_popover_origin(
        state.anchor,
        gpui::size(px(color_picker::PANEL_WIDTH), px(estimated_height)),
        window.viewport_size(),
    );
    let positioned = div().absolute().left(origin.x).top(origin.y).child(panel);

    div()
        .absolute()
        .inset_0()
        .bg(rgba(OVERLAY_BG))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _: &MouseDownEvent, _window, cx| {
                this.close_task_menu(cx);
            }),
        )
        .child(positioned)
        .with_animation(
            "task-color-backdrop-enter",
            Animation::new(motion::OVERLAY_ENTER).with_easing(motion::ease_out_strong()),
            |el, t| el.opacity(t),
        )
        .into_any_element()
}

fn render_confirm_modal(
    state: &TaskMenuState,
    delete_branch: bool,
    in_flight: bool,
    error: Option<&str>,
    cx: &mut Context<LaboLaboApp>,
) -> AnyElement {
    let title: SharedString = if state.worktree.is_some() {
        t!("task.menu.confirm.title_worktree").to_string().into()
    } else {
        t!("task.menu.confirm.title_attached", title = state.title)
            .to_string()
            .into()
    };

    let mut panel = div()
        .flex()
        .flex_col()
        .gap_3()
        .w(px(CONFIRM_WIDTH))
        .p_4()
        .rounded(px(theme::radius::OVERLAY))
        .bg(rgb(theme::surface::ROOT))
        .border_1()
        .border_color(rgb(theme::surface::STROKE))
        .shadow(theme::shadow::overlay())
        .on_mouse_down(MouseButton::Left, |_event, _window, cx: &mut App| {
            cx.stop_propagation();
        })
        .child(
            div()
                .text_size(px(15.0))
                .text_color(rgb(theme::text::PRIMARY))
                .child(title),
        );

    match &state.worktree {
        Some(info) => {
            // 第8波c: ディレクトリが既に見つからない worktree は「未コミット
            // 変更があれば拒否」の安全弁自体が働かない（中身を見られない
            // ので force しない remove の意味が無い -- module doc コメント
            // 参照）。通常の削除文言（intro + 中身確認の警告）ではなく、
            // 「ディレクトリは既に存在しません。登録を解除します」に分岐
            // する -- 警告文（未コミット変更のくだり）はここでは出さない
            // （そもそもチェックしていないので誤解を招く）。
            let intro = if state.missing {
                t!("task.menu.confirm.worktree_delete_missing_intro").to_string()
            } else {
                t!(
                    "task.menu.confirm.worktree_delete_intro",
                    title = state.title
                )
                .to_string()
            };
            // パスは省略表示 (第10波 §3、`path_abbrev`)。フルパスは
            // ツールチップに残す(同モジュールの契約)。
            let full_path: SharedString = info.path.clone().into();
            let shown_path: SharedString =
                path_abbrev::abbreviate_path(&info.path, path_abbrev::os_home().as_deref()).into();
            panel = panel.child(body_text(intro.into())).child(
                div()
                    .id("task-delete-worktree-path")
                    .text_size(px(theme::font_size::CAPTION))
                    .text_color(rgb(theme::text::SECONDARY))
                    .tooltip(move |_window, cx| {
                        cx.new(|_| crate::sidebar::IconTooltip(full_path.clone()))
                            .into()
                    })
                    .child(shown_path),
            );
            if !state.missing {
                panel = panel.child(body_text(
                    t!("task.menu.confirm.worktree_delete_warning")
                        .to_string()
                        .into(),
                ));
            }
            // 「ブランチも削除」チェックボックス（既定 off、実行中は不変）。
            let checkbox_label: SharedString = t!(
                "task.menu.confirm.delete_branch_checkbox",
                branch = info.branch
            )
            .to_string()
            .into();
            let mut checkbox = div()
                .id("task-delete-branch-toggle")
                .flex()
                .flex_row()
                .items_center()
                .gap_2()
                .p_2()
                .rounded_sm()
                .bg(rgb(theme::surface::RAISED))
                .text_color(rgb(theme::text::PRIMARY))
                .text_size(px(theme::font_size::LABEL))
                .child(if delete_branch {
                    "\u{2611}"
                } else {
                    "\u{2610}"
                })
                .child(checkbox_label);
            if !in_flight {
                checkbox = checkbox
                    .hover(|el| el.bg(rgb(theme::surface::ACTIVE)))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _: &MouseDownEvent, _window, cx| {
                            this.toggle_delete_branch(cx);
                        }),
                    );
            }
            panel = panel.child(checkbox);
        }
        None => {
            panel = panel.child(body_text(
                t!("task.menu.confirm.attached_delete_body")
                    .to_string()
                    .into(),
            ));
        }
    }

    if let Some(error) = error {
        panel = panel.child(
            div()
                .text_size(px(theme::font_size::CAPTION))
                .text_color(rgb(theme::status::CONFLICT))
                .child(SharedString::from(error.to_string())),
        );
    }

    if in_flight {
        panel = panel.child(
            div()
                .text_size(px(theme::font_size::LABEL))
                .text_color(rgb(theme::text::SECONDARY))
                .child(t!("task.menu.confirm.deleting").to_string()),
        );
    } else {
        let confirm_label: SharedString = if state.worktree.is_some() {
            t!("task.menu.confirm.delete_button_worktree")
                .to_string()
                .into()
        } else {
            t!("task.menu.confirm.delete_button_attached")
                .to_string()
                .into()
        };
        panel = panel.child(
            div()
                .flex()
                .flex_row()
                .justify_end()
                .gap_2()
                .child(dialog_button(
                    "task-delete-cancel",
                    t!("task.menu.confirm.cancel").to_string().into(),
                    false,
                    cx.listener(|this, _: &MouseDownEvent, _window, cx| {
                        this.close_task_menu(cx);
                    }),
                ))
                .child(dialog_button(
                    "task-delete-confirm",
                    confirm_label,
                    true,
                    cx.listener(|this, _: &MouseDownEvent, window, cx| {
                        this.execute_delete_task(window, cx);
                    }),
                )),
        );
    }

    centered_backdrop(panel.into_any_element(), "task-confirm-enter", cx)
}

fn render_notice_modal(message: &str, cx: &mut Context<LaboLaboApp>) -> AnyElement {
    let panel = div()
        .flex()
        .flex_col()
        .gap_3()
        .w(px(CONFIRM_WIDTH))
        .p_4()
        .rounded(px(theme::radius::OVERLAY))
        .bg(rgb(theme::surface::ROOT))
        .border_1()
        .border_color(rgb(theme::surface::STROKE))
        .shadow(theme::shadow::overlay())
        .on_mouse_down(MouseButton::Left, |_event, _window, cx: &mut App| {
            cx.stop_propagation();
        })
        .child(body_text(SharedString::from(message.to_string())))
        .child(div().flex().flex_row().justify_end().child(dialog_button(
            "task-notice-close",
            t!("common.close").to_string().into(),
            false,
            cx.listener(|this, _: &MouseDownEvent, _window, cx| {
                this.close_task_menu(cx);
            }),
        )));
    centered_backdrop(panel.into_any_element(), "task-notice-enter", cx)
}

fn body_text(text: SharedString) -> impl IntoElement {
    div()
        .text_size(px(theme::font_size::LABEL))
        .text_color(rgb(theme::text::PRIMARY))
        .child(text)
}

// Only the macOS-only "IDE で開く" section above inserts a separator today,
// so on other platforms this helper is dead code (a `-D warnings` error in
// the Linux CI job) -- cfg'd to match its one caller rather than `allow`'d,
// so a future cross-platform caller consciously removes the gate.
#[cfg(target_os = "macos")]
fn menu_separator() -> impl IntoElement {
    div().my_1().h(px(1.0)).bg(rgb(theme::surface::STROKE))
}

fn menu_row(
    id: String,
    label: SharedString,
    destructive: bool,
    on_click: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let text_color = if destructive {
        theme::status::CONFLICT
    } else {
        theme::text::PRIMARY
    };
    div()
        .id(SharedString::from(id))
        .px_2()
        .py_1()
        .text_size(px(theme::font_size::LABEL))
        .text_color(rgb(text_color))
        .hover(|el| el.bg(rgb(theme::surface::ACTIVE)))
        .active(|el| el.opacity(0.8))
        .on_mouse_down(MouseButton::Left, on_click)
        .child(label)
}

/// `pub(crate)` so `crate::app`'s missing-directory workspace placeholder
/// (第8波c) can reuse the same button chrome for its "再確認"/「この作業を
/// 削除…」actions instead of redefining it.
pub(crate) fn dialog_button(
    id: &'static str,
    label: SharedString,
    destructive: bool,
    on_click: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let text_color = if destructive {
        theme::status::CONFLICT
    } else {
        theme::text::PRIMARY
    };
    div()
        .id(id)
        .px_3()
        .py_1()
        .rounded_sm()
        .bg(rgb(theme::surface::RAISED))
        .text_color(rgb(text_color))
        .hover(|el| el.bg(rgb(theme::surface::ACTIVE)))
        .active(|el| el.opacity(0.8))
        .on_mouse_down(MouseButton::Left, on_click)
        .child(label)
}

fn centered_backdrop(
    panel: AnyElement,
    animation_id: &'static str,
    cx: &mut Context<LaboLaboApp>,
) -> AnyElement {
    div()
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .bg(rgba(OVERLAY_BG))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _: &MouseDownEvent, _window, cx| {
                this.close_task_menu(cx);
            }),
        )
        .child(panel)
        .with_animation(
            animation_id,
            Animation::new(motion::OVERLAY_ENTER).with_easing(motion::ease_out_strong()),
            |el, t| el.opacity(t),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{point, size};
    use labolabo_core::TileLayout;

    fn worktree_task() -> Task {
        Task::new_worktree(
            "/repo/.git",
            "/repo",
            "owner/repo",
            "feature/x",
            "main",
            "/repo/.worktrees/feature-x",
            TileLayout::default(),
            0,
        )
    }

    fn attached_task() -> Task {
        Task::new_attached(
            "/repo/.git",
            "/repo",
            "owner/repo",
            "/repo",
            TileLayout::default(),
            0,
        )
    }

    fn state(task: &Task) -> TaskMenuState {
        TaskMenuState::new(task, point(px(10.0), px(10.0)), false)
    }

    // MARK: - 相遷移の状態機械

    #[test]
    fn new_state_starts_in_the_menu_phase_with_task_snapshot() {
        let task = worktree_task();
        let state = state(&task);
        assert_eq!(state.phase, TaskMenuPhase::Menu);
        assert_eq!(state.task_id, task.id);
        let info = state.worktree.expect("worktree task carries git info");
        assert_eq!(info.branch, "feature/x");
        assert_eq!(info.path, "/repo/.worktrees/feature-x");
        assert_eq!(info.repo_root, "/repo");

        assert_eq!(state_from_attached().worktree, None);
    }

    fn state_from_attached() -> TaskMenuState {
        state(&attached_task())
    }

    /// 第8波c: `missing` はコンストラクタの引数どおりそのままスナップ
    /// ショットされる（開いた後にディレクトリの状態が変わっても、この
    /// メニュー/確認モーダルの文言は開いた瞬間の判定のまま -- 途中で
    /// 変わるとチェックボックス操作中に文言が変わる不安定な UI になる）。
    #[test]
    fn missing_flag_is_snapshotted_at_construction() {
        let task = worktree_task();
        let not_missing = TaskMenuState::new(&task, point(px(0.0), px(0.0)), false);
        assert!(!not_missing.missing);
        let missing = TaskMenuState::new(&task, point(px(0.0), px(0.0)), true);
        assert!(missing.missing);
    }

    // MARK: - 名前変更 / 色設定の相 (第10波)

    #[test]
    fn request_rename_prefills_the_field_with_the_current_title() {
        let task = worktree_task();
        let mut s = state(&task);
        s.request_rename();
        assert_eq!(
            s.phase,
            TaskMenuPhase::Rename {
                field: TextFieldState::new("feature/x"),
            }
        );
        // 確定値は trim 済みタイトル。プリフィル時点では元のタイトル。
        assert_eq!(s.rename_result().as_deref(), Some("feature/x"));
    }

    #[test]
    fn rename_result_is_none_for_blank_input_and_other_phases() {
        let task = worktree_task();
        let mut s = state(&task);
        assert_eq!(s.rename_result(), None, "Menu 相では None");
        s.request_rename();
        if let TaskMenuPhase::Rename { field } = &mut s.phase {
            field.text = "   ".to_string();
        }
        assert_eq!(s.rename_result(), None, "空白のみは確定不可");
        if let TaskMenuPhase::Rename { field } = &mut s.phase {
            field.text = "  新しい名前  ".to_string();
        }
        assert_eq!(s.rename_result().as_deref(), Some("新しい名前"));
    }

    #[test]
    fn request_color_pick_prefills_hex_with_the_current_color() {
        let task = worktree_task();
        let mut s = state(&task);
        s.request_color_pick(Some("#d0ff00".to_string()));
        assert_eq!(
            s.phase,
            TaskMenuPhase::ColorPick {
                hex_field: TextFieldState::new("#d0ff00"),
                hex_error: false,
            }
        );

        let mut no_color = state(&task);
        no_color.request_color_pick(None);
        assert_eq!(
            no_color.phase,
            TaskMenuPhase::ColorPick {
                hex_field: TextFieldState::new(""),
                hex_error: false,
            }
        );
    }

    /// Menu 相以外からは Rename/ColorPick へ遷移しない(確認モーダルの
    /// 最中にメニュー項目のハンドラが誤発火しても壊れない)。
    #[test]
    fn rename_and_color_pick_are_only_reachable_from_the_menu_phase() {
        let task = worktree_task();
        let mut s = state(&task);
        s.request_delete();
        let confirm = s.phase.clone();
        s.request_rename();
        assert_eq!(s.phase, confirm);
        s.request_color_pick(None);
        assert_eq!(s.phase, confirm);

        // Rename と ColorPick は互いにも遷移しない。
        let mut renaming = state(&task);
        renaming.request_rename();
        let rename_phase = renaming.phase.clone();
        renaming.request_color_pick(None);
        assert_eq!(renaming.phase, rename_phase);
    }

    /// Rename/ColorPick はいつでも閉じてよい(ConfirmDelete の in_flight
    /// だけが dismiss を塞ぐ、という既存規則が新しい相にも適用される)。
    #[test]
    fn rename_and_color_pick_phases_are_always_dismissible() {
        let task = worktree_task();
        let mut s = state(&task);
        s.request_rename();
        assert!(s.can_dismiss());
        let mut c = state(&task);
        c.request_color_pick(None);
        assert!(c.can_dismiss());
    }

    #[test]
    fn request_delete_moves_menu_to_confirm_with_branch_off_by_default() {
        let task = worktree_task();
        let mut s = state(&task);
        s.request_delete();
        assert_eq!(
            s.phase,
            TaskMenuPhase::ConfirmDelete {
                delete_branch: false,
                in_flight: false,
                error: None,
            }
        );
        assert!(!s.delete_branch_requested());
    }

    #[test]
    fn toggle_delete_branch_flips_only_for_worktree_tasks() {
        let task = worktree_task();
        let mut s = state(&task);
        s.request_delete();
        s.toggle_delete_branch();
        assert!(s.delete_branch_requested());
        s.toggle_delete_branch();
        assert!(!s.delete_branch_requested());

        let mut attached = state_from_attached();
        attached.request_delete();
        attached.toggle_delete_branch();
        assert!(!attached.delete_branch_requested());
    }

    #[test]
    fn begin_execution_flips_in_flight_once_and_clears_previous_error() {
        let task = worktree_task();
        let mut s = state(&task);
        // Menu 相からは実行できない（確認必須）。
        assert!(!s.begin_execution());
        s.request_delete();
        s.fail("前回の失敗".to_string());
        assert!(s.begin_execution());
        assert_eq!(
            s.phase,
            TaskMenuPhase::ConfirmDelete {
                delete_branch: false,
                in_flight: true,
                error: None,
            }
        );
        // 実行中の二度押しは拒否。
        assert!(!s.begin_execution());
    }

    #[test]
    fn toggle_delete_branch_is_frozen_while_in_flight() {
        let task = worktree_task();
        let mut s = state(&task);
        s.request_delete();
        assert!(s.begin_execution());
        s.toggle_delete_branch();
        assert!(!s.delete_branch_requested());
    }

    #[test]
    fn fail_returns_to_an_editable_confirm_with_the_message() {
        let task = worktree_task();
        let mut s = state(&task);
        s.request_delete();
        assert!(s.begin_execution());
        s.fail("未コミットの変更があるため削除できません".to_string());
        assert_eq!(
            s.phase,
            TaskMenuPhase::ConfirmDelete {
                delete_branch: false,
                in_flight: false,
                error: Some("未コミットの変更があるため削除できません".to_string()),
            }
        );
        // 失敗後は再実行できる。
        assert!(s.begin_execution());
    }

    #[test]
    fn dismissal_is_blocked_only_while_in_flight() {
        let task = worktree_task();
        let mut s = state(&task);
        assert!(s.can_dismiss());
        s.request_delete();
        assert!(s.can_dismiss());
        assert!(s.begin_execution());
        assert!(!s.can_dismiss());
        s.fail("x".to_string());
        assert!(s.can_dismiss());
        s.show_notice("done".to_string());
        assert!(s.can_dismiss());
    }

    #[test]
    fn notice_phase_carries_the_message() {
        let task = worktree_task();
        let mut s = state(&task);
        s.show_notice("worktree は削除しました。ブランチ削除は失敗".to_string());
        assert_eq!(
            s.phase,
            TaskMenuPhase::Notice {
                message: "worktree は削除しました。ブランチ削除は失敗".to_string()
            }
        );
    }

    // MARK: - clamp_popover_origin

    #[test]
    fn popover_stays_at_the_anchor_when_it_fits() {
        let origin = clamp_popover_origin(
            point(px(100.0), px(50.0)),
            size(px(240.0), px(120.0)),
            size(px(1000.0), px(700.0)),
        );
        assert_eq!(origin, point(px(100.0), px(50.0)));
    }

    #[test]
    fn popover_is_clamped_inside_the_right_and_bottom_edges() {
        let origin = clamp_popover_origin(
            point(px(950.0), px(680.0)),
            size(px(240.0), px(120.0)),
            size(px(1000.0), px(700.0)),
        );
        assert_eq!(origin, point(px(760.0), px(580.0)));
    }

    #[test]
    fn popover_never_goes_negative_even_on_a_tiny_viewport() {
        let origin = clamp_popover_origin(
            point(px(10.0), px(10.0)),
            size(px(240.0), px(120.0)),
            size(px(200.0), px(100.0)),
        );
        assert_eq!(origin, point(px(0.0), px(0.0)));
    }
}
