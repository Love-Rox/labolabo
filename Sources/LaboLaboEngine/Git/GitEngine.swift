import Foundation

/// High-level git operations for one or more worktrees, built on `GitRunner`
/// and the porcelain/diff parsers.
///
/// An `actor` so that mutating worktree operations are serialized; read
/// operations (status/diff) are cheap and safe to call concurrently from the UI.
public actor GitEngine {

    public init() {}

    // MARK: - Read

    /// `git status --porcelain=v2 --branch -z`
    public func status(worktree: URL) async throws -> GitStatus {
        let raw = try await GitRunner.run(
            ["status", "--porcelain=v2", "--branch", "-z"],
            in: worktree
        )
        return PorcelainStatusParser.parse(raw)
    }

    /// Unified diff for the whole worktree. `staged: true` uses the index (`--cached`).
    public func diff(worktree: URL, staged: Bool = false) async throws -> [FileDiff] {
        var args = ["diff"]
        if staged { args.append("--cached") }
        let raw = try await GitRunner.run(args, in: worktree)
        return UnifiedDiffParser.parse(raw)
    }

    /// Unified diff for a single path.
    public func diff(worktree: URL, path: String, staged: Bool = false) async throws -> FileDiff? {
        var args = ["diff"]
        if staged { args.append("--cached") }
        args += ["--", path]
        let raw = try await GitRunner.run(args, in: worktree)
        return UnifiedDiffParser.parse(raw).first
    }

    /// Per-file added/deleted line counts via `git diff --numstat`.
    /// Binary files report `nil` counts (numstat prints `-`).
    public func numstat(worktree: URL, staged: Bool = false) async throws -> [NumstatEntry] {
        var args = ["diff", "--numstat"]
        if staged { args.append("--cached") }
        let raw = try await GitRunner.run(args, in: worktree)
        return raw
            .split(separator: "\n", omittingEmptySubsequences: true)
            .compactMap { line in
                let cols = line.split(separator: "\t")
                guard cols.count >= 3 else { return nil }
                return NumstatEntry(
                    additions: Int(cols[0]),
                    deletions: Int(cols[1]),
                    path: String(cols[2])
                )
            }
    }

    /// Current on-disk contents of a file in the worktree (for the "whole file" view).
    public nonisolated func fileContents(worktree: URL, path: String) throws -> String {
        let url = worktree.appendingPathComponent(path)
        return try String(contentsOf: url, encoding: .utf8)
    }

    public func listWorktrees(repo: URL) async throws -> [Worktree] {
        let raw = try await GitRunner.run(["worktree", "list", "--porcelain"], in: repo)
        return WorktreeListParser.parse(raw)
    }

    /// 追跡中 + 未追跡（ただし .gitignore 対象は除外）の全ファイル相対パス。
    /// `git ls-files --cached --others --exclude-standard -z`。全体ツリー表示に使う。
    public func listFiles(worktree: URL) async throws -> [String] {
        let raw = try await GitRunner.run(
            ["ls-files", "--cached", "--others", "--exclude-standard", "-z"],
            in: worktree
        )
        return raw.split(separator: "\u{0}", omittingEmptySubsequences: true).map(String.init)
    }

    // MARK: - Mutate (serialized by the actor)

    /// `git worktree add -b <branch> <path> <baseRef>`
    public func addWorktree(repo: URL, path: URL, branch: String, baseRef: String) async throws {
        try await GitRunner.run(
            ["worktree", "add", "-b", branch, path.path, baseRef],
            in: repo
        )
    }

    /// `git worktree remove [--force] <path>`. Refuses dirty worktrees unless `force`.
    public func removeWorktree(repo: URL, path: URL, force: Bool = false) async throws {
        var args = ["worktree", "remove"]
        if force { args.append("--force") }
        args.append(path.path)
        try await GitRunner.run(args, in: repo)
    }
}

public struct NumstatEntry: Equatable, Sendable {
    public var additions: Int?
    public var deletions: Int?
    public var path: String

    public init(additions: Int?, deletions: Int?, path: String) {
        self.additions = additions
        self.deletions = deletions
        self.path = path
    }

    public var isBinary: Bool { additions == nil || deletions == nil }
}
