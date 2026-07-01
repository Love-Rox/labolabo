import SwiftUI
import UniformTypeIdentifiers
import LaboLaboEngine
import GhosttyTerminal

/// 上部バーの共有寸法。タイトルバーを隠して自前バー 1 本に統合する際に、
/// サイドバー側ヘッダーと詳細側バーの高さを揃え、信号機（traffic lights）を避ける。
enum LayoutMetrics {
    static let topBar: CGFloat = 52
    /// 信号機を避けるためのサイドバー左インセット。
    static let trafficLightInset: CGFloat = 78
    /// 折りたたみ時の詳細バー左インセット。展開ボタンが入る分、少し詰める。
    static let trafficLightInsetCompact: CGFloat = 70
}

struct ContentView: View {
    @State private var store = SessionStore()
    @State private var showImporter = false
    @State private var showChangelog = false
    @State private var columnVisibility: NavigationSplitViewVisibility = .all

    private var sidebarCollapsed: Bool { columnVisibility == .detailOnly }

    var body: some View {
        NavigationSplitView(columnVisibility: $columnVisibility) {
            VStack(spacing: 0) {
                sidebarHeader
                Divider()
                Color.clear.frame(height: 6) // 上部バーとセッション一覧の間に余白
                List(selection: Binding(get: { store.selection }, set: { store.select($0) })) {
                    if store.sessions.isEmpty {
                        Text("リポジトリを開いてください")
                            .foregroundStyle(.secondary)
                    } else {
                        ForEach(store.groupedSessions) { group in
                            Section {
                                ForEach(group.sessions) { session in
                                    SessionRow(session: session)
                                        .tag(session.id)
                                        .listRowBackground(rowBackground(colorID: store.colorID(forRepo: group.key)))
                                        .contextMenu {
                                            Button("セッションを閉じる", role: .destructive) {
                                                store.close(session.id)
                                            }
                                        }
                                }
                            } header: {
                                RepoGroupHeader(
                                    name: group.name,
                                    count: group.sessions.count,
                                    colorID: store.colorID(forRepo: group.key),
                                    onSelectColor: { store.setColorID($0, forRepo: group.key) }
                                )
                            }
                        }
                    }
                }
                .listStyle(.sidebar)
            }
            .ignoresSafeArea(.container, edges: .top)
            .navigationSplitViewColumnWidth(min: 224, ideal: 248)
            // NavigationSplitView が自動で出すサイドバー開閉ボタンを消す。自前ヘッダーの
            // ⓘ/＋ と重なるため（折りたたみは自前トグルで行う）。
            .toolbar(removing: .sidebarToggle)
            .fileImporter(isPresented: $showImporter, allowedContentTypes: [.folder]) { result in
                if case let .success(url) = result {
                    store.openRepository(at: url)
                }
            }
        } detail: {
            if let session = store.selected {
                SessionDetailView(
                    session: session,
                    onClose: { store.close(session.id) },
                    sidebarCollapsed: sidebarCollapsed,
                    onExpandSidebar: { columnVisibility = .all }
                )
                .id(session.id)
            } else {
                ContentUnavailableView {
                    Label("セッションがありません", systemImage: "sidebar.left")
                } description: {
                    Text("左上の ＋ から git リポジトリ（worktree）を開きます")
                } actions: {
                    Button("リポジトリを開く") { showImporter = true }
                }
            }
        }
    }

    /// セッション行の背景（リポジトリ色の淡いタイント）。色未設定は透明。
    @ViewBuilder
    private func rowBackground(colorID: String?) -> some View {
        if let colorID {
            RoundedRectangle(cornerRadius: 8, style: .continuous)
                .fill(RepoPalette.color(for: colorID).opacity(0.16))
                .padding(.vertical, 1)
                .padding(.horizontal, 6)
        } else {
            Color.clear
        }
    }

