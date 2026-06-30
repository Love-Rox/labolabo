import Foundation
import Observation
import LaboLaboEngine

/// A repository/worktree the user has opened. Phase-0 stand-in for the GRDB-backed
/// session record; persistence + restore-on-launch arrive in a later increment.
@MainActor
@Observable
final class RepoSession: Identifiable {
    let id = UUID()
    let worktreePath: URL
    var name: String
    var branch: String?

    init(worktreePath: URL) {
        self.worktreePath = worktreePath
        self.name = worktreePath.lastPathComponent
    }
}

@MainActor
@Observable
final class SessionStore {
    var sessions: [RepoSession] = []
    var selection: RepoSession.ID?

    private let git = GitEngine()

    var selected: RepoSession? {
        guard let selection else { return nil }
        return sessions.first { $0.id == selection }
    }

    func openRepository(at url: URL) {
        if let existing = sessions.first(where: { $0.worktreePath == url }) {
            selection = existing.id
            return
        }
        let session = RepoSession(worktreePath: url)
        sessions.append(session)
        selection = session.id
        Task { [git] in
            if let status = try? await git.status(worktree: url) {
                session.branch = status.branch
            }
        }
    }

    func close(_ id: RepoSession.ID) {
        sessions.removeAll { $0.id == id }
        if selection == id { selection = sessions.first?.id }
    }
}
