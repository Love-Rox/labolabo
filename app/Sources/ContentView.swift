import SwiftUI
import LaboLaboEngine

/// Placeholder session model for the Phase-0 shell. Real sessions come from the
/// GRDB store + GitEngine in a later increment.
struct SidebarSession: Identifiable, Hashable {
    let id = UUID()
    let repo: String
    let name: String
    let branch: String
    let workingDirectory: String
}

struct ContentView: View {
    @State private var selection: SidebarSession.ID?

    // Grouped by repo in the sidebar (Supacode-style repo/session tree).
    // Demo sessions for the Phase-0 shell; real sessions come from GRDB + GitEngine.
    private let sessions: [SidebarSession] = [
        .init(repo: "labolabo", name: "git-engine", branch: "feature/git-engine", workingDirectory: NSHomeDirectory()),
        .init(repo: "labolabo", name: "macos-app", branch: "feature/macos-app", workingDirectory: NSHomeDirectory()),
    ]

    private var repos: [String] {
        var seen: [String] = []
        for s in sessions where !seen.contains(s.repo) { seen.append(s.repo) }
        return seen
    }

    var body: some View {
        NavigationSplitView {
            List(selection: $selection) {
                ForEach(repos, id: \.self) { repo in
                    Section(repo) {
                        ForEach(sessions.filter { $0.repo == repo }) { session in
                            SessionRow(session: session).tag(session.id)
                        }
                    }
                }
            }
            .listStyle(.sidebar)
            .navigationTitle("LaboLabo")
        } detail: {
            if let id = selection, let session = sessions.first(where: { $0.id == id }) {
                SessionDetailView(session: session)
            } else {
                ContentUnavailableView("セッションを選択してください", systemImage: "sidebar.left")
            }
        }
    }
}

struct SessionRow: View {
    let session: SidebarSession

    var body: some View {
        HStack(spacing: 8) {
            Circle()
                .fill(Color.secondary)
                .frame(width: 8, height: 8)
            VStack(alignment: .leading, spacing: 1) {
                Text(session.name)
                Text(session.branch)
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
        .padding(.vertical, 2)
    }
}

struct SessionDetailView: View {
    let session: SidebarSession

    var body: some View {
        VStack(spacing: 0) {
            SessionHeader(session: session)
            Divider()
            HSplitView {
                TerminalAreaView(workingDirectory: session.workingDirectory)
                    .frame(minWidth: 360)
                WorkPaneView()
                    .frame(minWidth: 320)
            }
        }
    }
}

struct SessionHeader: View {
    let session: SidebarSession

    var body: some View {
        HStack(spacing: 10) {
            Text(session.name).font(.headline)
            Label(session.branch, systemImage: "arrow.triangle.branch")
                .font(.subheadline)
                .foregroundStyle(.secondary)
            Spacer()
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
    }
}

struct TerminalTab: Identifiable {
    let id = UUID()
    var title: String
    let workingDirectory: String
}

/// The multi-terminal area: one or more libghostty terminals as tabs. Panes are
/// kept mounted (opacity-hidden, not removed) so background surfaces stay alive.
/// Split panes are added in a later increment (#10).
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

struct WorkPaneView: View {
    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Text("Changes")
                .font(.headline)
                .padding(.horizontal, 12)
                .padding(.vertical, 8)
            Divider()
            List {
                Text("変更ファイル一覧 ＋ Diff ⇄ 全文切替（GitEngine 連携を次の増分で）")
                    .font(.callout)
                    .foregroundStyle(.secondary)
            }
        }
    }
}

#Preview {
    ContentView()
}
