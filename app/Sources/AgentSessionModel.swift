import Foundation
import Observation
import LaboLaboEngine

/// 1 セッション分のエージェント状態。AF_UNIX ソケットで Claude hooks を受信し、
/// `status` をライブ更新する。
///
/// hooks は worktree の `.claude/settings.local.json`（ローカル・gitignore 前提）へ
/// 注入する。これにより ✨ ボタン経由でも、ユーザーが既存端末で手で `claude` と
/// 打った場合でも、同じソケットへイベントが届く。既存ファイルはスナップショットして
/// マージし、セッション終了時に原本へ復元する（ユーザー設定を壊さない）。
@MainActor
@Observable
final class AgentSessionModel {
    private(set) var status: AgentStatus = .none
    private(set) var lastSessionID: String?
    private(set) var lastTranscriptPath: String?

    let socketPath: String
    private let bus: AgentStatusBus
    private let worktree: URL
    private var createdSettings = false

    private var claudeDir: URL { worktree.appendingPathComponent(".claude", isDirectory: true) }
    private var localSettingsURL: URL { claudeDir.appendingPathComponent("settings.local.json") }
    private var backupURL: URL { claudeDir.appendingPathComponent("settings.local.json.labolabo-bak") }

    static let localSettingsRelativePath = ".claude/settings.local.json"

    private static let events = [
        "SessionStart", "UserPromptSubmit", "PreToolUse",
        "PostToolUse", "Notification", "Stop", "SessionEnd",
    ]

    init(sessionID: UUID, worktree: URL) {
        self.worktree = worktree
        let short = sessionID.uuidString.replacingOccurrences(of: "-", with: "").prefix(10).lowercased()
        let dir = "/tmp/labolabo"
        try? FileManager.default.createDirectory(
            atPath: dir,
            withIntermediateDirectories: true,
            attributes: [.posixPermissions: 0o700]
        )
        socketPath = "\(dir)/\(short).sock"
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
        installLocalSettings()
    }

    func stop() {
        removeLocalSettings()
        bus.stop()
    }

    /// ✨ ボタンが新規端末で実行するコマンド。hooks は settings.local.json 経由で効くため、
    /// ここでは素の `claude` を実行するだけでよい。
    func launchCommand() -> String { "claude" }

    // MARK: - settings.local.json への安全な hooks 注入

    private func installLocalSettings() {
        let fm = FileManager.default
        try? fm.createDirectory(at: claudeDir, withIntermediateDirectories: true)

        // 前回クラッシュ等でバックアップが残っていたら原本を先に戻す。
        if fm.fileExists(atPath: backupURL.path) {
            try? fm.removeItem(at: localSettingsURL)
            try? fm.moveItem(at: backupURL, to: localSettingsURL)
        }

        var root: [String: Any] = [:]
        if let data = try? Data(contentsOf: localSettingsURL),
           let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any] {
            root = object
            try? data.write(to: backupURL) // 原本をスナップショット
            createdSettings = false
        } else {
            createdSettings = true
        }

        var hooks = (root["hooks"] as? [String: Any]) ?? [:]
        let entry = hookEntry()
        for event in Self.events {
            var array = (hooks[event] as? [[String: Any]]) ?? []
            array.append(entry)
            hooks[event] = array
        }
        root["hooks"] = hooks

        if let data = try? JSONSerialization.data(withJSONObject: root, options: [.prettyPrinted, .sortedKeys]) {
            try? data.write(to: localSettingsURL)
        }
    }

    private func removeLocalSettings() {
        let fm = FileManager.default
        if fm.fileExists(atPath: backupURL.path) {
            try? fm.removeItem(at: localSettingsURL)
            try? fm.moveItem(at: backupURL, to: localSettingsURL) // 原本へ復元
        } else if createdSettings {
            try? fm.removeItem(at: localSettingsURL) // 我々が作ったので消す
        }
    }

    private func hookEntry() -> [String: Any] {
        let binary = Bundle.main.executablePath ?? ""
        let forwarder = "\(Self.shellQuoted(binary)) --hook \(Self.shellQuoted(socketPath))"
        return [
            "matcher": "",
            "hooks": [["type": "command", "command": forwarder, "timeout": 5]],
        ]
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
