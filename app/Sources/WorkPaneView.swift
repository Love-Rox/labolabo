import SwiftUI
import LaboLaboEngine

/// "git 部分": branch/status bar + changed-files list. Lives as its own tile so it
/// can be moved/split independently of the diff. Shares one `WorkPaneModel` with
/// `FileDetailPane` (selecting a file here drives the diff there). The model's
/// FileWatcher lifecycle is owned by the session, not this view.
struct ChangedFilesPane: View {
    let model: WorkPaneModel

    private var listModeBinding: Binding<FileListMode> {
        Binding(get: { model.listMode }, set: { model.listMode = $0 })
    }

    var body: some View {
        VStack(spacing: 0) {
            BranchStatusBar(status: model.status)
            Divider()
            HStack {
                Picker("", selection: listModeBinding) {
                    ForEach(FileListMode.allCases) { Text($0.rawValue).tag($0) }
                }
                .pickerStyle(.segmented)
                .fixedSize()
                Spacer()
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 4)
            Divider()
            ChangedFilesList(model: model)
        }
    }
}

/// "diff 部分": the selected file's Diff ⇄ Whole-file view. Shares the same
/// `WorkPaneModel` as `ChangedFilesPane`.
struct FileDetailPane: View {
    let model: WorkPaneModel

    var body: some View {
        FileDetailView(model: model)
    }
}

/// Commit-history graph (git log --graph) for the worktree. Lives as its own tile.
struct CommitGraphPane: View {
    let model: WorkPaneModel

    var body: some View {
        if model.commits.isEmpty {
            ContentUnavailableView("コミットがありません", systemImage: "clock.arrow.circlepath")
        } else {
            ScrollView(.vertical) {
                LazyVStack(alignment: .leading, spacing: 0) {
                    ForEach(model.commits) { line in
                        CommitGraphRow(
                            line: line,
                            isSelected: line.commit.map { model.selectedCommit == $0.hash } ?? false
                        )
                        .contentShape(Rectangle())
                        .onTapGesture {
                            if let hash = line.commit?.hash { model.selectCommit(hash) }
                        }
                    }
                }
                .padding(.vertical, 4)
                .frame(maxWidth: .infinity, alignment: .leading)
            }
        }
    }
}

struct CommitGraphRow: View {
    let line: CommitGraphLine
    var isSelected: Bool = false

    var body: some View {
        HStack(spacing: 6) {
            Text(line.graph.isEmpty ? " " : line.graph)
                .font(.system(size: 11, design: .monospaced))
                .foregroundStyle(.secondary)
                .fixedSize()
            if let commit = line.commit {
                Text(commit.hash)
                    .font(.system(size: 11, design: .monospaced))
                    .foregroundStyle(Color.accentColor)
                if !commit.refs.isEmpty {
                    Text(commit.refs)
                        .font(.system(size: 10))
                        .lineLimit(1)
                        .padding(.horizontal, 5)
                        .padding(.vertical, 1)
                        .background(Capsule().fill(Color.orange.opacity(0.18)))
                        .foregroundStyle(.orange)
                }
                Text(commit.subject)
                    .font(.system(size: 11))
                    .lineLimit(1)
                    .truncationMode(.tail)
                Spacer(minLength: 8)
                Text(commit.author)
                    .font(.system(size: 10))
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                Text(commit.relativeDate)
                    .font(.system(size: 10))
                    .foregroundStyle(.tertiary)
                    .lineLimit(1)
                    .fixedSize()
            } else {
                Spacer(minLength: 0)
            }
        }
        .padding(.horizontal, 8)
        .padding(.vertical, 1)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(isSelected ? Color.accentColor.opacity(0.18) : Color.clear)
    }
}

struct BranchStatusBar: View {
    let status: GitStatus?

    var body: some View {
        HStack(spacing: 12) {
            if let status {
                Label(status.branch ?? "—", systemImage: "arrow.triangle.branch")
                if status.ahead > 0 { Label("\(status.ahead)", systemImage: "arrow.up") }
                if status.behind > 0 { Label("\(status.behind)", systemImage: "arrow.down") }
                Spacer()
                Text(status.isDirty ? "変更あり" : "クリーン")
                    .foregroundStyle(status.isDirty ? Color.orange : Color.secondary)
            } else {
                Text("読み込み中…").foregroundStyle(.secondary)
                Spacer()
            }
        }
        .font(.caption)
        .padding(.horizontal, 12)
        .padding(.vertical, 6)
    }
}

struct ChangedFilesList: View {
    let model: WorkPaneModel

    private var selectionBinding: Binding<String?> {
        Binding(
            get: { model.selectedPath },
            set: { newPath in if let newPath { model.select(path: newPath) } }
        )
    }

