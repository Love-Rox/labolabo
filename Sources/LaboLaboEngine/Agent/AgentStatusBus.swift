import Foundation

/// セッションごとの AF_UNIX ソケットサーバ。`labolabo --hook <socket>` フォワーダが
/// Claude の hook stdin(JSON) を 1 接続 = 1 イベントとして送ってくるのを受信し、
/// `AgentStatus` に変換して `onEvent` をメインキューで呼ぶ。
///
/// SOCK_STREAM なので接続単位でフレーミングされ、同時発火しても混線しない。
public final class AgentStatusBus: @unchecked Sendable {
    public let socketPath: String
    /// メインキューで呼ばれる受信コールバック。
    public var onEvent: ((AgentStatusEvent) -> Void)?

    private var listenFD: Int32 = -1
    private var running = false
    private var startedOnce = false
    private let startLock = NSLock()

    public init(socketPath: String) {
        self.socketPath = socketPath
    }

    public func start() {
        // 二重 start は 2 つの runServer が同一ソケットパスを奪い合い、
        // スレッドと fd を漏らすので 1 回だけに制限する（stop 後の再開は非対応）。
        startLock.lock()
        let alreadyStarted = startedOnce
        startedOnce = true
        startLock.unlock()
        guard !alreadyStarted else { return }
        // accept()/read() でブロックし続けるので、GCD のワーカープールを
        // 占有しないよう専用スレッドで待つ（セッション数ぶん常駐するため）。
        let thread = Thread { [weak self] in self?.runServer() }
        thread.name = "labolabo.agent.statusbus"
        thread.start()
    }

    public func stop() {
        running = false
        let fd = listenFD
        listenFD = -1
        if fd >= 0 {
            shutdown(fd, SHUT_RDWR)
            close(fd)
        }
        unlink(socketPath)
    }

    // MARK: - server

    private func runServer() {
        unlink(socketPath) // 残骸を掃除
        let fd = socket(AF_UNIX, SOCK_STREAM, 0)
        guard fd >= 0 else { return }

        guard var addr = Self.makeAddr(path: socketPath) else { close(fd); return }
        let size = socklen_t(MemoryLayout<sockaddr_un>.size)
        let bound = withUnsafePointer(to: &addr) { ptr in
            ptr.withMemoryRebound(to: sockaddr.self, capacity: 1) { bind(fd, $0, size) }
        }
        guard bound == 0 else { close(fd); return }
        // フォワーダ（同ユーザー）だけが書けるように。
        chmod(socketPath, 0o600)
        guard listen(fd, 16) == 0 else { close(fd); unlink(socketPath); return }

        listenFD = fd
        running = true
        while running {
            let client = accept(fd, nil, nil)
            if client < 0 {
                if running { continue } else { break }
            }
            handleClient(client)
        }
        close(fd)
        unlink(socketPath)
    }

    private func handleClient(_ client: Int32) {
        var data = Data()
        var buf = [UInt8](repeating: 0, count: 8192)
        while true {
            let n = read(client, &buf, buf.count)
            if n > 0 {
                data.append(contentsOf: buf[0 ..< n])
            } else {
                break
            }
        }
        close(client)
        parseAndEmit(data)
    }

    private func parseAndEmit(_ data: Data) {
        guard !data.isEmpty,
              let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else { return }
        let hookEvent = object["hook_event_name"] as? String ?? ""
        guard let status = AgentStatus.from(hookEvent: hookEvent) else { return }
        let event = AgentStatusEvent(
            hookEvent: hookEvent,
            status: status,
            sessionID: object["session_id"] as? String,
            transcriptPath: object["transcript_path"] as? String,
            cwd: object["cwd"] as? String
        )
        DispatchQueue.main.async { [weak self] in
            self?.onEvent?(event)
        }
    }

    private static func makeAddr(path: String) -> sockaddr_un? {
        var addr = sockaddr_un()
        addr.sun_family = sa_family_t(AF_UNIX)
        let capacity = MemoryLayout.size(ofValue: addr.sun_path) - 1 // 末尾 NUL 用に 1 残す
        let bytes = Array(path.utf8)
        guard bytes.count <= capacity else { return nil }
        withUnsafeMutableBytes(of: &addr.sun_path) { raw in
            raw.copyBytes(from: bytes)
        }
        return addr
    }
}
