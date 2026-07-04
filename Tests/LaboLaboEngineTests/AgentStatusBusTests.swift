import XCTest
@testable import LaboLaboEngine

/// `AgentStatusBus` の AF_UNIX ソケットサーバを、実際に POSIX クライアントを
/// つないでラウンドトリップさせる統合テスト。フォワーダが 1 接続 = 1 JSON を
/// 送ってくる想定を再現し、`onEvent`（メインキュー）が正しい `AgentStatus` を
/// 発火する／不正入力では発火しないことを検証する。
final class AgentStatusBusTests: XCTestCase {

    private var bus: AgentStatusBus?
    private var socketPath: String?

    override func tearDown() {
        bus?.stop()
        if let socketPath {
            unlink(socketPath)
        }
        bus = nil
        socketPath = nil
        super.tearDown()
    }

    // MARK: - 正常系（hook_event_name → AgentStatus のマッピング）

    func testNotificationRoundTripEmitsWaitingForInput() throws {
        let json = #"{"hook_event_name":"Notification","session_id":"s1","transcript_path":"/tmp/t.jsonl","cwd":"/tmp"}"#
        let event = try XCTUnwrap(expectEvent(sending: json), "bus は 1 イベントを発火するべき")
        XCTAssertEqual(event.status, .waitingForInput)
        XCTAssertEqual(event.hookEvent, "Notification")
        XCTAssertEqual(event.sessionID, "s1")
        XCTAssertEqual(event.transcriptPath, "/tmp/t.jsonl")
        XCTAssertEqual(event.cwd, "/tmp")
    }

    func testStopEventRoundTripEmitsIdle() throws {
        // 別イベント種別が別 status にマップされること（Stop → .idle）も 1 本で押さえる。
        let json = #"{"hook_event_name":"Stop","session_id":"s2","transcript_path":"/tmp/s2.jsonl","cwd":"/work"}"#
        let event = try XCTUnwrap(expectEvent(sending: json))
        XCTAssertEqual(event.status, .idle)
        XCTAssertEqual(event.hookEvent, "Stop")
        XCTAssertEqual(event.sessionID, "s2")
        XCTAssertEqual(event.cwd, "/work")
    }

    // MARK: - 異常系（無イベント）

    func testMalformedJSONProducesNoEvent() {
        // 接続・書き込みは成功するが JSON として壊れている → parse 段でドロップ。
        expectNoEvent(sending: "{ this is not valid json ")
    }

    func testEmptyPayloadProducesNoEvent() {
        // 接続して即クローズ（0 バイト）→ data.isEmpty ガードでドロップ。
        expectNoEvent(sending: "")
    }

