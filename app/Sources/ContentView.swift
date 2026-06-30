import SwiftUI
import UniformTypeIdentifiers
import LaboLaboEngine
import GhosttyTerminal

struct ContentView: View {
    @State private var store = SessionStore()
    @State private var showImporter = false

    var body: some View {
        NavigationSplitView {
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
            .navigationTitle("LaboLabo")
            .toolbar {
                ToolbarItem {
                    Button {
                        showImporter = true
                    } label: {
                        Label("リポジトリを開く", systemImage: "plus")
                    }
                }
            }
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
                    Text("ツールバーの ＋ から git リポジトリ（worktree）を開きます")
                } actions: {
                    Button("リポジトリを開く") { showImporter = true }
                }
            }
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
    private let configSource: TerminalController.ConfigSource

    init(session: RepoSession, onClose: @escaping () -> Void) {
        self.session = session
        self.onClose = onClose
        _work = State(initialValue: WorkPaneModel(worktree: session.worktreePath))
        _tiling = State(initialValue: PaneTilingModel.defaultLayout())
        configSource = GhosttyConfig.userConfigSource()
    }

    var body: some View {
        PaneTilingView(
            model: tiling,
            context: PaneContext(
                workingDirectory: session.worktreePath.path,
                work: work,
                configSource: configSource
            ),
            revision: tiling.revision
        )
        .navigationTitle(session.name)
        .navigationSubtitle(session.worktreePath.path)
        .toolbar { toolbarContent }
        .onAppear { work.start() }
        .onDisappear { work.stop() }
    }

    /// すべての操作系を "LaboLabo" タイトルのあるウインドウ上部ツールバーに集約する。
    @ToolbarContentBuilder
    private var toolbarContent: some ToolbarContent {
        ToolbarItem(placement: .principal) {
            GitStatusBadges(status: work.status, fallbackBranch: session.branch)
        }
        ToolbarItemGroup(placement: .primaryAction) {
            Button {
                tiling.addPane(PaneItem(kind: .terminal, title: "端末"))
            } label: {
                Label("端末", systemImage: "plus.rectangle")
            }
            .help("端末を追加")

            Button {
                tiling.addPaneIfAbsent(kind: .files, title: "変更ファイル")
            } label: {
                Label("ファイル", systemImage: "list.bullet.rectangle")
            }
            .disabled(tiling.hasPane(kind: .files))
            .help("変更ファイル一覧を追加")

            Button {
                tiling.addPaneIfAbsent(kind: .diff, title: "Diff")
            } label: {
                Label("Diff", systemImage: "doc.text")
            }
            .disabled(tiling.hasPane(kind: .diff))
            .help("Diff を追加")

            Button {
                tiling.addPaneIfAbsent(kind: .commits, title: "履歴")
            } label: {
                Label("履歴", systemImage: "clock.arrow.circlepath")
            }
            .disabled(tiling.hasPane(kind: .commits))
            .help("コミット履歴グラフを追加")

            IDEOpenMenu(worktree: session.worktreePath)
            SessionClock()

            Button(role: .destructive) {
                onClose()
            } label: {
                Image(systemName: "xmark.circle.fill")
            }
            .help("セッションを閉じる")
        }
    }
}
