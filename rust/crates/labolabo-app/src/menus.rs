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
//!   は単一ウィンドウで、`menu.window.title` キーの文言を優先し、「しまう」
//!   （`Window::minimize_window`）と「拡大/縮小」（`Window::zoom_window`）
//!   を自前アクションで配線する。
//! - **ファイル → 選択中の作業を IDE で開く**: タスク行「…」メニューの
//!   IDE 列挙（`task_menu.rs`）の簡易版で、検出済みエディタの先頭 1 つで
//!   開く（メニューは起動時に静的に組むため、動的なエディタ列挙サブメニュー
//!   はタスク行メニュー側に譲る -- 深いサブメニュー化はしない判断を PR に
//!   明記）。非 macOS ではこの項目自体を出さない。
//!
//! ## i18n (wave 6f)
//!
//! [`app_menus`] takes the current locale **explicitly** rather than reading
//! `rust_i18n::locale()` ambiently, unlike almost every other render
//! function in this crate (`crate::i18n`'s module doc comment covers the
//! general "read the ambient global locale" convention). Two reasons: (1)
//! `App::set_menus` is not itself a per-frame render call -- it must be
//! re-invoked explicitly (`LaboLaboApp::set_locale`, `app.rs`) whenever the
//! locale changes, so the caller already has the locale in hand at every
//! call site; (2) this keeps `app_menus`/the item-name unit tests below
//! fully deterministic without mutating `rust_i18n`'s process-global current
//! locale (which `cargo test` would run concurrently with every other test
//! in this binary -- a shared mutable global is exactly the kind of thing
//! that turns into flaky CI). [`render_about_overlay`], by contrast, *is* a
//! per-frame render function, so it uses the ordinary ambient `t!()` and
//! updates automatically on the next frame after a locale change.

use gpui::{
    div, prelude::*, px, rgb, rgba, Animation, AnimationExt, AnyElement, Context, IntoElement,
    Menu, MenuItem, MouseButton, MouseDownEvent, SharedString,
};
use rust_i18n::t;

#[cfg(target_os = "macos")]
use crate::app::OpenSelectedInIde;
use crate::app::{
    About, CloseTab, Copy, FocusNextPane, FocusPrevPane, ImportFromSwift, LaboLaboApp,
    MinimizeWindow, NewAttachedTask, NewTab, NewWorktreeTask, OpenGitCommitsPane, OpenGitDiffPane,
    OpenGitFilesPane, Paste, Quit, SplitDown, SplitRight, ToggleGitPane, ToggleSettings,
    ZoomWindow,
};
use crate::motion;
use crate::theme;

/// 表示用アプリ名（メニュー/About）。バンドル名
/// （`rust/scripts/bundle-macos.sh` の `APP_NAME`）と揃える。
pub const APP_NAME: &str = "LaboLabo-rs";

/// マーケティングバージョン。`build.rs` がコンパイル時に注入する
/// `LABOLABO_RS_VERSION`（単一ソース: `rust/VERSION`、CI からは env で
/// 上書き可 -- `build.rs` の doc コメント参照）と同じ値なので、
/// `rust/scripts/{bundle-macos.sh,package-linux.sh,package-windows.ps1}`
/// が生成する配布物のバージョンと常に一致する（手動での同期は不要）。
/// RC リリース波（`.github/workflows/rust-release.yml`）は Swift 版の
/// 0.7.x 系からメジャーバンプした 1.0.0 系で配布する決定 -- `rust/
/// README.md`「RC リリース手順」参照。
pub const APP_VERSION: &str = env!("LABOLABO_RS_VERSION");

/// ビルド番号: `git rev-list --count HEAD`（`build.rs` がコンパイル時に
/// 注入。Swift 版の CFBundleVersion / bundle-macos.sh の BUILD_NUMBER と
/// 同じ規約）。git の外でビルドされた場合は "0"。
pub const BUILD_NUMBER: &str = env!("LABOLABO_BUILD_NUMBER");

