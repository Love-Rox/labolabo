//! 「IDE で開く」(wave 6c 追加要望) -- Swift 版 `app/Sources/
//! SessionStatusBar.swift` の `IDEOpenMenu` の移植。
//!
//! 候補エディタは Swift 版と同一の 6 つ（bundle ID ごと、[`EDITOR_CANDIDATES`]）。
//! **インストール済みのものだけ**をメニューに出す: 検出は Spotlight 経由
//! （`mdfind "kMDItemCFBundleIdentifier == '<id>'"`）で、Swift 版の
//! `NSWorkspace.urlForApplication(withBundleIdentifier:)` の代替。objc
//! バインディングを足さずに済む代わりに Spotlight インデックスに依存する
//! （Spotlight を無効化した環境では検出ゼロになる -- その場合も「Finder で
//! 表示」は常に出す）。検出はアプリ起動時にバックグラウンドスレッドで一度
//! だけ実行してキャッシュする（`LaboLaboApp::installed_editors`）。
//!
//! 開く動作は `/usr/bin/open -b <bundleID> <dir>`（LaunchServices へ委譲、
//! Swift 版の `NSWorkspace.open` 相当）。「Finder で表示」は `open -R <dir>`
//! （Swift 版の `activateFileViewerSelecting` 相当 = 親フォルダで対象を
//! 選択表示）。どちらも blocking な `labolabo_core::process::run` なので
//! 呼び出し側は必ずバックグラウンド（`cx.background_spawn`）で呼ぶこと。
//!
//! 非 macOS: 検出は常に空、open 系は未対応エラー -- メニュー項目自体を
//! 出さない（呼び出し側 `task_menu.rs`/`menus.rs` が cfg で落とす）ので
//! 実際には呼ばれない。

use std::path::Path;

/// メニュー候補のエディタ 1 つ分（Swift 版 `Editor.Candidate` 相当）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EditorCandidate {
    pub name: &'static str,
    pub bundle_id: &'static str,
}

/// 主要エディタの候補。Swift 版 `Editor.candidates` と同一の 6 つ・同順。
///
/// 非 macOS の production 経路（[`detect_installed_editors`] の空実装）からは
/// この定数も下の純関数 2 つ（[`is_installed_output`]/[`installed_editors`]）
/// も参照されず dead code になる（Linux CI の `-D warnings` でエラー化）が、
/// いずれも OS 非依存の純ロジックで、ユニットテストは全プラットフォームで
/// 走らせたい（Linux CI の `cargo test` がまさに実行する）。よって cfg で
/// 落とさず、`allow(dead_code)` を非 macOS に限って付ける。
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub const EDITOR_CANDIDATES: [EditorCandidate; 6] = [
    EditorCandidate {
        name: "Visual Studio Code",
        bundle_id: "com.microsoft.VSCode",
    },
    EditorCandidate {
        name: "Cursor",
        bundle_id: "com.todesktop.230313mzl4w4u92",
    },
    EditorCandidate {
        name: "Zed",
        bundle_id: "dev.zed.Zed",
    },
    EditorCandidate {
        name: "Sublime Text",
        bundle_id: "com.sublimetext.4",
    },
    EditorCandidate {
        name: "JetBrains Fleet",
        bundle_id: "Fleet",
    },
    EditorCandidate {
        name: "Xcode",
        bundle_id: "com.apple.dt.Xcode",
    },
];

/// `mdfind` の出力（stdout）からインストール済みかを判定する純関数 --
/// 一致したアプリのパスが 1 行以上出れば「あり」。検出コマンドの実行
/// （プロセス起動）から切り離してこの判定だけをユニットテストする。
/// 非 macOS で `allow(dead_code)` な理由は [`EDITOR_CANDIDATES`] の doc
/// コメント参照。
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn is_installed_output(stdout: &str) -> bool {
    !stdout.trim().is_empty()
}

/// [`EDITOR_CANDIDATES`] から「インストール済み検出結果」でフィルタした
/// メニュー掲載リストを返す純関数。`detected` は各候補（同順）の検出結果。
/// 非 macOS で `allow(dead_code)` な理由は [`EDITOR_CANDIDATES`] の doc
/// コメント参照。
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn installed_editors(detected: &[bool]) -> Vec<EditorCandidate> {
    EDITOR_CANDIDATES
        .iter()
        .zip(detected)
        .filter_map(|(candidate, installed)| installed.then_some(*candidate))
        .collect()
}

