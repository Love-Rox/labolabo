//! メニューバー (wave 6c §1) と About オーバーレイ。
//!
//! gpui 0.2 の `App::set_menus`（`platform/app_menu.rs` の `Menu`/`MenuItem`
//! -- 実 API を確認済み）で標準的なメニュー構成を組み、**すべて既存の
//! gpui アクションへ配線**する。メニュー項目はアクション参照
//! （`MenuItem::action`）なので、キーバインドとメニュークリックのどちらも
//! 同じ 1 回のアクションディスパッチに収束する（macOS はメニューの
//! keyEquivalent に一致したキーをメニュー経由で 1 度だけ発火させる --
//! Zed が同じ機構で出荷している経路。実クリックでの二重発火確認は
//! ユーザー実機確認に委ねる）。ショートカット表記は gpui が keymap
//! （`main.rs` の `bind_keys`）から自動で引くため、**`set_menus` は
//! `bind_keys` の後**に呼ぶこと。
//!
//! ## 各メニューの判断メモ
//!
//! - **LaboLabo-rs**（アプリメニュー）: About は gpui に標準 About パネル
//!   API が無い（`platform/mac/platform.rs` に orderFrontStandardAboutPanel
//!   への経路なし -- 確認済み）ため、settings.rs のオーバーレイパターンを
//!   流用した簡易オーバーレイ（[`render_about_overlay`]）で出す。
//! - **編集**: コピー/ペーストは `OsAction` を使わず素の `MenuItem::action`
//!   にする。gpui の `OsAction::Copy` は NSMenuItem のセレクタを `copy:`
//!   （レスポンダチェーン任せ）へ振ってしまい、この app は NSTextView を
//!   持たないので既存の `Copy`/`Paste` アクション（端末 PTY への配線）に
//!   届かなくなるため。
//! - **ウィンドウ**: gpui の macOS 実装はメニュー名が英語の `"Window"` の
//!   ときだけ OS のウィンドウリスト（setWindowsMenu_）を差し込む。本アプリ
//!   は単一ウィンドウで、文言は日本語に揃える方針を優先し、「しまう」
//!   （`Window::minimize_window`）と「拡大/縮小」（`Window::zoom_window`）
//!   を自前アクションで配線する。
//! - **ファイル → 選択中の作業を IDE で開く**: タスク行「…」メニューの
//!   IDE 列挙（`task_menu.rs`）の簡易版で、検出済みエディタの先頭 1 つで
//!   開く（メニューは起動時に静的に組むため、動的なエディタ列挙サブメニュー
//!   はタスク行メニュー側に譲る -- 深いサブメニュー化はしない判断を PR に
//!   明記）。非 macOS ではこの項目自体を出さない。

use gpui::{
    div, prelude::*, px, rgb, rgba, Animation, AnimationExt, AnyElement, Context, IntoElement,
    Menu, MenuItem, MouseButton, MouseDownEvent, SharedString,
};

#[cfg(target_os = "macos")]
use crate::app::OpenSelectedInIde;
use crate::app::{
    About, CloseTab, Copy, FocusNextPane, FocusPrevPane, LaboLaboApp, MinimizeWindow,
    NewAttachedTask, NewTab, NewWorktreeTask, OpenGitCommitsPane, OpenGitDiffPane,
    OpenGitFilesPane, Paste, Quit, SplitDown, SplitRight, ToggleGitPane, ToggleSettings,
    ZoomWindow,
};
use crate::motion;
use crate::theme;

/// 表示用アプリ名（メニュー/About）。バンドル名
/// （`rust/scripts/bundle-macos.sh` の `APP_NAME`）と揃える。
pub const APP_NAME: &str = "LaboLabo-rs";

/// マーケティングバージョン。`rust/scripts/bundle-macos.sh` の `VERSION`
/// （CFBundleShortVersionString）と同じ値 -- Rust 版バンドルは Swift 版の
/// 0.7.x 系からメジャーバンプした 1.0.0 系で配布する決定（同スクリプトの
/// コメント参照）。ここを変えるときは bundle-macos.sh も揃えること。
pub const APP_VERSION: &str = "1.0.0";

