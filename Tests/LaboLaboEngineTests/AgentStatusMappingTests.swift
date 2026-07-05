import XCTest
@testable import LaboLaboEngine

final class AgentStatusMappingTests: XCTestCase {

    // MARK: - hook_event_name → AgentStatus マッピング（正常系）

    func testSessionStartMapsToStarting() {
        XCTAssertEqual(AgentStatus.from(hookEvent: "SessionStart"), .starting)
    }

    func testRunningEventsMapToRunning() {
        // 思考・ツール実行中はすべて .running に集約される。
        XCTAssertEqual(AgentStatus.from(hookEvent: "UserPromptSubmit"), .running)
        XCTAssertEqual(AgentStatus.from(hookEvent: "PreToolUse"), .running)
        XCTAssertEqual(AgentStatus.from(hookEvent: "PostToolUse"), .running)
    }

    func testNotificationMapsToWaitingForInput() {
        XCTAssertEqual(AgentStatus.from(hookEvent: "Notification"), .waitingForInput)
    }

    func testStopEventsMapToIdle() {
        // Stop も SubagentStop も応答完了＝待機。
        XCTAssertEqual(AgentStatus.from(hookEvent: "Stop"), .idle)
        XCTAssertEqual(AgentStatus.from(hookEvent: "SubagentStop"), .idle)
    }

    func testSessionEndMapsToEnded() {
        XCTAssertEqual(AgentStatus.from(hookEvent: "SessionEnd"), .ended)
    }

    // MARK: - 未知 / 空文字イベント（異常系）

    func testUnknownAndEmptyEventsMapToNil() {
        XCTAssertNil(AgentStatus.from(hookEvent: ""))
        XCTAssertNil(AgentStatus.from(hookEvent: "Bogus"))
        XCTAssertNil(AgentStatus.from(hookEvent: "sessionstart")) // 大文字小文字は区別される
        XCTAssertNil(AgentStatus.from(hookEvent: " SessionStart")) // 前後空白も未知扱い
        XCTAssertNil(AgentStatus.from(hookEvent: "PreToolUse ")) // 末尾空白も未知扱い
    }

    // MARK: - enum の raw value 契約（永続化 / hook との互換の要）

    func testRawValues() {
        XCTAssertEqual(AgentStatus.none.rawValue, "none")
        XCTAssertEqual(AgentStatus.starting.rawValue, "starting")
        XCTAssertEqual(AgentStatus.running.rawValue, "running")
        XCTAssertEqual(AgentStatus.waitingForInput.rawValue, "waitingForInput")
        XCTAssertEqual(AgentStatus.idle.rawValue, "idle")
        XCTAssertEqual(AgentStatus.ended.rawValue, "ended")
    }

    func testRawValueRoundTrip() {
        // raw value から復元でき、対称であること。
        XCTAssertEqual(AgentStatus(rawValue: "waitingForInput"), .waitingForInput)
        XCTAssertNil(AgentStatus(rawValue: "unknown-status"))
    }

    // MARK: - AgentStatusEvent は全フィールドを保持する

    func testEventStoresAllFields() {
        let event = AgentStatusEvent(
            hookEvent: "Notification",
            status: .waitingForInput,
            sessionID: "sess-42",
            transcriptPath: "/tmp/transcript.jsonl",
            cwd: "/Users/dev/repo"
        )
        XCTAssertEqual(event.hookEvent, "Notification")
        XCTAssertEqual(event.status, .waitingForInput)
        XCTAssertEqual(event.sessionID, "sess-42")
        XCTAssertEqual(event.transcriptPath, "/tmp/transcript.jsonl")
        XCTAssertEqual(event.cwd, "/Users/dev/repo")
    }

    func testEventAllowsNilOptionalFields() {
        // sessionID / transcriptPath / cwd は省略（nil）可能。
        let event = AgentStatusEvent(
            hookEvent: "SessionEnd",
            status: .ended,
            sessionID: nil,
            transcriptPath: nil,
            cwd: nil
        )
        XCTAssertEqual(event.hookEvent, "SessionEnd")
        XCTAssertEqual(event.status, .ended)
        XCTAssertNil(event.sessionID)
        XCTAssertNil(event.transcriptPath)
        XCTAssertNil(event.cwd)
    }
}
