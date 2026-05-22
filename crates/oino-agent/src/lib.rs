#![doc = r#"Stateful in-memory agent facade for Oino.

`oino-agent` wraps the pure `oino-agent-loop` crate with the mutable runtime state an
application needs between turns. It owns transcript state, steering and follow-up
queues, subscriber fan-out, cancellation, and prompt concurrency guards while keeping
sessions, provider serialization, UI, and filesystem behavior outside the crate.

## Boundary

This crate owns live in-memory agent state only. It snapshots and updates
[`AgentState`], drains queued [`oino_types::Message`] values into
[`oino_agent_loop::AgentLoopConfig`], relays [`oino_agent_loop::AgentEvent`] values to
subscribers, and exposes a single busy guard around the underlying loop. It does not
persist JSONL sessions, choose resource paths, build provider HTTP requests, execute raw
filesystem/process operations, render terminal UI, or define model-visible tool schemas.
Those concerns belong to `oino-session`, `oino-resource`, provider crates,
`oino-env`/`oino-tools`, `oino-tui`, and `oino-harness`.

## Public API map

- [`Agent`] is the clonable facade over shared async state. [`Agent::new`] starts from
  an [`oino_agent_loop::AgentLoopConfig`]; [`Agent::new_with_messages`] resumes from an
  already reconstructed transcript.
- [`AgentState`] is the current UI/runtime snapshot: messages, model, thinking level,
  system prompt, visible tool definitions, and whether a prompt is streaming.
- [`Agent::prompt`] runs a new user message and [`Agent::continue_from_current_context`]
  continues the reconstructed transcript without adding a new prompt.
- [`Agent::steer`] and [`Agent::follow_up`] append queued messages that the loop drains
  between provider turns. [`QueueMode`] chooses whether each drain takes every queued
  item or one message at a time.
- [`Agent::subscribe`] registers [`AgentEventSubscriber`] callbacks. Events first reach
  the configured loop sink, then every subscriber is awaited before emission completes.
- [`Agent::abort`] signals the current run, [`Agent::wait_for_idle`] is a small test/UI
  settlement helper, and the `set_*` methods keep [`AgentState`] and loop config in
  sync for model, thinking, system-prompt, and tool changes.
- [`AgentError`] and [`AgentResult`] keep the public error surface small: concurrent
  prompts return [`AgentError::Busy`], and loop failures stay typed as loop errors.

## Contributor rules

Keep this crate focused on async state coordination. Do not add provider JSON, session
file formats, TUI commands, model-visible tool definitions, or direct filesystem/process
side effects here. Preserve the single-run guard and make sure `is_streaming` plus the
current abort signal are reset on every exit path. Keep async mutex lock scopes short,
clone state/config before calling providers or tools, and avoid holding locks while
subscriber callbacks run. If queue draining, event fan-out, or busy/abort semantics
change, update the harness/TUI docs and the queue, subscriber, concurrency, and abort
tests together.
"#]
#![forbid(unsafe_code)]

use futures::future::join_all;
use oino_agent_loop::{
    run_agent_loop_continue, run_agent_loop_with_context, AbortSignal, AgentEvent, AgentLoopConfig,
    AgentLoopOutput, BoxFuture, EventSink, LoopError, LoopResult, ToolDefinition,
};
use oino_types::{Message, Model, ThinkingLevel};
use std::{collections::VecDeque, sync::Arc};
use thiserror::Error;
use tokio::sync::Mutex;

#[derive(Debug, Error)]
pub enum AgentError {
    #[error("agent is already running")]
    Busy,
    #[error(transparent)]
    Loop(#[from] LoopError),
}

pub type AgentResult<T> = Result<T, AgentError>;
pub type AgentEventSubscriber = Arc<dyn Fn(AgentEvent) -> BoxFuture<'static, ()> + Send + Sync>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueMode {
    All,
    OneAtATime,
}

#[derive(Clone)]
pub struct AgentState {
    pub messages: Vec<Message>,
    pub model: Model,
    pub thinking_level: ThinkingLevel,
    pub system_prompt: Option<String>,
    pub tools: Vec<ToolDefinition>,
    pub is_streaming: bool,
}

pub struct Agent {
    state: Arc<Mutex<AgentState>>,
    config: Arc<Mutex<AgentLoopConfig>>,
    steering: Arc<Mutex<VecDeque<Message>>>,
    follow_up: Arc<Mutex<VecDeque<Message>>>,
    subscribers: Arc<Mutex<Vec<AgentEventSubscriber>>>,
    current_signal: Arc<Mutex<Option<AbortSignal>>>,
    steering_mode: QueueMode,
    follow_up_mode: QueueMode,
}

