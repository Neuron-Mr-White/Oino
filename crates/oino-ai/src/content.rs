//! Content blocks (pi `TextContent`, `ThinkingContent`, `ImageContent`, `ToolCall`).

use serde::{Deserialize, Serialize};

use crate::json::JsonObject;

/// `{type:"text", text, textSignature?}`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextContent {
    /// The text.
    pub text: String,
    /// Provider signature replayed verbatim (e.g. the `OpenAI` Responses `textSignatureV1`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_signature: Option<String>,
}

impl TextContent {
    /// Plain text block without signature.
    #[must_use]
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            text_signature: None,
        }
    }
}

/// `{type:"thinking", thinking, thinkingSignature?, redacted?}`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThinkingContent {
    /// Reasoning text.
    pub thinking: String,
    /// Provider signature or opaque state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking_signature: Option<String>,
    /// `true` when the provider redacted the reasoning.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub redacted: Option<bool>,
}

/// `{type:"image", data, mimeType}` with base64 `data`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageContent {
    /// Base64-encoded bytes.
    pub data: String,
    /// MIME type, e.g. `image/png`.
    pub mime_type: String,
}

/// `{type:"toolCall", id, name, arguments, thoughtSignature?, namespace?, argumentsError?}`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCall {
    /// Provider tool-call id.
    pub id: String,
    /// Tool name.
    pub name: String,
    /// Parsed arguments (`{}` when the provider sent invalid JSON; see `arguments_error`).
    #[serde(default)]
    pub arguments: JsonObject,
    /// Gemini thought signature.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thought_signature: Option<String>,
    /// Tool namespace (provider specific).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    /// Parse error when arguments were not valid JSON.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arguments_error: Option<String>,
}

/// User / tool-result content block: text or image.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum UserContent {
    /// Text block.
    #[serde(rename = "text")]
    Text(TextContent),
    /// Image block.
    #[serde(rename = "image")]
    Image(ImageContent),
}

impl UserContent {
    /// Text block helper.
    #[must_use]
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text(TextContent::new(text))
    }
}

/// Assistant content block: text, thinking or tool call.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum AssistantContent {
    /// Text block.
    #[serde(rename = "text")]
    Text(TextContent),
    /// Reasoning block.
    #[serde(rename = "thinking")]
    Thinking(ThinkingContent),
    /// Tool call block.
    #[serde(rename = "toolCall")]
    ToolCall(ToolCall),
}

impl AssistantContent {
    /// Text block helper.
    #[must_use]
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text(TextContent::new(text))
    }
}

/// System content: pi `string | TextContent[]`. A JSON string stays a string.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum UserTextField {
    /// Plain string form.
    Text(String),
    /// Array of `{type:"text"}` blocks.
    Blocks(Vec<TypedText>),
}

impl Default for UserTextField {
    fn default() -> Self {
        Self::Text(String::new())
    }
}

impl UserTextField {
    /// pi `contentText(content)`: the string, or text blocks joined with `separator`.
    #[must_use]
    pub fn text(&self, separator: &str) -> String {
        match self {
            Self::Text(text) => text.clone(),
            Self::Blocks(blocks) => blocks
                .iter()
                .map(|TypedText::Text(block)| block.text.as_str())
                .collect::<Vec<_>>()
                .join(separator),
        }
    }
}

/// A `{type:"text", …}` block inside [`UserTextField::Blocks`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum TypedText {
    /// The only allowed variant.
    #[serde(rename = "text")]
    Text(TextContent),
}

/// User/custom content: pi `string | (TextContent | ImageContent)[]`. A JSON string stays a
/// string.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum UserContentField {
    /// Plain string form.
    Text(String),
    /// Block array form.
    Blocks(Vec<UserContent>),
}

impl Default for UserContentField {
    fn default() -> Self {
        Self::Blocks(Vec::new())
    }
}

impl UserContentField {
    /// Converts to block form (a string becomes one text block), as pi `convertToLlm` does.
    #[must_use]
    pub fn into_blocks(self) -> Vec<UserContent> {
        match self {
            Self::Text(text) => vec![UserContent::text(text)],
            Self::Blocks(blocks) => blocks,
        }
    }

    /// Text of the string form, or text blocks joined with `separator`.
    #[must_use]
    pub fn text(&self, separator: &str) -> String {
        match self {
            Self::Text(text) => text.clone(),
            Self::Blocks(blocks) => blocks
                .iter()
                .filter_map(|block| match block {
                    UserContent::Text(text) => Some(text.text.as_str()),
                    UserContent::Image(_) => None,
                })
                .collect::<Vec<_>>()
                .join(separator),
        }
    }
}

impl From<String> for UserContentField {
    fn from(text: String) -> Self {
        Self::Text(text)
    }
}

impl From<&str> for UserContentField {
    fn from(text: &str) -> Self {
        Self::Text(text.to_owned())
    }
}

impl From<Vec<UserContent>> for UserContentField {
    fn from(blocks: Vec<UserContent>) -> Self {
        Self::Blocks(blocks)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn blocks_use_pi_tags_and_names() {
        let tool = AssistantContent::ToolCall(ToolCall {
            id: "call_1".into(),
            name: "read".into(),
            arguments: json!({"path": "a"})
                .as_object()
                .cloned()
                .unwrap_or_default(),
            ..ToolCall::default()
        });
        assert_eq!(
            serde_json::to_value(&tool).ok(),
            Some(json!({"type":"toolCall","id":"call_1","name":"read","arguments":{"path":"a"}}))
        );
        let image = UserContent::Image(ImageContent {
            data: "AA==".into(),
            mime_type: "image/png".into(),
        });
        assert_eq!(
            serde_json::to_value(&image).ok(),
            Some(json!({"type":"image","data":"AA==","mimeType":"image/png"}))
        );
    }

    #[test]
    fn string_content_stays_a_string() {
        let field: UserContentField = serde_json::from_value(json!("hi")).unwrap_or_default();
        assert_eq!(field, UserContentField::Text("hi".into()));
        assert_eq!(serde_json::to_value(&field).ok(), Some(json!("hi")));
        let blocks: UserContentField =
            serde_json::from_value(json!([{"type":"text","text":"a"}])).unwrap_or_default();
        assert_eq!(blocks.text(" "), "a");
    }
}
