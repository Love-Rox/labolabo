import Foundation

/// 1 つのエージェント（Claude Code 等）セッションのライブ状態。
public enum AgentStatus: String, Sendable {
    case none             // 未起動 / 未接続
    case starting         // SessionStart
    case running          // UserPromptSubmit / PreToolUse / PostToolUse（思考・ツール実行中）
    case waitingForInput  // Notification（入力・許可待ち）
    case idle             // Stop（応答完了・待機）
    case ended            // SessionEnd

    /// Claude hook の `hook_event_name` から状態へマッピング（未知イベントは nil）。
    public static func from(hookEvent: String) -> AgentStatus? {
        switch hookEvent {
        case "SessionStart": return .starting
        case "UserPromptSubmit", "PreToolUse", "PostToolUse": return .running
        case "Notification": return .waitingForInput
        case "Stop", "SubagentStop": return .idle
        case "SessionEnd": return .ended
        default: return nil
        }
    }
}

/// hook フォワーダから受信した 1 イベント。
public struct AgentStatusEvent: Sendable {
    public let hookEvent: String
    public let status: AgentStatus
    public let sessionID: String?
    public let transcriptPath: String?
    public let cwd: String?

    public init(
        hookEvent: String,
        status: AgentStatus,
        sessionID: String?,
        transcriptPath: String?,
        cwd: String?
    ) {
        self.hookEvent = hookEvent
        self.status = status
        self.sessionID = sessionID
        self.transcriptPath = transcriptPath
        self.cwd = cwd
    }
}