/// ビルド番号: `git rev-list --count HEAD`（`build.rs` がコンパイル時に
/// 注入。Swift 版の CFBundleVersion / bundle-macos.sh の BUILD_NUMBER と
/// 同じ規約）。git の外でビルドされた場合は "0"。
pub const BUILD_NUMBER: &str = env!("LABOLABO_BUILD_NUMBER");

/// メニューバー全体の構成。`main.rs` が起動時に一度だけ
/// `cx.set_menus(app_menus())` する。
pub fn app_menus() -> Vec<Menu> {
    vec![
        Menu {
            name: APP_NAME.into(),
            items: vec![
                MenuItem::action(format!("{APP_NAME} について"), About),
                MenuItem::separator(),
                MenuItem::action("設定…", ToggleSettings),
                MenuItem::separator(),
                MenuItem::action(format!("{APP_NAME} を終了"), Quit),
            ],
        },
        Menu {
            name: "ファイル".into(),
            items: file_menu_items(),
        },
        Menu {
            name: "編集".into(),
            items: vec![
                MenuItem::action("コピー", Copy),
                MenuItem::action("ペースト", Paste),
            ],
        },
        Menu {
            name: "表示".into(),
            items: vec![
                MenuItem::action("Git ペインを表示/非表示", ToggleGitPane),
                MenuItem::separator(),
                // `plans` W6d §3.2: Git のタイルペインを開く導線 --
                // フォーカス中のタスクに、対応する種類のタイルが無ければ
                // 新規追加、既にあれば前面に出す
                // (`LaboLaboApp::open_git_tile_pane`)。
                MenuItem::action("変更ファイルをタイルとして開く", OpenGitFilesPane),
                MenuItem::action("Diff をタイルとして開く", OpenGitDiffPane),
                MenuItem::action("コミット履歴をタイルとして開く", OpenGitCommitsPane),
                MenuItem::separator(),
                MenuItem::action("右に分割", SplitRight),
                MenuItem::action("下に分割", SplitDown),
                MenuItem::separator(),
                MenuItem::action("次のペイン", FocusNextPane),
                MenuItem::action("前のペイン", FocusPrevPane),
            ],
        },
        Menu {
            name: "ウィンドウ".into(),
            items: vec![
                MenuItem::action("しまう", MinimizeWindow),
                MenuItem::action("拡大/縮小", ZoomWindow),
            ],
        },
    ]
}

fn file_menu_items() -> Vec<MenuItem> {
    let mut items = vec![
        MenuItem::action("新しい作業（フォルダ直付け）…", NewAttachedTask),
        MenuItem::action("新しい作業（worktree を作成）…", NewWorktreeTask),
    ];
    #[cfg(target_os = "macos")]
    {
        items.push(MenuItem::separator());
        items.push(MenuItem::action(
            "選択中の作業を IDE で開く",
            OpenSelectedInIde,
        ));
    }
    items.push(MenuItem::separator());
    items.push(MenuItem::action("新しいタブ", NewTab));
    items.push(MenuItem::action("タブを閉じる", CloseTab));
    items
}

// MARK: - About オーバーレイ
//
// settings.rs の render_settings_overlay と同じ「開いている間だけ要素が
// 存在する = マウントの瞬間にエントランスアニメーションが始まる」パターン。
// 閉じるのは明示的な「閉じる」ボタンのみ（同 module のクリック外閉じ非対応
// と同じ判断）。

const OVERLAY_BG: u32 = theme::with_alpha(0x000000, 0xb3);
const PANEL_WIDTH: f32 = 320.0;

