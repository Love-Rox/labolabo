import Foundation
import Observation
import LaboLaboEngine

enum FileViewMode: String, CaseIterable, Identifiable {
    case diff = "Diff"
    case whole = "Whole file"
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
    var diff: FileDiff?
    var wholeText: String?
    var commits: [CommitGraphLine] = []
    var loadError: String?

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
                items.append(.init(path: entry.path, section: .staged, adds: n?.additions, dels: n?.deletions))
            }
            for entry in status.unstaged where entry.kind != .unmerged {
                let n = unstagedCounts[entry.path]
                items.append(.init(path: entry.path, section: .unstaged, adds: n?.additions, dels: n?.deletions))
            }
            for entry in status.untracked {
                items.append(.init(path: entry.path, section: .untracked, adds: nil, dels: nil))
            }
            self.items = items

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
}
