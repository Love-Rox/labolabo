import SwiftUI

/// アプリ内の変更履歴ビューア。バンドルした CHANGELOG.md（release-please が
/// Conventional Commits から自動生成）を読み込んで表示する。まだリリースが無い間は
/// 空状態＋ GitHub Releases へのリンクを出す。
struct ChangelogView: View {
    @Environment(\.dismiss) private var dismiss

    private static let releasesURL = GitHubRepo.releasesPage

    private var version: String {
        (Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String) ?? "—"
    }

    private var build: String {
        (Bundle.main.object(forInfoDictionaryKey: "CFBundleVersion") as? String) ?? "—"
    }

    private var releaseLines: [ChangelogLine] {
        guard let url = Bundle.main.url(forResource: "CHANGELOG", withExtension: "md"),
              let text = try? String(contentsOf: url, encoding: .utf8) else { return [] }
        let all = ChangelogParser.parse(text)
        // 先頭のタイトル/前置きは省き、最初のバージョン見出し以降だけを見せる。
        guard let first = all.firstIndex(where: { $0.kind == .version }) else { return [] }
        return Array(all[first...])
    }

    var body: some View {
        VStack(spacing: 0) {
            header
            Divider()
            if releaseLines.isEmpty {
                ContentUnavailableView {
                    Label("まだリリースはありません", systemImage: "clock.badge.questionmark")
                } description: {
                    Text("リリースが作成されると、ここに変更履歴が表示されます。")
                } actions: {
                    Link("GitHub のリリースを見る", destination: Self.releasesURL)
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                ScrollView {
                    VStack(alignment: .leading, spacing: 3) {
                        ForEach(releaseLines) { ChangelogLineView(line: $0) }
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .textSelection(.enabled)
                    .padding(16)
                }
            }
        }
        .frame(width: 580, height: 540)
    }

    private var header: some View {
        HStack(spacing: 10) {
            VStack(alignment: .leading, spacing: 2) {
                Text("LaboLabo 変更履歴").font(.headline)
                Text("v\(version) (\(build))")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            Spacer()
            Link(destination: Self.releasesURL) {
                Label("GitHub", systemImage: "arrow.up.forward.square")
            }
            .font(.caption)
            Button("閉じる") { dismiss() }
        }
        .padding(12)
    }
}

// MARK: - 軽量 Markdown（行ベース）

struct ChangelogLine: Identifiable {
    enum Kind { case title, version, section, bullet, text, blank }
    let id: Int
    let kind: Kind
    let text: String
}

enum ChangelogParser {
    static func parse(_ markdown: String) -> [ChangelogLine] {
        markdown.components(separatedBy: "\n").enumerated().map { index, raw in
            let trimmed = raw.trimmingCharacters(in: .whitespaces)
            if trimmed.isEmpty {
                return ChangelogLine(id: index, kind: .blank, text: "")
            } else if trimmed.hasPrefix("### ") {
                return ChangelogLine(id: index, kind: .section, text: String(trimmed.dropFirst(4)))
            } else if trimmed.hasPrefix("## ") {
                return ChangelogLine(id: index, kind: .version, text: String(trimmed.dropFirst(3)))
            } else if trimmed.hasPrefix("# ") {
                return ChangelogLine(id: index, kind: .title, text: String(trimmed.dropFirst(2)))
            } else if trimmed.hasPrefix("* ") || trimmed.hasPrefix("- ") {
                return ChangelogLine(id: index, kind: .bullet, text: String(trimmed.dropFirst(2)))
            } else {
                return ChangelogLine(id: index, kind: .text, text: trimmed)
            }
        }
    }
}

struct ChangelogLineView: View {
    let line: ChangelogLine

    var body: some View {
        switch line.kind {
        case .version:
            Text(inline(line.text))
                .font(.title3.weight(.bold))
                .padding(.top, 12)
        case .section:
            Text(line.text)
                .font(.headline)
                .foregroundStyle(.secondary)
                .padding(.top, 4)
        case .bullet:
            HStack(alignment: .top, spacing: 6) {
                Text("•").foregroundStyle(.secondary)
                Text(inline(line.text))
            }
        case .blank:
            Color.clear.frame(height: 3)
        case .title, .text:
            Text(inline(line.text))
                .foregroundStyle(.secondary)
        }
    }

    /// リンクや強調などのインライン Markdown だけ解釈する。
    private func inline(_ string: String) -> AttributedString {
        (try? AttributedString(markdown: string)) ?? AttributedString(string)
    }
}
