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
        return compact(convert(root, prefix: ""))
    }

    /// 子が 1 つのディレクトリだけのフォルダを連結（VSCode の compact folders 相当）。
    private static func compact(_ nodes: [FileTreeNode]) -> [FileTreeNode] {
        nodes.map { node in
            guard node.isDirectory else { return node }
            var name = node.name
            var id = node.id
            var children = node.children
            while children.count == 1, children[0].isDirectory {
                let only = children[0]
                name += "/" + only.name
                id = only.id
                children = only.children
            }
            return FileTreeNode(id: id, name: name, isDirectory: true, children: compact(children), change: nil)
        }
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
                    .listRowSeparator(.hidden)
                    .listRowInsets(EdgeInsets(top: 0, leading: 8, bottom: 0, trailing: 8))
                    .contentShape(Rectangle())
                    .onTapGesture {
                        if entry.node.isDirectory { toggle(entry.node.id) }
                    }
            }
        }
        .listStyle(.plain)
        .environment(\.defaultMinListRowHeight, FileTreeRow.rowHeight)
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

    static let rowHeight: CGFloat = 22
    private static let indentWidth: CGFloat = 14

    var body: some View {
        HStack(spacing: 4) {
            // VSCode 風のインデントガイド（親シェブロン位置に揃えた淡い縦線）。
            // 行高いっぱいに描き、行を隙間なく詰めることで縦線が連続して見える。
            ForEach(0 ..< depth, id: \.self) { _ in
                Color.clear
                    .frame(width: Self.indentWidth, height: Self.rowHeight)
                    .overlay(alignment: .leading) {
                        Rectangle()
                            .fill(Color.secondary.opacity(0.22))
                            .frame(width: 1)
                            .offset(x: 5)
                    }
            }
            // 開閉シェブロン（フォルダのみ、展開で回転）。ファイルは同じ幅の空きで名前を揃える。
            Group {
                if node.isDirectory {
                    Image(systemName: "chevron.right")
                        .font(.system(size: 9, weight: .semibold))
                        .foregroundStyle(.secondary)
                        .rotationEffect(.degrees(expanded ? 90 : 0))
                } else {
                    Color.clear
                }
            }
            .frame(width: 10)
            Image(systemName: icon)
                .font(.system(size: 12))
                .foregroundStyle(iconColor)
                .frame(width: 16)
            Text(node.name)
                .foregroundStyle(node.isDirectory || node.isChanged ? Color.primary : Color.secondary)
                .lineLimit(1)
                .truncationMode(.middle)
            Spacer(minLength: 4)
            if let change = node.change {
                if let adds = change.adds, adds > 0 {
                    Text("+\(adds)").foregroundStyle(.green).font(.caption2.monospaced())
                }
                if let dels = change.dels, dels > 0 {
                    Text("-\(dels)").foregroundStyle(.red).font(.caption2.monospaced())
                }
            }
        }
        .frame(height: Self.rowHeight)
        .help(node.id)
    }

    private var ext: String { (node.name as NSString).pathExtension.lowercased() }

    private var icon: String {
        if node.isDirectory { return expanded ? "folder" : "folder.fill" }
        switch ext {
        case "swift": return "swift"
        case "js", "jsx", "mjs", "cjs", "ts", "tsx", "json": return "curlybraces"
        case "py": return "chevron.left.forwardslash.chevron.right"
        case "md", "markdown": return "doc.richtext"
        case "png", "jpg", "jpeg", "gif", "svg", "webp": return "photo"
        case "sh", "zsh", "bash": return "terminal"
        case "yml", "yaml", "toml", "cfg", "conf", "ini": return "gearshape"
        default: return "doc"
        }
    }

    private var iconColor: Color {
        if node.isDirectory { return Color.accentColor.opacity(0.85) }
        switch ext {
        case "swift": return .orange
        case "js", "jsx", "mjs", "cjs", "json": return .yellow
        case "ts", "tsx", "py": return .blue
        case "png", "jpg", "jpeg", "gif", "svg", "webp": return .purple
        default: return .secondary
        }
    }
}