/// About パネル（`app.about_open()` のときだけ `Some`）。呼び出し側
/// （`app.rs` の `Render`）はルートツリー末尾に `.children(..)` で足す。
pub fn render_about_overlay(
    app: &LaboLaboApp,
    cx: &mut Context<LaboLaboApp>,
) -> Option<AnyElement> {
    if !app.about_open() {
        return None;
    }

    let version_line: SharedString =
        format!("バージョン {APP_VERSION}（ビルド {BUILD_NUMBER}）").into();

    let close_button = div()
        .id("about-close")
        .px_3()
        .py_1()
        .rounded_sm()
        .bg(rgb(theme::surface::RAISED))
        .text_color(rgb(theme::text::PRIMARY))
        .hover(|el| el.bg(rgb(theme::surface::ACTIVE)))
        .active(|el| el.opacity(0.8))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _: &MouseDownEvent, _window, cx| {
                this.close_about(cx);
            }),
        )
        .child("閉じる");

    let panel = div()
        .flex()
        .flex_col()
        .items_center()
        .gap_2()
        .w(px(PANEL_WIDTH))
        .p_4()
        .rounded_md()
        .bg(rgb(theme::surface::ROOT))
        .border_1()
        .border_color(rgb(theme::surface::STROKE))
        .child(
            div()
                .text_size(px(16.0))
                .text_color(rgb(theme::text::PRIMARY))
                .child(APP_NAME),
        )
        .child(
            div()
                .text_size(px(theme::font_size::LABEL))
                .text_color(rgb(theme::text::SECONDARY))
                .child(version_line),
        )
        .child(
            div()
                .text_size(px(theme::font_size::CAPTION))
                .text_color(rgb(theme::text::MUTED))
                .child("LaboLabo の Rust クロスプラットフォーム版"),
        )
        .child(div().h(px(4.0)))
        .child(close_button);

    let backdrop = div()
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .bg(rgba(OVERLAY_BG))
        .child(panel)
        .with_animation(
            "about-backdrop-enter",
            Animation::new(motion::OVERLAY_ENTER).with_easing(motion::ease_out_strong()),
            |el, t| el.opacity(t),
        );

    Some(backdrop.into_any_element())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item_names(menu: &Menu) -> Vec<String> {
        menu.items
            .iter()
            .map(|item| match item {
                MenuItem::Action { name, .. } => name.to_string(),
                MenuItem::Separator => "---".to_string(),
                MenuItem::Submenu(submenu) => format!("submenu:{}", submenu.name),
                MenuItem::SystemMenu(os_menu) => format!("system:{}", os_menu.name),
            })
            .collect()
    }

    #[test]
    fn menu_bar_has_the_five_standard_menus_in_order() {
        let menus = app_menus();
        let names: Vec<String> = menus.iter().map(|m| m.name.to_string()).collect();
        assert_eq!(
            names,
            vec!["LaboLabo-rs", "ファイル", "編集", "表示", "ウィンドウ"]
        );
    }

    #[test]
    fn app_menu_contains_about_settings_and_quit() {
        let menus = app_menus();
        assert_eq!(
            item_names(&menus[0]),
            vec![
                "LaboLabo-rs について",
                "---",
                "設定…",
                "---",
                "LaboLabo-rs を終了",
            ]
        );
    }

    #[test]
    fn file_menu_wires_new_task_flows_and_tabs() {
        let menus = app_menus();
        let names = item_names(&menus[1]);
        assert_eq!(names[0], "新しい作業（フォルダ直付け）…");
        assert_eq!(names[1], "新しい作業（worktree を作成）…");
        #[cfg(target_os = "macos")]
        assert!(names.contains(&"選択中の作業を IDE で開く".to_string()));
        #[cfg(not(target_os = "macos"))]
        assert!(!names.contains(&"選択中の作業を IDE で開く".to_string()));
        assert_eq!(names[names.len() - 2], "新しいタブ");
        assert_eq!(names[names.len() - 1], "タブを閉じる");
    }

    #[test]
    fn edit_and_view_menus_reference_existing_actions() {
        let menus = app_menus();
        assert_eq!(item_names(&menus[2]), vec!["コピー", "ペースト"]);
        assert_eq!(
            item_names(&menus[3]),
            vec![
                "Git ペインを表示/非表示",
                "---",
                "変更ファイルをタイルとして開く",
                "Diff をタイルとして開く",
                "コミット履歴をタイルとして開く",
                "---",
                "右に分割",
                "下に分割",
                "---",
                "次のペイン",
                "前のペイン",
            ]
        );
    }

    #[test]
    fn window_menu_has_minimize_and_zoom() {
        let menus = app_menus();
        assert_eq!(item_names(&menus[4]), vec!["しまう", "拡大/縮小"]);
    }

    #[test]
    fn version_constants_look_sane() {
        // bundle-macos.sh の VERSION と揃える契約（doc コメント参照）。
        assert_eq!(APP_VERSION, "1.0.0");
        // build.rs 注入値: 数字のみ（git 外ビルドのフォールバック "0" を含む）。
        assert!(BUILD_NUMBER.chars().all(|c| c.is_ascii_digit()));
        assert!(!BUILD_NUMBER.is_empty());
    }
}
