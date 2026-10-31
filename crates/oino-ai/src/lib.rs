//! `oino-ai`: the LLM data model for Oino.
//!
//! Every type serialises to a fixed JSON shape (field names, tags, optionality), so
//! sessions, RPC records and WASM-boundary payloads stay interchangeable across processes and
//! tools that speak the same format. The crate has
//! no async runtime and builds for `wasm32-wasip2`; guests and host share these types (D5).
//!
//! # API map
//!
//! | Module | Contents |
//! |---|---|
//! | [`primitives`] | scalars: [`TimestampMs`], [`ThinkingLevel`] (+ clamping), [`StopReason`], [`Usage`] |
//! | [`content`] | content blocks: [`TextContent`], [`ThinkingContent`], [`ImageContent`], [`ToolCall`], [`UserContent`], [`AssistantContent`], [`UserContentField`] |
//! | [`message`] | [`Message`], [`AgentMessage`], [`SystemMessage`], [`AssistantMessage`], … |
//! | [`model`] | [`Model`], [`ModelKey`], [`ThinkingLevelMap`], costs and limits |
//! | [`stream`] | [`AssistantMessageEvent`], [`ProviderEvent`], [`PartialAssembler`] |
//! | [`transcript`] | system-message replay, [`transcript::diff_sections`], collapsing |
//! | [`transform`] | [`transform::transform_messages`] |
//! | [`json`] | [`JsonObject`], [`JsonValue`], [`json::json_semantic_eq`] |
//!
//! Unknown JSON fields are ignored on read; optional fields are omitted on write exactly where
//! the wire format omits them; integral `f64` costs serialise without a fraction like `JSON.stringify`.

#![forbid(unsafe_code)]
#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        reason = "tests assert by unwrapping"
    )
)]

pub mod content;
pub mod json;
pub mod message;
pub mod model;
pub mod primitives;
pub mod stream;
pub mod transcript;
pub mod transform;

pub use content::{
    AssistantContent, ImageContent, TextContent, ThinkingContent, ToolCall, TypedText, UserContent,
    UserContentField, UserTextField,
};
pub use json::{JsonObject, JsonValue};
pub use message::{
    AgentMessage, AssistantDiagnostic, AssistantMessage, BashExecutionMessage,
    BranchSummaryMessage, CompactionSummaryMessage, ConstrainedSampling, ConstrainedSamplingConfig,
    CustomMessage, DeferredHandle, DiagnosticError, FalseLiteral, Message, NestedStatus,
    NestedToolCallRecord, NestedToolCalls, Sections, StrictMode, SystemMessage, ToolDecl, ToolRef,
    ToolResultMessage, UserMessage,
};
pub use model::{
    AnyModel, ClassifierModel, ImageInputLimits, ImageModel, ImageResize, InputLimits, MapEntry,
    Model, ModelCost, ModelCostTier, ModelKey, ModelKindChat, PromptCache, ThinkingLevelMap,
};
pub use primitives::{
    CacheRetention, InputModality, StopReason, ThinkingLevel, TimestampMs, UnknownThinkingLevel,
    Usage, UsageCost,
};
pub use stream::{
    AssistantMessageEvent, Context, DoneReason, ErrorReason, PartialAssembler, ProviderEvent,
    StreamOptions, ThinkingBudgets, Transport,
};