/// メニューバー全体の構成。`main.rs` が起動時に一度、`LaboLaboApp::
/// set_locale` が言語切替のたびに `cx.set_menus(app_menus(locale))` する
/// (`locale`: `"ja"`/`"en"` -- `rust_i18n::locale()`/`crate::i18n::
/// LocaleSetting::resolve` の戻り値をそのまま渡せる)。この関数自身は
/// `rust_i18n` のグローバル現在ロケールを読まない -- このモジュールの
/// doc コメント「i18n (wave 6f)」参照。
pub fn app_menus(locale: &str) -> Vec<Menu> {
    vec![
        Menu {
            name: APP_NAME.into(),
            items: vec![
                MenuItem::action(
                    t!("menu.app.about", locale = locale, app = APP_NAME).to_string(),
                    About,
                ),
                MenuItem::separator(),
                MenuItem::action(
                    t!("menu.app.settings", locale = locale).to_string(),
                    ToggleSettings,
                ),
                MenuItem::separator(),
                MenuItem::action(
                    t!("menu.app.quit", locale = locale, app = APP_NAME).to_string(),
                    Quit,
                ),
            ],
        },
        Menu {
            name: t!("menu.file.title", locale = locale).to_string().into(),
            items: file_menu_items(locale),
        },
        Menu {
            name: t!("menu.edit.title", locale = locale).to_string().into(),
            items: vec![
                MenuItem::action(t!("menu.edit.copy", locale = locale).to_string(), Copy),
                MenuItem::action(t!("menu.edit.paste", locale = locale).to_string(), Paste),
            ],
        },
        Menu {
            name: t!("menu.view.title", locale = locale).to_string().into(),
            items: vec![
                MenuItem::action(
                    t!("menu.view.toggle_git_pane", locale = locale).to_string(),
                    ToggleGitPane,
                ),
                MenuItem::separator(),
                // `plans` W6d §3.2: Git のタイルペインを開く導線 --
                // フォーカス中のタスクに、対応する種類のタイルが無ければ
                // 新規追加、既にあれば前面に出す
                // (`LaboLaboApp::open_git_tile_pane`)。
                MenuItem::action(
                    t!("menu.view.open_git_files_pane", locale = locale).to_string(),
                    OpenGitFilesPane,
                ),
                MenuItem::action(
                    t!("menu.view.open_git_diff_pane", locale = locale).to_string(),
                    OpenGitDiffPane,
                ),
                MenuItem::action(
                    t!("menu.view.open_git_commits_pane", locale = locale).to_string(),
                    OpenGitCommitsPane,
                ),
                MenuItem::separator(),
                MenuItem::action(
                    t!("menu.view.split_right", locale = locale).to_string(),
                    SplitRight,
                ),
                MenuItem::action(
                    t!("menu.view.split_down", locale = locale).to_string(),
                    SplitDown,
                ),
                MenuItem::separator(),
                MenuItem::action(
                    t!("menu.view.focus_next_pane", locale = locale).to_string(),
                    FocusNextPane,
                ),
                MenuItem::action(
                    t!("menu.view.focus_prev_pane", locale = locale).to_string(),
                    FocusPrevPane,
                ),
            ],
        },
        Menu {
            name: t!("menu.window.title", locale = locale).to_string().into(),
            items: vec![
                MenuItem::action(
                    t!("menu.window.minimize", locale = locale).to_string(),
                    MinimizeWindow,
                ),
                MenuItem::action(
                    t!("menu.window.zoom", locale = locale).to_string(),
                    ZoomWindow,
                ),
            ],
        },
    ]
}

fn file_menu_items(locale: &str) -> Vec<MenuItem> {
    let mut items = vec![
        MenuItem::action(
            t!("menu.file.new_attached_task", locale = locale).to_string(),
            NewAttachedTask,
        ),
        MenuItem::action(
            t!("menu.file.new_worktree_task", locale = locale).to_string(),
            NewWorktreeTask,
        ),
        MenuItem::separator(),
        // Swift 版インポータ (`crate::swift_import`, `plans` W6e §3 のトリ
        // ガー②): 起動時の自動インポート(①)とは別に、いつでも手動で再実行
        // できる入口。同じ重複スキップ規則を使う。
        MenuItem::action(
            t!("menu.file.import_from_swift", locale = locale).to_string(),
            ImportFromSwift,
        ),
    ];
    #[cfg(target_os = "macos")]
    {
        items.push(MenuItem::separator());
        items.push(MenuItem::action(
            t!("menu.file.open_selected_in_ide", locale = locale).to_string(),
            OpenSelectedInIde,
        ));
    }
    items.push(MenuItem::separator());
    items.push(MenuItem::action(
        t!("menu.file.new_tab", locale = locale).to_string(),
        NewTab,
    ));
    items.push(MenuItem::action(
        t!("menu.file.close_tab", locale = locale).to_string(),
        CloseTab,
    ));
    items
}

// MARK: - About オーバーレイ
//
// settings.rs の render_settings_overlay と同じ「開いている間だけ要素が
// 存在する = マウントの瞬間にエントランスアニメーションが始まる」パターン。
// 閉じるのは明示的な「閉じる」ボタンのみ（同 module のクリック外閉じ非対応
// と同じ判断）。