    func testUnknownHookEventProducesNoEvent() {
        // JSON は妥当だが hook_event_name が未知 → AgentStatus.from が nil → 無イベント。
        expectNoEvent(sending: #"{"hook_event_name":"TotallyUnknown","session_id":"s3"}"#)
    }

    // MARK: - ヘルパ

    /// 新しい bus を短いソケットパスで生成し、tearDown 用に控える。
    private func makeBus(file: StaticString = #filePath, line: UInt = #line) -> AgentStatusBus {
        // sockaddr_un.sun_path は Darwin で 104 バイト。UUID 先頭 8 文字だけ使い、
        // フルパスを十分に短く保つ（例: /var/folders/.../T/lb-1a2b3c4d.sock）。
        let prefix = String(UUID().uuidString.prefix(8)).lowercased()
        let path = FileManager.default.temporaryDirectory
            .appendingPathComponent("lb-\(prefix).sock").path
        XCTAssertLessThan(path.utf8.count, 104, "socket path が sockaddr_un に収まらない", file: file, line: line)
        let b = AgentStatusBus(socketPath: path)
        self.bus = b
        self.socketPath = path
        return b
    }

    /// bus を立ち上げ、payload を送り、イベント発火を待って（最大 3s）返す。
    private func expectEvent(
        sending payload: String,
        file: StaticString = #filePath,
        line: UInt = #line
    ) -> AgentStatusEvent? {
        let bus = makeBus(file: file, line: line)
        let box = EventBox()
        let exp = expectation(description: "onEvent が発火する")
        bus.onEvent = { event in
            box.set(event)
            exp.fulfill()
        }
        bus.start()
        let sent = sendPayload(payload, to: bus.socketPath)
        XCTAssertTrue(sent, "クライアントが payload を送れなかった", file: file, line: line)
        wait(for: [exp], timeout: 3.0)
        return box.event
    }

    /// bus を立ち上げ、payload を送り、一定時間（1s）イベントが来ないことを確認。
    private func expectNoEvent(
        sending payload: String,
        file: StaticString = #filePath,
        line: UInt = #line
    ) {
        let bus = makeBus(file: file, line: line)
        let exp = expectation(description: "onEvent は発火しないべき")
        exp.isInverted = true
        bus.onEvent = { _ in exp.fulfill() }
        bus.start()
        // 送信自体は成功していること（＝サーバに確かに届いた上で無視された）を担保。
        let sent = sendPayload(payload, to: bus.socketPath)
        XCTAssertTrue(sent, "クライアントが payload を送れなかった", file: file, line: line)
        wait(for: [exp], timeout: 1.0)
    }

    /// AF_UNIX / SOCK_STREAM のクライアントを生成し、`payload` を送って書き込み側を
    /// 閉じる（サーバ側 read が EOF=0 でループを抜ける）。connect はサーバの
    /// bind/listen 完了を待つため、失敗のたびに fd を作り直して短くリトライする。
    @discardableResult
    private func sendPayload(_ payload: String, to path: String, maxAttempts: Int = 150) -> Bool {
        var addr = sockaddr_un()
        addr.sun_family = sa_family_t(AF_UNIX)
        let pathBytes = Array(path.utf8)
        let capacity = MemoryLayout.size(ofValue: addr.sun_path) - 1 // 末尾 NUL 用に 1 残す
        guard pathBytes.count <= capacity else { return false }
        withUnsafeMutableBytes(of: &addr.sun_path) { raw in
            raw.copyBytes(from: pathBytes)
        }
        let addrSize = socklen_t(MemoryLayout<sockaddr_un>.size)

        for _ in 0 ..< maxAttempts {
            let fd = socket(AF_UNIX, SOCK_STREAM, 0)
            guard fd >= 0 else { return false }
            let rc = withUnsafePointer(to: &addr) { p -> Int32 in
                p.withMemoryRebound(to: sockaddr.self, capacity: 1) { connect(fd, $0, addrSize) }
            }
            if rc == 0 {
                let ok = writeAll(fd: fd, bytes: Array(payload.utf8))
                shutdown(fd, SHUT_WR)
                close(fd)
                return ok
            }
            close(fd)
            usleep(20_000) // 20ms 待ってから再試行（サーバの bind/listen を待つ）
        }
        return false
    }

    /// バッファ全体を書き切る（部分書き込みに対応）。空なら何もせず成功扱い。
    private func writeAll(fd: Int32, bytes: [UInt8]) -> Bool {
        if bytes.isEmpty { return true }
        return bytes.withUnsafeBytes { raw -> Bool in
            guard let base = raw.baseAddress else { return false }
            var offset = 0
            while offset < bytes.count {
                let n = write(fd, base + offset, bytes.count - offset)
                if n <= 0 { return false }
                offset += n
            }
            return true
        }
    }
}

/// `onEvent`（メインキューで呼ばれる）で受け取ったイベントをロック保護で退避する箱。
/// Swift 6 strict concurrency 下でスレッド跨ぎの読み書きを安全にするため。
private final class EventBox: @unchecked Sendable {
    private let lock = NSLock()
    private var stored: AgentStatusEvent?

    func set(_ event: AgentStatusEvent) {
        lock.lock()
        stored = event
        lock.unlock()
    }

    var event: AgentStatusEvent? {
        lock.lock()
        defer { lock.unlock() }
        return stored
    }
}
