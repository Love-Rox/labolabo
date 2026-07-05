import SwiftUI
import LaboLaboEngine

/// エージェントの使用量（推定）を表示するポップオーバー。
/// stream-json 不在のため transcript から集計した概算で、コストで機能を gate しない。
struct UsagePopover: View {
    let usage: AgentUsage

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 6) {
                Image(systemName: "chart.bar.doc.horizontal")
                Text("使用量（推定）").font(.headline)
            }

            if let model = usage.model {
                HStack {
                    Text("モデル").foregroundStyle(.secondary)
                    Spacer()
                    Text(model).monospaced()
                }
                .font(.caption)
            }

            Divider()

            VStack(alignment: .leading, spacing: 4) {
                tokenRow(String(localized: "入力"), usage.inputTokens)
                tokenRow(String(localized: "出力"), usage.outputTokens)
                tokenRow(String(localized: "キャッシュ書込"), usage.cacheCreationTokens)
                tokenRow(String(localized: "キャッシュ読取"), usage.cacheReadTokens)
                Divider()
                tokenRow(String(localized: "合計トークン"), usage.totalTokens, bold: true)
            }
            .font(.caption.monospacedDigit())

            Divider()

            HStack {
                Text("推定コスト").font(.caption)
                Spacer()
                Text(costText).font(.caption.weight(.medium).monospacedDigit())
            }

            Text("\(usage.turns) ターン・transcript から算出した概算です。実際の課金と一致しない場合があります。")
                .font(.caption2)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
        }
        .padding(14)
        .frame(width: 288)
    }

    private func tokenRow(_ label: String, _ value: Int, bold: Bool = false) -> some View {
        HStack {
            Text(label).foregroundStyle(.secondary)
            Spacer()
            Text(value.formatted()).fontWeight(bold ? .semibold : .regular)
        }
    }

    private var costText: String {
        guard let cost = usage.estimatedCostUSD else { return String(localized: "価格未知（トークンのみ）") }
        return String(format: "$%.4f", cost)
    }
}
