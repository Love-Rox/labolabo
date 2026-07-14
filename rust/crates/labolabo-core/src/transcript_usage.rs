//! Faithful port of `Sources/LaboLaboEngine/Agent/TranscriptUsage.swift`.
//!
//! エージェント transcript(JSONL) から集計した使用量。stream-json が使えないための
//! **best-effort な推定**（UI では「推定」と明示し、コストで機能を gate しない）。

use std::path::Path;

use serde_json::Value;

/// Usage aggregated from an agent transcript (JSONL).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AgentUsage {
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_creation_tokens: i64,
    pub cache_read_tokens: i64,
    /// assistant 応答（ターン）数。
    pub turns: i64,
    /// 直近に観測したモデル ID（"claude-opus-4-8" 等）。コスト推定・表示に使う。
    pub model: Option<String>,
}

impl AgentUsage {
    pub fn total_tokens(&self) -> i64 {
        self.input_tokens + self.output_tokens + self.cache_creation_tokens + self.cache_read_tokens
    }

    pub fn is_empty(&self) -> bool {
        self.turns == 0 && self.total_tokens() == 0
    }

    /// 推定コスト(USD)。モデル価格が分かる場合のみ（未知は None＝トークンのみ表示）。
    pub fn estimated_cost_usd(&self) -> Option<f64> {
        let pricing = ModelPricing::for_model(self.model.as_deref())?;
        Some(pricing.cost(
            self.input_tokens,
            self.output_tokens,
            self.cache_creation_tokens,
            self.cache_read_tokens,
        ))
    }
}

/// 100 万トークンあたりの USD 価格（**推定・変動しうる**）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModelPricing {
    pub input: f64,
    pub output: f64,
    pub cache_write: f64,
    pub cache_read: f64,
}

impl ModelPricing {
    pub fn cost(&self, input: i64, output: i64, cache_write: i64, cache_read: i64) -> f64 {
        (input as f64 * self.input
            + output as f64 * self.output
            + cache_write as f64 * self.cache_write
            + cache_read as f64 * self.cache_read)
            / 1_000_000.0
    }

    /// モデル ID からファミリを推定して概算価格を返す（未知は None）。数値は目安。
    pub fn for_model(model: Option<&str>) -> Option<ModelPricing> {
        let m = model?.to_lowercase();
        if m.contains("opus") {
            return Some(ModelPricing {
                input: 15.0,
                output: 75.0,
                cache_write: 18.75,
                cache_read: 1.5,
            });
        }
        if m.contains("sonnet") {
            return Some(ModelPricing {
                input: 3.0,
                output: 15.0,
                cache_write: 3.75,
                cache_read: 0.3,
            });
        }
        if m.contains("haiku") {
            return Some(ModelPricing {
                input: 0.8,
                output: 4.0,
                cache_write: 1.0,
                cache_read: 0.08,
            });
        }
        None
    }
}

/// Mirrors Foundation's `JSONSerialization` + `as? Int` bridging for a JSON
/// object field, empirically verified (not assumed) against a real
/// `JSONSerialization` round trip on this port's target Swift toolchain:
///
/// - A JSON integer literal (`100`) bridges to `Int` — obviously.
/// - A JSON **float literal with no fractional part** (`100.0`) *also*
///   bridges to `Int` (`Optional(100)`, not `nil`) — `JSONSerialization`
///   backs both with the same `NSNumber`, and the Swift `as? Int` downcast
///   succeeds through it whenever the underlying value is a whole number
///   representable as `Int`.
/// - A JSON **boolean** also bridges to `Int` (`true` -> `Optional(1)`,
///   `false` -> `Optional(0)`) — another `NSNumber` bridging quirk.
/// - A fractional float (`100.5`), a string, `null`, an array, or an object
///   all fail the cast (`nil`).
/// - An out-of-range or non-finite float (too large to fit `Int`, `NaN`,
///   `inf`) also fails.
///
/// This means `(u["input_tokens"] as? Int) ?? 0` in the Swift source is
/// **not** simply "parse a JSON integer" — it silently accepts whole-number
/// floats and booleans too. `serde_json`'s own `Number::as_i64()` does
/// *not* replicate this (it returns `None` for any value that was parsed
/// from a JSON float literal, even a whole one), so this helper reimplements
/// the bridging by hand. See the `quirk_*` tests below and the
/// `whole_number_float_bridges_to_int` / `bool_bridges_to_int_quirk` golden
/// fixtures for concrete cases.
fn as_int(value: Option<&Value>) -> Option<i64> {
    match value? {
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Some(i)
            } else if let Some(u) = n.as_u64() {
                i64::try_from(u).ok()
            } else {
                let f = n.as_f64()?;
                if f.is_finite()
                    && f.fract() == 0.0
                    && (i64::MIN as f64..=i64::MAX as f64).contains(&f)
                {
                    Some(f as i64)
                } else {
                    None
                }
            }
        }
        Value::Bool(b) => Some(i64::from(*b)),
        _ => None,
    }
}

