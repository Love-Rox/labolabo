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
    /// 直近のエージェント（Claude）セッション ID。次回起動時の `--resume` に使う。
    var agentSessionID: String?
    /// 直近の transcript(JSONL) パス。
    var transcriptPath: String?

    init(
        id: UUID = UUID(),
        worktreePath: URL,
        name: String? = nil,
        branch: String? = nil,
        agentSessionID: String? = nil,
        transcriptPath: String? = nil
    ) {
        self.id = id
        self.worktreePath = worktreePath
        self.name = name ?? worktreePath.lastPathComponent
        self.branch = branch
        self.agentSessionID = agentSessionID
        self.transcriptPath = transcriptPath
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

    // MARK: - New Session（worktree 作成）

    /// リポジトリ選択後に New Session シートへ渡す情報。
    struct RepoInspect: Sendable, Equatable {
        let root: URL
        let name: String
        let current: String?
        let branches: [String]
    }

    /// 選んだフォルダから所属リポジトリの root/名前/現在ブランチ/ブランチ一覧を解決する。
    func inspectRepo(at url: URL) async -> RepoInspect? {
        guard let info = try? await git.repoInfo(worktree: url) else { return nil }
        let current = (try? await git.status(worktree: url))?.branch
        let branches = (try? await git.localBranches(worktree: url)) ?? []
        return RepoInspect(
            root: URL(fileURLWithPath: info.root, isDirectory: true),
            name: info.name,
            current: current,
            branches: branches
        )
    }

    /// ブランチ名から worktree ディレクトリ用の slug。
    static func worktreeSlug(_ branch: String) -> String {
        branch
            .replacingOccurrences(of: "/", with: "-")
            .replacingOccurrences(of: " ", with: "-")
            .replacingOccurrences(of: "..", with: "-")
    }

    /// 既定の worktree 配置先。ユーザー慣習に合わせて兄弟 `<repo>-wt-<slug>`。
    static func defaultWorktreePath(repoRoot: URL, branch: String) -> URL {
        let parent = repoRoot.deletingLastPathComponent()
        return parent.appendingPathComponent("\(repoRoot.lastPathComponent)-wt-\(worktreeSlug(branch))")
    }

    /// 新規ブランチ＋worktree を作成してセッション化する。失敗時は throw（呼び出し側で表示）。
    func createWorktreeSession(
        repoRoot: URL, baseRef: String, newBranch: String, name: String, worktreePath: URL
    ) async throws {
        try await git.addWorktree(repo: repoRoot, path: worktreePath, branch: newBranch, baseRef: baseRef)
        let session = RepoSession(worktreePath: worktreePath, name: name, branch: newBranch)
        sessions.append(session)
        persist(session)
        select(session.id)
        resolveRepo(session)
        fetchPR(session)
    }

    func close(_ id: RepoSession.ID) {
        sessions.removeAll { $0.id == id }
        try? db?.deleteSession(id: id.uuidString)
        if selection == id { select(sessions.first?.id) }
    }

    // MARK: - worktree の撤去（破壊的・確認付き）

    /// worktree に未コミット/未追跡の変更があるか（削除ダイアログの文言に使う）。
    func isWorktreeDirty(_ id: RepoSession.ID) async -> Bool {
        guard let session = sessions.first(where: { $0.id == id }) else { return false }
        return (try? await git.status(worktree: session.worktreePath))?.isDirty ?? false
    }

    /// `git worktree remove` で worktree を撤去し、成功したらセッションも閉じる。
    /// dirty は `force: true` のときのみ削除（呼び出し側で確認する）。`rm -rf` はしない。
    /// 失敗時（main worktree・git エラー等）は throw して呼び出し側で表示する。
    func removeWorktree(_ id: RepoSession.ID, force: Bool) async throws {
        guard let session = sessions.first(where: { $0.id == id }) else { return }
        let root: URL
        if let info = try? await git.repoInfo(worktree: session.worktreePath) {
            root = URL(fileURLWithPath: info.root, isDirectory: true)
        } else {
            root = session.worktreePath.deletingLastPathComponent()
        }
        try await git.removeWorktree(repo: root, path: session.worktreePath, force: force)
        close(id)
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
                branch: record.branch,
                agentSessionID: record.agentSessionId,
                transcriptPath: record.transcriptPath
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
            sortOrder: order,
            agentSessionId: session.agentSessionID,
            transcriptPath: session.transcriptPath
        )
        try? db.upsert(record)
    }

    /// hooks から受け取ったエージェントセッション ID/transcript を保存（次回起動の `--resume` 用）。
    func updateAgentSession(_ id: RepoSession.ID, agentSessionID: String, transcriptPath: String?) {
        guard let session = sessions.first(where: { $0.id == id }) else { return }
        // 変化がなければ書き込みしない（hook 連打での無駄な書き込みを避ける）。
        if session.agentSessionID == agentSessionID && (transcriptPath == nil || session.transcriptPath == transcriptPath) {
            return
        }
        session.agentSessionID = agentSessionID
        if let transcriptPath { session.transcriptPath = transcriptPath }
        persist(session)
    }
}
