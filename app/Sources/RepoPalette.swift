import SwiftUI
import LaboLaboEngine

extension PullRequestInfo.State {
    var icon: String {
        switch self {
        case .open: return "arrow.triangle.pull"
        case .draft: return "arrow.triangle.pull"
        case .merged: return "arrow.triangle.merge"
        case .closed: return "xmark.circle"
        }
    }

    var color: Color {
        switch self {
        case .open: return .green
        case .draft: return .secondary
        case .merged: return .purple
        case .closed: return .red
        }
    }

    var label: String {
        switch self {
        case .open: return "Open"
        case .draft: return "Draft"
        case .merged: return "Merged"
        case .closed: return "Closed"
        }
    }
}

extension PullRequestInfo.Checks {
    var glyph: String? {
        switch self {
        case .passing: return "checkmark.circle.fill"
        case .failing: return "xmark.circle.fill"
        case .pending: return "clock.fill"
        case .none: return nil
        }
    }

    var color: Color {
        switch self {
        case .passing: return .green
        case .failing: return .red
        case .pending: return .orange
        case .none: return .secondary
        }
    }
}

/// リポジトリ・グループの色パレット。永続化は id 文字列で行う。
enum RepoPalette {
    static let entries: [(id: String, name: String, color: Color)] = [
        ("gray", String(localized: "グレー"), .gray),
        ("blue", String(localized: "ブルー"), .blue),
        ("green", String(localized: "グリーン"), .green),
        ("orange", String(localized: "オレンジ"), .orange),
        ("red", String(localized: "レッド"), .red),
        ("purple", String(localized: "パープル"), .purple),
        ("pink", String(localized: "ピンク"), .pink),
        ("teal", String(localized: "ティール"), .teal),
        ("yellow", String(localized: "イエロー"), .yellow),
        ("indigo", String(localized: "インディゴ"), .indigo),
    ]

    static func color(for id: String?) -> Color {
        guard let id else { return .secondary }
        return entries.first { $0.id == id }?.color ?? .secondary
    }
}
