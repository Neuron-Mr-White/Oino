//! Scalar pi-ai types: timestamps, thinking levels, stop reasons, usage.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::json::js_f64;

/// Unix epoch milliseconds (pi `number` timestamps on messages).
pub type TimestampMs = i64;

/// Reasoning effort requested from a model (pi `ThinkingLevel`). Wire values are lowercase
/// (`"xhigh"`, not `"x_high"`).
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum ThinkingLevel {
    /// No reasoning.
    #[default]
    Off,
    /// Minimal reasoning.
    Minimal,
    /// Low reasoning.
    Low,
    /// Medium reasoning.
    Medium,
    /// High reasoning.
    High,
    /// Extra-high reasoning (only when the model maps it).
    Xhigh,
    /// Maximum reasoning (only when the model maps it).
    Max,
}

impl ThinkingLevel {
    /// Every level in pi order (`EXTENDED_THINKING_LEVELS`).
    pub const ALL: [Self; 7] = [
        Self::Off,
        Self::Minimal,
        Self::Low,
        Self::Medium,
        Self::High,
        Self::Xhigh,
        Self::Max,
    ];

    /// Wire string (`"off"`, …, `"xhigh"`, `"max"`).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Minimal => "minimal",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Xhigh => "xhigh",
            Self::Max => "max",
        }
    }

    /// pi `clampThinkingLevel`: the requested level if available, else the nearest *higher*
    /// available level, else the nearest lower one, else the first available (or `Off`).
    #[must_use]
    pub fn clamp(self, available: &[Self]) -> Self {
        if available.contains(&self) {
            return self;
        }
        let requested = self as usize;
        Self::ALL[requested..]
            .iter()
            .chain(Self::ALL[..requested].iter().rev())
            .copied()
            .find(|level| available.contains(level))
            .unwrap_or_else(|| available.first().copied().unwrap_or(Self::Off))
    }
}

impl fmt::Display for ThinkingLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Error returned when a string is not a pi thinking level.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("unknown thinking level: {0}")]
pub struct UnknownThinkingLevel(pub String);

impl FromStr for ThinkingLevel {
    type Err = UnknownThinkingLevel;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|level| level.as_str() == s)
            .ok_or_else(|| UnknownThinkingLevel(s.to_owned()))
    }
}

/// Why an assistant message stopped (pi `StopReason`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StopReason {
    /// Streaming partial (pi `message_start` assistants).
    Pending,
    /// Normal end of turn.
    Stop,
    /// Output token limit reached.
    Length,
    /// The model requested tool calls.
    ToolUse,
    /// Provider or transport error.
    Error,
    /// Aborted by the user.
    Aborted,
    /// Deferred (batch) response; see `AssistantMessage::deferred`.
    Deferred,
}

/// Input modality a model accepts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InputModality {
    /// Text input.
    Text,
    /// Image input.
    Image,
}

/// Prompt-cache retention requested from providers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CacheRetention {
    /// No caching.
    None,
    /// Short (default) retention.
    Short,
    /// Long retention (e.g. Anthropic 1h).
    Long,
}

/// Cost breakdown in USD (pi `Usage.cost`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageCost {
    /// Input token cost.
    #[serde(with = "js_f64")]
    pub input: f64,
    /// Output token cost.
    #[serde(with = "js_f64")]
    pub output: f64,
    /// Cache read cost.
    #[serde(with = "js_f64")]
    pub cache_read: f64,
    /// Cache write cost.
    #[serde(with = "js_f64")]
    pub cache_write: f64,
    /// Sum of all buckets.
    #[serde(with = "js_f64")]
    pub total: f64,
}

/// Token usage and cost (pi `Usage`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Usage {
    /// Uncached input tokens.
    pub input: u64,
    /// Output tokens.
    pub output: u64,
    /// Cache read tokens.
    pub cache_read: u64,
    /// Cache write tokens (all retentions).
    pub cache_write: u64,
    /// Portion of `cache_write` written with 1h retention.
    #[serde(
        rename = "cacheWrite1h",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub cache_write_1h: Option<u64>,
    /// Reasoning tokens; `None` means the provider did not report them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<u64>,
    /// Total tokens.
    pub total_tokens: u64,
    /// Cost breakdown (required).
    #[serde(default)]
    pub cost: UsageCost,
}

impl Usage {
    /// Adds `other` into `self` (token counts and cost buckets).
    pub fn accumulate(&mut self, other: &Self) {
        self.input += other.input;
        self.output += other.output;
        self.cache_read += other.cache_read;
        self.cache_write += other.cache_write;
        self.cache_write_1h = add_opt(self.cache_write_1h, other.cache_write_1h);
        self.reasoning = add_opt(self.reasoning, other.reasoning);
        self.total_tokens += other.total_tokens;
        self.cost.input += other.cost.input;
        self.cost.output += other.cost.output;
        self.cost.cache_read += other.cost.cache_read;
        self.cost.cache_write += other.cost.cache_write;
        self.cost.total += other.cost.total;
    }
}

fn add_opt(a: Option<u64>, b: Option<u64>) -> Option<u64> {
    match (a, b) {
        (None, None) => None,
        (a, b) => Some(a.unwrap_or(0) + b.unwrap_or(0)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn thinking_level_wire_values() {
        assert_eq!(
            serde_json::to_value(ThinkingLevel::Xhigh).ok(),
            Some(json!("xhigh"))
        );
        assert_eq!("max".parse::<ThinkingLevel>(), Ok(ThinkingLevel::Max));
        assert!("x_high".parse::<ThinkingLevel>().is_err());
    }

    #[test]
    fn clamp_prefers_higher_then_lower() {
        use ThinkingLevel::{High, Low, Max, Medium, Minimal, Off, Xhigh};
        let available = [Off, Minimal, Low, Medium, High];
        assert_eq!(Xhigh.clamp(&available), High);
        assert_eq!(Max.clamp(&available), High);
        assert_eq!(Low.clamp(&available), Low);
        assert_eq!(Minimal.clamp(&[Off, Medium]), Medium);
        assert_eq!(High.clamp(&[Off]), Off);
        assert_eq!(High.clamp(&[]), Off);
    }

    #[test]
    fn usage_round_trips_pi_shape() {
        let raw = json!({"input":0,"output":0,"cacheRead":50000,"cacheWrite":0,"totalTokens":50000,
            "cost":{"input":0,"output":0,"cacheRead":0.015,"cacheWrite":0,"total":0.015}});
        let usage: Usage = serde_json::from_value(raw.clone()).unwrap_or_default();
        assert_eq!(usage.cache_read, 50_000);
        assert_eq!(serde_json::to_value(usage).ok(), Some(raw));
        let text = serde_json::to_string(&usage).unwrap_or_default();
        assert!(text.contains("\"input\":0,"), "{text}");
    }
}
