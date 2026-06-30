import SwiftUI
import UniformTypeIdentifiers
import LaboLaboEngine

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

    var body: some View {
        VStack(spacing: 0) {
            SessionHeader(session: session, onClose: onClose)
            Divider()
            HSplitView {
                TerminalAreaView(workingDirectory: session.worktreePath.path)
                    .frame(minWidth: 320, idealWidth: 520)
                WorkPaneView(worktree: session.worktreePath)
                    .frame(minWidth: 420, idealWidth: 680)
            }
        }
    }
}

struct SessionHeader: View {
    let session: RepoSession
    let onClose: () -> Void

    var body: some View {
        HStack(spacing: 10) {
            Text(session.name).font(.headline)
            Label(session.branch ?? "—", systemImage: "arrow.triangle.branch")
                .font(.subheadline)
                .foregroundStyle(.secondary)
            Spacer()
            Text(session.worktreePath.path)
                .font(.caption)
                .foregroundStyle(.tertiary)
                .lineLimit(1)
                .truncationMode(.head)
            Button(role: .destructive) {
                onClose()
            } label: {
                Image(systemName: "xmark.circle.fill")
            }
            .buttonStyle(.borderless)
            .help("セッションを閉じる")
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
    }
}

// MARK: - Terminal area (multiple terminals as tabs)

struct TerminalTab: Identifiable {
    let id = UUID()
    var title: String
    let workingDirectory: String
}

/// One or more libghostty terminals as tabs. Panes are kept mounted
/// (opacity-hidden, not removed) so background surfaces stay alive. Split panes
/// arrive in a later increment (#10).
struct TerminalAreaView: View {
    let workingDirectory: String

    @State private var tabs: [TerminalTab]
    @State private var selected: UUID

    init(workingDirectory: String) {
        self.workingDirectory = workingDirectory
        let first = TerminalTab(title: "shell", workingDirectory: workingDirectory)
        _tabs = State(initialValue: [first])
        _selected = State(initialValue: first.id)
    }

    var body: some View {
        VStack(spacing: 0) {
            tabBar
            Divider()
            ZStack {
                Color.black
                ForEach(tabs) { tab in
                    GhosttyTerminalPane(workingDirectory: tab.workingDirectory)
                        .opacity(tab.id == selected ? 1 : 0)
                        .allowsHitTesting(tab.id == selected)
                }
            }
        }
    }

    private var tabBar: some View {
        HStack(spacing: 4) {
            ForEach(tabs) { tab in
                Text(tab.title)
                    .font(.caption)
                    .padding(.horizontal, 8)
                    .padding(.vertical, 3)
                    .background(
                        tab.id == selected ? Color.accentColor.opacity(0.25) : Color.clear,
                        in: RoundedRectangle(cornerRadius: 5)
                    )
                    .contentShape(Rectangle())
                    .onTapGesture { selected = tab.id }
            }
            Button {
                let tab = TerminalTab(title: "shell", workingDirectory: workingDirectory)
                tabs.append(tab)
                selected = tab.id
            } label: {
                Image(systemName: "plus")
            }
            .buttonStyle(.borderless)
            .help("新しい端末タブ")
            Spacer()
        }
        .padding(6)
    }
}
