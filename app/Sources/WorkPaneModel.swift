import Foundation
import Observation
import LaboLaboEngine

enum FileViewMode: String, CaseIterable, Identifiable {
    case diff = "Diff"
    case whole = "Whole file"
    var id: String { rawValue }
}

/// 変更ファイル一覧の並べ方。
enum FileListMode: String, CaseIterable, Identifiable {
    case changedTree = "変更"   // 変更ファイルだけのディレクトリツリー
    case fullTree = "全体"       // worktree 全体のツリー（変更をマーク）
    case recent = "更新順"       // 更新時刻の新しい順（フラット）
    var id: String { rawValue }

    // rawValue は enum の生値（コンパイル時定数）でありローカライズできないため、
    // 表示用のラベルは String Catalog 経由で解決する。
    var label: String {
        switch self {
        case .changedTree: return String(localized: "変更")
        case .fullTree: return String(localized: "全体")
        case .recent: return String(localized: "更新順")
        }
    }
}

/// 作業ディレクトリ配下の 1 リポジトリ。org ディレクトリでは複数になる。
struct RepoRef: Identifiable, Hashable {
    let root: URL
    let name: String
    var id: String { root.path }
}

struct ChangedFileItem: Identifiable, Hashable {
    enum Section: String, CaseIterable {
        case staged = "Staged"
        case unstaged = "Unstaged"
        case untracked = "Untracked"
    }

    /// 表示パス（複数リポ時は `repoName/相対パス`、単一リポ時は相対パス）。ツリーの
    /// グルーピングにも使う（複数リポではリポジトリ名がトップ階層になる）。
    let path: String
    let section: Section
    let adds: Int?
    let dels: Int?
    /// 作業ツリー上のファイルの最終更新時刻（削除済みなどで取れない場合は nil）。
    let modifiedAt: Date?
    /// このファイルが属するリポジトリのルート。
    let repoRoot: URL
    /// リポジトリ内の相対パス（git 操作に使う）。
    let repoRelativePath: String

    var id: String { "\(section.rawValue):\(path)" }
    var isUntracked: Bool { section == .untracked }
    var fileName: String { (path as NSString).lastPathComponent }
}

/// 作業ディレクトリ（単一 worktree もしくは org ディレクトリ配下の複数リポジトリ）の
/// git 状態＋選択ファイル差分をライブ更新する。
@MainActor
@Observable
final class WorkPaneModel {
    /// セッションの作業ディレクトリ（リポジトリ自体か、複数リポを含む org ディレクトリ）。
    let worktree: URL

    /// 検出したリポジトリ（1 つなら通常表示、複数なら横断表示）。
    var repos: [RepoRef] = []
    var multiRepo: Bool { repos.count > 1 }
    /// 履歴/ブランチ表示の対象リポジトリ（複数リポ時にセレクタで切替）。
    var selectedRepoID: String?
    var selectedRepo: RepoRef? {
        repos.first { $0.id == selectedRepoID } ?? repos.first
    }

    var status: GitStatus?
    var items: [ChangedFileItem] = []
    var allFiles: [String] = []
    var selectedPath: String?
    var viewMode: FileViewMode = .diff
    var listMode: FileListMode = .changedTree
    var diff: FileDiff?
    var wholeText: String?
    var commits: [CommitGraphRow] = []
    /// 選択中コミット（ハッシュ）。設定時は Diff ペインにそのコミットの差分を出す。
    var selectedCommit: String?
    var commitDiff: [FileDiff]?
    var loadError: String?

    /// 変更ツリーは既定で全展開（折り畳んだものだけ記録）、全体ツリーは既定折り畳み（展開だけ記録）。
    var changedTreeCollapsed: Set<String> = []
    var fullTreeExpanded: Set<String> = []

    /// 更新時刻の新しい順（取れないものは末尾、同点はファイル名昇順）。
    var itemsByRecent: [ChangedFileItem] {
        items.sorted { lhs, rhs in
            switch (lhs.modifiedAt, rhs.modifiedAt) {
            case let (l?, r?): return l == r ? lhs.fileName < rhs.fileName : l > r
            case (_?, nil): return true
            case (nil, _?): return false
            case (nil, nil): return lhs.fileName < rhs.fileName
            }
        }
    }

    private let git = GitEngine()
    private var watcher: FileWatcher?

    init(worktree: URL) {
        self.worktree = worktree
    }

    func start() {
        guard watcher == nil else { return }
        Task {
            await discoverRepos()
            await refresh()
        }
        let watcher = FileWatcher(path: worktree) { [weak self] paths in
            Task { @MainActor in self?.scheduleRefresh(paths: paths) }
        }
        watcher.start()
        self.watcher = watcher
    }

    func stop() {
        watcher?.stop()
        watcher = nil
    }

    private var refreshTask: Task<Void, Never>?
    private var pendingPaths: Set<String> = []

