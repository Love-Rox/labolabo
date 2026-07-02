import SwiftUI
import AppKit
import LaboLaboEngine

/// 現在ブランチから Pull Request を作成するシート。
/// push（`git push -u origin HEAD`）→ `gh pr create` の順で実行する。
struct PRCreateSheet: View {
    let store: SessionStore
    let session: RepoSession
    @Environment(\.dismiss) private var dismiss

    @State private var loading = true
    @State private var branches: [String] = []
    @State private var base = ""
    @State private var title = ""
    @State private var body_ = ""
    @State private var draft = true
    @State private var creating = false
    @State private var errorText: String?

    private var canCreate: Bool {
        !creating && !base.isEmpty && !title.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Text("Pull Request を作成")
                .font(.headline)
            HStack(spacing: 4) {
                Image(systemName: "arrow.triangle.branch").font(.caption2)
                Text(session.branch ?? "—")
                    .font(.caption.monospaced())
                Text("→ push してから PR を作成します")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            .padding(.bottom, 10)

            Form {
                if loading {
                    HStack { ProgressView().controlSize(.small); Text("リポジトリ情報を取得中…").foregroundStyle(.secondary) }
                } else {
                    Picker("ベースブランチ", selection: $base) {
                        ForEach(branches, id: \.self) { Text($0).tag($0) }
                    }
                    TextField("タイトル", text: $title)
                    // 本文は任意（Closes #N を書けば Issue と紐づく）。
                    VStack(alignment: .leading, spacing: 4) {
                        Text("本文（任意・Closes #N で Issue に紐づけ）")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                        TextEditor(text: $body_)
                            .font(.body)
                            .frame(height: 110)
                            .overlay(
                                RoundedRectangle(cornerRadius: 6)
                                    .strokeBorder(.quaternary, lineWidth: 1)
                            )
                    }
                    Toggle("Draft として作成", isOn: $draft)
                }

                if let errorText {
                    Label(errorText, systemImage: "exclamationmark.triangle")
                        .foregroundStyle(.red)
                        .font(.caption)
                        .textSelection(.enabled)
                }
            }
            .formStyle(.grouped)

            HStack {
                Spacer()
                Button("キャンセル", role: .cancel) { dismiss() }
                    .keyboardShortcut(.cancelAction)
                Button(creating ? "作成中…" : "push して作成") { create() }
                    .keyboardShortcut(.defaultAction)
                    .disabled(!canCreate)
            }
            .padding(.top, 12)
        }
        .padding(20)
        .frame(width: 560)
        .task { await load() }
    }

    private func load() async {
        let inspect = await store.inspectRepo(at: session.worktreePath)
        let current = inspect?.current ?? session.branch
        branches = (inspect?.branches ?? []).filter { $0 != current }
        // 運用に合わせて dev を優先、無ければ main、どちらも無ければ先頭。
        base = ["dev", "main"].first(where: { branches.contains($0) }) ?? branches.first ?? ""
        if title.isEmpty { title = inspect?.lastSubject ?? current ?? "" }
        loading = false
    }

    private func create() {
        creating = true
        errorText = nil
        Task {
            do {
                let url = try await store.createPullRequest(
                    session.id, base: base, title: title, body: body_, draft: draft
                )
                if let link = URL(string: url) { NSWorkspace.shared.open(link) }
                dismiss()
            } catch {
                creating = false
                errorText = (error as? GitCommandError)?
                    .stderr.trimmingCharacters(in: .whitespacesAndNewlines)
                    ?? error.localizedDescription
            }
        }
    }
}
