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
}

struct ChangedFileItem: Identifiable, Hashable {
    enum Section: String, CaseIterable {
        case staged = "Staged"
        case unstaged = "Unstaged"
        case untracked = "Untracked"
    }

    let path: String
    let section: Section
    let adds: Int?
    let dels: Int?
    /// 作業ツリー上のファイルの最終更新時刻（削除済みなどで取れない場合は nil）。
    let modifiedAt: Date?

    var id: String { "\(section.rawValue):\(path)" }
    var isUntracked: Bool { section == .untracked }
    var fileName: String { (path as NSString).lastPathComponent }
}

/// Loads and live-refreshes the git state + selected-file diff for one worktree.
@MainActor
@Observable
final class WorkPaneModel {
    let worktree: URL

    var status: GitStatus?
    var items: [ChangedFileItem] = []
    var allFiles: [String] = []
    var selectedPath: String?
    var viewMode: FileViewMode = .diff
    var listMode: FileListMode = .changedTree
    var diff: FileDiff?
    var wholeText: String?
    var commits: [CommitGraphLine] = []
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
        Task { await refresh() }
        let watcher = FileWatcher(path: worktree) { [weak self] in
            Task { @MainActor in await self?.refresh() }
        }
        watcher.start()
        self.watcher = watcher
    }

    func stop() {
        watcher?.stop()
        watcher = nil
    }

    func select(_ item: ChangedFileItem) { select(path: item.path) }

    /// パスで選択（ツリーの葉/フラット行 共通）。変更が無いファイル（全体ツリー）は全文表示。
    func select(path: String) {
        selectedPath = path
        let changed = items.first { $0.path == path }
        if changed == nil || changed?.isUntracked == true {
            viewMode = .whole
        }
        Task { await loadSelection() }
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

    /// 変更ファイルだけのディレクトリツリー。
    var changedTree: [FileTreeNode] {
        FileTreeBuilder.build(paths: items.map(\.path), changeByPath: changeByPath)
    }

    /// worktree 全体のツリー（変更をマーク）。
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

    func refresh() async {
        do {
            let status = try await git.status(worktree: worktree)
            self.status = status

            let unstaged = (try? await git.numstat(worktree: worktree, staged: false)) ?? []
            let staged = (try? await git.numstat(worktree: worktree, staged: true)) ?? []
            let unstagedCounts = Dictionary(unstaged.map { ($0.path, $0) }, uniquingKeysWith: { a, _ in a })
            let stagedCounts = Dictionary(staged.map { ($0.path, $0) }, uniquingKeysWith: { a, _ in a })

            var items: [ChangedFileItem] = []
            for entry in status.staged {
                let n = stagedCounts[entry.path]
                items.append(.init(path: entry.path, section: .staged, adds: n?.additions, dels: n?.deletions,
                                   modifiedAt: modifiedDate(for: entry.path)))
            }
            for entry in status.unstaged where entry.kind != .unmerged {
                let n = unstagedCounts[entry.path]
                items.append(.init(path: entry.path, section: .unstaged, adds: n?.additions, dels: n?.deletions,
                                   modifiedAt: modifiedDate(for: entry.path)))
            }
            for entry in status.untracked {
                items.append(.init(path: entry.path, section: .untracked, adds: nil, dels: nil,
                                   modifiedAt: modifiedDate(for: entry.path)))
            }
            // 我々が注入する hooks 設定は一覧に出さない（ノイズ回避）。
            self.items = items.filter { $0.path != AgentSessionModel.localSettingsRelativePath }
            allFiles = ((try? await git.listFiles(worktree: worktree)) ?? [])
                .filter { $0 != AgentSessionModel.localSettingsRelativePath }

            if let selectedPath,
               !self.items.contains(where: { $0.path == selectedPath }),
               !allFiles.contains(selectedPath) {
                self.selectedPath = nil
            }
            commits = (try? await git.commitGraph(worktree: worktree, limit: 300)) ?? []
            loadError = nil
            await loadSelection()
        } catch {
            loadError = String(describing: error)
        }
    }

    func loadSelection() async {
        guard let path = selectedPath else {
            diff = nil
            wholeText = nil
            return
        }
        if let item = items.first(where: { $0.path == path }) {
            diff = try? await git.diff(worktree: worktree, path: path, staged: item.section == .staged)
        } else {
            diff = nil // 全体ツリーの未変更ファイルは diff なし（全文のみ）
        }
        wholeText = try? git.fileContents(worktree: worktree, path: path)
    }

    /// 作業ツリー上のファイルの最終更新時刻。削除済み等で取得できなければ nil。
    private func modifiedDate(for path: String) -> Date? {
        let url = worktree.appendingPathComponent(path)
        let attrs = try? FileManager.default.attributesOfItem(atPath: url.path)
        return attrs?[.modificationDate] as? Date
    }
}
