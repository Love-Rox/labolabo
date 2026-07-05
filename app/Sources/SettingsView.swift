import SwiftUI
import AppKit

/// アプリ設定（⌘,）。今はアプリアイコンの表示モードのみ。将来 タブを増やす。
struct SettingsView: View {
    var body: some View {
        TabView {
            GeneralSettingsView()
                .tabItem { Label("一般", systemImage: "gearshape") }
        }
        .frame(width: 460)
        .scenePadding()
    }
}

struct GeneralSettingsView: View {
    @AppStorage(AppIconController.defaultsKey) private var iconModeRaw = AppIconMode.auto.rawValue
    @AppStorage(AgentNotifier.enabledKey) private var notifyWaiting = true
    @AppStorage(UpdateChecker.autoCheckKey) private var checkUpdatesOnLaunch = true
    /// ツール診断（@Observable シングルトン。body でのアクセスが追跡される）。
    private var doctor: ToolDoctor { .shared }
    /// アップデートチェッカ（@Observable シングルトン）。
    private var updates: UpdateChecker { .shared }

    private var iconMode: AppIconMode { AppIconMode(rawValue: iconModeRaw) ?? .auto }

    @State private var showBugReport = false

    var body: some View {
        Form {
            Section {
                Picker("アプリアイコン", selection: Binding(
                    get: { iconMode },
                    set: { iconModeRaw = $0.rawValue; AppIconController.shared.apply() }
                )) {
                    ForEach(AppIconMode.allCases) { mode in
                        Text(mode.label).tag(mode)
                    }
                }
                .pickerStyle(.radioGroup)

                LabeledContent("プレビュー") {
                    HStack(spacing: 12) {
                        iconPreview("AppIconDark", caption: String(localized: "ダーク"))
                        iconPreview("AppIconLight", caption: String(localized: "ライト"))
                    }
                }
            } footer: {
                Text("「自動」はシステムの外観（ライト/ダーク）に合わせて Dock アイコンを切り替えます。反映はアプリ実行中です。")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }

            Section {
                Toggle("入力待ちを通知する", isOn: $notifyWaiting)
            } header: {
                Text("通知")
            } footer: {
                Text("別のセッションで作業中や、アプリが非アクティブのときに、エージェントが入力・許可待ちになったら macOS 通知で知らせます。")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }

            Section {
                toolRow("git", doctor.git, note: String(localized: "差分・worktree 操作に必須"))
                toolRow("gh", doctor.gh, note: String(localized: "PR の表示・作成に必要"))
                toolRow("claude", doctor.claude, note: String(localized: "エージェント起動に必要"))
                HStack {
                    Button("再検査") { ToolDoctor.shared.check() }
                        .disabled(doctor.checking)
                    if doctor.checking { ProgressView().controlSize(.small) }
                }
            } header: {
                Text("ツール診断")
            } footer: {
                Text("見つからないツールに依存する機能（PR 作成・Claude 起動など）は無効になります。")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }

            Section {
                LabeledContent("現在のバージョン") {
                    Text(versionText).foregroundStyle(.secondary)
                }
                Toggle("起動時にアップデートを確認する", isOn: $checkUpdatesOnLaunch)
                HStack {
                    Button("アップデートを確認") { updates.check() }
                        .disabled(updates.state == .checking)
                    if updates.state == .checking { ProgressView().controlSize(.small) }
                    Spacer()
                    updateStatus
                }
            } header: {
                Text("アップデート")
            } footer: {
                Text("GitHub リリースを確認して新しいバージョンをお知らせします（自動ダウンロードは行いません／リリースページから取得）。")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }

            Section {
                Button("バグを報告…") { showBugReport = true }
            } header: {
                Text("フィードバック")
            } footer: {
                Text("バグや要望を GitHub の Issue として送信します。上記の環境情報が自動で添付されます。")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
        .formStyle(.grouped)
        .sheet(isPresented: $showBugReport) {
            BugReportSheet()
        }
    }

    private var versionText: String {
        let version = updates.currentVersion
        guard !version.isEmpty else { return String(localized: "不明") }
        let build = updates.currentBuild
        return build.isEmpty ? "v\(version)" : "v\(version) (\(build))"
    }

    @ViewBuilder
    private var updateStatus: some View {
        switch updates.state {
        case .idle, .checking:
            EmptyView()
        case .upToDate:
            Label("最新です", systemImage: "checkmark.circle.fill")
                .font(.caption).foregroundStyle(.green)
        case let .available(version, url):
            HStack(spacing: 6) {
                Text("v\(version) が利用可能").font(.caption).foregroundStyle(.orange)
                Button("開く") { NSWorkspace.shared.open(url) }
            }
        case .failed:
            Text("確認できませんでした").font(.caption).foregroundStyle(.secondary)
        }
    }

    @ViewBuilder
    private func toolRow(_ name: String, _ tool: ToolDoctor.Tool, note: String) -> some View {
        LabeledContent {
            HStack(spacing: 6) {
                Image(systemName: tool.found ? "checkmark.circle.fill" : "xmark.circle.fill")
                    .foregroundStyle(tool.found ? .green : .red)
                Text(tool.found ? (tool.version ?? String(localized: "検出済み")) : String(localized: "見つかりません"))
                    .foregroundStyle(tool.found ? .primary : .secondary)
                    .lineLimit(1)
                    .truncationMode(.middle)
            }
            .help(tool.path ?? "")
        } label: {
            VStack(alignment: .leading, spacing: 1) {
                Text(name).font(.body.monospaced())
                Text(note).font(.caption2).foregroundStyle(.secondary)
            }
        }
    }

    @ViewBuilder
    private func iconPreview(_ asset: String, caption: String) -> some View {
        let isActive = (asset == "AppIconLight") == (iconMode == .light)
            || (iconMode == .auto) // 自動時は両方を候補として弱めに強調しない
        VStack(spacing: 4) {
            if let ns = NSImage(named: asset) {
                Image(nsImage: ns)
                    .resizable()
                    .interpolation(.high)
                    .frame(width: 48, height: 48)
                    .clipShape(RoundedRectangle(cornerRadius: 11, style: .continuous))
                    .overlay(
                        RoundedRectangle(cornerRadius: 11, style: .continuous)
                            .strokeBorder(Color.accentColor, lineWidth: activeBorder(asset) ? 2 : 0)
                    )
            }
            Text(caption).font(.caption2).foregroundStyle(.secondary)
        }
        .opacity(isActive ? 1 : 0.6)
    }

    /// 手動選択時のみ、その背景を枠線で強調（自動時はどちらも強調しない）。
    private func activeBorder(_ asset: String) -> Bool {
        switch iconMode {
        case .dark: return asset == "AppIconDark"
        case .light: return asset == "AppIconLight"
        case .auto: return false
        }
    }
}
