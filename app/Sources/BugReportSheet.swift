import SwiftUI
import AppKit

/// バグ報告シート。サーバーを持たない本アプリでは「バグ報告 = GitHub Issue の作成」。
/// タイトル・内容を入力すると、環境情報（アプリ版・macOS・ツール診断）を折りたたみブロックで
/// 本文に自動添付し、プリフィルした `issues/new` ページをブラウザで開く。送信前に GitHub 上で
/// 確認・編集でき、サーバーへは何も送らない。ブラウザに頼らない導線として「内容をコピー」も用意する。
struct BugReportSheet: View {
    @Environment(\.dismiss) private var dismiss

    @State private var title = ""
    @State private var detail = ""
    @State private var copied = false

    /// ツール診断（@Observable シングルトン。body でのアクセスが追跡される）。
    private var doctor: ToolDoctor { .shared }

    private var canSubmit: Bool {
        !title.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Text("バグを報告")
                .font(.headline)
            Text("GitHub の Issue としてブラウザで開きます。サーバーには送信されません。")
                .font(.caption)
                .foregroundStyle(.secondary)
                .padding(.bottom, 10)

            Form {
                Section {
                    TextField("タイトル", text: $title)
                    VStack(alignment: .leading, spacing: 4) {
                        Text("どんな問題ですか？（再現手順・期待する動作など）")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                        TextEditor(text: $detail)
                            .font(.body)
                            .frame(height: 120)
                            .overlay(
                                RoundedRectangle(cornerRadius: 6)
                                    .strokeBorder(.quaternary, lineWidth: 1)
                            )
                    }
                }

                Section {
                    LabeledContent {
                        Text(appVersionText).foregroundStyle(.secondary)
                    } label: {
                        Text("アプリ")
                    }
                    LabeledContent {
                        Text(osVersion)
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                            .truncationMode(.middle)
                    } label: {
                        Text(verbatim: "macOS")
                    }
                    toolPreviewRow("git", doctor.git)
                    toolPreviewRow("gh", doctor.gh)
                    toolPreviewRow("claude", doctor.claude)
                } header: {
                    // 診断は起動時に一度だけ走るので、送信前に再検査できるようにする。
                    HStack {
                        Text("環境情報")
                        Spacer()
                        if doctor.checking {
                            ProgressView().controlSize(.small)
                        } else {
                            Button("再検査") { doctor.check() }
                                .buttonStyle(.borderless)
                                .font(.caption)
                        }
                    }
                } footer: {
                    Text("上記は本文に自動で添付されます。GitHub 上で送信前に確認・編集できます。")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }
            .formStyle(.grouped)

            HStack {
                Button("内容をコピー") { copyBody() }
                if copied {
                    Label("コピーしました", systemImage: "checkmark.circle.fill")
                        .font(.caption)
                        .foregroundStyle(.green)
                        .transition(.opacity)
                }
                Spacer()
                Button("キャンセル", role: .cancel) { dismiss() }
                    .keyboardShortcut(.cancelAction)
                Button("GitHub で Issue を作成") { submit() }
                    .keyboardShortcut(.defaultAction)
                    .disabled(!canSubmit)
            }
            .padding(.top, 12)
        }
        .padding(20)
        .frame(width: 560)
    }

    // MARK: - 環境情報

    /// SettingsView.versionText と同じ表記（"v1.2.3 (456)" / 版が読めなければ「不明」）。
    private var appVersionText: String {
        let version = (Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String) ?? ""
        guard !version.isEmpty else { return String(localized: "不明") }
        let build = (Bundle.main.object(forInfoDictionaryKey: "CFBundleVersion") as? String) ?? ""
        return build.isEmpty ? "v\(version)" : "v\(version) (\(build))"
    }

    private var osVersion: String {
        ProcessInfo.processInfo.operatingSystemVersionString
    }

    /// UI プレビュー用の 1 行（SettingsView.toolRow と同じ見た目）。
    @ViewBuilder
    private func toolPreviewRow(_ name: String, _ tool: ToolDoctor.Tool) -> some View {
        LabeledContent {
            HStack(spacing: 6) {
                Image(systemName: tool.found ? "checkmark.circle.fill" : "xmark.circle.fill")
                    .foregroundStyle(tool.found ? .green : .red)
                Text(toolValueText(tool))
                    .foregroundStyle(tool.found ? .primary : .secondary)
                    .lineLimit(1)
                    .truncationMode(.middle)
            }
            .help(tool.path ?? "")
        } label: {
            Text(name).font(.body.monospaced())
        }
    }

    private func toolValueText(_ tool: ToolDoctor.Tool) -> String {
        tool.found ? (tool.version ?? String(localized: "検出済み")) : String(localized: "見つかりません")
    }

    // MARK: - 本文の組み立て

    private var environmentLines: [String] {
        [
            String(localized: "アプリ") + ": \(appVersionText)",
            "macOS: \(osVersion)",
            toolBodyLine("git", doctor.git),
            toolBodyLine("gh", doctor.gh),
            toolBodyLine("claude", doctor.claude),
        ]
    }

    private func toolBodyLine(_ name: String, _ tool: ToolDoctor.Tool) -> String {
        guard tool.found else { return "\(name): " + String(localized: "見つかりません") }
        let version = tool.version ?? String(localized: "検出済み")
        if let path = tool.path, !path.isEmpty {
            return "\(name): \(version) (\(path))"
        }
        return "\(name): \(version)"
    }

    /// GitHub で折りたためる `<details>` ブロックにまとめた環境情報。
    /// `<summary>` の直後に空行を入れないと中の Markdown リストが描画されないので注意。
    private var environmentBody: String {
        let list = environmentLines.map { "- \($0)" }.joined(separator: "\n")
        let summary = String(localized: "環境情報")
        return "<details>\n<summary>\(summary)</summary>\n\n\(list)\n\n</details>"
    }

    /// Issue 本文（入力内容 + 環境情報）。入力が空なら環境情報のみ。
    private var issueBody: String {
        let detailText = detail.trimmingCharacters(in: .whitespacesAndNewlines)
        if detailText.isEmpty { return environmentBody }
        return "\(detailText)\n\n\(environmentBody)"
    }

    // MARK: - アクション

    private func submit() {
        NSWorkspace.shared.open(GitHubRepo.newIssueURL(title: title, body: issueBody))
        dismiss()
    }

    private func copyBody() {
        let pasteboard = NSPasteboard.general
        pasteboard.clearContents()
        pasteboard.setString(issueBody, forType: .string)
        withAnimation(LaboTheme.Motion.feedback) { copied = true }
        Task {
            try? await Task.sleep(for: .seconds(2))
            withAnimation(LaboTheme.Motion.feedback) { copied = false }
        }
    }
}
