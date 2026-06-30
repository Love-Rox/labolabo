import SwiftUI
import LaboLaboEngine

/// Placeholder session model for the Phase-0 shell. Real sessions come from the
/// GRDB store + GitEngine in a later increment.
struct SidebarSession: Identifiable, Hashable {
    let id = UUID()
    let repo: String
    let name: String
    let branch: String
}

struct ContentView: View {
    @State private var selection: SidebarSession.ID?

    // Grouped by repo in the sidebar (Supacode-style repo/session tree).
    private let sessions: [SidebarSession] = [
        .init(repo: "labolabo", name: "git-engine", branch: "feature/git-engine"),
        .init(repo: "labolabo", name: "macos-app", branch: "feature/macos-app"),
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
                TerminalAreaView()
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

/// Foreshadows the multi-terminal area (tabs + split panes). The real
/// libghostty `TerminalSurfaceView` instances are wired in the next increment.
struct TerminalAreaView: View {
    @State private var tabs: [String] = ["zsh"]
    @State private var selected = 0

    var body: some View {
        VStack(spacing: 0) {
            HStack(spacing: 4) {
                ForEach(Array(tabs.enumerated()), id: \.offset) { index, name in
                    Text(name)
                        .font(.caption)
                        .padding(.horizontal, 8)
                        .padding(.vertical, 3)
                        .background(
                            index == selected ? Color.accentColor.opacity(0.25) : Color.clear,
                            in: RoundedRectangle(cornerRadius: 5)
                        )
                        .onTapGesture { selected = index }
                }
                Button {
                    tabs.append("zsh")
                    selected = tabs.count - 1
                } label: {
                    Image(systemName: "plus")
                }
                .buttonStyle(.borderless)
                Spacer()
            }
            .padding(6)
            Divider()
            ZStack {
                Color.black
                Text("Terminal (libghostty) — 次の増分で埋め込み")
                    .font(.callout)
                    .foregroundStyle(.white.opacity(0.45))
            }
        }
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
