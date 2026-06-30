import SwiftUI
import LaboLaboEngine

/// "git 部分": branch/status bar + changed-files list. Lives as its own tile so it
/// can be moved/split independently of the diff. Shares one `WorkPaneModel` with
/// `FileDetailPane` (selecting a file here drives the diff there). The model's
/// FileWatcher lifecycle is owned by the session, not this view.
struct ChangedFilesPane: View {
    let model: WorkPaneModel

    var body: some View {
        VStack(spacing: 0) {
            BranchStatusBar(status: model.status)
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

    private var selectionBinding: Binding<ChangedFileItem.ID?> {
        Binding(
            get: { model.selectedID },
            set: { newID in
                if let newID, let item = model.items.first(where: { $0.id == newID }) {
                    model.select(item)
                }
            }
        )
    }

    var body: some View {
        List(selection: selectionBinding) {
            if model.items.isEmpty {
                Text("変更はありません").foregroundStyle(.secondary)
            }
            ForEach(ChangedFileItem.Section.allCases, id: \.self) { section in
                let items = model.items.filter { $0.section == section }
                if !items.isEmpty {
                    Section("\(section.rawValue) (\(items.count))") {
                        ForEach(items) { item in
                            ChangedFileRow(item: item).tag(item.id)
                        }
                    }
                }
            }
        }
    }
}

struct ChangedFileRow: View {
    let item: ChangedFileItem

    var body: some View {
        HStack(spacing: 6) {
            Text(item.fileName).lineLimit(1).truncationMode(.middle)
            Spacer()
            if let adds = item.adds, adds > 0 {
                Text("+\(adds)").foregroundStyle(.green).font(.caption.monospaced())
            }
            if let dels = item.dels, dels > 0 {
                Text("-\(dels)").foregroundStyle(.red).font(.caption.monospaced())
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
            HStack {
                if let item = model.selectedItem {
                    Text(item.path)
                        .font(.caption).foregroundStyle(.secondary)
                        .lineLimit(1).truncationMode(.middle)
                } else {
                    Text("ファイルを選択").font(.caption).foregroundStyle(.secondary)
                }
                Spacer()
                Picker("", selection: viewModeBinding) {
                    ForEach(FileViewMode.allCases) { Text($0.rawValue).tag($0) }
                }
                .pickerStyle(.segmented)
                .fixedSize()
                .disabled(model.selectedItem == nil)
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 6)
            Divider()

            if model.selectedItem == nil {
                ContentUnavailableView("変更ファイルを選択", systemImage: "doc.text.magnifyingglass")
            } else if model.viewMode == .diff {
                DiffView(diff: model.diff)
            } else {
                WholeFileView(text: model.wholeText)
            }
        }
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