const OVERLAY_BG: u32 = theme::OVERLAY_SCRIM;
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

    let version_line: SharedString = t!(
        "about.version_line",
        version = APP_VERSION,
        build = BUILD_NUMBER
    )
    .to_string()
    .into();

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
        .child(t!("common.close").to_string());

    let panel = div()
        .flex()
        .flex_col()
        .items_center()
        .gap_2()
        .w(px(PANEL_WIDTH))
        .p_4()
        .rounded(px(theme::radius::OVERLAY))
        .bg(rgb(theme::surface::ROOT))
        .border_1()
        .border_color(rgb(theme::surface::STROKE))
        .shadow(theme::shadow::overlay())
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
                .child(t!("about.tagline").to_string()),
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

    // `app_menus` takes its locale as an explicit parameter (this module's
    // "i18n (wave 6f)" doc comment) specifically so these tests can assert
    // on known-language text without touching `rust_i18n`'s process-global
    // current locale -- safe under `cargo test`'s default parallel-threads-
    // in-one-process execution. The ja-locale expectations below are the
    // exact pre-i18n-wave literal strings (this module's Japanese text was
    // unchanged by the wave, see the PR description); `..._in_english`
    // variants pin the new `en` locale's structure the same way.

    #[test]
    fn menu_bar_has_the_five_standard_menus_in_order() {
        let menus = app_menus("ja");
        let names: Vec<String> = menus.iter().map(|m| m.name.to_string()).collect();
        assert_eq!(
            names,
            vec!["LaboLabo-rs", "ファイル", "編集", "表示", "ウィンドウ"]
        );
    }

    #[test]
    fn menu_bar_has_the_five_standard_menus_in_order_in_english() {
        let menus = app_menus("en");
        let names: Vec<String> = menus.iter().map(|m| m.name.to_string()).collect();
        assert_eq!(names, vec!["LaboLabo-rs", "File", "Edit", "View", "Window"]);
    }

    #[test]
    fn app_menu_contains_about_settings_and_quit() {
        let menus = app_menus("ja");
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
    fn app_menu_contains_about_settings_and_quit_in_english() {
        let menus = app_menus("en");
        assert_eq!(
            item_names(&menus[0]),
            vec![
                "About LaboLabo-rs",
                "---",
                "Settings…",
                "---",
                "Quit LaboLabo-rs",
            ]
        );
    }

    #[test]
    fn file_menu_wires_new_task_flows_and_tabs() {
        let menus = app_menus("ja");
        let names = item_names(&menus[1]);
        assert_eq!(names[0], "新しい作業（フォルダ直付け）…");
        assert_eq!(names[1], "新しい作業（worktree を作成）…");
        assert!(names.contains(&"Swift 版からインポート…".to_string()));
        #[cfg(target_os = "macos")]
        assert!(names.contains(&"選択中の作業を IDE で開く".to_string()));
        #[cfg(not(target_os = "macos"))]
        assert!(!names.contains(&"選択中の作業を IDE で開く".to_string()));
        assert_eq!(names[names.len() - 2], "新しいタブ");
        assert_eq!(names[names.len() - 1], "タブを閉じる");
    }

    #[test]
    fn file_menu_wires_new_task_flows_and_tabs_in_english() {
        let menus = app_menus("en");
        let names = item_names(&menus[1]);
        assert_eq!(names[0], "New Task (Attach Folder)…");
        assert_eq!(names[1], "New Task (Create Worktree)…");
        assert!(names.contains(&"Import from Swift App…".to_string()));
        #[cfg(target_os = "macos")]
        assert!(names.contains(&"Open Selected Task in IDE".to_string()));
        assert_eq!(names[names.len() - 2], "New Tab");
        assert_eq!(names[names.len() - 1], "Close Tab");
    }

    #[test]
    fn edit_and_view_menus_reference_existing_actions() {
        let menus = app_menus("ja");
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
    fn edit_and_view_menus_reference_existing_actions_in_english() {
        let menus = app_menus("en");
        assert_eq!(item_names(&menus[2]), vec!["Copy", "Paste"]);
        assert_eq!(
            item_names(&menus[3]),
            vec![
                "Toggle Git Pane",
                "---",
                "Open Changed Files as Tile",
                "Open Diff as Tile",
                "Open Commit History as Tile",
                "---",
                "Split Right",
                "Split Down",
                "---",
                "Next Pane",
                "Previous Pane",
            ]
        );
    }

    #[test]
    fn window_menu_has_minimize_and_zoom() {
        let menus = app_menus("ja");
        assert_eq!(item_names(&menus[4]), vec!["しまう", "拡大/縮小"]);
    }

    #[test]
    fn window_menu_has_minimize_and_zoom_in_english() {
        let menus = app_menus("en");
        assert_eq!(item_names(&menus[4]), vec!["Minimize", "Zoom"]);
    }

    #[test]
    fn version_constants_look_sane() {
        // `build.rs` が `rust/VERSION` から注入する値 -- ハードコードした
        // リテラルと比べるとリリースのたびにこのテストを更新する羽目になる
        // ので、構造的な妥当性のみ確認する（doc コメント参照: 単一ソースは
        // `rust/VERSION`、CI は env で上書き）。
        assert!(!APP_VERSION.is_empty());
        assert!(APP_VERSION
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_digit()));
        assert!(APP_VERSION.contains('.'));
        // build.rs 注入値: 数字のみ（git 外ビルドのフォールバック "0" を含む）。
        assert!(BUILD_NUMBER.chars().all(|c| c.is_ascii_digit()));
        assert!(!BUILD_NUMBER.is_empty());
    }
}
