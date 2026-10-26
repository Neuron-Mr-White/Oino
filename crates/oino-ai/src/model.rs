//! pi-ai `Model` and related catalogue types.

use std::fmt;

use indexmap::IndexMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::json::{JsonObject, JsonValue, js_f64, js_f64_opt};
use crate::primitives::{InputModality, ThinkingLevel};

/// Chat model (pi `Model<Api>`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Model {
    /// Model id within the provider.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Wire API.
    pub api: String,
    /// Provider id.
    pub provider: String,
    /// Base URL.
    #[serde(default)]
    pub base_url: String,
    /// Accepted input modalities.
    #[serde(default)]
    pub input: Vec<InputModality>,
    /// Request size / image limits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_limits: Option<InputLimits>,
    /// Pricing per million tokens.
    #[serde(default)]
    pub cost: ModelCost,
    /// Extra request headers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub headers: Option<IndexMap<String, String>>,
    /// `"chat"` when present.
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<ModelKindChat>,
    /// Whether the model supports reasoning.
    #[serde(default)]
    pub reasoning: bool,
    /// Mapping of pi levels to provider effort strings (`null` = unsupported).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking_level_map: Option<ThinkingLevelMap>,
    /// Prompt cache pricing/retention hints.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_cache: Option<PromptCache>,
    /// Context window in tokens.
    #[serde(default)]
    pub context_window: u64,
    /// Maximum output tokens.
    #[serde(default)]
    pub max_tokens: u64,
    /// Default sampling parameters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sampling_params: Option<JsonObject>,
    /// Sampling parameters per thinking level.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sampling_params_by_thinking_level: Option<IndexMap<ThinkingLevel, JsonObject>>,
    /// Provider compatibility flags (opaque to core).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compat: Option<JsonValue>,
}

/// The literal `"chat"` model type.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ModelKindChat {
    /// `"chat"`.
    #[serde(rename = "chat")]
    Chat,
}

/// `"provider/id"` model key.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ModelKey {
    /// Provider id.
    pub provider: String,
    /// Model id.
    pub id: String,
}

impl fmt::Display for ModelKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.provider, self.id)
    }
}

impl Model {
    /// `provider/id` key.
    #[must_use]
    pub fn key(&self) -> ModelKey {
        ModelKey {
            provider: self.provider.clone(),
            id: self.id.clone(),
        }
    }

    /// pi `getSupportedThinkingLevels`: `[Off]` for non-reasoning models; otherwise every level
    /// not mapped to `null`, with `xhigh`/`max` only when explicitly mapped.
    #[must_use]
    pub fn available_thinking_levels(&self) -> Vec<ThinkingLevel> {
        if !self.reasoning {
            return vec![ThinkingLevel::Off];
        }
        ThinkingLevel::ALL
            .into_iter()
            .filter(|level| {
                let mapped = self
                    .thinking_level_map
                    .as_ref()
                    .map_or(MapEntry::Absent, |map| map.get(*level));
                match mapped {
                    MapEntry::Null => false,
                    MapEntry::Absent => !matches!(level, ThinkingLevel::Xhigh | ThinkingLevel::Max),
                    MapEntry::Value(_) => true,
                }
            })
            .collect()
    }
}

/// Pricing (USD per million tokens).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelCost {
    /// Input price.
    #[serde(with = "js_f64")]
    pub input: f64,
    /// Output price.
    #[serde(with = "js_f64")]
    pub output: f64,
    /// Cache read price.
    #[serde(with = "js_f64")]
    pub cache_read: f64,
    /// Cache write price.
    #[serde(with = "js_f64")]
    pub cache_write: f64,
    /// Tiered prices above input-token thresholds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tiers: Option<Vec<ModelCostTier>>,
}

/// One price tier.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelCostTier {
    /// Input price.
    #[serde(with = "js_f64")]
    pub input: f64,
    /// Output price.
    #[serde(with = "js_f64")]
    pub output: f64,
    /// Cache read price.
    #[serde(with = "js_f64")]
    pub cache_read: f64,
    /// Cache write price.
    #[serde(with = "js_f64")]
    pub cache_write: f64,
    /// Threshold in input tokens.
    pub input_tokens_above: u64,
}

