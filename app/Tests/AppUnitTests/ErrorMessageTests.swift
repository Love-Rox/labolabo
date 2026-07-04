import XCTest
@testable import LaboLabo
// GitCommandError の合成メンバーワイズイニシャライザは internal のため、
// テストから構築するには @testable import が必要（plain import では到達できない）。
@testable import LaboLaboEngine

/// `Error.sessionUIMessage` は git/gh 失敗をユーザーに見せる際の共通文言。
/// `GitCommandError` は stderr（空/空白なら localizedDescription）を優先し、
/// 最終的に必ず非空の文言を返す、という契約を検証する。
final class ErrorMessageTests: XCTestCase {

    /// stderr が非空なら、前後の空白を除去した値をそのまま返す。
    func testGitCommandErrorReturnsTrimmedStderr() {
        let error = GitCommandError(
            arguments: ["push", "origin", "HEAD"],
            exitCode: 1,
            stderr: "  fatal: remote error: access denied\n"
        )
        XCTAssertEqual(error.sessionUIMessage, "fatal: remote error: access denied")
    }

    /// トリムは前後の空白のみで、内部の改行やスペースは保持される。
    func testGitCommandErrorPreservesInternalWhitespace() {
        let error = GitCommandError(
            arguments: ["merge"],
            exitCode: 128,
            stderr: "\n  line1\nline2  \n\n"
        )
        // 外側だけがトリムされ、中間の "line1\nline2" は残る。
        XCTAssertEqual(error.sessionUIMessage, "line1\nline2")
    }

    /// stderr が空白のみのときは stderr を採用せず、非空のフォールバック文言を返す。
    func testGitCommandErrorWithWhitespaceStderrFallsBackToNonEmpty() {
        let whitespace = "   \n\t \n"
        let error = GitCommandError(
            arguments: ["status"],
            exitCode: 2,
            stderr: whitespace
        )
        let message = error.sessionUIMessage
        // 空ラベルにならないこと（＝空白 stderr をそのまま返していないこと）。
        XCTAssertFalse(message.isEmpty)
        XCTAssertFalse(
            message.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
            "フォールバック文言が空白のみであってはならない"
        )
        XCTAssertNotEqual(message, whitespace)
    }

    /// stderr が完全な空文字列でも、非空のフォールバック文言を返す。
    func testGitCommandErrorWithEmptyStderrFallsBackToNonEmpty() {
        let error = GitCommandError(
            arguments: ["worktree", "remove"],
            exitCode: 1,
            stderr: ""
        )
        XCTAssertFalse(error.sessionUIMessage.isEmpty)
    }

    /// GitCommandError 以外（プレーンな NSError）は localizedDescription をそのまま返す。
    func testPlainNSErrorReturnsLocalizedDescription() {
        let error: Error = NSError(
            domain: "com.love-rox.labolabo.test",
            code: 42,
            userInfo: [NSLocalizedDescriptionKey: "boom"]
        )
        XCTAssertEqual(error.sessionUIMessage, "boom")
    }

    /// localizedDescription が空のエラーでも最終フォールバックで必ず非空を返す（契約の核心）。
    func testEmptyLocalizedDescriptionFallsBackToDefaultMessage() {
        struct BlankError: LocalizedError {
            var errorDescription: String? { "" }
        }
        let error: Error = BlankError()
        // stderr 経路にも localizedDescription 経路にも頼れない最悪ケースでも空にしない。
        XCTAssertFalse(error.sessionUIMessage.isEmpty)
    }

    /// 種類の異なる複数のエラーで、sessionUIMessage が一貫して非空であることを保証する。
    func testMessageIsNeverEmptyAcrossErrorKinds() {
        let errors: [Error] = [
            GitCommandError(arguments: ["a"], exitCode: 1, stderr: "real stderr"),
            GitCommandError(arguments: ["b"], exitCode: 1, stderr: ""),
            GitCommandError(arguments: ["c"], exitCode: 1, stderr: "   "),
            NSError(
                domain: "test",
                code: 7,
                userInfo: [NSLocalizedDescriptionKey: "network unreachable"]
            ),
        ]
        for error in errors {
            XCTAssertFalse(
                error.sessionUIMessage.isEmpty,
                "sessionUIMessage は常に非空でなければならない: \(error)"
            )
        }
    }
}
