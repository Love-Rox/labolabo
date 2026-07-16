//! ディレクトリパスの省略表示 (第10波 パーソナライズ §3)。
//!
//! ディレクトリ直付けタスクのグループ見出しや missing プレースホルダなど、
//! フルパスをそのまま出すと煩雑な箇所向けの純関数。方針:
//!
//! 1. `$HOME` プレフィクスを `~` に置き換える。
//! 2. それでも [`MAX_DISPLAY_CHARS`] を超える場合は**中間省略** --
//!    「(ルート +) 先頭 1 要素 + `…` + 末尾 2 要素」
//!    (例: `~/ghq/github.com/lapras-inc/lapras` → `~/ghq/…/lapras-inc/lapras`)。
//!    それでも超える場合は先頭要素も落として `…/末尾 2 要素`(最終手段 --
//!    末尾要素自体が長い場合は超過を許容する。切り詰めて情報を壊すより
//!    「末尾 = 一番識別に効く部分」を完全な形で残すことを優先)。
//!
//! **ツールチップには常にフルパスを残す**のが適用側の契約 -- この関数は
//! あくまで一覧・見出し等の「一瞥用」表示専用で、省略した情報への到達手段
//! (ツールチップ)を呼び出し側が別途保証する(モジュールを使う側の
//! `sidebar.rs` / `app.rs` 参照)。
//!
//! 区切り文字は入力に `/` が含まれればそれ、含まれなければ `\`
//! (Windows パス)を使い、出力も同じ区切りで組み立てる -- OS 判定ではなく
//! 入力駆動なので、テストは 3 OS の CI すべてで同じ結果になる。

/// 省略を始める表示長のしきい値(文字数 -- バイトではない)。サイドバー幅
/// (`sidebar::SIDEBAR_WIDTH` = 220px)と `font_size::CAPTION`(11px)を
/// 前提に 32 字(指示の「32〜40 字目安」の下限 -- 想定ユースケースの
/// `~/ghq/github.com/owner/repo` 級=34 字前後がちゃんと省略対象に入る値)。
pub const MAX_DISPLAY_CHARS: usize = 32;

/// `path` を人に見せる用に省略する。`home` は `$HOME` の絶対パス
/// (`None`/不一致なら `~` 置換はしない)。仕様はモジュール doc コメント参照。
pub fn abbreviate_path(path: &str, home: Option<&str>) -> String {
    let sep = if path.contains('/') { '/' } else { '\\' };

    // 1. ~ 置換: home そのもの、または home 直下以深のときだけ。
    let tilde = match home {
        Some(home) if !home.is_empty() && path == home => "~".to_string(),
        Some(home)
            if !home.is_empty()
                && path.starts_with(home)
                && path[home.len()..].starts_with(sep) =>
        {
            format!("~{}", &path[home.len()..])
        }
        _ => path.to_string(),
    };

    if tilde.chars().count() <= MAX_DISPLAY_CHARS {
        return tilde;
    }

    // 2. 中間省略。ルート(絶対パスの "/"、~ 置換後の "~/")は要素として
    //    ではなく接頭辞として保持する -- 「先頭 1 要素」は ~ や / の
    //    *次* の要素(例の `~/ghq/…` の "ghq")を指す。
    let (root, rest) =
        if let Some(after_tilde) = tilde.strip_prefix('~').and_then(|r| r.strip_prefix(sep)) {
            (format!("~{sep}"), after_tilde)
        } else if let Some(after_root) = tilde.strip_prefix(sep) {
            (sep.to_string(), after_root)
        } else {
            (String::new(), tilde.as_str())
        };
    let components: Vec<&str> = rest.split(sep).filter(|c| !c.is_empty()).collect();
    // 省略しても縮まない形(先頭 1 + … + 末尾 2 で全要素が残る)なら
    // そのまま返す。
    if components.len() < 4 {
        return tilde;
    }

    let first = components[0];
    let tail = &components[components.len() - 2..];
    let elided = format!("{root}{first}{sep}\u{2026}{sep}{}{sep}{}", tail[0], tail[1]);
    if elided.chars().count() <= MAX_DISPLAY_CHARS {
        return elided;
    }
    // 3. 最終手段: 先頭要素も落とす(それでも長い場合は超過を許容 --
    //    モジュール doc コメント参照)。
    format!("\u{2026}{sep}{}{sep}{}", tail[0], tail[1])
}

/// `name` が絶対パスに見えるときだけ [`abbreviate_path`] を適用する。
/// サイドバーのグループ見出し(`Task::repo_name`)用: リポジトリなら
/// `owner/repo` かフォルダ名(そのまま出す)、git 管理外のディレクトリ
/// 直付けだけがフルパスになる(`new_task::resolve_attached_repo` の
/// フォールバック)ため、「パスらしさ」で振り分ける。
pub fn abbreviate_if_path(name: &str, home: Option<&str>) -> String {
    let looks_like_path = name.starts_with('/')
        || name.starts_with('~')
        || name.starts_with('\\')
        // Windows のドライブレター (`C:\...` / `C:/...`)。
        || (name.len() >= 3
            && name.as_bytes()[1] == b':'
            && (name.as_bytes()[2] == b'\\' || name.as_bytes()[2] == b'/'));
    if looks_like_path {
        abbreviate_path(name, home)
    } else {
        name.to_string()
    }
}