    /// FSEvents からの再取得要求。実行中の refresh には合流し（多重実行しない）、
    /// バーストは 0.5 秒のデバウンスで 1 回にまとめる。refresh 中に届いた変更は
    /// 完了後にもう 1 回だけ拾う。変更パスは蓄積して部分リフレッシュに使う。
    private func scheduleRefresh(paths: [String]) {
        pendingPaths.formUnion(paths)
        guard refreshTask == nil else { return }
        refreshTask = Task { [weak self] in
            while let self, !self.pendingPaths.isEmpty {
                try? await Task.sleep(nanoseconds: 500_000_000)
                let changed = self.pendingPaths
                self.pendingPaths = []
                await self.refresh(changedPaths: changed)
            }
            self?.refreshTask = nil
        }
    }

    private func discoverRepos() async {
        let roots = await git.discoverRepos(under: worktree)
        repos = roots.map { RepoRef(root: $0, name: $0.lastPathComponent) }
        if selectedRepoID == nil { selectedRepoID = repos.first?.id }
    }

    func selectRepo(_ id: String) {
        selectedRepoID = id
        // ブランチバーは切替先のスキャン結果（キャッシュ）を即時反映する。
        if let cached = scans[id]?.status { status = cached }
        Task {
            await loadCommits()
            if selectedCommit != nil { await loadCommitDiff() }
        }
    }

    func select(_ item: ChangedFileItem) { select(path: item.path) }

    /// パスで選択（ツリーの葉/フラット行 共通）。変更が無いファイル（全体ツリー）は全文表示。
    func select(path: String) {
        selectedCommit = nil // ファイル選択に切替
        commitDiff = nil
        selectedPath = path
        let changed = items.first { $0.path == path }
        // diff が無いファイル（未変更/untracked）は全文へ。変更ファイルは Diff に戻す
        // （直前の全文表示が引き継がれて diff が見えないままになるのを防ぐ）。
        viewMode = (changed == nil || changed?.isUntracked == true) ? .whole : .diff
        Task { await loadSelection() }
    }

    /// コミットを選択（履歴グラフ）。Diff ペインにそのコミットの差分（全ファイル）を出す。
    func selectCommit(_ hash: String) {
        selectedPath = nil // コミット選択に切替
        selectedCommit = hash
        Task { await loadCommitDiff() }
    }

    private func loadCommitDiff() async {
        guard let hash = selectedCommit, let repo = selectedRepo else { commitDiff = nil; return }
        commitDiff = (try? await git.commitDiff(worktree: repo.root, hash: hash)) ?? []
    }

    private func loadCommits() async {
        guard let repo = selectedRepo else { commits = []; return }
        commits = (try? await git.commitGraph(worktree: repo.root, limit: 300)) ?? []
    }

    var selectedItem: ChangedFileItem? {
        items.first { $0.path == selectedPath }
    }

    // MARK: - ツリー

    private var changeByPath: [String: FileTreeNode.Change] {
        Dictionary(
            items.map { ($0.path, FileTreeNode.Change(section: $0.section, adds: $0.adds, dels: $0.dels)) },
            uniquingKeysWith: { first, _ in first }
        )
    }

    /// 変更ファイルだけのディレクトリツリー（複数リポ時はリポジトリ名がトップ階層）。
    var changedTree: [FileTreeNode] {
        FileTreeBuilder.build(paths: items.map(\.path), changeByPath: changeByPath)
    }

    /// 作業ディレクトリ全体のツリー（変更をマーク）。
    var fullTree: [FileTreeNode] {
        var paths = Set(allFiles)
        for item in items { paths.insert(item.path) }
        return FileTreeBuilder.build(paths: Array(paths), changeByPath: changeByPath)
    }

    func isExpanded(_ id: String, mode: FileListMode) -> Bool {
        mode == .fullTree ? fullTreeExpanded.contains(id) : !changedTreeCollapsed.contains(id)
    }

    func toggleExpanded(_ id: String, mode: FileListMode) {
        if mode == .fullTree {
            if !fullTreeExpanded.insert(id).inserted { fullTreeExpanded.remove(id) }
        } else {
            if !changedTreeCollapsed.insert(id).inserted { changedTreeCollapsed.remove(id) }
        }
    }

    /// repo 単位のスキャン結果。部分リフレッシュ時に未変更 repo の分を使い回す。
    private struct RepoScan {
        var items: [ChangedFileItem] = []
        var files: [String] = []
        var status: GitStatus?
    }

    private var scans: [String: RepoScan] = [:]

    func refresh() async {
        await refresh(changedPaths: nil)
    }

