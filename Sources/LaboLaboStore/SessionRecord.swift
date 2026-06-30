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

    public init(
        id: String,
        worktreePath: String,
        name: String,
        branch: String? = nil,
        addedAt: Date,
        sortOrder: Int
    ) {
        self.id = id
        self.worktreePath = worktreePath
        self.name = name
        self.branch = branch
        self.addedAt = addedAt
        self.sortOrder = sortOrder
    }

    public static let databaseTableName = "session"
}
