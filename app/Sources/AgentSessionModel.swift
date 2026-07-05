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
    /// transcript から集計した使用量（best-effort・推定）。hooks 方式のときだけ得られる。
    private(set) var usage: AgentUsage?

    let socketPath: String
    /// このセッションのエージェント種別（Claude / Codex / Gemini …）。
    let adapter: AgentAdapter
    private let bus: AgentStatusBus
    private let worktree: URL
    private var createdSettings = false

    /// hooks 方式のときだけ settings.local.json を注入して状態を受信する。
    private var usesHooks: Bool { adapter.capabilities.statusReporting == .hooks }

    /// 前回起動時に永続化されたエージェントセッション ID（あれば `--resume` する）。
    private let initialResumeID: String?
    /// 新しいセッション ID/transcript を受け取ったら永続化するためのコールバック。
    private let onSessionID: ((String, String?) -> Void)?
    /// 状態が変化したときに呼ばれる（入力待ちの通知などに使う）。
    private let onStatusChange: ((AgentStatus) -> Void)?

    /// resume に使う ID。今回受信済みなら最新、無ければ前回永続化分。
    private var resumeID: String? { lastSessionID ?? initialResumeID }

    private var claudeDir: URL { worktree.appendingPathComponent(".claude", isDirectory: true) }
    private var localSettingsURL: URL { claudeDir.appendingPathComponent("settings.local.json") }
    private var backupURL: URL { claudeDir.appendingPathComponent("settings.local.json.labolabo-bak") }

    static let localSettingsRelativePath = ".claude/settings.local.json"

    private static let events = [
        "SessionStart", "UserPromptSubmit", "PreToolUse",
        "PostToolUse", "Notification", "Stop", "SessionEnd",
    ]

    init(
        sessionID: UUID,
        worktree: URL,
        adapter: AgentAdapter = AgentAdapters.default,
        resumeID: String? = nil,
        onSessionID: ((String, String?) -> Void)? = nil,
        onStatusChange: ((AgentStatus) -> Void)? = nil
    ) {
        self.worktree = worktree
        self.adapter = adapter
        self.initialResumeID = resumeID
        self.onSessionID = onSessionID
        self.onStatusChange = onStatusChange
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
            let previous = status
            status = event.status
            if let path = event.transcriptPath { lastTranscriptPath = path }
            if let id = event.sessionID {
                lastSessionID = id
                onSessionID?(id, event.transcriptPath) // 次回起動の --resume 用に永続化
            }
            if status != previous { onStatusChange?(status) }
            // 応答完了/終了時に transcript から使用量を集計（推定）。
            if status == .idle || status == .ended, let path = lastTranscriptPath {
                refreshUsage(path: path)
            }
        }
        bus.start()
        if usesHooks { installLocalSettings() }
    }

    /// transcript を読み usage を更新する。ファイル読み取り＋パースはバックグラウンドで。
    private func refreshUsage(path: String) {
        Task.detached(priority: .utility) { [weak self] in
            guard let parsed = TranscriptUsage.read(path: path) else { return }
            await self?.applyUsage(parsed)
        }
    }

    /// 集計した usage をメインで反映する。@MainActor 隔離のセッターにすることで、
    /// detached タスクから `self` をクロージャへ送らずに済ませる（Xcode 16 SDK の
    /// strict concurrency では `MainActor.run { self?... }` が「sending self」で拒否される）。
    private func applyUsage(_ value: AgentUsage) {
        usage = value
    }

    func stop() {
        if usesHooks { removeLocalSettings() }
        bus.stop()
    }

    /// ✨ ボタンが新規端末で実行するコマンド。hooks は settings.local.json 経由で効くため、
    /// コマンド自体は素の実行名。再開対応かつ前回 ID があれば継続起動する。
    func launchCommand() -> String {
        adapter.launchCommand(resumeID: resumeID)
    }

    /// resume 可能か（UI 側で「再開」表示の出し分けに使う）。アダプタが再開対応で、
    /// かつ再開に使える ID を持っているとき true。
    var canResume: Bool { adapter.capabilities.resume && resumeID?.isEmpty == false }

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
        case .starting: return String(localized: "起動中")
        case .running: return String(localized: "実行中")
        case .waitingForInput: return String(localized: "入力待ち")
        case .idle: return String(localized: "待機")
        case .ended: return String(localized: "終了")
        }
    }
}