    /// `changedPaths` が nil なら全 repo を、指定時は該当 repo だけ git でスキャンし、
    /// 残りはキャッシュから合成する。パスがどの repo にも紐づかないとき（シンボリック
    /// リンク差異・新規 clone 等）は全量スキャンへフォールバックする。
    func refresh(changedPaths: Set<String>?) async {
        if repos.isEmpty { await discoverRepos() }
        guard !repos.isEmpty else {
            // git リポジトリが 1 つも見つからない作業ディレクトリ。
            items = []; allFiles = []; status = nil
            loadError = nil
            return
        }

        var targets = repos
        if let changedPaths {
            let affected = repos.filter { repo in
                let root = repo.root.path
                return changedPaths.contains {
                    $0 == root || $0.hasPrefix(root + "/") || root.hasPrefix($0 + "/")
                }
            }
            if affected.isEmpty {
                await discoverRepos() // 新規 repo が増えた可能性も拾ってから全量へ
                targets = repos
            } else {
                targets = affected
            }
        }

        let prefixWithRepo = repos.count > 1

        for repo in targets {
            guard let status = try? await git.status(worktree: repo.root) else { continue }
            var scan = RepoScan(status: status)

            let unstaged = (try? await git.numstat(worktree: repo.root, staged: false)) ?? []
            let staged = (try? await git.numstat(worktree: repo.root, staged: true)) ?? []
            let unstagedCounts = Dictionary(unstaged.map { ($0.path, $0) }, uniquingKeysWith: { a, _ in a })
            let stagedCounts = Dictionary(staged.map { ($0.path, $0) }, uniquingKeysWith: { a, _ in a })

            func displayPath(_ rel: String) -> String { prefixWithRepo ? "\(repo.name)/\(rel)" : rel }

            func makeItem(_ rel: String, _ section: ChangedFileItem.Section, _ n: NumstatEntry?) -> ChangedFileItem {
                ChangedFileItem(
                    path: displayPath(rel), section: section,
                    adds: n?.additions, dels: n?.deletions,
                    modifiedAt: modifiedDate(repo: repo.root, path: rel),
                    repoRoot: repo.root, repoRelativePath: rel
                )
            }

            for entry in status.staged {
                scan.items.append(makeItem(entry.path, .staged, stagedCounts[entry.path]))
            }
            for entry in status.unstaged where entry.kind != .unmerged {
                scan.items.append(makeItem(entry.path, .unstaged, unstagedCounts[entry.path]))
            }
            for entry in status.untracked {
                scan.items.append(makeItem(entry.path, .untracked, nil))
            }

            let files = ((try? await git.listFiles(worktree: repo.root)) ?? [])
                .filter { $0 != AgentSessionModel.localSettingsRelativePath }
            scan.files = prefixWithRepo ? files.map { "\(repo.name)/\($0)" } : files

            scans[repo.id] = scan
        }

        // 全 repo ぶんを（未変更分はキャッシュから）表示順に合成する。
        var aggregated: [ChangedFileItem] = []
        var aggregatedFiles: [String] = []
        for repo in repos {
            guard let scan = scans[repo.id] else { continue }
            aggregated.append(contentsOf: scan.items)
            aggregatedFiles.append(contentsOf: scan.files)
        }

        // 我々が注入する hooks 設定は一覧に出さない（ノイズ回避）。
        items = aggregated.filter { !$0.repoRelativePath.hasSuffix(AgentSessionModel.localSettingsRelativePath) }
        allFiles = aggregatedFiles

        // ブランチバー・履歴は対象リポジトリの状態を表示。対象 repo が今回スキャン
        // されていなければ状態は変わっていないので、git を追加で叩き直さない。
        let shown = selectedRepo ?? repos[0]
        if targets.contains(where: { $0.id == shown.id }) {
            status = scans[shown.id]?.status
            await loadCommits()
        } else if status == nil {
            status = scans[shown.id]?.status
        }

        if let selectedPath,
           !items.contains(where: { $0.path == selectedPath }),
           !allFiles.contains(selectedPath) {
            self.selectedPath = nil
        }
        loadError = nil
        await loadSelection()
    }

    func loadSelection() async {
        guard let path = selectedPath else {
            diff = nil
            wholeText = nil
            return
        }
        if let item = items.first(where: { $0.path == path }) {
            diff = try? await git.diff(worktree: item.repoRoot, path: item.repoRelativePath, staged: item.section == .staged)
            wholeText = try? git.fileContents(worktree: item.repoRoot, path: item.repoRelativePath)
        } else {
            // 全体ツリーの未変更ファイル: 表示パスからリポジトリを解決して全文表示。
            diff = nil
            wholeText = resolveWholeText(displayPath: path)
        }
    }

    /// 全体ツリーの表示パス（複数リポ時は `repoName/相対`）を実ファイルへ解決して読む。
    private func resolveWholeText(displayPath: String) -> String? {
        if repos.count > 1 {
            for repo in repos where displayPath.hasPrefix("\(repo.name)/") {
                let rel = String(displayPath.dropFirst(repo.name.count + 1))
                if let text = try? git.fileContents(worktree: repo.root, path: rel) { return text }
            }
            return nil
        }
        guard let repo = repos.first else { return nil }
        return try? git.fileContents(worktree: repo.root, path: displayPath)
    }

    /// 作業ツリー上のファイルの最終更新時刻。削除済み等で取得できなければ nil。
    private func modifiedDate(repo: URL, path: String) -> Date? {
        let url = repo.appendingPathComponent(path)
        let attrs = try? FileManager.default.attributesOfItem(atPath: url.path)
        return attrs?[.modificationDate] as? Date
    }
}
