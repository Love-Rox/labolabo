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
}

struct ContentView: View {
    @State private var store = SessionStore()
    @State private var showImporter = false
    @State private var showChangelog = false

    var body: some View {
        NavigationSplitView {
            VStack(spacing: 0) {
                sidebarHeader
                Divider()
                List(selection: Binding(get: { store.selection }, set: { store.select($0) })) {
                    if store.sessions.isEmpty {
                        Text("リポジトリを開いてください")
                            .foregroundStyle(.secondary)
                    } else {
                        ForEach(store.sessions) { session in
                            SessionRow(session: session)
                                .tag(session.id)
                                .contextMenu {
                                    Button("セッションを閉じる", role: .destructive) {
                                        store.close(session.id)
                                    }
                                }
                        }
                    }
                }
                .listStyle(.sidebar)
            }
            .ignoresSafeArea(.container, edges: .top)
            .fileImporter(isPresented: $showImporter, allowedContentTypes: [.folder]) { result in
                if case let .success(url) = result {
                    store.openRepository(at: url)
                }
            }
        } detail: {
            if let session = store.selected {
                SessionDetailView(session: session, onClose: { store.close(session.id) })
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

    /// サイドバー上部のヘッダー（OS タイトルバーの代わり）。信号機を避ける左インセット付き。
    private var sidebarHeader: some View {
        HStack(spacing: 8) {
            Text("LaboLabo")
                .font(.headline)
            Spacer()
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
        }
        .padding(.leading, LayoutMetrics.trafficLightInset)
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
            Circle()
                .fill(Color.secondary)
                .frame(width: 8, height: 8)
            VStack(alignment: .leading, spacing: 1) {
                Text(session.name)
                Text(session.branch ?? "—")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
        .padding(.vertical, 2)
    }
}

struct SessionDetailView: View {
    let session: RepoSession
    let onClose: () -> Void

    @State private var work: WorkPaneModel
    @State private var tiling: PaneTilingModel
    @State private var agent: AgentSessionModel
    private let configSource: TerminalController.ConfigSource

    init(session: RepoSession, onClose: @escaping () -> Void) {
        self.session = session
        self.onClose = onClose
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
        .padding(.horizontal, 12)
        .frame(height: LayoutMetrics.topBar)
        .background(.bar)
    }
}