    /// サイドバー上部のヘッダー（OS タイトルバーの代わり）。信号機を避ける左インセット付き。
    private var sidebarHeader: some View {
        HStack(spacing: 6) {
            Text("LaboLabo")
                .font(.headline)
                .lineLimit(1)
                .fixedSize()
            Spacer(minLength: 4)
            Button {
                showChangelog = true
            } label: {
                Image(systemName: "info.circle")
            }
            .buttonStyle(.borderless)
            .help("変更履歴を表示")
            Button {
                showImporter = true
            } label: {
                Image(systemName: "plus")
            }
            .buttonStyle(.borderless)
            .help("リポジトリを開く")
            Button {
                columnVisibility = .detailOnly
            } label: {
                Image(systemName: "sidebar.leading")
            }
            .buttonStyle(.borderless)
            .help("サイドバーを折りたたむ")
        }
        // macOS 26 のカード型サイドバーでは信号機はカードの上にあるため、
        // 左は通常のパディングでよい（信号機回避の大きな左インセットは不要）。
        .padding(.leading, 14)
        .padding(.trailing, 12)
        .frame(height: LayoutMetrics.topBar)
        .background(.bar)
        .sheet(isPresented: $showChangelog) {
            ChangelogView()
        }
    }
}

struct SessionRow: View {
    let session: RepoSession

    var body: some View {
        HStack(spacing: 8) {
            VStack(alignment: .leading, spacing: 1) {
                Text(session.name).lineLimit(1).truncationMode(.middle)
                Text(session.branch ?? "—")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .truncationMode(.middle)
            }
            Spacer(minLength: 4)
            if let pr = session.pullRequest {
                PRBadge(pr: pr)
            }
        }
        .padding(.vertical, 2)
        .padding(.trailing, 8)
    }
}

/// セッション行の PR バッジ（状態アイコン + 番号 + checks）。
struct PRBadge: View {
    let pr: PullRequestInfo

    var body: some View {
        HStack(spacing: 2) {
            Image(systemName: pr.state.icon)
                .foregroundStyle(pr.state.color)
                .font(.caption2)
            Text("#\(pr.number)")
                .font(.caption2.monospaced())
                .foregroundStyle(.secondary)
            if let glyph = pr.checks.glyph {
                Image(systemName: glyph)
                    .foregroundStyle(pr.checks.color)
                    .font(.system(size: 9))
            }
        }
        .help(helpText)
    }

    private var helpText: String {
        var text = "PR #\(pr.number)（\(pr.state.label)）\(pr.title)"
        if let issue = pr.issue { text += "\nIssue #\(issue)" }
        return text
    }
}

/// サイドバーのリポジトリ・グループ見出し（色ドット + 名前 + セッション数）。
/// 右クリックで色を変更できる。
struct RepoGroupHeader: View {
    let name: String
    let count: Int
    let colorID: String?
    var onSelectColor: (String?) -> Void

    var body: some View {
        HStack(spacing: 6) {
            Circle()
                .fill(RepoPalette.color(for: colorID))
                .frame(width: 8, height: 8)
            Text(name)
                .font(.caption)
                .fontWeight(.semibold)
                .lineLimit(1)
                .truncationMode(.middle)
            Spacer()
            Text("\(count)")
                .font(.caption2)
                .foregroundStyle(.tertiary)
        }
        .contextMenu {
            Menu("色を変更") {
                ForEach(RepoPalette.entries, id: \.id) { entry in
                    Button {
                        onSelectColor(entry.id)
                    } label: {
                        if entry.id == colorID {
                            Label(entry.name, systemImage: "checkmark")
                        } else {
                            Text(entry.name)
                        }
                    }
                }
                Divider()
                Button("なし") { onSelectColor(nil) }
            }
        }
    }
}

struct SessionDetailView: View {
    let session: RepoSession
    let onClose: () -> Void
    var sidebarCollapsed: Bool = false
    var onExpandSidebar: () -> Void = {}

    @State private var work: WorkPaneModel
    @State private var tiling: PaneTilingModel
    @State private var agent: AgentSessionModel
    private let configSource: TerminalController.ConfigSource

