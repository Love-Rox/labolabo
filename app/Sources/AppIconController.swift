import AppKit
import SwiftUI

/// Dock アイコンの表示モード。設定画面から選ぶ。
enum AppIconMode: String, CaseIterable, Identifiable {
    /// システムの外観（ライト/ダーク）に追従。
    case auto
    /// 常にダーク背景。
    case dark
    /// 常にライト背景。
    case light

    var id: String { rawValue }

    var label: String {
        switch self {
        case .auto: return "自動（外観に追従）"
        case .dark: return "ダーク"
        case .light: return "ライト"
        }
    }

    /// この行に対応するアセット名（設定画面のプレビュー用）。
    var previewAsset: String { self == .light ? "AppIconLight" : "AppIconDark" }
}

/// 実行中アプリの Dock アイコンを、設定（自動/ダーク/ライト）とシステム外観に応じて
/// 切り替える。macOS はアプリ実行中のみ `NSApp.applicationIconImage` で差し替えられる。
@MainActor
final class AppIconController {
    static let shared = AppIconController()
    static let defaultsKey = "appIconMode"

    private var observing = false

    private init() {}

    /// 現在の設定（UserDefaults ＝ @AppStorage と共有）。既定は自動。
    var mode: AppIconMode {
        AppIconMode(rawValue: UserDefaults.standard.string(forKey: Self.defaultsKey) ?? "") ?? .auto
    }

    /// システム外観の変更監視を開始し、初回適用する。
    func start() {
        if !observing {
            observing = true
            DistributedNotificationCenter.default().addObserver(
                self,
                selector: #selector(systemAppearanceChanged),
                name: NSNotification.Name("AppleInterfaceThemeChangedNotification"),
                object: nil
            )
        }
        apply()
    }

    @objc private func systemAppearanceChanged() {
        // 通知直後は effectiveAppearance がまだ更新前のことがあるため次ループで適用。
        DispatchQueue.main.async { [weak self] in self?.apply() }
    }

    /// 現在の設定＋外観に合わせて Dock アイコンを更新する。
    func apply() {
        let useDark: Bool
        switch mode {
        case .dark: useDark = true
        case .light: useDark = false
        case .auto: useDark = Self.systemIsDark()
        }
        let name = useDark ? "AppIconDark" : "AppIconLight"
        if let image = NSImage(named: name) {
            NSApp.applicationIconImage = image
        }
    }

    private static func systemIsDark() -> Bool {
        NSApp.effectiveAppearance.bestMatch(from: [.aqua, .darkAqua]) == .darkAqua
    }
}