/// JSONL 文字列から集計する。1 行 = 1 JSON。`type == "assistant"` の
/// `message.usage` を合算し、`message.model` を最新として採用する。
///
/// Lines are split on plain `\n` (Swift splits on any `Character` satisfying
/// `isNewline`, which also treats a lone `\r` or the Unicode line separators
/// as boundaries). Real Claude Code transcripts are `\n`-terminated JSONL,
/// so this simplification does not change behavior in practice: a `\r\n`
/// line ending leaves a harmless trailing `\r`, which JSON parsers accept as
/// trailing whitespace; a lone `\r` (old-Mac style, never seen in practice
/// here) would instead merge two records onto one line and fail to parse
/// either — a documented, deliberately accepted gap, not a silent behavior
/// change for real input.
pub fn parse(jsonl: &str) -> AgentUsage {
    let mut usage = AgentUsage::default();
    for line in jsonl.split('\n') {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let Some(object) = value.as_object() else {
            continue;
        };
        if object.get("type").and_then(Value::as_str) != Some("assistant") {
            continue;
        }
        let Some(message) = object.get("message").and_then(Value::as_object) else {
            continue;
        };
        let Some(u) = message.get("usage").and_then(Value::as_object) else {
            continue;
        };
        usage.input_tokens += as_int(u.get("input_tokens")).unwrap_or(0);
        usage.output_tokens += as_int(u.get("output_tokens")).unwrap_or(0);
        usage.cache_creation_tokens += as_int(u.get("cache_creation_input_tokens")).unwrap_or(0);
        usage.cache_read_tokens += as_int(u.get("cache_read_input_tokens")).unwrap_or(0);
        usage.turns += 1;
        if let Some(model) = message.get("model").and_then(Value::as_str) {
            if !model.is_empty() {
                usage.model = Some(model.to_string());
            }
        }
    }
    usage
}

