import SwiftUI

/// org ディレクトリ配下の git リポジトリを検出し、選んだものを個別セッションとして
/// まとめて開くシート。
struct OrgOpenSheet: View {
    let store: SessionStore
    let folder: URL
    @Environment(\.dismiss) private var dismiss

    @State private var repos: [URL] = []
    @State private var selected: Set<String> = []
    @State private var loading = true

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Text("リポジトリを開く")
                .font(.headline)
            Text(folder.path)
                .font(.caption)
                .foregroundStyle(.secondary)
                .lineLimit(1)
                .truncationMode(.middle)
                .padding(.bottom, 10)

            if loading {
                HStack { ProgressView().controlSize(.small); Text("リポジトリを検索中…").foregroundStyle(.secondary) }
                    .frame(maxWidth: .infinity, minHeight: 200)
            } else if repos.isEmpty {
                ContentUnavailableView("git リポジトリが見つかりませんでした", systemImage: "folder.badge.questionmark")
                    .frame(minHeight: 200)
            } else {
                List {
                    ForEach(repos, id: \.path) { repo in
                        Toggle(isOn: binding(for: repo)) {
                            VStack(alignment: .leading, spacing: 1) {
                                Text(repo.lastPathComponent)
                                Text(relativePath(repo))
                                    .font(.caption2)
                                    .foregroundStyle(.secondary)
                                    .lineLimit(1)
                                    .truncationMode(.middle)
                            }
                        }
                    }
                }
                .frame(height: 260)
                HStack(spacing: 8) {
                    Button("全選択") { selected = Set(repos.map(\.path)) }
                    Button("全解除") { selected = [] }
                    Text("\(repos.count) 件").font(.caption).foregroundStyle(.secondary)
                }
                .padding(.top, 4)
            }

            HStack {
                Spacer()
                Button("キャンセル", role: .cancel) { dismiss() }
                    .keyboardShortcut(.cancelAction)
                Button("開く（\(selected.count)）") { open() }
                    .keyboardShortcut(.defaultAction)
                    .disabled(selected.isEmpty)
            }
            .padding(.top, 12)
        }
        .padding(20)
        .frame(width: 540)
        .task { await load() }
    }

    private func binding(for repo: URL) -> Binding<Bool> {
        Binding(
            get: { selected.contains(repo.path) },
            set: { on in
                if on { selected.insert(repo.path) } else { selected.remove(repo.path) }
            }
        )
    }

    /// org フォルダからの相対パス（見やすさ用）。
    private func relativePath(_ repo: URL) -> String {
        let base = folder.path.hasSuffix("/") ? folder.path : folder.path + "/"
        return repo.path.hasPrefix(base) ? String(repo.path.dropFirst(base.count)) : repo.path
    }

    private func load() async {
        repos = await store.discoverRepos(under: folder)
        selected = Set(repos.map(\.path))
        loading = false
    }

    private func open() {
        let urls = repos.filter { selected.contains($0.path) }
        store.openRepositories(urls)
        dismiss()
    }
}
