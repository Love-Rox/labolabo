import Foundation

/// エージェントの状態検出方式。UI は精度に応じて表示を出し分ける。
public enum StatusReporting: String, Sendable, CaseIterable {
    case hooks         // Claude: hook → socket（権威ある session_id/transcript も得られ最も正確）
    case escapeSeq     // OSC/エスケープシーケンス経由（端末タイトル等）
    case screenScrape  // セルバッファのスクレイプ（脆い・最終手段）
    case none          // 状態検出なし（起動/終了しか分からない）

    /// UI 表示用ラベル。
    public var label: String {
        switch self {
        case .hooks: return "hooks（正確）"
        case .escapeSeq: return "エスケープシーケンス"
        case .screenScrape: return "画面スクレイプ"
        case .none: return "状態検出なし"
        }
    }

    /// hooks 以外はライブ状態が得られない/脆いことを UI に示すためのフラグ。
    public var providesLiveStatus: Bool { self == .hooks }
}

/// アダプタの能力フラグ。UI が機能（再開ボタン・状態ピル等）を出し分けるのに使う。
public struct AgentCapabilities: Sendable, Equatable {
    /// 状態検出方式。
    public let statusReporting: StatusReporting
    /// `--resume` 相当でセッションを再開できるか。
    public let resume: Bool
    /// worktree を CLI 側のフラグで扱えるか（false なら LaboLabo が worktree を用意し cwd 指定）。
    public let nativeWorktreeFlag: Bool

    public init(statusReporting: StatusReporting, resume: Bool, nativeWorktreeFlag: Bool) {
        self.statusReporting = statusReporting
        self.resume = resume
        self.nativeWorktreeFlag = nativeWorktreeFlag
    }
}

/// 1 つの AI コーディング CLI（Claude Code / Codex / Gemini …）を抽象化したアダプタ。
///
/// 差分の大半はデータ（実行名・表示名・能力・再開の引数形）なので protocol ではなく
/// 値型で表現し、`Sendable`/`Equatable` を安価に満たす。状態検出（hooks socket）の
/// 実体は `AgentSessionModel` が持ち、ここは「何ができるか」と「どう起動するか」を担う。
public struct AgentAdapter: Sendable, Identifiable, Equatable {
    public let id: String            // "claude" / "codex" / "gemini"
    public let displayName: String   // "Claude Code" / "Codex" / "Gemini"
    /// 端末で起動する実行ファイル名（doctor / PATH 解決に使う）。
    public let executable: String
    public let capabilities: AgentCapabilities
    /// 再開時に付ける引数テンプレート（`%@` に session id）。無ければ再開引数なし。
    private let resumeArgumentTemplate: String?

    public init(
        id: String,
        displayName: String,
        executable: String,
        capabilities: AgentCapabilities,
        resumeArgumentTemplate: String? = nil
    ) {
        self.id = id
        self.displayName = displayName
        self.executable = executable
        self.capabilities = capabilities
        self.resumeArgumentTemplate = resumeArgumentTemplate
    }

    /// 端末に流す起動コマンド文字列。resume 対応かつ id があれば継続起動する。
    public func launchCommand(resumeID: String?) -> String {
        if capabilities.resume,
           let template = resumeArgumentTemplate,
           let resumeID, !resumeID.isEmpty {
            let arg = template.replacingOccurrences(of: "%@", with: Self.shellQuoted(resumeID))
            return "\(executable) \(arg)"
        }
        return executable
    }

    static func shellQuoted(_ value: String) -> String {
        "'" + value.replacingOccurrences(of: "'", with: "'\\''") + "'"
    }
}

/// 利用可能なアダプタの登録簿。既定は Claude Code。
///
/// Codex / Gemini は「hooks 無しで degrade する」抽象の検証用。状態検出は現状 hooks のみ
/// 実装のため none（起動/終了のみ）。再開の可否・引数形はベストエフォート（実験的）。
public enum AgentAdapters {
    public static let claude = AgentAdapter(
        id: "claude",
        displayName: "Claude Code",
        executable: "claude",
        capabilities: AgentCapabilities(statusReporting: .hooks, resume: true, nativeWorktreeFlag: true),
        resumeArgumentTemplate: "--resume %@"
    )

    public static let codex = AgentAdapter(
        id: "codex",
        displayName: "Codex",
        executable: "codex",
        capabilities: AgentCapabilities(statusReporting: .none, resume: true, nativeWorktreeFlag: false),
        resumeArgumentTemplate: "resume %@"
    )

    public static let gemini = AgentAdapter(
        id: "gemini",
        displayName: "Gemini",
        executable: "gemini",
        capabilities: AgentCapabilities(statusReporting: .none, resume: false, nativeWorktreeFlag: false),
        resumeArgumentTemplate: nil
    )

    public static let all: [AgentAdapter] = [claude, codex, gemini]

    public static let `default` = claude

    /// id からアダプタを解決（未知/nil は既定＝Claude）。
    public static func find(id: String?) -> AgentAdapter {
        all.first { $0.id == id } ?? `default`
    }
}
