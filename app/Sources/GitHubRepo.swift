import Foundation

/// リポジトリの識別と GitHub URL を一元管理する。複数箇所（更新チェック・Changelog リンク）で
/// スラッグをハードコード重複させないための単一情報源。
enum GitHubRepo {
    static let slug = "Love-Rox/labolabo"

    /// リポジトリのトップページ（人間向け）。
    static var homeURL: URL { URL(string: "https://github.com/\(slug)")! }

    /// リリース一覧ページ（人間向け・ダウンロード導線）。
    static var releasesPage: URL { URL(string: "https://github.com/\(slug)/releases")! }

    /// 新規 Issue 作成ページ（プリフィルなし。ヘルプメニューの「問題を報告」用）。
    static var newIssuePage: URL { URL(string: "https://github.com/\(slug)/issues/new")! }

    /// 最新リリースの REST API（更新チェック用）。
    static var latestReleaseAPI: URL { URL(string: "https://api.github.com/repos/\(slug)/releases/latest")! }

    /// バグ報告用に、タイトル・本文をプリフィルした Issue 作成ページ。
    /// パーセントエンコードは URLComponents に任せる（二重エンコードしない）。
    /// URLComponents は値中の '+' を素通しし GitHub 側で空白と解釈されるため、'+' のみ %2B に置換する
    /// （'&' '#' 空白 改行などは queryItems が正しくエンコードするので触らない）。
    static func newIssueURL(title: String, body: String) -> URL {
        let base = "https://github.com/\(slug)/issues/new"
        var components = URLComponents(string: base)!
        components.queryItems = [
            URLQueryItem(name: "title", value: title),
            URLQueryItem(name: "body", value: body),
        ]
        components.percentEncodedQuery = components.percentEncodedQuery?
            .replacingOccurrences(of: "+", with: "%2B")
        return components.url ?? URL(string: base)!
    }
}
