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
    case tree = "ツリー"
    case recent = "更新順"
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
    var selectedID: ChangedFileItem.ID?
    var viewMode: FileViewMode = .diff
    var listMode: FileListMode = .tree
    var diff: FileDiff?
    var wholeText: String?
    var commits: [CommitGraphLine] = []
    var loadError: String?

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

    func select(_ item: ChangedFileItem) {
        selectedID = item.id
        if item.isUntracked { viewMode = .whole }
        Task { await loadSelection() }
    }

    var selectedItem: ChangedFileItem? {
        items.first { $0.id == selectedID }
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

            if let selectedID, !items.contains(where: { $0.id == selectedID }) {
                self.selectedID = nil
            }
            commits = (try? await git.commitGraph(worktree: worktree, limit: 300)) ?? []
            loadError = nil
            await loadSelection()
        } catch {
            loadError = String(describing: error)
        }
    }

    func loadSelection() async {
        guard let item = selectedItem else {
            diff = nil
            wholeText = nil
            return
        }
        diff = try? await git.diff(worktree: worktree, path: item.path, staged: item.section == .staged)
        wholeText = try? git.fileContents(worktree: worktree, path: item.path)
    }

    /// 作業ツリー上のファイルの最終更新時刻。削除済み等で取得できなければ nil。
    private func modifiedDate(for path: String) -> Date? {
        let url = worktree.appendingPathComponent(path)
        let attrs = try? FileManager.default.attributesOfItem(atPath: url.path)
        return attrs?[.modificationDate] as? Date
    }
}
