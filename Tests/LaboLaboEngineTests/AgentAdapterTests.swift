import XCTest
@testable import LaboLaboEngine

final class AgentAdapterTests: XCTestCase {

    // MARK: - 能力フラグ（UI の出し分けの根拠）

    func testClaudeCapabilities() {
        let c = AgentAdapters.claude.capabilities
        XCTAssertEqual(c.statusReporting, .hooks)
        XCTAssertTrue(c.resume)
        XCTAssertTrue(c.nativeWorktreeFlag)
        XCTAssertTrue(c.statusReporting.providesLiveStatus)
    }

    func testCodexDegradesStatusButKeepsResume() {
        let c = AgentAdapters.codex.capabilities
        XCTAssertEqual(c.statusReporting, .none)
        XCTAssertFalse(c.statusReporting.providesLiveStatus)
        XCTAssertTrue(c.resume)
        XCTAssertFalse(c.nativeWorktreeFlag)
    }

    func testGeminiHasNoResumeNoLiveStatus() {
        let c = AgentAdapters.gemini.capabilities
        XCTAssertEqual(c.statusReporting, .none)
        XCTAssertFalse(c.resume)
        XCTAssertFalse(c.statusReporting.providesLiveStatus)
    }

    // MARK: - launchCommand（resume の出し分け）

    func testClaudeLaunchWithoutResumeID() {
        XCTAssertEqual(AgentAdapters.claude.launchCommand(resumeID: nil), "claude")
        XCTAssertEqual(AgentAdapters.claude.launchCommand(resumeID: ""), "claude")
    }

    func testClaudeLaunchWithResumeID() {
        XCTAssertEqual(
            AgentAdapters.claude.launchCommand(resumeID: "abc-123"),
            "claude --resume 'abc-123'"
        )
    }

    func testCodexLaunchWithResumeID() {
        XCTAssertEqual(
            AgentAdapters.codex.launchCommand(resumeID: "sess9"),
            "codex resume 'sess9'"
        )
    }

    func testGeminiIgnoresResumeIDWhenNotResumable() {
        // resume 非対応なので id があっても素の実行名。
        XCTAssertEqual(AgentAdapters.gemini.launchCommand(resumeID: "whatever"), "gemini")
    }

    func testResumeIDIsShellQuotedAgainstInjection() {
        let cmd = AgentAdapters.claude.launchCommand(resumeID: "a'b")
        XCTAssertEqual(cmd, "claude --resume 'a'\\''b'")
    }

    // MARK: - レジストリ

    func testFindKnownAndUnknown() {
        XCTAssertEqual(AgentAdapters.find(id: "codex").id, "codex")
        XCTAssertEqual(AgentAdapters.find(id: "gemini").id, "gemini")
        XCTAssertEqual(AgentAdapters.find(id: nil).id, AgentAdapters.default.id)
        XCTAssertEqual(AgentAdapters.find(id: "does-not-exist").id, "claude")
    }

    func testAllContainsThreeAdapters() {
        XCTAssertEqual(AgentAdapters.all.map(\.id), ["claude", "codex", "gemini"])
    }
}