impl Agent {
    #[must_use]
    pub fn new(config: AgentLoopConfig) -> Self {
        Self::new_with_messages(config, Vec::new())
    }

    #[must_use]
    pub fn new_with_messages(config: AgentLoopConfig, messages: Vec<Message>) -> Self {
        let state = AgentState {
            messages,
            model: config.model.clone(),
            thinking_level: config.thinking_level,
            system_prompt: config.system_prompt.clone(),
            tools: config
                .tools
                .values()
                .map(|tool| tool.definition())
                .collect(),
            is_streaming: false,
        };
        Self {
            state: Arc::new(Mutex::new(state)),
            config: Arc::new(Mutex::new(config)),
            steering: Arc::new(Mutex::new(VecDeque::new())),
            follow_up: Arc::new(Mutex::new(VecDeque::new())),
            subscribers: Arc::new(Mutex::new(Vec::new())),
            current_signal: Arc::new(Mutex::new(None)),
            steering_mode: QueueMode::All,
            follow_up_mode: QueueMode::All,
        }
    }

    #[must_use]
    pub fn with_queue_modes(mut self, steering: QueueMode, follow_up: QueueMode) -> Self {
        self.steering_mode = steering;
        self.follow_up_mode = follow_up;
        self
    }

    pub async fn state(&self) -> AgentState {
        self.state.lock().await.clone()
    }
    pub async fn messages(&self) -> Vec<Message> {
        self.state.lock().await.messages.clone()
    }
    pub async fn is_idle(&self) -> bool {
        !self.state.lock().await.is_streaming
    }

    pub async fn set_model(&self, model: Model) {
        self.state.lock().await.model = model.clone();
        self.config.lock().await.model = model;
    }

    pub async fn set_thinking_level(&self, thinking_level: ThinkingLevel) {
        self.state.lock().await.thinking_level = thinking_level;
        self.config.lock().await.thinking_level = thinking_level;
    }

    pub async fn set_system_prompt(&self, system_prompt: Option<String>) {
        self.state.lock().await.system_prompt = system_prompt.clone();
        self.config.lock().await.system_prompt = system_prompt;
    }

    pub async fn set_tools(
        &self,
        tools: std::collections::BTreeMap<String, Arc<dyn oino_agent_loop::Tool>>,
    ) {
        self.state.lock().await.tools = tools.values().map(|tool| tool.definition()).collect();
        self.config.lock().await.tools = tools;
    }

    pub async fn reset(&self) {
        self.replace_messages(Vec::new()).await;
    }

    pub async fn replace_messages(&self, messages: Vec<Message>) {
        self.state.lock().await.messages = messages;
        self.steering.lock().await.clear();
        self.follow_up.lock().await.clear();
    }

    pub async fn subscribe(&self, subscriber: AgentEventSubscriber) {
        self.subscribers.lock().await.push(subscriber);
    }

    pub async fn steer(&self, message: Message) -> LoopResult<()> {
        let pending = {
            let mut queue = self.steering.lock().await;
            queue.push_back(message);
            queue.len()
        };
        self.config
            .lock()
            .await
            .event_sink
            .emit(AgentEvent::QueueUpdate {
                queue: "steering".into(),
                pending,
            })
            .await
    }

    pub async fn follow_up(&self, message: Message) -> LoopResult<()> {
        let pending = {
            let mut queue = self.follow_up.lock().await;
            queue.push_back(message);
            queue.len()
        };
        self.config
            .lock()
            .await
            .event_sink
            .emit(AgentEvent::QueueUpdate {
                queue: "follow_up".into(),
                pending,
            })
            .await
    }

    pub async fn abort(&self) {
        if let Some(signal) = self.current_signal.lock().await.as_ref() {
            signal.abort();
        }
    }