/// Value of one [`ThinkingLevelMap`] slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MapEntry<'a> {
    /// Key absent.
    Absent,
    /// Explicit `null`: level unsupported.
    Null,
    /// Provider effort string.
    Value(&'a str),
}

/// pi `ThinkingLevelMap`: each key is absent, `null`, or a provider string.
#[allow(
    clippy::option_option,
    reason = "absent, null and string are three distinct pi states"
)]
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThinkingLevelMap {
    /// `off`.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "tri")]
    pub off: Option<Option<String>>,
    /// `minimal`.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "tri")]
    pub minimal: Option<Option<String>>,
    /// `low`.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "tri")]
    pub low: Option<Option<String>>,
    /// `medium`.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "tri")]
    pub medium: Option<Option<String>>,
    /// `high`.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "tri")]
    pub high: Option<Option<String>>,
    /// `xhigh`.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "tri")]
    pub xhigh: Option<Option<String>>,
    /// `max`.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "tri")]
    pub max: Option<Option<String>>,
}

impl ThinkingLevelMap {
    /// Slot for `level`.
    #[must_use]
    pub fn get(&self, level: ThinkingLevel) -> MapEntry<'_> {
        let slot = match level {
            ThinkingLevel::Off => &self.off,
            ThinkingLevel::Minimal => &self.minimal,
            ThinkingLevel::Low => &self.low,
            ThinkingLevel::Medium => &self.medium,
            ThinkingLevel::High => &self.high,
            ThinkingLevel::Xhigh => &self.xhigh,
            ThinkingLevel::Max => &self.max,
        };
        match slot {
            None => MapEntry::Absent,
            Some(None) => MapEntry::Null,
            Some(Some(value)) => MapEntry::Value(value),
        }
    }
}

/// Serde for `Option<Option<String>>`: absent / `null` / string.
#[allow(
    clippy::option_option,
    reason = "absent, null and string are three distinct pi states"
)]
mod tri {
    use super::{Deserialize, Deserializer, Serializer};

    #[allow(
        clippy::ref_option,
        reason = "signature required by serde(serialize_with)"
    )]
    pub(super) fn serialize<S: Serializer>(
        value: &Option<Option<String>>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        match value {
            Some(Some(text)) => serializer.serialize_str(text),
            _ => serializer.serialize_none(),
        }
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<Option<String>>, D::Error> {
        Option::<String>::deserialize(deserializer).map(Some)
    }
}

/// Prompt cache pricing hints.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PromptCache {
    /// Short-retention multiplier.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "js_f64_opt")]
    pub short: Option<f64>,
    /// Long-retention multiplier.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "js_f64_opt")]
    pub long: Option<f64>,
}

/// Request limits.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InputLimits {
    /// Maximum request body size.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_request_bytes: Option<u64>,
    /// Image limits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub images: Option<ImageInputLimits>,
}

/// Image limits.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageInputLimits {
    /// Resize policy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resize: Option<ImageResize>,
    /// Images per message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_per_message: Option<u32>,
    /// Images per request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_per_request: Option<u32>,
}

/// Image resize policy.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageResize {
    /// Maximum width.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_width: Option<u32>,
    /// Maximum height.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_height: Option<u32>,
    /// Maximum encoded size.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_bytes: Option<u64>,
    /// JPEG quality used when re-encoding.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jpeg_quality: Option<u8>,
}

/// Image-generation model (pi `ImageModel`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageModel {
    /// Model id.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Wire API.
    pub api: String,
    /// Provider id.
    pub provider: String,
    /// Base URL.
    #[serde(default)]
    pub base_url: String,
    /// Input modalities.
    #[serde(default)]
    pub input: Vec<InputModality>,
    /// Output modalities (`text`, `image`).
    #[serde(default)]
    pub output: Vec<InputModality>,
    /// Pricing.
    #[serde(default)]
    pub cost: ModelCost,
    /// Extra headers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub headers: Option<IndexMap<String, String>>,
}