    var body: some View {
        switch model.listMode {
        case .changedTree:
            if model.items.isEmpty {
                List { Text("変更はありません").foregroundStyle(.secondary) }
            } else {
                FileTreeView(
                    roots: model.changedTree,
                    selection: selectionBinding,
                    isExpanded: { model.isExpanded($0, mode: .changedTree) },
                    toggle: { model.toggleExpanded($0, mode: .changedTree) }
                )
            }
        case .fullTree:
            FileTreeView(
                roots: model.fullTree,
                selection: selectionBinding,
                isExpanded: { model.isExpanded($0, mode: .fullTree) },
                toggle: { model.toggleExpanded($0, mode: .fullTree) }
            )
        case .recent:
            List(selection: selectionBinding) {
                if model.items.isEmpty {
                    Text("変更はありません").foregroundStyle(.secondary)
                }
                ForEach(model.itemsByRecent) { item in
                    ChangedFileRow(item: item, showsSection: true).tag(item.path)
                }
            }
        }
    }
}

struct ChangedFileRow: View {
    let item: ChangedFileItem
    var showsSection: Bool = false

    var body: some View {
        HStack(spacing: 6) {
            Text(item.fileName).lineLimit(1).truncationMode(.middle)
            if showsSection {
                Text(item.section.rawValue)
                    .font(.caption2)
                    .foregroundStyle(.secondary)
                    .padding(.horizontal, 4)
                    .padding(.vertical, 1)
                    .background(Capsule().fill(Color.secondary.opacity(0.15)))
            }
            Spacer()
            if let adds = item.adds, adds > 0 {
                Text("+\(adds)").foregroundStyle(.green).font(.caption.monospaced())
            }
            if let dels = item.dels, dels > 0 {
                Text("-\(dels)").foregroundStyle(.red).font(.caption.monospaced())
            }
            if let modifiedAt = item.modifiedAt {
                Text(modifiedAt, format: .relative(presentation: .numeric, unitsStyle: .narrow))
                    .font(.caption2)
                    .foregroundStyle(.tertiary)
                    .lineLimit(1)
            }
        }
        .help(item.path)
    }
}

struct FileDetailView: View {
    let model: WorkPaneModel

    private var viewModeBinding: Binding<FileViewMode> {
        Binding(
            get: { model.viewMode },
            set: { model.viewMode = $0 }
        )
    }

    var body: some View {
        VStack(spacing: 0) {
            header
            Divider()
            content
        }
    }

    @ViewBuilder
    private var header: some View {
        HStack {
            if let hash = model.selectedCommit {
                Image(systemName: "point.3.connected.trianglepath.dotted")
                    .font(.caption2).foregroundStyle(.secondary)
                Text("コミット \(hash)")
                    .font(.caption.monospaced()).foregroundStyle(.secondary)
                    .lineLimit(1)
                Spacer()
            } else if let path = model.selectedPath {
                Text(path)
                    .font(.caption).foregroundStyle(.secondary)
                    .lineLimit(1).truncationMode(.middle)
                Spacer()
                // 変更ファイルのみ Diff⇄全文 を切替（未変更ファイルは全文のみ）。
                if model.selectedItem != nil {
                    Picker("", selection: viewModeBinding) {
                        ForEach(FileViewMode.allCases) { Text($0.rawValue).tag($0) }
                    }
                    .pickerStyle(.segmented)
                    .fixedSize()
                }
            } else {
                Text("ファイル / コミットを選択").font(.caption).foregroundStyle(.secondary)
                Spacer()
            }
        }
        .padding(.horizontal, 12)
        .frame(minHeight: 28)
        .padding(.vertical, 6)
    }

    @ViewBuilder
    private var content: some View {
        if model.selectedCommit != nil {
            CommitDiffView(diffs: model.commitDiff)
        } else if model.selectedPath == nil {
            ContentUnavailableView("ファイル / コミットを選択", systemImage: "doc.text.magnifyingglass")
        } else if model.selectedItem != nil, model.viewMode == .diff {
            DiffView(diff: model.diff)
        } else {
            WholeFileView(text: model.wholeText)
        }
    }
}

/// 1 コミットの差分（複数ファイル）をまとめて表示する。
struct CommitDiffView: View {
    let diffs: [FileDiff]?

    var body: some View {
        if let diffs, !diffs.isEmpty {
            GeometryReader { geo in
                ScrollView([.vertical, .horizontal]) {
                    VStack(alignment: .leading, spacing: 0) {
                        ForEach(Array(diffs.enumerated()), id: \.offset) { _, file in
                            Text(fileName(file))
                                .font(.caption.monospaced().weight(.semibold))
                                .padding(.vertical, 3)
                                .padding(.horizontal, 8)
                                .frame(minWidth: geo.size.width, alignment: .leading)
                                .background(Color.accentColor.opacity(0.12))
                            if file.isBinary {
                                Text("(バイナリ)")
                                    .font(.caption).foregroundStyle(.secondary)
                                    .padding(.vertical, 2).padding(.horizontal, 8)
                                    .frame(minWidth: geo.size.width, alignment: .leading)
                            } else {
                                ForEach(Array(file.hunks.enumerated()), id: \.offset) { _, hunk in
                                    Text(hunk.header)
                                        .font(.caption.monospaced())
                                        .foregroundStyle(.secondary)
                                        .padding(.vertical, 2).padding(.horizontal, 8)
                                        .frame(minWidth: geo.size.width, alignment: .leading)
                                        .background(Color.gray.opacity(0.12))
                                    ForEach(Array(hunk.lines.enumerated()), id: \.offset) { _, line in
                                        DiffLineRow(line: line, minWidth: geo.size.width)
                                    }
                                }
                            }
                        }
                    }
                    .padding(.vertical, 4)
                    .frame(minWidth: geo.size.width, minHeight: geo.size.height, alignment: .topLeading)
                }
            }
        } else {
            ContentUnavailableView("差分なし", systemImage: "equal")
        }
    }

