import XCTest
@testable import LaboLaboEngine

final class TranscriptUsageTests: XCTestCase {

    // MARK: - parse（assistant 行の usage を合算）

    func testSumsAcrossAssistantTurns() {
        let jsonl = """
        {"type":"user","message":{"role":"user"}}
        {"type":"assistant","message":{"model":"claude-opus-4-8","usage":{"input_tokens":100,"output_tokens":20,"cache_creation_input_tokens":10,"cache_read_input_tokens":5}}}
        {"type":"assistant","message":{"model":"claude-opus-4-8","usage":{"input_tokens":200,"output_tokens":30,"cache_creation_input_tokens":0,"cache_read_input_tokens":50}}}
        """
        let u = TranscriptUsage.parse(jsonl: jsonl)
        XCTAssertEqual(u.inputTokens, 300)
        XCTAssertEqual(u.outputTokens, 50)
        XCTAssertEqual(u.cacheCreationTokens, 10)
        XCTAssertEqual(u.cacheReadTokens, 55)
        XCTAssertEqual(u.turns, 2)
        XCTAssertEqual(u.model, "claude-opus-4-8")
        XCTAssertEqual(u.totalTokens, 415)
    }

    func testIgnoresNonAssistantAndMalformedLines() {
        let jsonl = """
        not-json
        {"type":"user","message":{"usage":{"input_tokens":999}}}
        {"type":"assistant","message":{"model":"claude-sonnet-5","usage":{"input_tokens":10,"output_tokens":2}}}
        """
        let u = TranscriptUsage.parse(jsonl: jsonl)
        XCTAssertEqual(u.turns, 1)
        XCTAssertEqual(u.inputTokens, 10)
        XCTAssertEqual(u.outputTokens, 2)
        XCTAssertEqual(u.cacheCreationTokens, 0)
        XCTAssertEqual(u.model, "claude-sonnet-5")
    }

    func testEmptyTranscriptIsEmpty() {
        XCTAssertTrue(TranscriptUsage.parse(jsonl: "").isEmpty)
        XCTAssertTrue(TranscriptUsage.parse(jsonl: "{\"type\":\"user\"}").isEmpty)
    }

    // MARK: - コスト推定

    func testOpusCostEstimate() {
        var u = AgentUsage()
        u.inputTokens = 1_000_000
        u.outputTokens = 1_000_000
        u.model = "claude-opus-4-8"
        // input $15 + output $75 = $90（キャッシュ 0）。
        XCTAssertEqual(try XCTUnwrap(u.estimatedCostUSD), 90, accuracy: 0.0001)
    }

    func testCacheTokensContributeToCost() {
        var u = AgentUsage()
        u.cacheReadTokens = 1_000_000
        u.model = "claude-sonnet-5"
        // sonnet cacheRead $0.30 / MTok。
        XCTAssertEqual(try XCTUnwrap(u.estimatedCostUSD), 0.30, accuracy: 0.0001)
    }

    func testUnknownModelHasNoCost() {
        var u = AgentUsage()
        u.inputTokens = 1_000
        u.model = "some-unknown-model"
        XCTAssertNil(u.estimatedCostUSD)

        u.model = nil
        XCTAssertNil(u.estimatedCostUSD)
    }

    func testPricingFamilyMatch() {
        XCTAssertNotNil(ModelPricing.forModel("claude-opus-4-8"))
        XCTAssertNotNil(ModelPricing.forModel("claude-sonnet-5"))
        XCTAssertNotNil(ModelPricing.forModel("claude-haiku-4-5-20251001"))
        XCTAssertNil(ModelPricing.forModel("gpt-4"))
        XCTAssertNil(ModelPricing.forModel(nil))
    }
}
