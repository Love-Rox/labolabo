import SwiftUI
import UniformTypeIdentifiers
import LaboLaboEngine

/// 新規セッション作成シート。リポジトリを選び、ベースブランチから新しいブランチ＋
/// worktree を作ってセッション化する（1 セッション = 1 worktree）。
struct NewSessionSheet: View {
    let store: SessionStore
    @Environment(\.dismiss) private var dismiss

    @State private var repoURL: URL?
    @State private var inspect: SessionStore.RepoInspect?
    @State private var loading = false
    @State private var baseRef = ""
    @State private var newBranch = ""
    @State private var sessionName = ""
    @State private var adapterID = AgentAdapters.default.id
    @State private var showRepoPicker = false

    private var adapter: AgentAdapter { AgentAdapters.find(id: adapterID) }
    @State private var creating = false
    @State private var errorText: String?

    private var trimmedBranch: String {
        newBranch.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private var worktreePath: URL? {
        guard let root = inspect?.root, !trimmedBranch.isEmpty else { return nil }
        return SessionStore.defaultWorktreePath(repoRoot: root, branch: trimmedBranch)
    }

    private var canCreate: Bool {
        inspect != nil && !trimmedBranch.isEmpty && !baseRef.isEmpty && !creating
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Text("新しいセッション")
                .font(.headline)
                .padding(.bottom, 10)

            Form {
                LabeledContent("リポジトリ") {
                    HStack(spacing: 8) {
                        Text(repoLabel)
                            .foregroundStyle(inspect == nil ? .secondary : .primary)
                            .lineLimit(1)
                            .truncationMode(.middle)
                        Spacer(minLength: 8)
                        if loading { ProgressView().controlSize(.small) }
                        Button(repoURL == nil ? "選択…" : "変更…") { showRepoPicker = true }
                    }
                }

                if let inspect {
                    Picker("ベースブランチ", selection: $baseRef) {
                        ForEach(inspect.branches, id: \.self) { Text($0).tag($0) }
                    }

                    TextField("新しいブランチ名", text: $newBranch, prompt: Text("feature/xxx"))
                        .onChange(of: newBranch) { autofillName() }

                    TextField("セッション名", text: $sessionName, prompt: Text(defaultName))

                    Picker("エージェント", selection: $adapterID) {
                        ForEach(AgentAdapters.all) { Text($0.displayName).tag($0.id) }
                    }
                    if !adapter.capabilities.statusReporting.providesLiveStatus {
                        Label(
                            "このエージェントはライブ状態検出に非対応です（\(adapter.capabilities.statusReporting.label)）。起動/終了のみ表示します。",
                            systemImage: "info.circle"
                        )
                        .font(.caption)
                        .foregroundStyle(.secondary)
                    }

                    if let worktreePath {
                        LabeledContent("配置先") {
                            Text(worktreePath.path)
                                .font(.caption)
                                .foregroundStyle(.secondary)
                                .lineLimit(1)
                                .truncationMode(.middle)
                                .help(worktreePath.path)
                        }
                    }
                }

                if let errorText {
                    Label(errorText, systemImage: "exclamationmark.triangle")
                        .foregroundStyle(.red)
                        .font(.caption)
                }
            }
            .formStyle(.grouped)

            HStack {
                Spacer()
                Button("キャンセル", role: .cancel) { dismiss() }
                    .keyboardShortcut(.cancelAction)
                Button(creating ? "作成中…" : "作成") { create() }
                    .keyboardShortcut(.defaultAction)
                    .disabled(!canCreate)
            }
            .padding(.top, 12)
        }
        .padding(20)
        .frame(width: 540)
        .fileImporter(isPresented: $showRepoPicker, allowedContentTypes: [.folder]) { result in
            if case let .success(url) = result { pick(url) }
        }
    }

    private var repoLabel: String {
        if let inspect { return inspect.name }
        if let repoURL { return repoURL.lastPathComponent }
        return "リポジトリを選択してください"
    }

    /// ブランチ名の末尾から既定セッション名。
    private var defaultName: String {
        trimmedBranch.split(separator: "/").last.map(String.init) ?? trimmedBranch
    }

    private func pick(_ url: URL) {
        repoURL = url
        inspect = nil
        loading = true
        errorText = nil
        Task {
            let info = await store.inspectRepo(at: url)
            loading = false
            guard let info else {
                errorText = "git リポジトリを解決できませんでした"
                return
            }
            inspect = info
            baseRef = info.current ?? info.branches.first ?? ""
        }
    }

    /// セッション名が空/自動補完のままならブランチ名末尾で補う（手入力は尊重）。
    @State private var lastAutofill = ""
    private func autofillName() {
        let suggestion = defaultName
        if sessionName.isEmpty || sessionName == lastAutofill {
            sessionName = suggestion
        }
        lastAutofill = suggestion
    }

    private func create() {
        guard let inspect, let worktreePath else { return }
        let branch = trimmedBranch
        let name = sessionName.isEmpty ? defaultName : sessionName
        creating = true
        errorText = nil
        Task {
            do {
                try await store.createWorktreeSession(
                    repoRoot: inspect.root, baseRef: baseRef,
                    newBranch: branch, name: name, worktreePath: worktreePath,
                    adapterID: adapterID
                )
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
