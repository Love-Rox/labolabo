import Foundation
import Observation
import LaboLaboEngine
import LaboLaboStore

/// A repository/worktree the user has opened. `id` is stable across launches so
/// persisted selection survives restarts.
@MainActor
@Observable
final class RepoSession: Identifiable {
    let id: UUID
    let worktreePath: URL
    var name: String
    var branch: String?

    init(id: UUID = UUID(), worktreePath: URL, name: String? = nil, branch: String? = nil) {
        self.id = id
        self.worktreePath = worktreePath
        self.name = name ?? worktreePath.lastPathComponent
        self.branch = branch
    }
}

/// Owns the open sessions and persists them (GRDB) so the previous set + selection
/// is restored on launch.
@MainActor
@Observable
final class SessionStore {
    var sessions: [RepoSession] = []
    var selection: RepoSession.ID?

    private let git = GitEngine()
    private let db: SessionDatabase?

    init() {
        db = try? SessionDatabase(url: SessionDatabase.defaultURL())
        restore()
    }

    var selected: RepoSession? {
        guard let selection else { return nil }
        return sessions.first { $0.id == selection }
    }

    func openRepository(at url: URL) {
        if let existing = sessions.first(where: { $0.worktreePath == url }) {
            select(existing.id)
            return
        }
        let session = RepoSession(worktreePath: url)
        sessions.append(session)
        persist(session)
        select(session.id)
        refreshBranch(session)
    }

    func close(_ id: RepoSession.ID) {
        sessions.removeAll { $0.id == id }
        try? db?.deleteSession(id: id.uuidString)
        if selection == id { select(sessions.first?.id) }
    }

    func select(_ id: RepoSession.ID?) {
        selection = id
        try? db?.setSelectedSessionID(id?.uuidString)
    }

    // MARK: - Restore on launch

    private func restore() {
        guard let db else { return }
        let records = (try? db.allSessions()) ?? []
        for record in records {
            guard let uuid = UUID(uuidString: record.id) else { continue }
            let session = RepoSession(
                id: uuid,
                worktreePath: URL(fileURLWithPath: record.worktreePath),
                name: record.name,
                branch: record.branch
            )
            sessions.append(session)
            refreshBranch(session)
        }

        let storedSelection = (try? db.selectedSessionID()) ?? nil
        if let storedSelection,
           let uuid = UUID(uuidString: storedSelection),
           sessions.contains(where: { $0.id == uuid }) {
            selection = uuid
        } else {
            selection = sessions.first?.id
        }
    }

    // MARK: - Persistence helpers

    private func refreshBranch(_ session: RepoSession) {
        Task { [git, weak self] in
            if let status = try? await git.status(worktree: session.worktreePath) {
                session.branch = status.branch
                self?.persist(session)
            }
        }
    }

    private func persist(_ session: RepoSession) {
        guard let db else { return }
        let order = sessions.firstIndex { $0.id == session.id } ?? sessions.count
        let record = SessionRecord(
            id: session.id.uuidString,
            worktreePath: session.worktreePath.path,
            name: session.name,
            branch: session.branch,
            addedAt: Date(),
            sortOrder: order
        )
        try? db.upsert(record)
    }
}
