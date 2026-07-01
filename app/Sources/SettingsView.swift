import SwiftUI

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

    private var iconMode: AppIconMode { AppIconMode(rawValue: iconModeRaw) ?? .auto }

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
                        iconPreview("AppIconDark", caption: "ダーク")
                        iconPreview("AppIconLight", caption: "ライト")
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
        }
        .formStyle(.grouped)
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
