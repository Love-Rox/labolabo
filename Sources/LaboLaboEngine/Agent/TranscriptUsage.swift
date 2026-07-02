import Foundation

/// エージェント transcript(JSONL) から集計した使用量。stream-json が使えないための
/// **best-effort な推定**（UI では「推定」と明示し、コストで機能を gate しない）。
public struct AgentUsage: Sendable, Equatable {
    public var inputTokens: Int
    public var outputTokens: Int
    public var cacheCreationTokens: Int
    public var cacheReadTokens: Int
    /// assistant 応答（ターン）数。
    public var turns: Int
    /// 直近に観測したモデル ID（"claude-opus-4-8" 等）。コスト推定・表示に使う。
    public var model: String?

    public init(
        inputTokens: Int = 0,
        outputTokens: Int = 0,
        cacheCreationTokens: Int = 0,
        cacheReadTokens: Int = 0,
        turns: Int = 0,
        model: String? = nil
    ) {
        self.inputTokens = inputTokens
        self.outputTokens = outputTokens
        self.cacheCreationTokens = cacheCreationTokens
        self.cacheReadTokens = cacheReadTokens
        self.turns = turns
        self.model = model
    }

    public var totalTokens: Int {
        inputTokens + outputTokens + cacheCreationTokens + cacheReadTokens
    }

    public var isEmpty: Bool { turns == 0 && totalTokens == 0 }

    /// 推定コスト(USD)。モデル価格が分かる場合のみ（未知は nil＝トークンのみ表示）。
    public var estimatedCostUSD: Double? {
        guard let pricing = ModelPricing.forModel(model) else { return nil }
        return pricing.cost(
            input: inputTokens, output: outputTokens,
            cacheWrite: cacheCreationTokens, cacheRead: cacheReadTokens
        )
    }
}

/// 100 万トークンあたりの USD 価格（**推定・変動しうる**）。
public struct ModelPricing: Sendable, Equatable {
    public let input: Double
    public let output: Double
    public let cacheWrite: Double
    public let cacheRead: Double

    public init(input: Double, output: Double, cacheWrite: Double, cacheRead: Double) {
        self.input = input
        self.output = output
        self.cacheWrite = cacheWrite
        self.cacheRead = cacheRead
    }

    public func cost(input i: Int, output o: Int, cacheWrite cw: Int, cacheRead cr: Int) -> Double {
        (Double(i) * input + Double(o) * output + Double(cw) * cacheWrite + Double(cr) * cacheRead) / 1_000_000
    }

    /// モデル ID からファミリを推定して概算価格を返す（未知は nil）。数値は目安。
    public static func forModel(_ model: String?) -> ModelPricing? {
        guard let m = model?.lowercased() else { return nil }
        if m.contains("opus") {
            return ModelPricing(input: 15, output: 75, cacheWrite: 18.75, cacheRead: 1.5)
        }
        if m.contains("sonnet") {
            return ModelPricing(input: 3, output: 15, cacheWrite: 3.75, cacheRead: 0.3)
        }
        if m.contains("haiku") {
            return ModelPricing(input: 0.8, output: 4, cacheWrite: 1.0, cacheRead: 0.08)
        }
        return nil
    }
}

/// Claude Code の transcript(JSONL) を読み、assistant 行の usage を合算する。
public enum TranscriptUsage {

    /// JSONL 文字列から集計する。1 行 = 1 JSON。`type == "assistant"` の
    /// `message.usage` を合算し、`message.model` を最新として採用する。
    public static func parse(jsonl: String) -> AgentUsage {
        var usage = AgentUsage()
        for line in jsonl.split(whereSeparator: \.isNewline) {
            guard let data = line.data(using: .utf8),
                  let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
                  (object["type"] as? String) == "assistant",
                  let message = object["message"] as? [String: Any],
                  let u = message["usage"] as? [String: Any] else { continue }
            usage.inputTokens += (u["input_tokens"] as? Int) ?? 0
            usage.outputTokens += (u["output_tokens"] as? Int) ?? 0
            usage.cacheCreationTokens += (u["cache_creation_input_tokens"] as? Int) ?? 0
            usage.cacheReadTokens += (u["cache_read_input_tokens"] as? Int) ?? 0
            usage.turns += 1
            if let model = message["model"] as? String, !model.isEmpty { usage.model = model }
        }
        return usage
    }

    /// ファイルから読み取って集計する（読めない/空なら nil）。
    public static func read(path: String) -> AgentUsage? {
        guard let text = try? String(contentsOfFile: path, encoding: .utf8) else { return nil }
        let usage = parse(jsonl: text)
        return usage.isEmpty ? nil : usage
    }
}
