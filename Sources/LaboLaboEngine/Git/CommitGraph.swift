import Foundation

/// One row of the commit graph = exactly one commit, plus the lane layout needed
/// to draw its node and the edges crossing that row. Unlike `git log --graph`'s
/// ASCII (which shifts every lane left whenever a column closes, making unrelated
/// lanes wiggle), lanes here are **stable columns**: a branch keeps its column
/// until it merges, so passing lanes are straight and only real branch/merge rows
/// bend. Layout is computed from each commit's parents (`%P`).
public struct CommitGraphRow: Sendable, Equatable, Identifiable {
    public let id: Int
    public let commit: Commit
    /// Column of this commit's node.
    public let nodeLane: Int
    /// Edges crossing this row (passing lanes + connections to the node).
    public let edges: [Edge]

    public struct Commit: Sendable, Equatable {
        public let hash: String
        public let subject: String
        public let author: String
        /// 作者日時（%at の UNIX 秒から）。相対表示は UI 側で短縮整形する。
        public let date: Date?
        /// Decorations like `HEAD -> main, origin/main` (parens stripped); empty if none.
        public let refs: String

        public init(hash: String, subject: String, author: String, date: Date?, refs: String) {
            self.hash = hash
            self.subject = subject
            self.author = author
            self.date = date
            self.refs = refs
        }
    }

    /// A single edge segment within a row.
    /// - `through`: a lane passing straight through this row (top→bottom, same column).
    /// - `nodeIn`: a child line entering the node from above (its column → node, top half).
    /// - `nodeOut`: a parent line leaving the node downward (node → its column, bottom half).
    public struct Edge: Sendable, Equatable {
        public enum Shape: Sendable, Equatable { case through, nodeIn, nodeOut }
        public let shape: Shape
        /// The column this edge attaches to (the non-node end).
        public let lane: Int
        /// Which lane index determines the color (kept == `lane` for stable coloring).
        public let colorLane: Int

        public init(shape: Shape, lane: Int, colorLane: Int) {
            self.shape = shape
            self.lane = lane
            self.colorLane = colorLane
        }
    }

    public init(id: Int, commit: Commit, nodeLane: Int, edges: [Edge]) {
        self.id = id
        self.commit = commit
        self.nodeLane = nodeLane
        self.edges = edges
    }
}

extension GitEngine {
    /// Commit graph for the worktree with **stable lane layout** computed from
    /// parent links. `--all` includes branch/merge lanes; `--topo-order` keeps a
    /// branch's commits contiguous so lanes stay readable.
    public func commitGraph(worktree: URL, limit: Int = 300) async throws -> [CommitGraphRow] {
        let us = "\u{1f}"
        // %H=full hash (for parent matching), %P=parent hashes (space-separated).
        let format = "%H\(us)%h\(us)%s\(us)%an\(us)%at\(us)%P\(us)%d"
        let raw = try await GitRunner.run(
            ["log", "--all", "--topo-order", "--color=never",
             "--pretty=format:\(format)", "-n", "\(limit)"],
            in: worktree
        )
        return CommitGraphLayout.build(raw)
    }
}

enum CommitGraphLayout {
    private struct RawCommit {
        let full: String
        let short: String
        let subject: String
        let author: String
        let date: Date?
        let parents: [String]
        let refs: String
    }

    static func build(_ raw: String) -> [CommitGraphRow] {
        let us: Character = "\u{1f}"
        var commits: [RawCommit] = []
        for rawLine in raw.split(separator: "\n", omittingEmptySubsequences: true) {
            let parts = rawLine.split(separator: us, omittingEmptySubsequences: false).map(String.init)
            guard parts.count >= 6 else { continue }
            func part(_ i: Int) -> String { i < parts.count ? parts[i] : "" }
            let parents = part(5).split(separator: " ").map(String.init)
            let date = TimeInterval(part(4)).map { Date(timeIntervalSince1970: $0) }
            let refs = part(6).trimmingCharacters(in: CharacterSet(charactersIn: " ()"))
            commits.append(RawCommit(
                full: part(0), short: part(1), subject: part(2),
                author: part(3), date: date, parents: parents, refs: refs
            ))
        }

        // lanes[i] = the (full) hash the lane is currently waiting to reach, or nil if free.
        var lanes: [String?] = []
        var rows: [CommitGraphRow] = []

        func firstFree() -> Int {
            if let i = lanes.firstIndex(where: { $0 == nil }) { return i }
            lanes.append(nil)
            return lanes.count - 1
        }

        for (idx, c) in commits.enumerated() {
            var edges: [CommitGraphRow.Edge] = []

            // Children (already-drawn commits above) whose lane awaits this commit.
            let myCols = lanes.indices.filter { lanes[$0] == c.full }
            let nodeLane = myCols.min() ?? firstFree()

            // Lanes not involved with this node pass straight through.
            for i in lanes.indices where lanes[i] != nil && !myCols.contains(i) {
                edges.append(.init(shape: .through, lane: i, colorLane: i))
            }
            // Child lines converge into the node (top half).
            for col in myCols {
                edges.append(.init(shape: .nodeIn, lane: col, colorLane: col))
            }
            // The node consumes those lanes; free them before assigning parents.
            for col in myCols { lanes[col] = nil }

            if let first = c.parents.first {
                // First parent continues straight down in the node's lane.
                lanes[nodeLane] = first
                edges.append(.init(shape: .nodeOut, lane: nodeLane, colorLane: nodeLane))
                // Additional parents (merge): reuse an existing lane if one already
                // awaits that parent, otherwise open a new lane.
                for parent in c.parents.dropFirst() {
                    if let existing = lanes.firstIndex(where: { $0 == parent }) {
                        edges.append(.init(shape: .nodeOut, lane: existing, colorLane: existing))
                    } else {
                        let col = firstFree()
                        lanes[col] = parent
                        edges.append(.init(shape: .nodeOut, lane: col, colorLane: col))
                    }
                }
            } else if nodeLane < lanes.count {
                // Root commit: the lane ends here.
                lanes[nodeLane] = nil
            }

            let commit = CommitGraphRow.Commit(
                hash: c.short, subject: c.subject, author: c.author,
                date: c.date, refs: c.refs
            )
            rows.append(CommitGraphRow(id: idx, commit: commit, nodeLane: nodeLane, edges: edges))
        }
        return rows
    }
}
