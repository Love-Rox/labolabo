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

    /// worktree が属するリポジトリの識別（同一 repo の worktree はグルーピング用に同じ key を持つ）。
    /// key = 共有 git ディレクトリの絶対パス。name = origin リモート（owner/repo）優先、無ければフォルダ名。
    public func repoInfo(worktree: URL) async throws -> RepoInfo {
        let common = try await GitRunner.run(
            ["rev-parse", "--path-format=absolute", "--git-common-dir"], in: worktree
        ).trimmingCharacters(in: .whitespacesAndNewlines)

        let root: String = (common as NSString).lastPathComponent == ".git"
            ? (common as NSString).deletingLastPathComponent
            : common

        var name = (root as NSString).lastPathComponent
        if let remote = try? await GitRunner.run(["remote", "get-url", "origin"], in: worktree)
            .trimmingCharacters(in: .whitespacesAndNewlines),
            !remote.isEmpty, let parsed = Self.repoName(fromRemote: remote) {
            name = parsed
        }
        return RepoInfo(key: common, name: name, root: root)
    }

    /// `git@host:owner/repo(.git)` / `https://host/owner/repo(.git)` → `owner/repo`。
    static func repoName(fromRemote remote: String) -> String? {
        var value = remote
        if value.hasSuffix(".git") { value = String(value.dropLast(4)) }
        if value.hasPrefix("git@"), let colon = value.firstIndex(of: ":") {
            let path = String(value[value.index(after: colon)...])
            return path.isEmpty ? nil : path
        }
        if let url = URL(string: value), url.host != nil {
            let path = url.path.hasPrefix("/") ? String(url.path.dropFirst()) : url.path
            return path.isEmpty ? nil : path
        }
        return nil
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

public struct RepoInfo: Equatable, Sendable {
    /// グルーピング用の安定キー（共有 git ディレクトリの絶対パス）。
    public let key: String
    /// 表示名（owner/repo もしくはフォルダ名）。
    public let name: String
    /// リポジトリのルートパス。
    public let root: String

    public init(key: String, name: String, root: String) {
        self.key = key
        self.name = name
        self.root = root
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
