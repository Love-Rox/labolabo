import Foundation

/// セッション間の「同じファイルを同時に編集中」検出（純粋関数）。
///
/// 1 セッション = 1 worktree だが、同じリポジトリ（共有 `.git`）の worktree 同士は
/// 同じ相対パスのファイルを別ブランチで編集しうる。ここでは同一 `repoKey` の
/// セッション間で変更パスが重なったものを列挙し、UI の衝突警告に使う。
public enum CrossSessionConflicts {

    /// 1 セッションの入力（id・所属リポジトリ・変更中パス集合）。
    public struct Session: Sendable, Equatable {
        public let id: String
        /// 所属リポジトリの安定キー（共有 git ディレクトリ）。未解決なら nil。
        public let repoKey: String?
        /// worktree ルート相対の変更中パス集合。
        public let changed: Set<String>

        public init(id: String, repoKey: String?, changed: Set<String>) {
            self.id = id
            self.repoKey = repoKey
            self.changed = changed
        }
    }

    /// 1 ファイルの衝突（パスと、それを共有する他セッション id）。
    public struct Conflict: Sendable, Equatable {
        public let path: String
        public let others: [String]

        public init(path: String, others: [String]) {
            self.path = path
            self.others = others
        }
    }

    /// `id` のセッションが変更中で、かつ**同一 repoKey の別セッション**も変更中のパス一覧。
    /// パス昇順。`others` は入力順の他セッション id。
    public static func conflicts(for id: String, among sessions: [Session]) -> [Conflict] {
        guard let me = sessions.first(where: { $0.id == id }),
              let repoKey = me.repoKey, !me.changed.isEmpty else { return [] }
        let siblings = sessions.filter { $0.id != id && $0.repoKey == repoKey }
        guard !siblings.isEmpty else { return [] }

        return me.changed.sorted().compactMap { path in
            let others = siblings.filter { $0.changed.contains(path) }.map(\.id)
            return others.isEmpty ? nil : Conflict(path: path, others: others)
        }
    }
}
