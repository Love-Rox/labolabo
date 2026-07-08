import Foundation
import Observation
import AppKit
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
    /// このセッションのエージェント種別（"claude" / "codex" / "gemini"）。
    var adapterID: String
    /// このセッションのエージェント状態モデル（開いている間 store が保持・監視）。
    /// 背景セッションでも hooks を受信するため、選択の有無に関係なく生かす。
    var agent: AgentSessionModel?

    init(
        id: UUID = UUID(),
        worktreePath: URL,
        name: String? = nil,
        branch: String? = nil,
        agentSessionID: String? = nil,
        transcriptPath: String? = nil,
        adapterID: String = AgentAdapters.default.id
    ) {
        self.id = id
        self.worktreePath = worktreePath
        self.name = name ?? worktreePath.lastPathComponent
        self.branch = branch
        self.agentSessionID = agentSessionID
        self.transcriptPath = transcriptPath
        self.adapterID = adapterID
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
    /// セッション毎の「変更中ファイル」（worktree ルート相対パス）の逆引き。
    /// 同一 repo に複数セッションがあり同じパスを触っていればコンフリクト警告に使う。
    private(set) var changedFiles: [RepoSession.ID: Set<String>] = [:]

    private let git = GitEngine()
    private let github = GitHubEngine()
    private let db: SessionDatabase?

    init() {
        db = try? SessionDatabase(url: SessionDatabase.defaultURL())
        loadRepoColors()
        loadPresets()
    }

    private var started = false

    /// セッション復元と常駐監視の開始。`@State` の初期値式は View の再 init のたびに
    /// 評価され使い捨てインスタンスが生まれるため、プロセス起動・スレッド生成・
    /// ソケット bind などの副作用は init に置かず、表示側から 1 回だけ呼ぶ。
    func start() {
        guard !started else { return }
        started = true
        restore()
        startConflictWatch()
    }

    /// このブランチに対応する PR を gh から取得（失敗/未検出なら nil）。
    private func fetchPR(_ session: RepoSession) {
        Task { [github] in
            let pr = (try? await github.pullRequest(worktree: session.worktreePath)) ?? nil
            session.pullRequest = pr
        }
    }

    /// 現在ブランチを push して PR を作成し、PR の URL を返す（失敗は throw）。
    func createPullRequest(
        _ id: RepoSession.ID, base: String, title: String, body: String, draft: Bool
    ) async throws -> String {
        guard let session = sessions.first(where: { $0.id == id }) else {
            throw NSError(domain: "SessionStore", code: 1, userInfo: [
                NSLocalizedDescriptionKey: String(localized: "セッションが見つかりません（閉じられた可能性があります）。"),
            ])
        }
        try await git.push(worktree: session.worktreePath)
        let url = try await github.createPullRequest(
            worktree: session.worktreePath, base: base, title: title, body: body, draft: draft
        )
        fetchPR(session)
        return url
    }

    // MARK: - エージェント状態（セッション寿命で監視）

    /// セッションのエージェント状態モデルを生成・起動して保持する。選択に関係なく
    /// 生かすことで、背景セッションの「入力待ち」も検知・通知できる。
    private func attachAgent(_ session: RepoSession) {
        guard session.agent == nil else { return }
        let sid = session.id
        let agent = AgentSessionModel(
            sessionID: sid,
            worktree: session.worktreePath,
            adapter: AgentAdapters.find(id: session.adapterID),
            resumeID: session.agentSessionID,
            onSessionID: { [weak self] id, tp in
                self?.updateAgentSession(sid, agentSessionID: id, transcriptPath: tp)
            },
            onStatusChange: { [weak self] status in
                self?.handleStatusChange(sid, status)
            }
        )
        session.agent = agent
        agent.start()
    }

    /// 入力待ちに入ったら通知（前面かつ当該セッション選択中は不要＝ピルで見えているため）。
    private func handleStatusChange(_ id: RepoSession.ID, _ status: AgentStatus) {
        guard status == .waitingForInput,
              let session = sessions.first(where: { $0.id == id }) else { return }
        let visibleNow = NSApp.isActive && selection == id
        if !visibleNow {
            AgentNotifier.notifyWaiting(sessionName: session.name, branch: session.branch)
        }
    }

    // MARK: - セッション間の変更ファイル逆引き＋コンフリクト警告

    /// 1 つのファイルについて、他セッションでも編集中であることを表す。
    struct FileConflict: Identifiable, Equatable {
        /// worktree ルート相対パス。
        let path: String
        /// 同じパスを触っている他セッションの表示名（ブランチ優先）。
        let others: [String]
        var id: String { path }
    }

    /// 変更ファイルの逆引きを更新する。コンフリクトは同一 repo に複数セッションが
    /// あるときだけ起きるので、その対象セッションについてのみ `git status` を回す。
    func refreshChangedFiles() {
        let byRepo = Dictionary(grouping: sessions.filter { $0.repoKey != nil }) { $0.repoKey! }
        let targetIDs = Set(byRepo.filter { $0.value.count >= 2 }.flatMap { $0.value.map(\.id) })

        // 対象から外れたセッションの記録は消す（1 つに戻れば警告も消える）。
        for id in Array(changedFiles.keys) where !targetIDs.contains(id) {
            changedFiles.removeValue(forKey: id)
        }

        for session in sessions where targetIDs.contains(session.id) {
            let sid = session.id
            let worktree = session.worktreePath
            Task { [weak self] in
                guard let self else { return }
                let status = try? await self.git.status(worktree: worktree)
                let paths = Set((status?.entries ?? [])
                    .filter { $0.kind != .ignored }
                    .flatMap { entry in [entry.path, entry.originalPath].compactMap { $0 } })
                self.changedFiles[sid] = paths
            }
        }
    }

    /// このセッションが変更中で、かつ同一 repo の別セッションも変更中のファイル一覧。
    /// 検出ロジックはエンジンの純粋関数に委譲し、他セッション id を表示名へ変換する。
    func conflicts(for id: RepoSession.ID) -> [FileConflict] {
        let inputs = sessions.map {
            CrossSessionConflicts.Session(
                id: $0.id.uuidString, repoKey: $0.repoKey, changed: changedFiles[$0.id] ?? []
            )
        }
        let raw = CrossSessionConflicts.conflicts(for: id.uuidString, among: inputs)
        guard !raw.isEmpty else { return [] }
        return raw.map { conflict in
            let labels = conflict.others.compactMap { oid in
                sessions.first { $0.id.uuidString == oid }.map { $0.branch ?? $0.name }
            }
            return FileConflict(path: conflict.path, others: labels)
        }
    }

    /// 背景セッション（エージェントが編集中など）でも検知できるよう、変更ファイルの
    /// 逆引きを定期リフレッシュする。多重 repo が無ければ処理はほぼ空で軽い。
    private func startConflictWatch() {
        Task { [weak self] in
            while !Task.isCancelled {
                try? await Task.sleep(nanoseconds: 5_000_000_000) // 5s
                guard let self else { return }
                self.refreshChangedFiles()
            }
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
        Task { [weak self, git] in
            if let info = try? await git.repoInfo(worktree: session.worktreePath) {
                session.repoKey = info.key
                session.repoName = info.name
                // repo が判明したら逆引きを更新（同一 repo の同居が分かる）。
                self?.refreshChangedFiles()
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
        attachAgent(session)
    }

    // MARK: - New Session（worktree 作成）

    /// リポジトリ選択後に New Session / PR 作成シートへ渡す情報。
    struct RepoInspect: Sendable, Equatable {
        let root: URL
        let name: String
        let current: String?
        let branches: [String]
        /// 直近コミットの件名（PR タイトルの初期値用）。
        let lastSubject: String?
    }

    /// 選んだフォルダから所属リポジトリの root/名前/現在ブランチ/ブランチ一覧を解決する。
    func inspectRepo(at url: URL) async -> RepoInspect? {
        guard let info = try? await git.repoInfo(worktree: url) else { return nil }
        let current = (try? await git.status(worktree: url))?.branch
        let branches = (try? await git.localBranches(worktree: url)) ?? []
        let lastSubject = try? await git.lastCommitSubject(worktree: url)
        return RepoInspect(
            root: URL(fileURLWithPath: info.root, isDirectory: true),
            name: info.name,
            current: current,
            branches: branches,
            lastSubject: (lastSubject?.isEmpty == false) ? lastSubject : nil
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
        repoRoot: URL, baseRef: String, newBranch: String, name: String, worktreePath: URL,
        adapterID: String = AgentAdapters.default.id
    ) async throws {
        try await git.addWorktree(repo: repoRoot, path: worktreePath, branch: newBranch, baseRef: baseRef)
        let session = RepoSession(worktreePath: worktreePath, name: name, branch: newBranch, adapterID: adapterID)
        sessions.append(session)
        persist(session)
        select(session.id)
        resolveRepo(session)
        fetchPR(session)
        attachAgent(session)
    }

    /// org ディレクトリ配下の git リポジトリを検出（Part A の候補列挙）。
    func discoverRepos(under url: URL) async -> [URL] {
        await git.discoverRepos(under: url)
    }

    /// 複数リポジトリをまとめてセッション化する（org を個別セッションで開く）。
    /// 既に開いているパスはスキップ。最初に新規追加したものを選択する。
    func openRepositories(_ urls: [URL]) {
        var firstNew: RepoSession.ID?
        for url in urls {
            if let existing = sessions.first(where: { $0.worktreePath == url }) {
                firstNew = firstNew ?? existing.id
                continue
            }
            let session = RepoSession(worktreePath: url)
            sessions.append(session)
            persist(session)
            refreshBranch(session)
            resolveRepo(session)
            fetchPR(session)
            attachAgent(session)
            firstNew = firstNew ?? session.id
        }
        if let firstNew { select(firstNew) }
    }

    func close(_ id: RepoSession.ID) {
        if let session = sessions.first(where: { $0.id == id }) {
            session.agent?.stop()  // hooks 撤去 + ソケット停止
        }
        sessions.removeAll { $0.id == id }
        changedFiles.removeValue(forKey: id)
        try? db?.deleteSession(id: id.uuidString)
        if selection == id { select(sessions.first?.id) }
        refreshChangedFiles() // 同居セッションが 1 つに戻れば警告が消える
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
                transcriptPath: record.transcriptPath,
                adapterID: record.adapterId ?? AgentAdapters.default.id
            )
            sessions.append(session)
            refreshBranch(session)
            resolveRepo(session)
            fetchPR(session)
            attachAgent(session)
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
            transcriptPath: session.transcriptPath,
            adapterId: session.adapterID
        )
        try? db.upsert(record)
    }

    // MARK: - ペイン配置（セッション別 ＋ 名前付きプリセット）

    private static let defaultLayoutKey = "paneLayout:default"

    /// セッション固有の保存済み配置。無ければ nil。
    func paneLayout(for id: RepoSession.ID) -> TileLayout? {
        decodeLayout((try? db?.appState(forKey: "paneLayout:" + id.uuidString)) ?? nil)
    }

    /// 新規セッションが継承する既定配置（直近に保存された配置）。
    func defaultPaneLayout() -> TileLayout? {
        decodeLayout((try? db?.appState(forKey: Self.defaultLayoutKey)) ?? nil)
    }

    /// セッションの配置を保存し、同時に「新規セッションが継承する既定配置」も更新する。
    func savePaneLayout(_ id: RepoSession.ID, _ layout: TileLayout) {
        guard let json = encodeLayout(layout) else { return }
        try? db?.setAppState(json, forKey: "paneLayout:" + id.uuidString)
        try? db?.setAppState(json, forKey: Self.defaultLayoutKey)
    }

    /// 名前付きプリセット一覧（全セッション共通）。
    private(set) var presets: [LayoutPreset] = []

    private func loadPresets() {
        guard let json = (try? db?.appState(forKey: "panePresets")) ?? nil,
              let data = json.data(using: .utf8),
              let list = try? JSONDecoder().decode([LayoutPreset].self, from: data) else {
            presets = []
            return
        }
        presets = list
    }

    /// 現在の配置を名前付きプリセットとして保存（同名は上書き）。
    func savePreset(name: String, layout: TileLayout) {
        let trimmed = name.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return }
        var list = presets.filter { $0.name != trimmed }
        list.append(LayoutPreset(name: trimmed, layout: layout))
        list.sort { $0.name.localizedStandardCompare($1.name) == .orderedAscending }
        persistPresets(list)
    }

    func deletePreset(name: String) {
        persistPresets(presets.filter { $0.name != name })
    }

    private func persistPresets(_ list: [LayoutPreset]) {
        presets = list
        if let data = try? JSONEncoder().encode(list), let json = String(data: data, encoding: .utf8) {
            try? db?.setAppState(json, forKey: "panePresets")
        }
    }

    private func encodeLayout(_ layout: TileLayout) -> String? {
        guard let data = try? JSONEncoder().encode(layout) else { return nil }
        return String(data: data, encoding: .utf8)
    }

    private func decodeLayout(_ json: String?) -> TileLayout? {
        guard let json, let data = json.data(using: .utf8) else { return nil }
        return try? JSONDecoder().decode(TileLayout.self, from: data)
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
