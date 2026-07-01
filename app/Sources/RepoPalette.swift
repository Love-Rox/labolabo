import SwiftUI

/// リポジトリ・グループの色パレット。永続化は id 文字列で行う。
enum RepoPalette {
    static let entries: [(id: String, name: String, color: Color)] = [
        ("gray", "グレー", .gray),
        ("blue", "ブルー", .blue),
        ("green", "グリーン", .green),
        ("orange", "オレンジ", .orange),
        ("red", "レッド", .red),
        ("purple", "パープル", .purple),
        ("pink", "ピンク", .pink),
        ("teal", "ティール", .teal),
        ("yellow", "イエロー", .yellow),
        ("indigo", "インディゴ", .indigo),
    ]

    static func color(for id: String?) -> Color {
        guard let id else { return .secondary }
        return entries.first { $0.id == id }?.color ?? .secondary
    }
}