/// ファイルから読み取って集計する（読めない/空なら None）。
pub fn read(path: &Path) -> Option<AgentUsage> {
    let text = std::fs::read_to_string(path).ok()?;
    let usage = parse(&text);
    if usage.is_empty() {
        None
    } else {
        Some(usage)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Ported 1:1 from Tests/LaboLaboEngineTests/TranscriptUsageTests.swift.

    // MARK: - parse（assistant 行の usage を合算）

    #[test]
    fn sums_across_assistant_turns() {
        let jsonl = concat!(
            r#"{"type":"user","message":{"role":"user"}}"#,
            "\n",
            r#"{"type":"assistant","message":{"model":"claude-opus-4-8","usage":{"input_tokens":100,"output_tokens":20,"cache_creation_input_tokens":10,"cache_read_input_tokens":5}}}"#,
            "\n",
            r#"{"type":"assistant","message":{"model":"claude-opus-4-8","usage":{"input_tokens":200,"output_tokens":30,"cache_creation_input_tokens":0,"cache_read_input_tokens":50}}}"#,
        );
        let u = parse(jsonl);
        assert_eq!(u.input_tokens, 300);
        assert_eq!(u.output_tokens, 50);
        assert_eq!(u.cache_creation_tokens, 10);
        assert_eq!(u.cache_read_tokens, 55);
        assert_eq!(u.turns, 2);
        assert_eq!(u.model.as_deref(), Some("claude-opus-4-8"));
        assert_eq!(u.total_tokens(), 415);
    }

    #[test]
    fn ignores_non_assistant_and_malformed_lines() {
        let jsonl = concat!(
            "not-json\n",
            r#"{"type":"user","message":{"usage":{"input_tokens":999}}}"#,
            "\n",
            r#"{"type":"assistant","message":{"model":"claude-sonnet-5","usage":{"input_tokens":10,"output_tokens":2}}}"#,
        );
        let u = parse(jsonl);
        assert_eq!(u.turns, 1);
        assert_eq!(u.input_tokens, 10);
        assert_eq!(u.output_tokens, 2);
        assert_eq!(u.cache_creation_tokens, 0);
        assert_eq!(u.model.as_deref(), Some("claude-sonnet-5"));
    }

    #[test]
    fn empty_transcript_is_empty() {
        assert!(parse("").is_empty());
        assert!(parse(r#"{"type":"user"}"#).is_empty());
    }

    // MARK: - コスト推定

    #[test]
    fn opus_cost_estimate() {
        let u = AgentUsage {
            input_tokens: 1_000_000,
            output_tokens: 1_000_000,
            model: Some("claude-opus-4-8".to_string()),
            ..Default::default()
        };
        // input $15 + output $75 = $90（キャッシュ 0）。
        let cost = u.estimated_cost_usd().expect("cost");
        assert!((cost - 90.0).abs() < 0.0001);
    }

    #[test]
    fn cache_tokens_contribute_to_cost() {
        let u = AgentUsage {
            cache_read_tokens: 1_000_000,
            model: Some("claude-sonnet-5".to_string()),
            ..Default::default()
        };
        // sonnet cacheRead $0.30 / MTok。
        let cost = u.estimated_cost_usd().expect("cost");
        assert!((cost - 0.30).abs() < 0.0001);
    }

    #[test]
    fn unknown_model_has_no_cost() {
        let mut u = AgentUsage {
            input_tokens: 1_000,
            model: Some("some-unknown-model".to_string()),
            ..Default::default()
        };
        assert_eq!(u.estimated_cost_usd(), None);

        u.model = None;
        assert_eq!(u.estimated_cost_usd(), None);
    }

    #[test]
    fn pricing_family_match() {
        assert!(ModelPricing::for_model(Some("claude-opus-4-8")).is_some());
        assert!(ModelPricing::for_model(Some("claude-sonnet-5")).is_some());
        assert!(ModelPricing::for_model(Some("claude-haiku-4-5-20251001")).is_some());
        assert!(ModelPricing::for_model(Some("gpt-4")).is_none());
        assert!(ModelPricing::for_model(None).is_none());
    }

    // MARK: - `as_int` NSNumber-bridging quirk (see doc comment above).
    // Not part of the ported Swift suite (which never exercised these
    // shapes directly) but documents empirically-verified Foundation
    // bridging behavior this port must replicate; also covered end-to-end
    // by the `whole_number_float_bridges_to_int` / `bool_bridges_to_int_quirk`
    // / `fractional_float_and_string_fall_back_to_zero` golden fixtures.

    #[test]
    fn quirk_whole_number_float_bridges_to_int() {
        let jsonl = r#"{"type":"assistant","message":{"model":"claude-opus-4-8","usage":{"input_tokens":100.0}}}"#;
        assert_eq!(parse(jsonl).input_tokens, 100);
    }

    #[test]
    fn quirk_bool_bridges_to_int() {
        let jsonl = r#"{"type":"assistant","message":{"model":"claude-opus-4-8","usage":{"input_tokens":true,"output_tokens":false}}}"#;
        let u = parse(jsonl);
        assert_eq!(u.input_tokens, 1);
        assert_eq!(u.output_tokens, 0);
    }

    #[test]
    fn quirk_fractional_float_and_string_fall_back_to_zero() {
        let jsonl = r#"{"type":"assistant","message":{"model":"claude-opus-4-8","usage":{"input_tokens":100.5,"output_tokens":"5"}}}"#;
        let u = parse(jsonl);
        assert_eq!(u.input_tokens, 0);
        assert_eq!(u.output_tokens, 0);
    }

    #[test]
    fn empty_model_string_does_not_overwrite() {
        // Swift: `if let model = ..., !model.isEmpty { usage.model = model }`
        // -- an empty-string model on a later turn must not clobber a
        // previously observed non-empty model.
        let jsonl = concat!(
            r#"{"type":"assistant","message":{"model":"claude-opus-4-8","usage":{"input_tokens":1}}}"#,
            "\n",
            r#"{"type":"assistant","message":{"model":"","usage":{"input_tokens":1}}}"#,
        );
        assert_eq!(parse(jsonl).model.as_deref(), Some("claude-opus-4-8"));
    }
}
