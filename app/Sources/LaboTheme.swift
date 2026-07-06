import AppKit
import SwiftUI

/// Web ランディング（src/styles.css / app-ui.tsx）のデザイントークンを Swift に集約したもの。
/// 他のビューからは `LaboTheme.brand` のように参照する。
/// NSColor の static 定数は置かない（NSColor は Sendable でないため、Swift 6 の
/// strict concurrency でグローバル定数にできない）。Color は Sendable なので安全。
enum LaboTheme {
    // MARK: - ブランドカラー

    /// ブランドのライムイエロー #D0FF00。塗り（fill）やステータスドット用。両外観共通。
    static let brand = Color(hex: 0xD0FF00)

    /// ブランド色の文字用。ライトでは #D0FF00 のままだとコントラスト不足のため
    /// 暗いオリーブ #5C7300 に落とす。ダークではブランド色をそのまま使う。
    static let brandText = Color(light: Color(hex: 0x5C7300), dark: Color(hex: 0xD0FF00))

    // MARK: - アクセント / 状態色

    /// 琥珀色（警告・実行中などのアクセント）。
    static let amber = Color(light: Color(hex: 0xB45309), dark: Color(hex: 0xFFC53D))

    /// ローズ（エラー・削除などのアクセント）。
    static let rose = Color(light: Color(hex: 0xE11D48), dark: Color(hex: 0xFB7185))

    /// アイドル状態のステータスドット用の淡いグレー。
    static let statusIdle = Color(
        light: Color.black.opacity(0.25),
        dark: Color.white.opacity(0.30)
    )

    // MARK: - 背景 / パネル

    /// ほぼ黒のインク色 #0B0B0E（Web のベース背景）。両外観共通。
    static let ink = Color(hex: 0x0B0B0E)

    /// パネル背景。
    static let panel = Color(light: Color(hex: 0xF4F4F5), dark: Color(hex: 0x131318))

    /// 一段浮いたパネル背景（カード・ホバーなど）。
    static let panelRaised = Color(light: Color(hex: 0xECECEE), dark: Color(hex: 0x1A1A21))

    /// 罫線・枠線。
    static let border = Color(
        light: Color.black.opacity(0.10),
        dark: Color.white.opacity(0.08)
    )

    // MARK: - Diff 背景

    /// Diff の追加行の背景（ブランド色の 10%）。
    static let diffAddBg = brand.opacity(0.10)

    /// Diff の削除行の背景（ローズの 10%）。
    static let diffDelBg = rose.opacity(0.10)
}

// MARK: - 補助イニシャライザ

extension Color {
    /// `0xRRGGBB` 形式の 16 進値から不透明色を作る。
    fileprivate init(hex: UInt32) {
        self.init(
            red: Double((hex >> 16) & 0xFF) / 255.0,
            green: Double((hex >> 8) & 0xFF) / 255.0,
            blue: Double(hex & 0xFF) / 255.0
        )
    }

    /// ライト/ダーク外観で切り替わる動的な色を作る。
    /// closure が捕捉するのは Sendable な `Color` のみなので strict concurrency 下でも安全。
    fileprivate init(light: Color, dark: Color) {
        self.init(nsColor: NSColor(name: nil) { appearance in
            appearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua
                ? NSColor(dark)
                : NSColor(light)
        })
    }
}
