import Foundation

/// One entry from `git worktree list --porcelain`.
public struct Worktree: Equatable, Sendable {
    public var path: String
    public var head: String?
    public var branch: String?
    public var isDetached: Bool
    public var isLocked: Bool
    public var isBare: Bool

    public init(
        path: String,
        head: String? = nil,
        branch: String? = nil,
        isDetached: Bool = false,
        isLocked: Bool = false,
        isBare: Bool = false
    ) {
        self.path = path
        self.head = head
        self.branch = branch
        self.isDetached = isDetached
        self.isLocked = isLocked
        self.isBare = isBare
    }

    /// Short branch name (`refs/heads/feature/x` -> `feature/x`).
    public var shortBranch: String? {
        guard let branch else { return nil }
        return branch.hasPrefix("refs/heads/") ? String(branch.dropFirst("refs/heads/".count)) : branch
    }
}

public enum WorktreeListParser {
    /// Parses the blank-line-separated blocks of `git worktree list --porcelain`.
    public static func parse(_ raw: String) -> [Worktree] {
        var worktrees: [Worktree] = []
        var current: Worktree?

        func flush() {
            if let c = current { worktrees.append(c); current = nil }
        }

        for line in raw.split(separator: "\n", omittingEmptySubsequences: false) {
            if line.isEmpty {
                flush()
                continue
            }
            let parts = line.split(separator: " ", maxSplits: 1)
            let key = String(parts[0])
            let value = parts.count > 1 ? String(parts[1]) : ""
            switch key {
            case "worktree":
                flush()
                current = Worktree(path: value)
            case "HEAD":
                current?.head = value
            case "branch":
                current?.branch = value
            case "detached":
                current?.isDetached = true
            case "locked":
                current?.isLocked = true
            case "bare":
                current?.isBare = true
            default:
                break
            }
        }
        flush()
        return worktrees
    }
}