    private func fileName(_ file: FileDiff) -> String {
        file.newPath ?? file.oldPath ?? "(unknown)"
    }
}

private enum DiffGutter {
    static let numberWidth: CGFloat = 44
    static let signWidth: CGFloat = 16
}

struct DiffView: View {
    let diff: FileDiff?

    var body: some View {
        if let diff, !diff.hunks.isEmpty {
            GeometryReader { geo in
                ScrollView([.vertical, .horizontal]) {
                    VStack(alignment: .leading, spacing: 0) {
                        ForEach(Array(diff.hunks.enumerated()), id: \.offset) { _, hunk in
                            Text(hunk.header)
                                .font(.caption.monospaced())
                                .foregroundStyle(.secondary)
                                .padding(.vertical, 2)
                                .padding(.horizontal, 8)
                                .frame(minWidth: geo.size.width, alignment: .leading)
                                .background(Color.gray.opacity(0.12))
                            ForEach(Array(hunk.lines.enumerated()), id: \.offset) { _, line in
                                DiffLineRow(line: line, minWidth: geo.size.width)
                            }
                        }
                    }
                    .padding(.vertical, 4)
                    .frame(minWidth: geo.size.width, minHeight: geo.size.height, alignment: .topLeading)
                }
            }
        } else if diff?.isBinary == true {
            ContentUnavailableView("バイナリファイル", systemImage: "doc")
        } else {
            ContentUnavailableView("差分なし", systemImage: "equal")
        }
    }
}

struct DiffLineRow: View {
    let line: DiffLine
    let minWidth: CGFloat

    private var background: Color {
        switch line.kind {
        case .addition: return .green.opacity(0.15)
        case .deletion: return .red.opacity(0.15)
        default: return .clear
        }
    }

    private var sign: String {
        switch line.kind {
        case .addition: return "+"
        case .deletion: return "-"
        case .noNewline: return "\\"
        case .context: return " "
        }
    }

    private func gutter(_ number: Int?) -> String { number.map(String.init) ?? "" }

    var body: some View {
        HStack(spacing: 0) {
            Text(gutter(line.oldLineNumber))
                .frame(width: DiffGutter.numberWidth, alignment: .trailing)
                .foregroundStyle(.secondary)
            Text(gutter(line.newLineNumber))
                .frame(width: DiffGutter.numberWidth, alignment: .trailing)
                .foregroundStyle(.secondary)
            Text(sign)
                .frame(width: DiffGutter.signWidth, alignment: .center)
                .foregroundStyle(.secondary)
            Text(line.text.isEmpty ? " " : line.text)
                .fixedSize(horizontal: true, vertical: false)
                .textSelection(.enabled)
                .padding(.leading, 4)
        }
        .font(.caption.monospaced())
        .padding(.vertical, 1)
        .frame(minWidth: minWidth, alignment: .leading)
        .background(background)
    }
}

struct WholeFileView: View {
    let text: String?

    private var lines: [String] {
        guard let text else { return [] }
        if text.isEmpty { return ["(空ファイル)"] }
        return text.components(separatedBy: "\n")
    }

    var body: some View {
        if text != nil {
            GeometryReader { geo in
                ScrollView([.vertical, .horizontal]) {
                    VStack(alignment: .leading, spacing: 0) {
                        ForEach(Array(lines.enumerated()), id: \.offset) { index, line in
                            HStack(spacing: 0) {
                                Text("\(index + 1)")
                                    .frame(width: DiffGutter.numberWidth, alignment: .trailing)
                                    .foregroundStyle(.secondary)
                                Text(line.isEmpty ? " " : line)
                                    .fixedSize(horizontal: true, vertical: false)
                                    .textSelection(.enabled)
                                    .padding(.leading, 8)
                            }
                            .font(.caption.monospaced())
                            .frame(minWidth: geo.size.width, alignment: .leading)
                        }
                    }
                    .padding(.vertical, 4)
                    .frame(minWidth: geo.size.width, minHeight: geo.size.height, alignment: .topLeading)
                }
            }
        } else {
            ContentUnavailableView("表示できません", systemImage: "doc")
        }
    }
}
