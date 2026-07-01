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
    @State private var showNewSession = false
    @State private var showChangelog = false
    @State private var columnVisibility: NavigationSplitViewVisibility = .all
    @State private var removalTarget: RemovalRequest?
    @State private var removalError: String?

    /// worktree 削除の確認対象（dirty なら文言を「強制削除」に変える）。
    struct RemovalRequest: Identifiable {
        let session: RepoSession
        let dirty: Bool
        var id: RepoSession.ID { session.id }
    }

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
                                            Button("セッションを閉じる") {
                                                store.close(session.id)
                                            }
                                            Divider()
                                            Button("worktree を削除…", role: .destructive) {
                                                requestRemoval(session)
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
            .sheet(isPresented: $showNewSession) {
                NewSessionSheet(store: store)
            }
            .confirmationDialog(
                "worktree を削除",
                isPresented: Binding(
                    get: { removalTarget != nil },
                    set: { if !$0 { removalTarget = nil } }
                ),
                presenting: removalTarget
            ) { req in
                Button(req.dirty ? "強制削除（変更を破棄）" : "削除", role: .destructive) {
                    performRemoval(req)
                }
                Button("キャンセル", role: .cancel) {}
            } message: { req in
                Text(removalMessage(req))
            }
            .alert(
                "worktree を削除できません",
                isPresented: Binding(
                    get: { removalError != nil },
                    set: { if !$0 { removalError = nil } }
                ),
                presenting: removalError
            ) { _ in
                Button("OK", role: .cancel) {}
            } message: { message in
                Text(message)
            }
        } detail: {
            if let session = store.selected {
                SessionDetailView(
                    session: session,
                    store: store,
                    onClose: { store.close(session.id) },
                    sidebarCollapsed: sidebarCollapsed,
                    onExpandSidebar: { columnVisibility = .all }
                )
                .id(session.id)
            } else {
                ContentUnavailableView {
                    Label("セッションがありません", systemImage: "sidebar.left")
                } description: {
                    Text("左上の ＋ から新規セッション（worktree 作成）または既存フォルダを開きます")
                } actions: {
                    Button("新規セッション…") { showNewSession = true }
                    Button("既存のフォルダを開く…") { showImporter = true }
                }
            }
        }
    }

    /// worktree 削除の確認を開始（dirty 判定を待ってからダイアログ表示）。
    private func requestRemoval(_ session: RepoSession) {
        Task {
            let dirty = await store.isWorktreeDirty(session.id)
            removalTarget = RemovalRequest(session: session, dirty: dirty)
        }
    }

    /// 確認後の実削除。dirty なら force で削除。失敗は alert 表示。
    private func performRemoval(_ req: RemovalRequest) {
        Task {
            do {
                try await store.removeWorktree(req.session.id, force: req.dirty)
            } catch {
                removalError = (error as? GitCommandError)?
                    .stderr.trimmingCharacters(in: .whitespacesAndNewlines)
                    ?? error.localizedDescription
            }
        }
    }

    private func removalMessage(_ req: RemovalRequest) -> String {
        let path = req.session.worktreePath.path
        if req.dirty {
            return "「\(req.session.name)」には未コミット/未追跡の変更があります。強制削除すると変更は失われます。\n\n\(path)"
        }
        return "「\(req.session.name)」の worktree を削除します。\n\n\(path)"
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
            Menu {
                Button("新規セッション（worktree を作成）…") { showNewSession = true }
                Button("既存のフォルダを開く…") { showImporter = true }
            } label: {
                Image(systemName: "plus")
            }
            .menuStyle(.borderlessButton)
            .menuIndicator(.hidden)
            .fixedSize()
            .help("セッションを追加")
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
            AgentStatusIndicator(status: session.agent?.status ?? .none)
            VStack(alignment: .leading, spacing: 1) {
                Text(session.name).lineLimit(1).truncationMode(.middle)
                    .help(session.name)
                Text(session.branch ?? "—")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .truncationMode(.middle)
                    .help(session.branch ?? "—")
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

/// セッション行のエージェント状態ドット。入力待ちはオレンジ＋パルスで目立たせる。
struct AgentStatusIndicator: View {
    let status: AgentStatus
    @State private var animate = false

    var body: some View {
        ZStack {
            if status == .waitingForInput {
                Circle()
                    .fill(Color.orange.opacity(0.35))
                    .frame(width: 15, height: 15)
                    .scaleEffect(animate ? 1.0 : 0.4)
                    .opacity(animate ? 0 : 0.85)
            }
            Circle()
                .fill(tint ?? .clear)
                .frame(width: 7, height: 7)
        }
        .frame(width: 12, height: 12)
        .help(status == .none ? "" : status.label)
        .onAppear { restartPulse() }
        .onChange(of: status) { _, _ in restartPulse() }
    }

    private func restartPulse() {
        animate = false
        guard status == .waitingForInput else { return }
        withAnimation(.easeOut(duration: 1.0).repeatForever(autoreverses: false)) {
            animate = true
        }
    }

    private var tint: Color? {
        switch status {
        case .waitingForInput: return .orange
        case .running, .starting: return .blue
        case .idle: return .green
        case .none, .ended: return nil
        }
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
    let store: SessionStore
    let onClose: () -> Void
    var sidebarCollapsed: Bool = false
    var onExpandSidebar: () -> Void = {}

    @State private var work: WorkPaneModel
    @State private var tiling: PaneTilingModel
    @State private var showSavePreset = false
    @State private var presetName = ""
    private let configSource: TerminalController.ConfigSource

    init(
        session: RepoSession,
        store: SessionStore,
        onClose: @escaping () -> Void,
        sidebarCollapsed: Bool = false,
        onExpandSidebar: @escaping () -> Void = {}
    ) {
        self.session = session
        self.store = store
        self.onClose = onClose
        self.sidebarCollapsed = sidebarCollapsed
        self.onExpandSidebar = onExpandSidebar
        _work = State(initialValue: WorkPaneModel(worktree: session.worktreePath))

        // 保存済み配置（セッション別 → 無ければ既定プリセット）から復元。
        let sid = session.id
        let saved = store.paneLayout(for: sid) ?? store.defaultPaneLayout()
        let model = saved.flatMap { PaneTilingModel.model(from: $0) } ?? PaneTilingModel.defaultLayout()
        model.onLayoutChanged = { [weak model] in
            guard let model else { return }
            store.savePaneLayout(sid, model.snapshot())
        }
        _tiling = State(initialValue: model)
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
        .onAppear { work.start() }
        .onDisappear {
            work.stop()
            // ratio ドラッグは bump しないので、離脱時に最終配置を保存する。
            store.savePaneLayout(session.id, tiling.snapshot())
        }
        .alert("プリセットとして保存", isPresented: $showSavePreset) {
            TextField("プリセット名", text: $presetName)
            Button("保存") {
                store.savePreset(name: presetName, layout: tiling.snapshot())
                presetName = ""
            }
            Button("キャンセル", role: .cancel) { presetName = "" }
        } message: {
            Text("現在のペイン配置に名前を付けて保存します。以後どのセッションにも適用できます。")
        }
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
                agentStatus: session.agent?.status ?? .none
            )

            Spacer(minLength: 12)

            Button {
                tiling.launchInNewTerminal(title: "Claude", command: session.agent?.launchCommand() ?? "claude")
            } label: {
                ClaudeMark().frame(width: 15, height: 15)
            }
            .buttonStyle(CircleIconButtonStyle(tint: Color(red: 0.85, green: 0.47, blue: 0.34)))
            .help((session.agent?.canResume ?? false)
                ? "Claude を再開（前回のセッションを --resume）"
                : "Claude を起動（状態検出 hooks 付き）")

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

            Menu {
                if !store.presets.isEmpty {
                    Section("プリセットを適用") {
                        ForEach(store.presets) { preset in
                            Button(preset.name) { tiling.apply(preset.layout) }
                        }
                    }
                }
                Section {
                    Button("現在の配置をプリセットとして保存…", systemImage: "plus") {
                        showSavePreset = true
                    }
                    Button("既定の配置に戻す", systemImage: "arrow.uturn.backward") {
                        tiling.resetToDefault()
                    }
                }
                if !store.presets.isEmpty {
                    Menu("プリセットを削除") {
                        ForEach(store.presets) { preset in
                            Button(preset.name, role: .destructive) {
                                store.deletePreset(name: preset.name)
                            }
                        }
                    }
                }
            } label: {
                Image(systemName: "rectangle.3.group")
            }
            .menuStyle(.button)
            .buttonStyle(CircleIconButtonStyle())
            .menuIndicator(.hidden)
            .fixedSize()
            .help("ペイン配置プリセット（保存・適用）")

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
