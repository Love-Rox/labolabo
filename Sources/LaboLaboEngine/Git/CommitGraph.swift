import Foundation

/// One rendered line of `git log --graph`. Either a commit row (with `commit`
/// populated) or a pure connector row (e.g. `│ ╲`, `commit == nil`). `graph`
/// holds the ASCII lane art prefix so a monospaced view reproduces the DAG.
public struct CommitGraphLine: Sendable, Equatable, Identifiable {
    public let id: Int
    public let graph: String
    public let commit: Commit?

    public struct Commit: Sendable, Equatable {
        public let hash: String
        public let subject: String
        public let author: String
        public let relativeDate: String
        /// Decorations like `HEAD -> main, origin/main` (parens stripped); empty if none.
        public let refs: String

        public init(hash: String, subject: String, author: String, relativeDate: String, refs: String) {
            self.hash = hash
            self.subject = subject
            self.author = author
            self.relativeDate = relativeDate
            self.refs = refs
        }
    }

    public init(id: Int, graph: String, commit: Commit?) {
        self.id = id
        self.graph = graph
        self.commit = commit
    }
}

extension GitEngine {
    /// `git log --graph` for the worktree, parsed into rows. Commit fields are
    /// delimited by US (0x1f) so subjects/authors with arbitrary text stay intact.
    public func commitGraph(worktree: URL, limit: Int = 300) async throws -> [CommitGraphLine] {
        let us = "\u{1f}"
        let format = "\(us)%h\(us)%s\(us)%an\(us)%ar\(us)%d"
        // `--all` so branch/merge lanes actually show (a single linear branch would
        // otherwise render as one straight column). `--topo-order` keeps the lanes
        // visually coherent rather than strictly by date.
        let raw = try await GitRunner.run(
            ["log", "--graph", "--all", "--topo-order", "--color=never",
             "--pretty=format:\(format)", "-n", "\(limit)"],
            in: worktree
        )
        return CommitGraphParser.parse(raw)
    }
}

enum CommitGraphParser {
    static func parse(_ raw: String) -> [CommitGraphLine] {
        let separator: Character = "\u{1f}"
        var lines: [CommitGraphLine] = []

        for (index, rawLine) in raw.split(separator: "\n", omittingEmptySubsequences: false).enumerated() {
            let line = String(rawLine)
            guard let sep = line.firstIndex(of: separator) else {
                // Connector-only line (no commit payload).
                lines.append(CommitGraphLine(id: index, graph: line, commit: nil))
                continue
            }
            let graph = String(line[line.startIndex ..< sep])
            let rest = String(line[line.index(after: sep)...])
            let parts = rest.components(separatedBy: String(separator))
            func part(_ i: Int) -> String { i < parts.count ? parts[i] : "" }
            let refs = part(4).trimmingCharacters(in: CharacterSet(charactersIn: " ()"))
            let commit = CommitGraphLine.Commit(
                hash: part(0),
                subject: part(1),
                author: part(2),
                relativeDate: part(3),
                refs: refs
            )
            lines.append(CommitGraphLine(id: index, graph: graph, commit: commit))
        }

        if let last = lines.last, last.graph.isEmpty, last.commit == nil {
            lines.removeLast()
        }
        return lines
    }
}
