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
            filesBar
            Divider()
            ChangedFilesList(model: model)
        }
    }

    /// ブランチ状態と表示切替を 1 本のバーにまとめる。表示切替は幅に余裕があれば
    /// セグメント、狭ければプルダウン（ViewThatFits で自動切替）。
    private var filesBar: some View {
        HStack(spacing: 8) {
            branchStatus
                .layoutPriority(0)
            Spacer(minLength: 6)
            modeSelector
                .layoutPriority(1)
        }
        .font(.caption)
        .padding(.horizontal, 10)
        .frame(minHeight: 30)
    }

    @ViewBuilder
    private var branchStatus: some View {
        if let status = model.status {
            HStack(spacing: 6) {
                Label(status.branch ?? "—", systemImage: "arrow.triangle.branch")
                    .lineLimit(1)
                    .truncationMode(.middle)
                    .help(status.branch ?? "—")
                if status.ahead > 0 {
                    Label("\(status.ahead)", systemImage: "arrow.up").labelStyle(.titleAndIcon)
                }
                if status.behind > 0 {
                    Label("\(status.behind)", systemImage: "arrow.down").labelStyle(.titleAndIcon)
                }
            }
            .foregroundStyle(.secondary)
        } else {
            Text("読み込み中…").foregroundStyle(.tertiary)
        }
    }

    private var modeSelector: some View {
        ViewThatFits(in: .horizontal) {
            Picker("", selection: listModeBinding) {
                ForEach(FileListMode.allCases) { Text($0.rawValue).tag($0) }
            }
            .pickerStyle(.segmented)
            .fixedSize()

            Menu {
                Picker("表示", selection: listModeBinding) {
                    ForEach(FileListMode.allCases) { Text($0.rawValue).tag($0) }
                }
            } label: {
                Label(model.listMode.rawValue, systemImage: "line.3.horizontal.decrease")
            }
            .menuStyle(.borderlessButton)
            .fixedSize()
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

    /// レーン数（gutter 幅を全行で揃えてグラフ列の幅を決める）。
    private var laneCount: Int {
        let maxLen = model.commits.map(\.graph.count).max() ?? 0
        return max(1, (maxLen + 1) / 2)
    }

    /// グラフ（レーン）列の自然幅。
    private var graphWidth: CGFloat { CGFloat(laneCount) * CommitGraphGutter.laneWidth }

    /// これを超えたらグラフ列だけを横スクロールにして、右のコミット情報を画面内に守る。
    private let graphMaxWidth: CGFloat = 132

    private func isSelected(_ line: CommitGraphLine) -> Bool {
        line.commit.map { model.selectedCommit == $0.hash } ?? false
    }

    private func select(_ line: CommitGraphLine) {
        if let hash = line.commit?.hash { model.selectCommit(hash) }
    }

    var body: some View {
        if model.commits.isEmpty {
            ContentUnavailableView("コミットがありません", systemImage: "clock.arrow.circlepath")
        } else {
            // グラフ列とコミット情報列を分離。グラフが広い（分岐が多い）ときは
            // グラフ列だけを横スクロールにし、情報列は常に読める位置へ残す。
            ScrollView(.vertical) {
                HStack(alignment: .top, spacing: 0) {
                    graphColumn
                    infoColumn
                }
                .padding(.vertical, 4)
            }
        }
    }

    /// グラフ（レーン）列。幅が上限を超えるときだけ横スクロールにする。
    @ViewBuilder
    private var graphColumn: some View {
        if graphWidth > graphMaxWidth {
            ScrollView(.horizontal, showsIndicators: true) {
                gutterStack
            }
            .frame(width: graphMaxWidth)
        } else {
            gutterStack
        }
    }

    private var gutterStack: some View {
        LazyVStack(spacing: 0) {
            ForEach(model.commits) { line in
                CommitGraphGutter(graph: line.graph, laneCount: laneCount)
                    .frame(height: CommitGraphGutter.rowHeight)
                    .background(isSelected(line) ? Color.accentColor.opacity(0.18) : Color.clear)
                    .contentShape(Rectangle())
                    .onTapGesture { select(line) }
            }
        }
    }

    private var infoColumn: some View {
        LazyVStack(spacing: 0) {
            ForEach(model.commits) { line in
                CommitInfoRow(line: line, isSelected: isSelected(line))
                    .contentShape(Rectangle())
                    .onTapGesture { select(line) }
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

/// コミット情報の 1 行（gutter を除いた右側）。gutter とは列を分けて配置し、
/// グラフが広くても件名・作者・時刻が画面内に残るようにする。
struct CommitInfoRow: View {
    let line: CommitGraphLine
    var isSelected: Bool = false

    var body: some View {
        HStack(spacing: 8) {
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
                        .help(commit.refs)
                }
                Text(commit.subject)
                    .font(.system(size: 11))
                    .lineLimit(1)
                    .truncationMode(.tail)
                    .help(commit.subject)
                Spacer(minLength: 8)
                Text(commit.author)
                    .font(.system(size: 10))
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .help(commit.author)
                if let date = commit.date {
                    Text(date, format: .relative(presentation: .numeric, unitsStyle: .narrow))
                        .font(.system(size: 10))
                        .foregroundStyle(.tertiary)
                        .lineLimit(1)
                        .fixedSize()
                        // narrow 相対表示は省略が強いので、ホバーでロケール対応のフル日時を出す。
                        .help(date.formatted(date: .complete, time: .shortened))
                }
            } else {
                Spacer(minLength: 0)
            }
        }
        .padding(.leading, 8)
        .padding(.trailing, 8)
        .frame(height: CommitGraphGutter.rowHeight)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(isSelected ? Color.accentColor.opacity(0.18) : Color.clear)
    }
}

/// `git log --graph` の ASCII レーンを解析し、色分けした滑らかなベクター
/// （縦線・丸みのある分岐/合流・ノード円）で描く。
struct CommitGraphGutter: View {
    let graph: String
    let laneCount: Int

    static let laneWidth: CGFloat = 14
    static let rowHeight: CGFloat = 22
    private static let colors: [Color] = [
        .blue, .teal, .green, .orange, .pink, .purple, .red, .yellow, .indigo, .mint,
    ]

    private func laneX(_ k: Int) -> CGFloat { CGFloat(k) * Self.laneWidth + Self.laneWidth / 2 }
    private func color(_ k: Int) -> Color { Self.colors[((k % Self.colors.count) + Self.colors.count) % Self.colors.count] }

    var body: some View {
        Canvas { context, size in
            let h = size.height
            let mid = h / 2
            for (i, ch) in Array(graph).enumerated() {
                let k = i / 2
                let x = laneX(k)
                if i % 2 == 0 {
                    switch ch {
                    case "*":
                        line(context, CGPoint(x: x, y: 0), CGPoint(x: x, y: h), color(k))
                        let r: CGFloat = 3.5
                        let rect = CGRect(x: x - r, y: mid - r, width: r * 2, height: r * 2)
                        context.fill(Path(ellipseIn: rect), with: .color(color(k)))
                    case "|":
                        line(context, CGPoint(x: x, y: 0), CGPoint(x: x, y: h), color(k))
                    case "_":
                        line(context, CGPoint(x: x, y: mid), CGPoint(x: laneX(k + 1), y: mid), color(k))
                    default:
                        break
                    }
                } else {
                    let xR = laneX(k + 1)
                    switch ch {
                    // 斜めコネクタは常に「外側レーン(k+1)」の枝を表す（`\`=分岐/第2親、
                    // `/`=合流/フォールド）。両者とも k+1 の色にして、隣接する縦線の色と
                    // つながって見えるようにする（`/` を k 側の色にすると合流で色が食い違う）。
                    case "\\":
                        curve(context, CGPoint(x: x, y: 0), CGPoint(x: xR, y: h), color(k + 1))
                    case "/":
                        curve(context, CGPoint(x: xR, y: 0), CGPoint(x: x, y: h), color(k + 1))
                    case "|":
                        line(context, CGPoint(x: x, y: 0), CGPoint(x: x, y: h), color(k))
                    default:
                        break
                    }
                }
            }
        }
        .frame(width: CGFloat(laneCount) * Self.laneWidth, height: Self.rowHeight)
    }

    private func line(_ ctx: GraphicsContext, _ from: CGPoint, _ to: CGPoint, _ c: Color) {
        var path = Path()
        path.move(to: from)
        path.addLine(to: to)
        ctx.stroke(path, with: .color(c.opacity(0.9)), style: StrokeStyle(lineWidth: 1.5, lineCap: .round))
    }

    private func curve(_ ctx: GraphicsContext, _ from: CGPoint, _ to: CGPoint, _ c: Color) {
        let mid = (from.y + to.y) / 2
        var path = Path()
        path.move(to: from)
        path.addCurve(to: to, control1: CGPoint(x: from.x, y: mid), control2: CGPoint(x: to.x, y: mid))
        ctx.stroke(path, with: .color(c.opacity(0.9)), style: StrokeStyle(lineWidth: 1.5, lineCap: .round))
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

    private var hasSelection: Bool {
        model.selectedCommit != nil || model.selectedPath != nil
    }

    var body: some View {
        VStack(spacing: 0) {
            // 未選択時は見出しと仕切り線を出さず、空状態だけをすっきり見せる。
            if hasSelection {
                header
                Divider()
            }
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
                    .help(path)
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
