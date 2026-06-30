import Foundation
import Observation
import LaboLaboEngine

/// 1 セッション分のエージェント状態。AF_UNIX ソケットで Claude hooks を受信し、
/// `status` をライブ更新する。`launchCommand()` は hooks 注入付きの起動コマンドを返す。
@MainActor
@Observable
final class AgentSessionModel {
    private(set) var status: AgentStatus = .none
    private(set) var lastSessionID: String?
    private(set) var lastTranscriptPath: String?

    private let bus: AgentStatusBus
    let socketPath: String
    private let settingsPath: String

    init(sessionID: UUID) {
        let short = sessionID.uuidString.replacingOccurrences(of: "-", with: "").prefix(10).lowercased()
        let dir = "/tmp/labolabo"
        try? FileManager.default.createDirectory(
            atPath: dir,
            withIntermediateDirectories: true,
            attributes: [.posixPermissions: 0o700]
        )
        socketPath = "\(dir)/\(short).sock"
        settingsPath = "\(dir)/\(short).settings.json"
        bus = AgentStatusBus(socketPath: socketPath)
    }

    func start() {
        bus.onEvent = { [weak self] event in
            guard let self else { return }
            status = event.status
            if let id = event.sessionID { lastSessionID = id }
            if let path = event.transcriptPath { lastTranscriptPath = path }
        }
        bus.start()
    }

    func stop() {
        bus.stop()
    }

    /// hooks 注入付きで Claude を起動するコマンド（末尾に改行付き。端末へそのまま送る）。
    /// 設定の書き出しに失敗したら nil。
    func launchCommand() -> String? {
        guard writeSettings() else { return nil }
        // 端末の Enter はキャリッジリターン(\r)。\n だと実行されず改行止まりになる。
        return "claude --settings \(Self.shellQuoted(settingsPath))\r"
    }

    // MARK: - settings

    private func writeSettings() -> Bool {
        guard let binary = Bundle.main.executablePath else { return false }
        let forwarder = "\(Self.shellQuoted(binary)) --hook \(Self.shellQuoted(socketPath))"
        // command hook はブロッキング（既定 600s）。フォワーダは即終了だが、万一の
        // ハングで Claude を止めないよう短い timeout を付ける。
        let commandHooks: [[String: Any]] = [[
            "type": "command",
            "command": forwarder,
            "timeout": 5,
        ]]

        // 公式ドキュメント準拠: 全イベントで matcher: ""（空＝全対象）の統一スキーマ。
        let events = [
            "SessionStart", "UserPromptSubmit", "PreToolUse",
            "PostToolUse", "Notification", "Stop", "SessionEnd",
        ]
        var hooks: [String: Any] = [:]
        for event in events {
            hooks[event] = [["matcher": "", "hooks": commandHooks]]
        }
        let settings: [String: Any] = ["hooks": hooks]

        guard let data = try? JSONSerialization.data(withJSONObject: settings, options: [.prettyPrinted]) else {
            return false
        }
        return (try? data.write(to: URL(fileURLWithPath: settingsPath))) != nil
    }

    private static func shellQuoted(_ value: String) -> String {
        "'" + value.replacingOccurrences(of: "'", with: "'\\''") + "'"
    }
}

extension AgentStatus {
    /// ステータスピル用の表示ラベル。
    var label: String {
        switch self {
        case .none: return "—"
        case .starting: return "起動中"
        case .running: return "実行中"
        case .waitingForInput: return "入力待ち"
        case .idle: return "待機"
        case .ended: return "終了"
        }
    }
}
