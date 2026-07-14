import Foundation

/// フォワーダからの 1 メッセージ（= 1 hook イベントの生ペイロード）を届けるトランスポートの契約。
/// macOS/Linux は AF_UNIX（`UnixSocketEventTransport`）。Windows は Named Pipe / loopback TCP を
/// 将来ここへ差し込む（選定はクロスプラットフォーム化の Spike 3）。トランスポートはバイト列の
/// 受信だけに責任を持ち、解釈（JSON → AgentStatusEvent）は `AgentEventParser` が担う。
/// ワイヤ仕様は docs/hooks-protocol.md を正とする。
public protocol AgentEventTransport: AnyObject {
    /// 受信コールバック。呼び出しスレッドは実装依存（受信側でキュー移送すること）。start 前に設定する。
    var onMessage: (@Sendable (Data) -> Void)? { get set }
    func start()
    func stop()
}

/// セッションごとの hook イベント受信バス。トランスポート（既定: AF_UNIX）からの生メッセージを
/// `AgentEventParser` で解釈し、`onEvent` をメインキューで呼ぶ。
public final class AgentStatusBus: @unchecked Sendable {
    public let socketPath: String
    /// メインキューで呼ばれる受信コールバック。
    public var onEvent: ((AgentStatusEvent) -> Void)?

    private let transport: AgentEventTransport

    /// `transport` を差し替えられるのはテストと将来の OS 別実装のため。既定は AF_UNIX。
    public init(socketPath: String, transport: AgentEventTransport? = nil) {
        self.socketPath = socketPath
        self.transport = transport ?? UnixSocketEventTransport(socketPath: socketPath)
    }

    public func start() {
        transport.onMessage = { [weak self] data in
            // 解釈は受信スレッドで行い（純関数）、UI へ渡す直前だけメインへ移送する。
            guard let event = AgentEventParser.parse(data) else { return }
            DispatchQueue.main.async { self?.onEvent?(event) }
        }
        transport.start()
    }

    public func stop() {
        transport.stop()
    }
}

/// セッションごとの AF_UNIX ソケットサーバ（`AgentEventTransport` の macOS/Linux 実装）。
/// `labolabo --hook <socket>` フォワーダが Claude の hook stdin(JSON) を
/// 1 接続 = 1 イベントとして送ってくるのを受信する。
///
/// SOCK_STREAM なので接続単位でフレーミングされ、同時発火しても混線しない。
public final class UnixSocketEventTransport: AgentEventTransport, @unchecked Sendable {
    public let socketPath: String
    public var onMessage: (@Sendable (Data) -> Void)?

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
        guard !data.isEmpty else { return }
        onMessage?(data)
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
