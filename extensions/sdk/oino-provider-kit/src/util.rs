//! Small pi-ai helpers: `shortHash`, JavaScript string lengths, cost calculation, thinking-level
//! clamping and prompt-cache key clamping.

use oino_ai::{MapEntry, Model, ThinkingLevel, Usage};
use serde_json::Value;

/// pi `shortHash` (`utils/hash.js`) over UTF-16 code units.
#[must_use]
pub fn short_hash(input: &str) -> String {
    let mut h1: u32 = 0xdead_beef;
    let mut h2: u32 = 0x41c6_ce57;
    for unit in input.encode_utf16() {
        let ch = u32::from(unit);
        h1 = (h1 ^ ch).wrapping_mul(2_654_435_761);
        h2 = (h2 ^ ch).wrapping_mul(1_597_334_677);
    }
    h1 = (h1 ^ (h1 >> 16)).wrapping_mul(2_246_822_507)
        ^ (h2 ^ (h2 >> 13)).wrapping_mul(3_266_489_909);
    h2 = (h2 ^ (h2 >> 16)).wrapping_mul(2_246_822_507)
        ^ (h1 ^ (h1 >> 13)).wrapping_mul(3_266_489_909);
    format!("{}{}", base36(h2), base36(h1))
}

fn base36(mut value: u32) -> String {
    const DIGITS: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    if value == 0 {
        return "0".to_owned();
    }
    let mut out = Vec::new();
    while value > 0 {
        out.push(char::from(DIGITS[(value % 36) as usize]));
        value /= 36;
    }
    out.iter().rev().collect()
}

/// JavaScript `string.length` (UTF-16 code units).
#[must_use]
pub fn js_len(text: &str) -> usize {
    text.encode_utf16().count()
}

/// pi `calculateCost` (`models.js`): fills `usage.cost` from the model's (tiered) prices.
#[allow(
    clippy::cast_precision_loss,
    reason = "token counts are far below 2^52, as in JavaScript numbers"
)]
pub fn calculate_cost(model: &Model, usage: &mut Usage) {
    let input_tokens = usage.input + usage.cache_read + usage.cache_write;
    let cost = &model.cost;
    let (mut input, mut output, mut read, mut write) =
        (cost.input, cost.output, cost.cache_read, cost.cache_write);
    let mut threshold: Option<u64> = None;
    for tier in cost.tiers.iter().flatten() {
        if input_tokens > tier.input_tokens_above
            && threshold.is_none_or(|t| tier.input_tokens_above > t)
        {
            (input, output, read, write) =
                (tier.input, tier.output, tier.cache_read, tier.cache_write);
            threshold = Some(tier.input_tokens_above);
        }
    }
    let long_write = usage.cache_write_1h.unwrap_or(0);
    let short_write = usage.cache_write.saturating_sub(long_write);
    usage.cost.input = (input / 1_000_000.0) * usage.input as f64;
    usage.cost.output = (output / 1_000_000.0) * usage.output as f64;
    usage.cost.cache_read = (read / 1_000_000.0) * usage.cache_read as f64;
    usage.cost.cache_write =
        (write * short_write as f64 + input * 2.0 * long_write as f64) / 1_000_000.0;
    usage.cost.total =
        usage.cost.input + usage.cost.output + usage.cost.cache_read + usage.cost.cache_write;
}

/// pi `clampThinkingLevel(model, level)`.
#[must_use]
pub fn clamp_thinking_level(model: &Model, level: ThinkingLevel) -> ThinkingLevel {
    level.clamp(&model.available_thinking_levels())
}

/// `model.thinkingLevelMap?.[level]`.
#[must_use]
pub fn mapped_level(model: &Model, level: ThinkingLevel) -> MapEntry<'_> {
    model
        .thinking_level_map
        .as_ref()
        .map_or(MapEntry::Absent, |map| map.get(level))
}

/// `model.thinkingLevelMap?.[level] ?? level` as JSON (`null` stays `null` only when mapped).
#[must_use]
pub fn mapped_or_level(model: &Model, level: ThinkingLevel) -> Value {
    match mapped_level(model, level) {
        MapEntry::Value(value) => Value::String(value.to_owned()),
        MapEntry::Null | MapEntry::Absent => Value::String(level.as_str().to_owned()),
    }
}

/// `model.thinkingLevelMap?.off !== null`.
#[must_use]
pub fn off_not_null(model: &Model) -> bool {
    mapped_level(model, ThinkingLevel::Off) != MapEntry::Null
}

/// pi `clampOpenAIPromptCacheKey`: at most 64 code points.
#[must_use]
pub fn clamp_prompt_cache_key(key: &str) -> String {
    key.chars().take(64).collect()
}

/// `model.compat?.[key]` as a bool.
#[must_use]
pub fn compat_bool(model: &Model, key: &str) -> Option<bool> {
    compat_value(model, key).and_then(Value::as_bool)
}

/// `model.compat?.[key]` as a string.
#[must_use]
pub fn compat_str<'a>(model: &'a Model, key: &str) -> Option<&'a str> {
    compat_value(model, key).and_then(Value::as_str)
}

/// `model.compat?.[key]`.
#[must_use]
pub fn compat_value<'a>(model: &'a Model, key: &str) -> Option<&'a Value> {
    model.compat.as_ref().and_then(|compat| compat.get(key))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_hash_matches_pi() {
        // Values computed with pi-ai 1.4.0 `shortHash`.
        assert_eq!(short_hash(""), "k4n83c7h0j2b");
        assert_eq!(short_hash("abc"), "y0biex7f9bbh");
        assert_eq!(short_hash("é😀"), "102l7zrk951la");
    }
}
