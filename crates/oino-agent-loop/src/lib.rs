#![doc = r#"Pure async agent loop for Oino.

`oino-agent-loop` consumes typed `AssistantStreamEvent`s, executes tools, and emits
ordered `AgentEvent`s. It intentionally does not know provider JSON payload formats,
API keys, sessions, filesystems, UI, or persistence. Harness/provider adapters own those
concerns and pass typed events into this crate.
"#]
#![forbid(unsafe_code)]

use async_trait::async_trait;
use futures::{stream::FuturesUnordered, StreamExt};
use oino_types::{
    AssistantStreamEvent, ContentBlock, Message, Model, OinoId, ProviderMetadata, StopReason,
    ThinkingLevel, Usage,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    future::Future,
    pin::Pin,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
use thiserror::Error;
use tokio::sync::Mutex;
use uuid::Uuid;

pub type LoopResult<T> = Result<T, LoopError>;
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Debug, Error)]
pub enum LoopError {
    #[error("stream error: {0}")]
    Stream(String),
    #[error("tool `{0}` not found")]
    ToolNotFound(String),
    #[error("tool error: {0}")]
    Tool(String),
    #[error("event sink error: {0}")]
    EventSink(String),
    #[error("aborted")]
    Aborted,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AgentEvent {
    AgentStart {
        run_id: OinoId,
    },
    AgentEnd {
        run_id: OinoId,
        stop_reason: StopReason,
    },
    TurnStart {
        turn: u32,
    },
    TurnEnd {
        turn: u32,
        stop_reason: StopReason,
    },
    MessageStart {
        message_id: OinoId,
        role: String,
    },
    MessageUpdate {
        message_id: OinoId,
        content: Vec<ContentBlock>,
    },
    MessageEnd {
        message: Message,
    },
    ToolExecutionStart {
        call: ToolCall,
    },
    ToolExecutionUpdate {
        call_id: OinoId,
        update: ToolUpdate,
    },
    ToolExecutionEnd {
        call_id: OinoId,
        result: ToolResult,
    },
    QueueUpdate {
        queue: String,
        pending: usize,
    },
    SavePoint {
        label: String,
    },
    Settled,
}

#[async_trait]
pub trait EventSink: Send + Sync {
    async fn emit(&self, event: AgentEvent) -> LoopResult<()>;
}

#[derive(Debug, Default)]
pub struct NoopEventSink;

#[async_trait]
impl EventSink for NoopEventSink {
    async fn emit(&self, _event: AgentEvent) -> LoopResult<()> {
        Ok(())
    }
}

#[derive(Debug, Default, Clone)]
pub struct VecEventSink {
    events: Arc<Mutex<Vec<AgentEvent>>>,
}

impl VecEventSink {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
    pub async fn events(&self) -> Vec<AgentEvent> {
        self.events.lock().await.clone()
    }
}

#[async_trait]
impl EventSink for VecEventSink {
    async fn emit(&self, event: AgentEvent) -> LoopResult<()> {
        self.events.lock().await.push(event);
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub input_schema: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ToolExecutionMode {
    Parallel,
    Sequential,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ToolCall {
    pub id: OinoId,
    pub name: String,
    pub arguments: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ToolUpdate {
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ToolResult {
    pub call_id: OinoId,
    pub tool_name: String,
    pub content: Vec<ContentBlock>,
    pub is_error: bool,
    pub terminate: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

impl ToolResult {
    #[must_use]
    pub fn text(call: &ToolCall, text: impl Into<String>) -> Self {
        Self {
            call_id: call.id,
            tool_name: call.name.clone(),
            content: vec![ContentBlock::Text { text: text.into() }],
            is_error: false,
            terminate: false,
            details: None,
        }
    }
    #[must_use]
    pub fn error(call: &ToolCall, text: impl Into<String>) -> Self {
        Self {
            call_id: call.id,
            tool_name: call.name.clone(),
            content: vec![ContentBlock::Text { text: text.into() }],
            is_error: true,
            terminate: false,
            details: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct AbortSignal {
    flag: Arc<AtomicBool>,
}
impl AbortSignal {
    #[must_use]
    pub fn new() -> Self {
        Self {
            flag: Arc::new(AtomicBool::new(false)),
        }
    }
    pub fn abort(&self) {
        self.flag.store(true, Ordering::SeqCst);
    }
    #[must_use]
    pub fn is_aborted(&self) -> bool {
        self.flag.load(Ordering::SeqCst)
    }
}
impl Default for AbortSignal {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone)]
pub struct ToolUpdateCallback {
    sink: Arc<dyn EventSink>,
    call_id: OinoId,
}
impl ToolUpdateCallback {
    #[must_use]
    pub fn new(sink: Arc<dyn EventSink>, call_id: OinoId) -> Self {
        Self { sink, call_id }
    }
    pub async fn update(&self, update: ToolUpdate) -> LoopResult<()> {
        self.sink
            .emit(AgentEvent::ToolExecutionUpdate {
                call_id: self.call_id,
                update,
            })
            .await
    }
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn definition(&self) -> ToolDefinition;
    fn execution_mode(&self) -> ToolExecutionMode {
        ToolExecutionMode::Parallel
    }
    async fn prepare_arguments(&self, arguments: Value) -> LoopResult<Value> {
        Ok(arguments)
    }
    async fn execute(
        &self,
        call: ToolCall,
        updates: ToolUpdateCallback,
        signal: AbortSignal,
    ) -> LoopResult<ToolResult>;
}

#[derive(Debug, Clone)]
pub struct StreamRequest {
    pub model: Model,
    pub thinking_level: ThinkingLevel,
    pub system_prompt: Option<String>,
    pub messages: Vec<Message>,
    pub tools: Vec<ToolDefinition>,
}

#[async_trait]
pub trait StreamProvider: Send + Sync {
    async fn stream(
        &self,
        request: StreamRequest,
        signal: AbortSignal,
    ) -> LoopResult<Vec<AssistantStreamEvent>>;
}

#[derive(Debug, Clone)]
pub enum BeforeToolCallResult {
    Allow(ToolCall),
    Block(String),
}

pub type TransformContext =
    Arc<dyn Fn(Vec<Message>) -> BoxFuture<'static, LoopResult<Vec<Message>>> + Send + Sync>;
pub type BeforeToolCall =
    Arc<dyn Fn(ToolCall) -> BoxFuture<'static, LoopResult<BeforeToolCallResult>> + Send + Sync>;
pub type AfterToolCall =
    Arc<dyn Fn(ToolResult) -> BoxFuture<'static, LoopResult<ToolResult>> + Send + Sync>;
pub type ShouldStopAfterTurn = Arc<dyn Fn(&[Message]) -> bool + Send + Sync>;
pub type DrainMessages =
    Arc<dyn Fn() -> BoxFuture<'static, LoopResult<Vec<Message>>> + Send + Sync>;

#[derive(Clone)]
pub struct AgentLoopConfig {
    pub model: Model,
    pub thinking_level: ThinkingLevel,
    pub system_prompt: Option<String>,
    pub tools: BTreeMap<String, Arc<dyn Tool>>,
    pub stream: Arc<dyn StreamProvider>,
    pub event_sink: Arc<dyn EventSink>,
    pub transform_context: Option<TransformContext>,
    pub before_tool_call: Option<BeforeToolCall>,
    pub after_tool_call: Option<AfterToolCall>,
    pub tool_execution: ToolExecutionMode,
    pub should_stop_after_turn: Option<ShouldStopAfterTurn>,
    pub get_steering_messages: Option<DrainMessages>,
    pub get_follow_up_messages: Option<DrainMessages>,
    pub max_turns: u32,
    pub abort_signal: AbortSignal,
}

impl AgentLoopConfig {
    #[must_use]
    pub fn new(model: Model, stream: Arc<dyn StreamProvider>) -> Self {
        Self {
            model,
            thinking_level: ThinkingLevel::default(),
            system_prompt: None,
            tools: BTreeMap::new(),
            stream,
            event_sink: Arc::new(NoopEventSink),
            transform_context: None,
            before_tool_call: None,
            after_tool_call: None,
            tool_execution: ToolExecutionMode::Parallel,
            should_stop_after_turn: None,
            get_steering_messages: None,
            get_follow_up_messages: None,
            max_turns: 16,
            abort_signal: AbortSignal::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct AgentLoopOutput {
    pub messages: Vec<Message>,
    pub stop_reason: StopReason,
}

pub async fn run_agent_loop(
    prompt: Message,
    config: AgentLoopConfig,
) -> LoopResult<AgentLoopOutput> {
    let messages = vec![prompt];
    if let Some(first) = messages.first() {
        config
            .event_sink
            .emit(AgentEvent::AgentStart {
                run_id: Uuid::new_v4(),
            })
            .await?;
        config
            .event_sink
            .emit(AgentEvent::MessageStart {
                message_id: first.id(),
                role: "user".into(),
            })
            .await?;
        config
            .event_sink
            .emit(AgentEvent::MessageEnd {
                message: first.clone(),
            })
            .await?;
    }
    run_agent_loop_continue_inner(messages, config, false).await
}

pub async fn run_agent_loop_continue(
    messages: Vec<Message>,
    config: AgentLoopConfig,
) -> LoopResult<AgentLoopOutput> {
    config
        .event_sink
        .emit(AgentEvent::AgentStart {
            run_id: Uuid::new_v4(),
        })
        .await?;
    run_agent_loop_continue_inner(messages, config, false).await
}

async fn run_agent_loop_continue_inner(
    mut messages: Vec<Message>,
    config: AgentLoopConfig,
    agent_started: bool,
) -> LoopResult<AgentLoopOutput> {
    let run_id = Uuid::new_v4();
    if agent_started {
        config
            .event_sink
            .emit(AgentEvent::AgentStart { run_id })
            .await?;
    }
    let mut final_stop = StopReason::Unknown;
    for turn in 0..config.max_turns {
        if config.abort_signal.is_aborted() {
            final_stop = StopReason::Aborted;
            break;
        }
        config
            .event_sink
            .emit(AgentEvent::TurnStart { turn })
            .await?;
        let context = if let Some(transform) = &config.transform_context {
            transform(messages.clone()).await?
        } else {
            messages.clone()
        };
        let request = StreamRequest {
            model: config.model.clone(),
            thinking_level: config.thinking_level,
            system_prompt: config.system_prompt.clone(),
            messages: context,
            tools: config
                .tools
                .values()
                .map(|tool| tool.definition())
                .collect(),
        };
        let assistant = consume_stream(&config, request).await?;
        let stop_reason = assistant_stop(&assistant);
        final_stop = stop_reason.clone();
        messages.push(assistant.clone());
        config
            .event_sink
            .emit(AgentEvent::TurnEnd {
                turn,
                stop_reason: stop_reason.clone(),
            })
            .await?;

        if config.abort_signal.is_aborted()
            || matches!(
                stop_reason,
                StopReason::Aborted | StopReason::Error | StopReason::Length | StopReason::EndTurn
            )
        {
            break;
        }

        let tool_calls = assistant_tool_calls(&assistant);
        if tool_calls.is_empty() {
            if drain_queues(&mut messages, &config).await? == 0 {
                break;
            }
            continue;
        }
        let tool_results = execute_tool_batch(tool_calls, &config).await?;
        let all_terminate = tool_results.iter().all(|result| result.terminate);
        for result in tool_results {
            let message = Message::ToolResult {
                id: Uuid::new_v4(),
                tool_call_id: result.call_id,
                tool_name: result.tool_name.clone(),
                content: result.content.clone(),
                is_error: result.is_error,
                terminate: result.terminate,
                details: result.details.clone(),
            };
            config
                .event_sink
                .emit(AgentEvent::MessageStart {
                    message_id: message.id(),
                    role: "tool_result".into(),
                })
                .await?;
            config
                .event_sink
                .emit(AgentEvent::MessageEnd {
                    message: message.clone(),
                })
                .await?;
            messages.push(message);
        }
        if all_terminate {
            final_stop = StopReason::ToolUse;
            break;
        }
        if let Some(should_stop) = &config.should_stop_after_turn {
            if should_stop(&messages) {
                break;
            }
        }
        let drained = drain_queues(&mut messages, &config).await?;
        if drained > 0 {
            continue;
        }
    }
    config
        .event_sink
        .emit(AgentEvent::AgentEnd {
            run_id,
            stop_reason: final_stop.clone(),
        })
        .await?;
    Ok(AgentLoopOutput {
        messages,
        stop_reason: final_stop,
    })
}

async fn consume_stream(config: &AgentLoopConfig, request: StreamRequest) -> LoopResult<Message> {
    let id = Uuid::new_v4();
    let mut content: Vec<ContentBlock> = Vec::new();
    let mut text = String::new();
    let mut thinking = String::new();
    let mut usage: Option<Usage> = None;
    let mut provider: Option<ProviderMetadata> = None;
    let mut stop_reason = StopReason::Unknown;
    config
        .event_sink
        .emit(AgentEvent::MessageStart {
            message_id: id,
            role: "assistant".into(),
        })
        .await?;
    let events = match config
        .stream
        .stream(request, config.abort_signal.clone())
        .await
    {
        Ok(events) => events,
        Err(err) => vec![AssistantStreamEvent::Error {
            message: err.to_string(),
        }],
    };
    for event in events {
        if config.abort_signal.is_aborted() {
            stop_reason = StopReason::Aborted;
            break;
        }
        match event {
            AssistantStreamEvent::TextDelta { delta } => {
                text.push_str(&delta);
                let mut update = content_without_text(&content);
                update.insert(0, ContentBlock::Text { text: text.clone() });
                config
                    .event_sink
                    .emit(AgentEvent::MessageUpdate {
                        message_id: id,
                        content: update,
                    })
                    .await?;
            }
            AssistantStreamEvent::ThinkingDelta { delta } => {
                thinking.push_str(&delta);
            }
            AssistantStreamEvent::ToolCallDelta { .. } => {}
            AssistantStreamEvent::ToolCallDone {
                id,
                name,
                arguments,
            } => content.push(ContentBlock::ToolCall {
                id,
                name,
                arguments,
            }),
            AssistantStreamEvent::Usage { usage: reported } => usage = Some(reported),
            AssistantStreamEvent::Done {
                stop_reason: reason,
                provider: meta,
            } => {
                stop_reason = reason;
                provider = meta;
            }
            AssistantStreamEvent::Error { message } => {
                text.push_str(&message);
                stop_reason = StopReason::Error;
            }
            AssistantStreamEvent::Aborted => {
                stop_reason = StopReason::Aborted;
            }
        }
    }
    if !text.is_empty() {
        content.insert(0, ContentBlock::Text { text });
    }
    if !thinking.is_empty() {
        content.insert(
            0,
            ContentBlock::Thinking {
                text: thinking,
                redacted: false,
            },
        );
    }
    if content
        .iter()
        .any(|block| matches!(block, ContentBlock::ToolCall { .. }))
        && matches!(stop_reason, StopReason::Unknown | StopReason::EndTurn)
    {
        stop_reason = StopReason::ToolUse;
    }
    let message = Message::Assistant {
        id,
        content,
        stop_reason: Some(stop_reason),
        usage,
        provider,
    };
    config
        .event_sink
        .emit(AgentEvent::MessageEnd {
            message: message.clone(),
        })
        .await?;
    Ok(message)
}

fn content_without_text(content: &[ContentBlock]) -> Vec<ContentBlock> {
    content
        .iter()
        .filter(|block| !matches!(block, ContentBlock::Text { .. }))
        .cloned()
        .collect()
}

fn assistant_stop(message: &Message) -> StopReason {
    if let Message::Assistant { stop_reason, .. } = message {
        stop_reason.clone().unwrap_or(StopReason::Unknown)
    } else {
        StopReason::Unknown
    }
}

fn assistant_tool_calls(message: &Message) -> Vec<ToolCall> {
    if let Message::Assistant { content, .. } = message {
        content
            .iter()
            .filter_map(|block| match block {
                ContentBlock::ToolCall {
                    id,
                    name,
                    arguments,
                } => Some(ToolCall {
                    id: *id,
                    name: name.clone(),
                    arguments: arguments.clone(),
                }),
                _ => None,
            })
            .collect()
    } else {
        Vec::new()
    }
}

async fn execute_tool_batch(
    calls: Vec<ToolCall>,
    config: &AgentLoopConfig,
) -> LoopResult<Vec<ToolResult>> {
    let mut prepared = Vec::with_capacity(calls.len());
    let mut sequential = matches!(config.tool_execution, ToolExecutionMode::Sequential);
    for call in calls {
        let Some(tool) = config.tools.get(&call.name) else {
            prepared.push((
                call.clone(),
                None,
                ToolResult::error(&call, format!("tool `{}` not found", call.name)),
            ));
            continue;
        };
        if matches!(tool.execution_mode(), ToolExecutionMode::Sequential) {
            sequential = true;
        }
        let prepared_args = match tool.prepare_arguments(call.arguments.clone()).await {
            Ok(args) => args,
            Err(err) => {
                prepared.push((
                    call.clone(),
                    None,
                    ToolResult::error(&call, err.to_string()),
                ));
                continue;
            }
        };
        let mut prepared_call = ToolCall {
            arguments: prepared_args,
            ..call
        };
        if let Some(before) = &config.before_tool_call {
            match before(prepared_call.clone()).await? {
                BeforeToolCallResult::Allow(rewritten) => prepared_call = rewritten,
                BeforeToolCallResult::Block(reason) => {
                    let blocked = ToolResult::error(&prepared_call, reason);
                    prepared.push((prepared_call, None, blocked));
                    continue;
                }
            }
        }
        prepared.push((
            prepared_call,
            Some(Arc::clone(tool)),
            ToolResult::error(
                &ToolCall {
                    id: Uuid::nil(),
                    name: String::new(),
                    arguments: Value::Null,
                },
                "placeholder",
            ),
        ));
    }

    if sequential {
        let mut results = Vec::with_capacity(prepared.len());
        for (call, tool, fallback) in prepared {
            results.push(execute_one(call, tool, fallback, config).await?);
        }
        Ok(results)
    } else {
        let mut futures = FuturesUnordered::new();
        for (idx, (call, tool, fallback)) in prepared.into_iter().enumerate() {
            futures.push(async move { (idx, call, tool, fallback) });
        }
        let mut task_futures = FuturesUnordered::new();
        while let Some((idx, call, tool, fallback)) = futures.next().await {
            let cfg = config.clone();
            task_futures.push(async move { (idx, execute_one(call, tool, fallback, &cfg).await) });
        }
        let mut by_index: BTreeMap<usize, ToolResult> = BTreeMap::new();
        while let Some((idx, result)) = task_futures.next().await {
            by_index.insert(idx, result?);
        }
        Ok(by_index.into_values().collect())
    }
}

async fn execute_one(
    call: ToolCall,
    tool: Option<Arc<dyn Tool>>,
    fallback: ToolResult,
    config: &AgentLoopConfig,
) -> LoopResult<ToolResult> {
    if call.id != Uuid::nil() {
        config
            .event_sink
            .emit(AgentEvent::ToolExecutionStart { call: call.clone() })
            .await?;
    }
    let mut result = if let Some(tool) = tool {
        let updates = ToolUpdateCallback::new(Arc::clone(&config.event_sink), call.id);
        match tool
            .execute(call.clone(), updates, config.abort_signal.clone())
            .await
        {
            Ok(result) => result,
            Err(err) => ToolResult::error(&call, err.to_string()),
        }
    } else {
        fallback
    };
    if let Some(after) = &config.after_tool_call {
        result = after(result).await?;
    }
    if call.id != Uuid::nil() {
        config
            .event_sink
            .emit(AgentEvent::ToolExecutionEnd {
                call_id: call.id,
                result: result.clone(),
            })
            .await?;
    }
    Ok(result)
}

async fn drain_queues(messages: &mut Vec<Message>, config: &AgentLoopConfig) -> LoopResult<usize> {
    let mut count = 0;
    if let Some(drain) = &config.get_steering_messages {
        let drained = drain().await?;
        count += drained.len();
        messages.extend(drained);
    }
    if let Some(drain) = &config.get_follow_up_messages {
        let drained = drain().await?;
        count += drained.len();
        messages.extend(drained);
    }
    Ok(count)
}

#[derive(Clone)]
pub struct FauxStream {
    events: Arc<Mutex<Vec<Vec<AssistantStreamEvent>>>>,
}
impl FauxStream {
    #[must_use]
    pub fn once(events: Vec<AssistantStreamEvent>) -> Self {
        Self {
            events: Arc::new(Mutex::new(vec![events])),
        }
    }
    #[must_use]
    pub fn turns(turns: Vec<Vec<AssistantStreamEvent>>) -> Self {
        Self {
            events: Arc::new(Mutex::new(turns)),
        }
    }
}

#[async_trait]
impl StreamProvider for FauxStream {
    async fn stream(
        &self,
        _request: StreamRequest,
        signal: AbortSignal,
    ) -> LoopResult<Vec<AssistantStreamEvent>> {
        if signal.is_aborted() {
            return Ok(vec![AssistantStreamEvent::Aborted]);
        }
        let mut locked = self.events.lock().await;
        if locked.is_empty() {
            return Ok(vec![AssistantStreamEvent::Done {
                stop_reason: StopReason::EndTurn,
                provider: None,
            }]);
        }
        Ok(locked.remove(0))
    }
}

pub struct FakeTool {
    pub definition: ToolDefinition,
    pub mode: ToolExecutionMode,
    pub result: String,
    pub fail: bool,
    pub terminate: bool,
    pub delay_ms: u64,
}

impl FakeTool {
    #[must_use]
    pub fn new(name: impl Into<String>, result: impl Into<String>) -> Self {
        let name = name.into();
        Self {
            definition: ToolDefinition {
                name,
                description: "fake tool".into(),
                input_schema: serde_json::json!({"type":"object"}),
            },
            mode: ToolExecutionMode::Parallel,
            result: result.into(),
            fail: false,
            terminate: false,
            delay_ms: 0,
        }
    }
}

#[async_trait]
impl Tool for FakeTool {
    fn definition(&self) -> ToolDefinition {
        self.definition.clone()
    }
    fn execution_mode(&self) -> ToolExecutionMode {
        self.mode
    }
    async fn execute(
        &self,
        call: ToolCall,
        updates: ToolUpdateCallback,
        signal: AbortSignal,
    ) -> LoopResult<ToolResult> {
        if self.delay_ms > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(self.delay_ms)).await;
        }
        if signal.is_aborted() {
            return Ok(ToolResult {
                call_id: call.id,
                tool_name: call.name,
                content: vec![ContentBlock::Text {
                    text: "aborted".into(),
                }],
                is_error: true,
                terminate: true,
                details: None,
            });
        }
        updates
            .update(ToolUpdate {
                message: "fake update".into(),
                details: None,
            })
            .await?;
        if self.fail {
            return Err(LoopError::Tool(self.result.clone()));
        }
        let mut result = ToolResult::text(&call, self.result.clone());
        result.terminate = self.terminate;
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model() -> Model {
        Model::new("test", "faux")
    }

    #[tokio::test]
    async fn text_stream_event_order() {
        let sink = VecEventSink::new();
        let stream = Arc::new(FauxStream::once(vec![
            AssistantStreamEvent::TextDelta {
                delta: "hel".into(),
            },
            AssistantStreamEvent::TextDelta { delta: "lo".into() },
            AssistantStreamEvent::Done {
                stop_reason: StopReason::EndTurn,
                provider: None,
            },
        ]));
        let mut config = AgentLoopConfig::new(model(), stream);
        config.event_sink = Arc::new(sink.clone());
        let output = run_agent_loop(Message::user_text("hi"), config).await;
        assert!(output.is_ok());
        let events = sink.events().await;
        assert!(matches!(
            events.first(),
            Some(AgentEvent::AgentStart { .. })
        ));
        assert!(events
            .iter()
            .any(|event| matches!(event, AgentEvent::MessageUpdate { .. })));
        assert!(events
            .iter()
            .any(|event| matches!(event, AgentEvent::AgentEnd { .. })));
    }

    #[tokio::test]
    async fn stream_error_becomes_assistant_error() {
        let stream = Arc::new(FauxStream::once(vec![AssistantStreamEvent::Error {
            message: "boom".into(),
        }]));
        let config = AgentLoopConfig::new(model(), stream);
        let output = run_agent_loop(Message::user_text("hi"), config).await;
        let output = match output {
            Ok(output) => output,
            Err(err) => panic!("loop failed: {err}"),
        };
        assert_eq!(output.stop_reason, StopReason::Error);
    }

    #[tokio::test]
    async fn tool_result_messages_preserve_source_order() {
        let call_a = Uuid::new_v4();
        let call_b = Uuid::new_v4();
        let stream = Arc::new(FauxStream::turns(vec![
            vec![
                AssistantStreamEvent::ToolCallDone {
                    id: call_a,
                    name: "a".into(),
                    arguments: serde_json::json!({}),
                },
                AssistantStreamEvent::ToolCallDone {
                    id: call_b,
                    name: "b".into(),
                    arguments: serde_json::json!({}),
                },
                AssistantStreamEvent::Done {
                    stop_reason: StopReason::ToolUse,
                    provider: None,
                },
            ],
            vec![AssistantStreamEvent::Done {
                stop_reason: StopReason::EndTurn,
                provider: None,
            }],
        ]));
        let mut config = AgentLoopConfig::new(model(), stream);
        let mut a = FakeTool::new("a", "A");
        a.delay_ms = 20;
        let b = FakeTool::new("b", "B");
        config.tools.insert("a".into(), Arc::new(a));
        config.tools.insert("b".into(), Arc::new(b));
        let output = run_agent_loop(Message::user_text("tools"), config).await;
        let output = match output {
            Ok(output) => output,
            Err(err) => panic!("loop failed: {err}"),
        };
        let tool_names: Vec<String> = output
            .messages
            .into_iter()
            .filter_map(|msg| match msg {
                Message::ToolResult { tool_name, .. } => Some(tool_name),
                _ => None,
            })
            .collect();
        assert_eq!(tool_names, vec!["a".to_string(), "b".to_string()]);
    }

    #[tokio::test]
    async fn abort_during_streaming_stops_as_aborted() {
        let signal = AbortSignal::new();
        signal.abort();
        let stream = Arc::new(FauxStream::once(vec![AssistantStreamEvent::TextDelta {
            delta: "no".into(),
        }]));
        let mut config = AgentLoopConfig::new(model(), stream);
        config.abort_signal = signal;
        let output = run_agent_loop(Message::user_text("hi"), config).await;
        let output = match output {
            Ok(output) => output,
            Err(err) => panic!("loop failed: {err}"),
        };
        assert_eq!(output.stop_reason, StopReason::Aborted);
    }
}
