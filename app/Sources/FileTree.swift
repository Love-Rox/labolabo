import SwiftUI

/// ディレクトリツリーの 1 ノード。葉（ファイル）は `change` に変更情報を持ちうる。
struct FileTreeNode: Identifiable, Hashable {
    struct Change: Hashable {
        let section: ChangedFileItem.Section
        let adds: Int?
        let dels: Int?
    }

    let id: String            // 相対パス
    let name: String
    let isDirectory: Bool
    var children: [FileTreeNode]
    let change: Change?       // 変更ファイルなら非 nil

    var isChanged: Bool { change != nil }
}

enum FileTreeBuilder {
    /// 相対パス群 + 変更情報からディレクトリツリーを構築（ディレクトリ先・名前昇順）。
    static func build(paths: [String], changeByPath: [String: FileTreeNode.Change]) -> [FileTreeNode] {
        final class Builder {
            var children: [String: Builder] = [:]
            var isFile = false
        }
        let root = Builder()
        for path in paths {
            let parts = path.split(separator: "/").map(String.init)
            guard !parts.isEmpty else { continue }
            var node = root
            for (index, part) in parts.enumerated() {
                let child = node.children[part] ?? {
                    let created = Builder()
                    node.children[part] = created
                    return created
                }()
                node = child
                if index == parts.count - 1 { node.isFile = true }
            }
        }

        func convert(_ builder: Builder, prefix: String) -> [FileTreeNode] {
            builder.children.map { name, child -> FileTreeNode in
                let path = prefix.isEmpty ? name : "\(prefix)/\(name)"
                if child.isFile, child.children.isEmpty {
                    return FileTreeNode(
                        id: path, name: name, isDirectory: false,
                        children: [], change: changeByPath[path]
                    )
                } else {
                    return FileTreeNode(
                        id: path, name: name, isDirectory: true,
                        children: convert(child, prefix: path), change: nil
                    )
                }
            }
            .sorted { lhs, rhs in
                if lhs.isDirectory != rhs.isDirectory { return lhs.isDirectory }
                return lhs.name.localizedStandardCompare(rhs.name) == .orderedAscending
            }
        }
        return convert(root, prefix: "")
    }
}

/// 展開状態を自前管理してフラット化描画するツリービュー（変更ツリーは既定展開、
/// 全体ツリーは既定折り畳みにできる）。ディレクトリ行タップで開閉、ファイル行は選択。
struct FileTreeView: View {
    let roots: [FileTreeNode]
    let selection: Binding<String?>
    /// ディレクトリが展開中か。
    let isExpanded: (String) -> Bool
    let toggle: (String) -> Void

    var body: some View {
        List(selection: selection) {
            ForEach(flattened(), id: \.node.id) { entry in
                FileTreeRow(node: entry.node, depth: entry.depth, expanded: isExpanded(entry.node.id))
                    .tag(entry.node.id)
                    .selectionDisabled(entry.node.isDirectory)
                    .contentShape(Rectangle())
                    .onTapGesture {
                        if entry.node.isDirectory { toggle(entry.node.id) }
                    }
            }
        }
    }

    private struct Entry { let node: FileTreeNode; let depth: Int }

    private func flattened() -> [Entry] {
        var out: [Entry] = []
        func walk(_ nodes: [FileTreeNode], depth: Int) {
            for node in nodes {
                out.append(Entry(node: node, depth: depth))
                if node.isDirectory, isExpanded(node.id) {
                    walk(node.children, depth: depth + 1)
                }
            }
        }
        walk(roots, depth: 0)
        return out
    }
}

struct FileTreeRow: View {
    let node: FileTreeNode
    let depth: Int
    let expanded: Bool

    var body: some View {
        HStack(spacing: 4) {
            Color.clear.frame(width: CGFloat(depth) * 12)
            if node.isDirectory {
                Image(systemName: expanded ? "chevron.down" : "chevron.right")
                    .font(.system(size: 9))
                    .foregroundStyle(.secondary)
                    .frame(width: 12)
                Image(systemName: "folder")
                    .foregroundStyle(.secondary)
                    .font(.caption)
            } else {
                Color.clear.frame(width: 12)
                Image(systemName: "doc")
                    .foregroundStyle(iconColor)
                    .font(.caption)
            }
            Text(node.name)
                .foregroundStyle(node.isDirectory || node.isChanged ? Color.primary : Color.secondary)
                .lineLimit(1)
                .truncationMode(.middle)
            Spacer()
            if let change = node.change {
                if let adds = change.adds, adds > 0 {
                    Text("+\(adds)").foregroundStyle(.green).font(.caption2.monospaced())
                }
                if let dels = change.dels, dels > 0 {
                    Text("-\(dels)").foregroundStyle(.red).font(.caption2.monospaced())
                }
            }
        }
        .help(node.id)
    }

    private var iconColor: Color {
        switch node.change?.section {
        case .staged: return .green
        case .unstaged: return .orange
        case .untracked: return .secondary
        case .none: return .secondary
        }
    }
}
