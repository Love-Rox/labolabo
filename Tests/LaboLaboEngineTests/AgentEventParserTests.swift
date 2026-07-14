import XCTest
@testable import LaboLaboEngine

/// `AgentEventParser`（トランスポート非依存の解釈層）の単体テスト。
/// ワイヤ仕様は docs/hooks-protocol.md が正。ソケット込みのラウンドトリップは
/// AgentStatusBusTests が担うので、ここでは解釈規則だけを網羅する。
final class AgentEventParserTests: XCTestCase {
    private func parse(_ json: String) -> AgentStatusEvent? {
        AgentEventParser.parse(Data(json.utf8))
    }

    func testParsesFullEvent() throws {
        let event = try XCTUnwrap(parse(
            #"{"hook_event_name":"SessionStart","session_id":"s1","transcript_path":"/t.jsonl","cwd":"/w","labolabo_pane_id":"P1"}"#
        ))
        XCTAssertEqual(event.status, .starting)
        XCTAssertEqual(event.hookEvent, "SessionStart")
        XCTAssertEqual(event.sessionID, "s1")
        XCTAssertEqual(event.transcriptPath, "/t.jsonl")
        XCTAssertEqual(event.cwd, "/w")
        XCTAssertEqual(event.paneID, "P1")
    }

    func testOptionalFieldsMayBeAbsent() throws {
        let event = try XCTUnwrap(parse(#"{"hook_event_name":"Stop"}"#))
        XCTAssertEqual(event.status, .idle)
        XCTAssertNil(event.sessionID)
        XCTAssertNil(event.transcriptPath)
        XCTAssertNil(event.cwd)
        XCTAssertNil(event.paneID)
    }

    func testUnknownHookEventIsDropped() {
        XCTAssertNil(parse(#"{"hook_event_name":"Mystery"}"#))
    }

    func testMalformedOrEmptyPayloadIsDropped() {
        XCTAssertNil(parse("{ broken"))
        XCTAssertNil(AgentEventParser.parse(Data()))
    }

    /// 未知フィールドは無視される（仕様書の前方互換方針: フィールド追加は互換）。
    func testUnknownFieldsAreIgnored() throws {
        let event = try XCTUnwrap(parse(#"{"hook_event_name":"Notification","future_field":123}"#))
        XCTAssertEqual(event.status, .waitingForInput)
    }
}

/// トランスポート差し替え（AgentEventTransport 注入）の契約テスト。
/// 将来の Windows 実装（Named Pipe 等）がこの契約を満たせば bus 側は無変更で動く。
final class AgentStatusBusTransportInjectionTests: XCTestCase {
    private final class FakeTransport: AgentEventTransport, @unchecked Sendable {
        var onMessage: (@Sendable (Data) -> Void)?
        private(set) var started = false
        func start() { started = true }
        func stop() {}
    }

    func testInjectedTransportDrivesOnEventOnMainQueue() {
        let fake = FakeTransport()
        let bus = AgentStatusBus(socketPath: "/tmp/labolabo-test-unused.sock", transport: fake)
        let exp = expectation(description: "event")
        nonisolated(unsafe) var received: AgentStatusEvent?
        bus.onEvent = { event in
            received = event
            XCTAssertTrue(Thread.isMainThread, "onEvent はメインキューで呼ばれる契約")
            exp.fulfill()
        }
        bus.start()
        XCTAssertTrue(fake.started, "bus.start はトランスポートを start する")
        fake.onMessage?(Data(#"{"hook_event_name":"SessionEnd","session_id":"z"}"#.utf8))
        wait(for: [exp], timeout: 2)
        XCTAssertEqual(received?.status, .ended)
        XCTAssertEqual(received?.sessionID, "z")
    }
}
