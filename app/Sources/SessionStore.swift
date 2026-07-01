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
    /// 所属リポジトリの安定キー（共有 git ディレクトリ）。グルーピングに使う。
    var repoKey: String?
    /// 所属リポジトリの表示名（owner/repo もしくはフォルダ名）。
    var repoName: String?
    /// このブランチに対応する PR（gh 取得。無ければ nil）。
    var pullRequest: PullRequestInfo?

    init(id: UUID = UUID(), worktreePath: URL, name: String? = nil, branch: String? = nil) {
        self.id = id
        self.worktreePath = worktreePath
        self.name = name ?? worktreePath.lastPathComponent
        self.branch = branch
    }
}

/// サイドバーでリポジトリごとにまとめた 1 グループ。
struct SessionGroup: Identifiable {
    let key: String
    let name: String
    let sessions: [RepoSession]
    var id: String { key }
}

/// Owns the open sessions and persists them (GRDB) so the previous set + selection
/// is restored on launch.
@MainActor
@Observable
final class SessionStore {
    var sessions: [RepoSession] = []
    var selection: RepoSession.ID?
    /// リポジトリキー → 色 id。
    private(set) var repoColors: [String: String] = [:]

    private let git = GitEngine()
    private let github = GitHubEngine()
    private let db: SessionDatabase?

    init() {
        db = try? SessionDatabase(url: SessionDatabase.defaultURL())
        loadRepoColors()
        restore()
    }

    /// このブランチに対応する PR を gh から取得（失敗/未検出なら nil）。
    private func fetchPR(_ session: RepoSession) {
        Task { [github] in
            let pr = (try? await github.pullRequest(worktree: session.worktreePath)) ?? nil
            session.pullRequest = pr
        }
    }

    // MARK: - リポジトリの色

    func colorID(forRepo repoKey: String) -> String? { repoColors[repoKey] }

    func setColorID(_ id: String?, forRepo repoKey: String) {
        if let id {
            repoColors[repoKey] = id
        } else {
            repoColors.removeValue(forKey: repoKey)
        }
        try? db?.setAppState(id, forKey: "repoColor:" + repoKey)
    }

    private func loadRepoColors() {
        guard let db, let entries = try? db.appStateEntries(prefix: "repoColor:") else { return }
        var colors: [String: String] = [:]
        for (key, value) in entries {
            colors[String(key.dropFirst("repoColor:".count))] = value
        }
        repoColors = colors
    }

    var selected: RepoSession? {
        guard let selection else { return nil }
        return sessions.first { $0.id == selection }
    }

    /// リポジトリごとにまとめたグループ（名前昇順）。未解決の間は worktree の親フォルダで暫定グループ。
    var groupedSessions: [SessionGroup] {
        let grouped = Dictionary(grouping: sessions) { session in
            session.repoKey ?? session.worktreePath.deletingLastPathComponent().path
        }
        return grouped.map { key, group in
            let name = group.first?.repoName
                ?? group.first?.worktreePath.deletingLastPathComponent().lastPathComponent
                ?? "…"
            return SessionGroup(key: key, name: name, sessions: group)
        }
        .sorted { $0.name.localizedStandardCompare($1.name) == .orderedAscending }
    }

    private func resolveRepo(_ session: RepoSession) {
        Task { [git] in
            if let info = try? await git.repoInfo(worktree: session.worktreePath) {
                session.repoKey = info.key
                session.repoName = info.name
            }
        }
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
        resolveRepo(session)
        fetchPR(session)
    }

    func close(_ id: RepoSession.ID) {
        sessions.removeAll { $0.id == id }
        try? db?.deleteSession(id: id.uuidString)
        if selection == id { select(sessions.first?.id) }
    }

    func select(_ id: RepoSession.ID?) {
        selection = id
        try? db?.setSelectedSessionID(id?.uuidString)
        if let session = sessions.first(where: { $0.id == id }) { fetchPR(session) }
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
            resolveRepo(session)
            fetchPR(session)
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
