import Foundation

/// Parsed snapshot of `git status --porcelain=v2 --branch -z` for one worktree.
public struct GitStatus: Equatable, Sendable {
    public var headSha: String?
    public var branch: String?
    public var upstream: String?
    public var ahead: Int
    public var behind: Int
    public var entries: [GitFileEntry]

    public init(
        headSha: String? = nil,
        branch: String? = nil,
        upstream: String? = nil,
        ahead: Int = 0,
        behind: Int = 0,
        entries: [GitFileEntry] = []
    ) {
        self.headSha = headSha
        self.branch = branch
        self.upstream = upstream
        self.ahead = ahead
        self.behind = behind
        self.entries = entries
    }

    /// `true` when the branch is in a detached-HEAD state (porcelain reports `(detached)`).
    public var isDetached: Bool { branch == "(detached)" }

    /// Files with index-side (staged) changes.
    public var staged: [GitFileEntry] {
        entries.filter { $0.isStaged }
    }

    /// Files with worktree-side (unstaged) changes.
    public var unstaged: [GitFileEntry] {
        entries.filter { $0.isUnstaged }
    }

    /// Untracked paths.
    public var untracked: [GitFileEntry] {
        entries.filter { $0.kind == .untracked }
    }

    /// Conflicted (unmerged) paths.
    public var conflicted: [GitFileEntry] {
        entries.filter { $0.kind == .unmerged }
    }

    public var isDirty: Bool {
        entries.contains { $0.kind != .ignored }
    }
}

/// A single changed-path record from porcelain v2.
public struct GitFileEntry: Equatable, Sendable {
    public enum Kind: Equatable, Sendable {
        case ordinary
        case renamedOrCopied
        case unmerged
        case untracked
        case ignored
    }

    /// Status code for one side of an `XY` pair. `.unmodified` is porcelain's `.`.
    public enum Change: Character, Equatable, Sendable {
        case unmodified = "."
        case modified = "M"
        case typeChanged = "T"
        case added = "A"
        case deleted = "D"
        case renamed = "R"
        case copied = "C"
        case updatedButUnmerged = "U"

        public init(porcelain: Character) {
            self = Change(rawValue: porcelain) ?? .unmodified
        }
    }

    public var kind: Kind
    /// Index (staged) side of the `XY` field.
    public var index: Change
    /// Worktree (unstaged) side of the `XY` field.
    public var worktree: Change
    public var path: String
    /// Original path for rename/copy entries.
    public var originalPath: String?
    /// Similarity/score for rename/copy entries (0–100), if present.
    public var score: Int?

    public init(
        kind: Kind,
        index: Change = .unmodified,
        worktree: Change = .unmodified,
        path: String,
        originalPath: String? = nil,
        score: Int? = nil
    ) {
        self.kind = kind
        self.index = index
        self.worktree = worktree
        self.path = path
        self.originalPath = originalPath
        self.score = score
    }

    public var isStaged: Bool {
        switch kind {
        case .ordinary, .renamedOrCopied:
            return index != .unmodified
        default:
            return false
        }
    }

    public var isUnstaged: Bool {
        switch kind {
        case .ordinary, .renamedOrCopied:
            return worktree != .unmodified
        case .unmerged:
            return true
        default:
            return false
        }
    }
}
