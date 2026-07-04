import Foundation

/// リポジトリの識別と GitHub URL を一元管理する。複数箇所（更新チェック・Changelog リンク）で
/// スラッグをハードコード重複させないための単一情報源。
enum GitHubRepo {
    static let slug = "Love-Rox/labolabo"

    /// リリース一覧ページ（人間向け・ダウンロード導線）。
    static var releasesPage: URL { URL(string: "https://github.com/\(slug)/releases")! }

    /// 最新リリースの REST API（更新チェック用）。
    static var latestReleaseAPI: URL { URL(string: "https://api.github.com/repos/\(slug)/releases/latest")! }
}