/// Spotlight（`mdfind`）で候補 6 つの有無を調べ、インストール済みの
/// 候補だけを返す。ブロッキング（候補ごとに `mdfind` を 1 回起動）なので
/// バックグラウンドスレッドから呼ぶこと。`mdfind` 自体の失敗（Spotlight
/// 無効等）はその候補を「なし」扱いにする。
#[cfg(target_os = "macos")]
pub fn detect_installed_editors() -> Vec<EditorCandidate> {
    let detected: Vec<bool> = EDITOR_CANDIDATES
        .iter()
        .map(|candidate| {
            let query = format!("kMDItemCFBundleIdentifier == '{}'", candidate.bundle_id);
            labolabo_core::process::run(Path::new("/usr/bin/mdfind"), &[query], None, None)
                .map(|output| output.status == 0 && is_installed_output(&output.stdout))
                .unwrap_or(false)
        })
        .collect();
    installed_editors(&detected)
}

#[cfg(not(target_os = "macos"))]
pub fn detect_installed_editors() -> Vec<EditorCandidate> {
    Vec::new()
}

/// `open -b <bundle_id> <directory>`。ブロッキング（バックグラウンドから
/// 呼ぶこと）。`open` は LaunchServices へ引き渡してすぐ返る。
#[cfg(target_os = "macos")]
pub fn open_in_editor(bundle_id: &str, directory: &Path) -> Result<(), String> {
    run_open(&[
        "-b".to_string(),
        bundle_id.to_string(),
        directory.to_string_lossy().into_owned(),
    ])
}

/// `open -R <directory>` -- Finder で（親フォルダ内で選択して）表示。
#[cfg(target_os = "macos")]
pub fn reveal_in_finder(directory: &Path) -> Result<(), String> {
    run_open(&["-R".to_string(), directory.to_string_lossy().into_owned()])
}

#[cfg(target_os = "macos")]
fn run_open(args: &[String]) -> Result<(), String> {
    match labolabo_core::process::run(Path::new("/usr/bin/open"), args, None, None) {
        Ok(output) if output.status == 0 => Ok(()),
        Ok(output) => Err(format!(
            "open {} failed (exit {}): {}",
            args.join(" "),
            output.status,
            output.stderr.trim()
        )),
        Err(err) => Err(format!("failed to launch open: {err}")),
    }
}

#[cfg(not(target_os = "macos"))]
pub fn open_in_editor(_bundle_id: &str, _directory: &Path) -> Result<(), String> {
    Err("IDE で開く は macOS のみ対応です".to_string())
}

#[cfg(not(target_os = "macos"))]
pub fn reveal_in_finder(_directory: &Path) -> Result<(), String> {
    Err("Finder で表示 は macOS のみ対応です".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidates_match_the_swift_list() {
        // Swift 版 SessionStatusBar.swift `Editor.candidates` と同一・同順。
        let ids: Vec<&str> = EDITOR_CANDIDATES.iter().map(|c| c.bundle_id).collect();
        assert_eq!(
            ids,
            vec![
                "com.microsoft.VSCode",
                "com.todesktop.230313mzl4w4u92",
                "dev.zed.Zed",
                "com.sublimetext.4",
                "Fleet",
                "com.apple.dt.Xcode",
            ]
        );
    }

    #[test]
    fn mdfind_output_nonempty_means_installed() {
        assert!(is_installed_output("/Applications/Zed.app\n"));
        assert!(is_installed_output(
            "/Applications/Zed.app\n/Users/x/Applications/Zed.app\n"
        ));
        assert!(!is_installed_output(""));
        assert!(!is_installed_output("\n"));
        assert!(!is_installed_output("   \n"));
    }

    #[test]
    fn installed_editors_filters_by_detection_preserving_order() {
        let detected = [true, false, true, false, false, false];
        let installed = installed_editors(&detected);
        assert_eq!(
            installed.iter().map(|c| c.name).collect::<Vec<_>>(),
            vec!["Visual Studio Code", "Zed"]
        );
    }

    #[test]
    fn installed_editors_with_all_false_is_empty() {
        assert!(installed_editors(&[false; 6]).is_empty());
    }

    #[test]
    fn installed_editors_tolerates_short_detection_slice() {
        // zip なので detection が候補数より短くても panic しない（残りは
        // 未検出扱い）。検出側のバグに対する安全弁。
        let installed = installed_editors(&[true]);
        assert_eq!(installed.len(), 1);
        assert_eq!(installed[0].name, "Visual Studio Code");
    }
}