    pub async fn wait_for_idle(&self) {
        loop {
            if self.is_idle().await {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    }

    pub async fn prompt(&self, message: Message) -> AgentResult<AgentLoopOutput> {
        self.run(Some(message)).await
    }

    pub async fn continue_from_current_context(&self) -> AgentResult<AgentLoopOutput> {
        self.run(None).await
    }

    async fn run(&self, prompt: Option<Message>) -> AgentResult<AgentLoopOutput> {
        {
            let mut state = self.state.lock().await;
            if state.is_streaming {
                return Err(AgentError::Busy);
            }
            state.is_streaming = true;
        }

        let signal = AbortSignal::new();
        *self.current_signal.lock().await = Some(signal.clone());
        let mut config = self.config.lock().await.clone();
        config.abort_signal = signal;
        config.get_steering_messages = Some(queue_drain_fn(
            Arc::clone(&self.steering),
            self.steering_mode,
        ));
        config.get_follow_up_messages = Some(queue_drain_fn(
            Arc::clone(&self.follow_up),
            self.follow_up_mode,
        ));
        config.event_sink = Arc::new(SubscriberSink {
            inner: Arc::clone(&config.event_sink),
            subscribers: Arc::clone(&self.subscribers),
        });

        let messages = self.state.lock().await.messages.clone();
        let result = if let Some(message) = prompt {
            run_agent_loop_with_context(messages, message, config).await
        } else {
            run_agent_loop_continue(messages, config).await
        };

        *self.current_signal.lock().await = None;
        let mut state = self.state.lock().await;
        state.is_streaming = false;
        match result {
            Ok(output) => {
                state.messages = output.messages.clone();
                Ok(output)
            }
            Err(err) => Err(AgentError::Loop(err)),
        }
    }
}

fn queue_drain_fn(
    queue: Arc<Mutex<VecDeque<Message>>>,
    mode: QueueMode,
) -> Arc<dyn Fn() -> oino_agent_loop::BoxFuture<'static, LoopResult<Vec<Message>>> + Send + Sync> {
    Arc::new(move || {
        let queue = Arc::clone(&queue);
        Box::pin(async move {
            let mut locked = queue.lock().await;
            let mut drained = Vec::new();
            match mode {
                QueueMode::All => {
                    while let Some(message) = locked.pop_front() {
                        drained.push(message);
                    }
                }
                QueueMode::OneAtATime => {
                    if let Some(message) = locked.pop_front() {
                        drained.push(message);
                    }
                }
            }
            Ok(drained)
        })
    })
}

struct SubscriberSink {
    inner: Arc<dyn EventSink>,
    subscribers: Arc<Mutex<Vec<AgentEventSubscriber>>>,
}

#[async_trait::async_trait]
impl EventSink for SubscriberSink {
    async fn emit(&self, event: AgentEvent) -> LoopResult<()> {
        self.inner.emit(event.clone()).await?;
        let subscribers = self.subscribers.lock().await.clone();
        let futures = subscribers
            .into_iter()
            .map(|subscriber| subscriber(event.clone()));
        join_all(futures).await;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oino_agent_loop::{
        AgentLoopConfig, FauxStream, StreamProvider, ToolExecutionMode, VecEventSink,
    };
    use oino_types::{AssistantStreamEvent, ContentBlock, StopReason};

    fn config() -> AgentLoopConfig {
        AgentLoopConfig::new(
            Model::new("test", "faux"),
            Arc::new(FauxStream::once(vec![AssistantStreamEvent::Done {
                stop_reason: StopReason::EndTurn,
                provider: None,
            }])) as Arc<dyn StreamProvider>,
        )
    }

    #[tokio::test]
    async fn prompt_updates_state() {
        let agent = Agent::new(config());
        let output = agent.prompt(Message::user_text("hi")).await;
        assert!(output.is_ok());
        assert!(!agent.messages().await.is_empty());
    }

    #[tokio::test]
    async fn prompt_preserves_existing_transcript() {
        let stream = Arc::new(FauxStream::turns(vec![
            vec![AssistantStreamEvent::Done {
                stop_reason: StopReason::EndTurn,
                provider: None,
            }],
            vec![AssistantStreamEvent::Done {
                stop_reason: StopReason::EndTurn,
                provider: None,
            }],
        ])) as Arc<dyn StreamProvider>;
        let agent = Agent::new(AgentLoopConfig::new(Model::new("test", "faux"), stream));
        let first = match agent.prompt(Message::user_text("one")).await {
            Ok(output) => output,
            Err(err) => panic!("first prompt failed: {err}"),
        };
        assert_eq!(first.messages.len(), 2);
        let second = match agent.prompt(Message::user_text("two")).await {
            Ok(output) => output,
            Err(err) => panic!("second prompt failed: {err}"),
        };
        assert_eq!(second.messages.len(), 4);
        assert!(matches!(
            second.messages.first(),
            Some(Message::User { .. })
        ));
        assert!(matches!(second.messages.get(2), Some(Message::User { .. })));
    }

    #[tokio::test]
    async fn abort_signal_is_callable() {
        let agent = Agent::new(config());
        agent.abort().await;
        assert!(agent.is_idle().await);
    }

    #[tokio::test]
    async fn queue_delivery_one_at_a_time() {
        let stream = Arc::new(FauxStream::turns(vec![
            vec![AssistantStreamEvent::Done {
                stop_reason: StopReason::EndTurn,
                provider: None,
            }],
            vec![AssistantStreamEvent::Done {
                stop_reason: StopReason::EndTurn,
                provider: None,
            }],
        ]));
        let mut cfg = AgentLoopConfig::new(Model::new("test", "faux"), stream);
        cfg.max_turns = 2;
        let agent = Agent::new(cfg).with_queue_modes(QueueMode::OneAtATime, QueueMode::OneAtATime);
        let queued = agent.follow_up(Message::user_text("next")).await;
        assert!(queued.is_ok());
        let output = agent.prompt(Message::user_text("hi")).await;
        assert!(output.is_ok());
    }

    #[tokio::test]
    async fn concurrent_prompt_is_rejected() {
        let stream = Arc::new(FauxStream::once(vec![
            AssistantStreamEvent::ToolCallDone {
                id: uuid::Uuid::new_v4(),
                name: "slow".into(),
                arguments: serde_json::json!({}),
            },
            AssistantStreamEvent::Done {
                stop_reason: StopReason::ToolUse,
                provider: None,
            },
        ]));
        let mut cfg = AgentLoopConfig::new(Model::new("test", "faux"), stream);
        let mut slow = oino_agent_loop::FakeTool::new("slow", "ok");
        slow.delay_ms = 50;
        slow.mode = ToolExecutionMode::Sequential;
        cfg.tools.insert("slow".into(), Arc::new(slow));
        let agent = Arc::new(Agent::new(cfg));
        let running = Arc::clone(&agent);
        let handle = tokio::spawn(async move { running.prompt(Message::user_text("first")).await });
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        let second = agent.prompt(Message::user_text("second")).await;
        assert!(matches!(second, Err(AgentError::Busy)));
        let first = handle.await;
        assert!(matches!(first, Ok(Ok(_))));
    }

    #[tokio::test]
    async fn abort_during_tools_returns_error_tool_result() {
        let stream = Arc::new(FauxStream::once(vec![
            AssistantStreamEvent::ToolCallDone {
                id: uuid::Uuid::new_v4(),
                name: "slow".into(),
                arguments: serde_json::json!({}),
            },
            AssistantStreamEvent::Done {
                stop_reason: StopReason::ToolUse,
                provider: None,
            },
        ]));
        let mut cfg = AgentLoopConfig::new(Model::new("test", "faux"), stream);
        let mut slow = oino_agent_loop::FakeTool::new("slow", "ok");
        slow.delay_ms = 50;
        cfg.tools.insert("slow".into(), Arc::new(slow));
        let agent = Arc::new(Agent::new(cfg));
        let running = Arc::clone(&agent);
        let handle = tokio::spawn(async move { running.prompt(Message::user_text("tools")).await });
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        agent.abort().await;
        let output = match handle.await {
            Ok(Ok(output)) => output,
            Ok(Err(err)) => panic!("agent failed: {err}"),
            Err(err) => panic!("join failed: {err}"),
        };
        assert!(output.messages.iter().any(|message| matches!(
            message,
            Message::ToolResult { is_error: true, content, .. }
                if matches!(content.first(), Some(ContentBlock::Text { text }) if text == "aborted")
        )));
    }

    #[tokio::test]
    async fn queue_updates_are_emitted_and_tools_are_in_state() {
        let sink = VecEventSink::new();
        let mut cfg = config();
        cfg.event_sink = Arc::new(sink.clone());
        let agent = Agent::new(cfg);
        let mut tools = std::collections::BTreeMap::new();
        tools.insert(
            "visible".into(),
            Arc::new(oino_agent_loop::FakeTool::new("visible", "ok"))
                as Arc<dyn oino_agent_loop::Tool>,
        );
        agent.set_tools(tools).await;
        let queued = agent.follow_up(Message::user_text("next")).await;
        assert!(queued.is_ok());
        assert_eq!(agent.state().await.tools.len(), 1);
        let events = sink.events().await;
        assert!(events.iter().any(|event| matches!(
            event,
            AgentEvent::QueueUpdate { queue, pending: 1 } if queue == "follow_up"
        )));
    }

    #[tokio::test]
    async fn subscribers_are_awaited() {
        let agent = Agent::new(config());
        let seen = Arc::new(Mutex::new(0usize));
        let seen_clone = Arc::clone(&seen);
        agent
            .subscribe(Arc::new(move |_event| {
                let seen = Arc::clone(&seen_clone);
                Box::pin(async move {
                    *seen.lock().await += 1;
                })
            }))
            .await;
        let output = agent.prompt(Message::user_text("hi")).await;
        assert!(output.is_ok());
        assert!(*seen.lock().await > 0);
    }
}