/// Classifier model (pi `ClassifierModel`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClassifierModel {
    /// Model id.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Wire API.
    pub api: String,
    /// Provider id.
    pub provider: String,
    /// Base URL.
    #[serde(default)]
    pub base_url: String,
    /// Input modalities.
    #[serde(default)]
    pub input: Vec<InputModality>,
    /// Pricing.
    #[serde(default)]
    pub cost: ModelCost,
    /// Extra headers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub headers: Option<IndexMap<String, String>>,
    /// Context window.
    #[serde(default)]
    pub context_window: u64,
    /// Provider compatibility flags.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compat: Option<JsonValue>,
}

/// Any registry model (pi `AnyModel`), discriminated by `type` (absent ⇒ chat).
#[derive(Clone, Debug, PartialEq)]
pub enum AnyModel {
    /// Chat model (`type` absent or `"chat"`).
    Chat(Box<Model>),
    /// `type: "image"`.
    Image(Box<ImageModel>),
    /// `type: "classifier"`.
    Classifier(Box<ClassifierModel>),
}

impl Serialize for AnyModel {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::Error;
        let (mut value, tag) = match self {
            Self::Chat(model) => return model.serialize(serializer),
            Self::Image(model) => (
                serde_json::to_value(model).map_err(S::Error::custom)?,
                "image",
            ),
            Self::Classifier(model) => (
                serde_json::to_value(model).map_err(S::Error::custom)?,
                "classifier",
            ),
        };
        if let Some(object) = value.as_object_mut() {
            object.insert("type".into(), JsonValue::String(tag.into()));
        }
        value.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for AnyModel {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        let value = JsonValue::deserialize(deserializer)?;
        let kind = value
            .get("type")
            .and_then(JsonValue::as_str)
            .map(str::to_owned);
        match kind.as_deref() {
            None | Some("chat") => serde_json::from_value(value)
                .map(|m| Self::Chat(Box::new(m)))
                .map_err(D::Error::custom),
            Some("image") => serde_json::from_value(value)
                .map(|m| Self::Image(Box::new(m)))
                .map_err(D::Error::custom),
            Some("classifier") => serde_json::from_value(value)
                .map(|m| Self::Classifier(Box::new(m)))
                .map_err(D::Error::custom),
            Some(other) => Err(D::Error::custom(format!("unknown model type: {other}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn model(raw: JsonValue) -> Option<Model> {
        serde_json::from_value(raw).ok()
    }

    #[test]
    fn thinking_levels_follow_map() {
        let base = json!({"id":"m","name":"M","api":"a","provider":"p","baseUrl":"","input":["text"],
            "cost":{"input":3,"output":15,"cacheRead":0.3,"cacheWrite":3.75},"reasoning":true,
            "contextWindow":200_000,"maxTokens":8192});
        let plain = model(base.clone());
        assert_eq!(plain.map(|m| m.available_thinking_levels().len()), Some(5));
        let mut mapped = base;
        mapped["thinkingLevelMap"] = json!({"minimal": null, "xhigh": "max"});
        let levels = model(mapped.clone()).map(|m| m.available_thinking_levels());
        assert_eq!(
            levels,
            Some(vec![
                ThinkingLevel::Off,
                ThinkingLevel::Low,
                ThinkingLevel::Medium,
                ThinkingLevel::High,
                ThinkingLevel::Xhigh
            ])
        );
        assert_eq!(
            model(mapped.clone()).and_then(|m| serde_json::to_value(m).ok()),
            Some(mapped)
        );
    }

    #[test]
    fn any_model_dispatches_on_type() {
        let image: Option<AnyModel> = serde_json::from_value(json!({"type":"image","id":"i","name":"I","api":"x",
            "provider":"p","baseUrl":"","input":["text"],"output":["image"],"cost":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0}})).ok();
        assert!(matches!(image, Some(AnyModel::Image(_))));
        let raw = image
            .and_then(|m| serde_json::to_value(m).ok())
            .unwrap_or_default();
        assert_eq!(raw["type"], json!("image"));
    }
}
