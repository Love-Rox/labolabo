import AppKit
import SwiftUI

/// アプリの外観モード。設定画面から選ぶ。
enum AppAppearanceMode: String, CaseIterable, Identifiable {
    /// システムの外観（ライト/ダーク）に追従（既定）。
    case system
    /// 常にライト。
    case light
    /// 常にダーク。
    case dark

    var id: String { rawValue }

    var label: String {
        switch self {
        case .system: return String(localized: "システムに合わせる")
        case .light: return String(localized: "ライト")
        case .dark: return String(localized: "ダーク")
        }
    }
}

/// アプリ全体の外観（ライト/ダーク/システム準拠）を設定に応じて適用する。
/// `NSApp.appearance` の上書き（nil ＝ システム準拠）で実現し、LaboTheme の
/// 動的色や SwiftUI の semantic color はそのまま追従する。
@MainActor
final class AppearanceController {
    static let shared = AppearanceController()
    static let defaultsKey = "appAppearanceMode"

    private init() {}

    /// 現在の設定（UserDefaults ＝ @AppStorage と共有）。既定はシステム準拠。
    var mode: AppAppearanceMode {
        AppAppearanceMode(rawValue: UserDefaults.standard.string(forKey: Self.defaultsKey) ?? "") ?? .system
    }

    /// 設定を NSApp.appearance に適用する（即時に全ウインドウへ反映され、再起動不要）。
    func apply() {
        switch mode {
        case .system: NSApp.appearance = nil
        case .light: NSApp.appearance = NSAppearance(named: .aqua)
        case .dark: NSApp.appearance = NSAppearance(named: .darkAqua)
        }
        // Dock アイコンの「自動」は effectiveAppearance 基準なので、外観変更後に再適用する。
        AppIconController.shared.apply()
    }
}