    init(
        session: RepoSession,
        onClose: @escaping () -> Void,
        sidebarCollapsed: Bool = false,
        onExpandSidebar: @escaping () -> Void = {}
    ) {
        self.session = session
        self.onClose = onClose
        self.sidebarCollapsed = sidebarCollapsed
        self.onExpandSidebar = onExpandSidebar
        _work = State(initialValue: WorkPaneModel(worktree: session.worktreePath))
        _tiling = State(initialValue: PaneTilingModel.defaultLayout())
        _agent = State(initialValue: AgentSessionModel(sessionID: session.id, worktree: session.worktreePath))
        configSource = GhosttyConfig.userConfigSource()
    }

    var body: some View {
        VStack(spacing: 0) {
            sessionBar
            Divider()
            PaneTilingView(
                model: tiling,
                context: PaneContext(
                    workingDirectory: session.worktreePath.path,
                    work: work,
                    configSource: configSource
                ),
                revision: tiling.revision
            )
        }
        // 詳細（ターミナル）側に左余白を入れ、カード型サイドバーとの密着を解消する。
        // サイドバー非表示時は隙間不要なので密着させる。
        .padding(.leading, sidebarCollapsed ? 0 : 10)
        .ignoresSafeArea(.container, edges: .top)
        .navigationTitle(session.name)
        .onAppear { work.start(); agent.start() }
        .onDisappear { work.stop(); agent.stop() }
    }

    /// 操作系を集約した自前の 1 本バー。macOS のツールバーが要素をまとめて 1 枚の
    /// 大きなピルに収めてしまうのを避け、ステータスピル・丸ボタン・IDE/時計ピルを
    /// それぞれ単一枠で正確に並べる。
    private var sessionBar: some View {
        HStack(spacing: 12) {
            if sidebarCollapsed {
                Button(action: onExpandSidebar) {
                    Image(systemName: "sidebar.leading")
                }
                .buttonStyle(.borderless)
                .help("サイドバーを表示")
            }

            Text(session.name)
                .font(.headline)
                .lineLimit(1)
                .help(session.worktreePath.path)

            SessionStatusPill(
                status: work.status,
                fallbackBranch: session.branch,
                changedCount: work.items.count,
                agentStatus: agent.status
            )

            Spacer(minLength: 12)

            Button {
                tiling.launchInNewTerminal(title: "Claude", command: agent.launchCommand())
            } label: {
                Image(systemName: "sparkles")
            }
            .buttonStyle(CircleIconButtonStyle(tint: .purple))
            .help("Claude を起動（状態検出 hooks 付き）")

            Button {
                tiling.addPane(PaneItem(kind: .terminal, title: "端末"))
            } label: {
                Image(systemName: "plus.rectangle")
            }
            .buttonStyle(CircleIconButtonStyle())
            .help("端末を追加")

            Button {
                tiling.addPaneIfAbsent(kind: .files, title: "変更ファイル")
            } label: {
                Image(systemName: "list.bullet.rectangle")
            }
            .buttonStyle(CircleIconButtonStyle())
            .disabled(tiling.hasPane(kind: .files))
            .help("変更ファイル一覧を追加")

            Button {
                tiling.addPaneIfAbsent(kind: .diff, title: "Diff")
            } label: {
                Image(systemName: "doc.text")
            }
            .buttonStyle(CircleIconButtonStyle())
            .disabled(tiling.hasPane(kind: .diff))
            .help("Diff を追加")

            Button {
                tiling.addPaneIfAbsent(kind: .commits, title: "履歴")
            } label: {
                Image(systemName: "point.3.connected.trianglepath.dotted")
            }
            .buttonStyle(CircleIconButtonStyle())
            .disabled(tiling.hasPane(kind: .commits))
            .help("コミット履歴グラフを追加")

            IDEOpenMenu(worktree: session.worktreePath)
            SessionClock()

            Button {
                onClose()
            } label: {
                Image(systemName: "xmark")
            }
            .buttonStyle(CircleIconButtonStyle(tint: .red))
            .help("セッションを閉じる")
        }
        // サイドバー折りたたみ時は詳細が信号機の下に来るので左インセットで避ける。
        // 展開ボタンが入る分、通常より少し詰める。
        .padding(.leading, sidebarCollapsed ? LayoutMetrics.trafficLightInsetCompact : 12)
        .padding(.trailing, 12)
        .frame(height: LayoutMetrics.topBar)
        .background(.bar)
    }
}