/// 実行環境のホームディレクトリ(表示用 -- [`abbreviate_path`] の `home`
/// 引数)。Unix は `$HOME`、Windows は `%USERPROFILE%`。
pub fn os_home() -> Option<String> {
    #[cfg(windows)]
    let var = "USERPROFILE";
    #[cfg(not(windows))]
    let var = "HOME";
    std::env::var(var).ok().filter(|h| !h.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOME: Option<&str> = Some("/Users/me");

    // MARK: - ~ 置換のみで収まるケース

    #[test]
    fn short_home_relative_path_gets_tilde_but_no_elision() {
        assert_eq!(
            abbreviate_path("/Users/me/projects/app", HOME),
            "~/projects/app"
        );
    }

    #[test]
    fn home_itself_becomes_a_bare_tilde() {
        assert_eq!(abbreviate_path("/Users/me", HOME), "~");
    }

    /// home と同じ文字列で*始まるだけ*の別ディレクトリ (`/Users/me2`) を
    /// 誤って `~2` にしない。
    #[test]
    fn a_sibling_directory_sharing_the_home_prefix_is_not_tilde_substituted() {
        assert_eq!(abbreviate_path("/Users/me2/x", HOME), "/Users/me2/x");
    }

    #[test]
    fn short_path_outside_home_is_returned_verbatim() {
        assert_eq!(abbreviate_path("/opt/work/repo", HOME), "/opt/work/repo");
        assert_eq!(abbreviate_path("/tmp", None), "/tmp");
    }

    // MARK: - 中間省略

    #[test]
    fn deep_home_path_is_middle_elided_to_first_plus_last_two() {
        // 実例そのまま: ~ 置換後 34 字 > 32 なので中間省略が入る。
        assert_eq!(
            abbreviate_path("/Users/me/ghq/github.com/lapras-inc/lapras", HOME),
            "~/ghq/\u{2026}/lapras-inc/lapras"
        );
    }

    #[test]
    fn deep_non_home_path_keeps_the_absolute_root() {
        assert_eq!(
            abbreviate_path("/Volumes/external/backups/2026/projects/repo-name", HOME),
            "/Volumes/\u{2026}/projects/repo-name"
        );
    }

    /// 4 要素未満は中間省略しても縮まないので、長くてもそのまま返す。
    #[test]
    fn long_but_shallow_path_is_not_elided() {
        let path = "/aaaaaaaaaaaaaaaaaaaa/bbbbbbbbbbbbbbbbbbbb/cccc";
        assert_eq!(abbreviate_path(path, HOME), path);
    }

    /// 先頭 1 要素を残しても収まらないときは `…/末尾 2 要素` まで落とす。
    #[test]
    fn falls_back_to_ellipsis_plus_last_two_when_still_too_long() {
        let path = "/first-component-is-quite-long/x/y/component-b/component-c-is-long";
        assert_eq!(
            abbreviate_path(path, None),
            "\u{2026}/component-b/component-c-is-long"
        );
    }

    /// 日本語パス: 文字数(chars)で判定するので、マルチバイトでも
    /// バイト境界 panic なく、また過剰に省略されない。
    #[test]
    fn japanese_path_counts_characters_not_bytes() {
        // 15 文字 -- バイト数では 36 を超えるが、文字数では収まる。
        assert_eq!(
            abbreviate_path("/Users/me/書類/開発/アプリ", HOME),
            "~/書類/開発/アプリ"
        );
        // 深い日本語パスは同じ規則で中間省略される。
        assert_eq!(
            abbreviate_path(
                "/Users/me/とても長いディレクトリ名/中間の階層をいくつも/挟んだ/最後のプロジェクト/リポジトリ",
                HOME
            ),
            "~/とても長いディレクトリ名/\u{2026}/最後のプロジェクト/リポジトリ"
        );
    }

    /// Windows 風パス(`\` 区切り)は同じ区切りで組み立て直す。
    #[test]
    fn backslash_paths_are_elided_with_backslashes() {
        assert_eq!(
            abbreviate_path(
                r"C:\Users\me\ghq\github.com\lapras-inc\lapras-long-repo",
                None
            ),
            "C:\\\u{2026}\\lapras-inc\\lapras-long-repo"
        );
    }

    // MARK: - abbreviate_if_path

    #[test]
    fn repo_names_are_left_alone() {
        assert_eq!(abbreviate_if_path("owner/repo", HOME), "owner/repo");
        assert_eq!(abbreviate_if_path("labolabo", HOME), "labolabo");
    }

    #[test]
    fn absolute_paths_are_abbreviated() {
        assert_eq!(
            abbreviate_if_path("/Users/me/ghq/github.com/lapras-inc/lapras", HOME),
            "~/ghq/\u{2026}/lapras-inc/lapras"
        );
    }

    #[test]
    fn windows_drive_paths_are_recognized_as_paths() {
        assert_eq!(
            abbreviate_if_path(r"C:\Users\me\ghq\github.com\lapras-inc\repo", None),
            "C:\\\u{2026}\\lapras-inc\\repo"
        );
    }
}
