import XCTest
@testable import LaboLaboEngine

final class GitHubEngineTests: XCTestCase {

    // MARK: - parsePRURL（gh pr create の stdout から URL を抽出）

    func testParsePRURLPlainURL() {
        let out = "https://github.com/Love-Rox/labolabo/pull/42\n"
        XCTAssertEqual(GitHubEngine.parsePRURL(from: out), "https://github.com/Love-Rox/labolabo/pull/42")
    }

    func testParsePRURLIgnoresLeadingAdvisoryLines() {
        // gh が URL の前に助言行を stdout に出しても URL 行を拾う。
        let out = """

        Warning: 3 uncommitted changes
        Creating pull request for feature/x into dev in Love-Rox/labolabo

        https://github.com/Love-Rox/labolabo/pull/7
        """
        XCTAssertEqual(GitHubEngine.parsePRURL(from: out), "https://github.com/Love-Rox/labolabo/pull/7")
    }

    func testParsePRURLFallsBackToTrimmedOutput() {
        // URL 行が無ければ全体を trim して返す（従来挙動を維持）。
        let out = "  some-non-url-output  \n"
        XCTAssertEqual(GitHubEngine.parsePRURL(from: out), "some-non-url-output")
    }

    // MARK: - parseChecks（#33 で status 変数を削除した回帰確認）

    func testParseChecksInProgressIsPending() {
        // CheckRun が実行中: conclusion 空・state 空 → pending に分類される。
        let items: [[String: Any]] = [
            ["conclusion": "", "state": ""],
            ["conclusion": "SUCCESS"],
        ]
        XCTAssertEqual(GitHubEngine.parseChecks(items), .pending)
    }

    func testParseChecksFailureWins() {
        let items: [[String: Any]] = [
            ["conclusion": "SUCCESS"],
            ["conclusion": "FAILURE"],
            ["conclusion": ""],
        ]
        XCTAssertEqual(GitHubEngine.parseChecks(items), .failing)
    }

    func testParseChecksAllSuccess() {
        let items: [[String: Any]] = [
            ["conclusion": "SUCCESS"],
            ["state": "SUCCESS"],
        ]
        XCTAssertEqual(GitHubEngine.parseChecks(items), .passing)
    }

    func testParseChecksEmptyIsNone() {
        XCTAssertEqual(GitHubEngine.parseChecks([[String: Any]]()), PullRequestInfo.Checks.none)
    }
}
