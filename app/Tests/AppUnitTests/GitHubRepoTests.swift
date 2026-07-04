import XCTest
@testable import LaboLabo

/// `GitHubRepo` はリポジトリ識別子と各種 GitHub URL の単一情報源。
/// スラッグと導出 URL が想定どおりであることを検証する。
final class GitHubRepoTests: XCTestCase {
    func testSlugIsExpectedRepository() {
        XCTAssertEqual(GitHubRepo.slug, "Love-Rox/labolabo")
    }

    func testReleasesPageURL() {
        XCTAssertEqual(
            GitHubRepo.releasesPage.absoluteString,
            "https://github.com/Love-Rox/labolabo/releases"
        )
    }

    func testLatestReleaseAPIURL() {
        XCTAssertEqual(
            GitHubRepo.latestReleaseAPI.absoluteString,
            "https://api.github.com/repos/Love-Rox/labolabo/releases/latest"
        )
    }

    /// リリースページと API はスラッグから導出される。両者にスラッグが含まれることを確認。
    func testDerivedURLsContainSlug() {
        XCTAssertTrue(GitHubRepo.releasesPage.absoluteString.contains(GitHubRepo.slug))
        XCTAssertTrue(GitHubRepo.latestReleaseAPI.absoluteString.contains(GitHubRepo.slug))
    }

    /// releasesPage は人間向け github.com、latestReleaseAPI は api.github.com を指す。
    func testURLHosts() {
        XCTAssertEqual(GitHubRepo.releasesPage.host, "github.com")
        XCTAssertEqual(GitHubRepo.latestReleaseAPI.host, "api.github.com")
    }

    /// 計算プロパティが呼び出しごとに同値の URL を返す（決定的）ことを確認。
    func testComputedURLsAreStable() {
        XCTAssertEqual(GitHubRepo.releasesPage, GitHubRepo.releasesPage)
        XCTAssertEqual(GitHubRepo.latestReleaseAPI, GitHubRepo.latestReleaseAPI)
    }
}
