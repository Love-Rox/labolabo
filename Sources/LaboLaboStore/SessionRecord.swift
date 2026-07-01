import Foundation
import GRDB

/// A persisted, opened repository/worktree session. Phase-0 restore-on-launch
/// reopens these; richer state (terminal tabs/panes, agent session ids) is added
/// in later migrations following the cmux model.
public struct SessionRecord: Codable, FetchableRecord, PersistableRecord, Identifiable, Equatable, Sendable {
    public var id: String            // UUID string (stable across launches)
    public var worktreePath: String
    public var name: String
    public var branch: String?
    public var addedAt: Date
    public var sortOrder: Int
    /// 直近のエージェント（Claude）セッション ID。次回起動時の `--resume` に使う。
    public var agentSessionId: String?
    /// 直近の transcript(JSONL) パス。usage/cost の best-effort 取得などに使う。
    public var transcriptPath: String?

    public init(
        id: String,
        worktreePath: String,
        name: String,
        branch: String? = nil,
        addedAt: Date,
        sortOrder: Int,
        agentSessionId: String? = nil,
        transcriptPath: String? = nil
    ) {
        self.id = id
        self.worktreePath = worktreePath
        self.name = name
        self.branch = branch
        self.addedAt = addedAt
        self.sortOrder = sortOrder
        self.agentSessionId = agentSessionId
        self.transcriptPath = transcriptPath
    }

    public static let databaseTableName = "session"
}
