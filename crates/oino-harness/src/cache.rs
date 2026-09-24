//! Host-owned prompt-cache telemetry and warming (P4.2).
//!
//! [`CacheTracker`] fingerprints every provider request's cache-relevant
//! prefix (leading system, sections, tools, messages), computes the cache hit
//! rate after each response, diagnoses the first diverging block on a miss,
//! and — when configured — sends a 1-output-token warming request before the
//! provider-side cache lifetime expires.

use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::time::Instant;

use oino_agent_loop::{AbortSignal, AgentEvent, StreamProvider, StreamRequest};
use oino_types::{
    transcript::{current_system, current_tools},
    AssistantStreamEvent, Message, Model, Usage,
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::sync::Mutex as AsyncMutex;

/// Cache-relevant catalog data for one model (from a wasm provider's
/// `list_models` JSON or a native catalog): prompt-cache lifetimes in seconds
/// and prices used for the avoided-cost estimate.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModelCacheInfo {
    pub short_secs: Option<u64>,
    pub long_secs: Option<u64>,
    pub input_price: Option<f64>,
    pub cache_read_price: Option<f64>,
}

/// Resolves catalog cache info for a request model.
pub type ModelCacheInfoLookup = Arc<dyn Fn(&Model) -> Option<ModelCacheInfo> + Send + Sync>;

/// Warming trigger mode (`cacheWarming` setting).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CacheWarmingMode {
    Off,
    /// Warm while a tool is still executing near lifetime expiry.
    #[default]
    Tools,
    /// Also warm when the agent is idle (settled), up to 3 warms in a row.
    ToolsAndIdle,
}

/// Host cache-warming configuration pushed by the app.
#[derive(Default)]
pub struct CacheWarmingConfig {
    pub mode: CacheWarmingMode,
    /// Show cache-miss notices (`showCacheMissNotices` setting).
    pub show_miss_notices: bool,
    pub model_info: Option<ModelCacheInfoLookup>,
}

/// Snapshot for `/session` output.
#[derive(Debug, Clone, Default)]
pub struct CacheStatus {
    pub last_hit_rate: Option<f64>,
    pub last_diagnosis: Option<String>,
    pub next_decision: Option<String>,
}

/// A notice raised when a request's cache hit rate is low.
#[derive(Debug, Clone)]
pub struct CacheMissNotice {
    pub diagnosis: String,
    pub hit_rate: f64,
    pub idle_secs: u64,
}

pub type CacheMissNotifier = Arc<dyn Fn(CacheMissNotice) + Send + Sync>;
/// Veto consult for `cache_warming_decision`: `true` vetoes the warm.
pub type CacheWarmingVeto =
    Arc<dyn Fn(Value) -> oino_agent_loop::BoxFuture<'static, bool> + Send + Sync>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Fingerprint {
    kind: &'static str,
    label: String,
    hash: String,
}

impl Fingerprint {
    fn new(kind: &'static str, label: impl Into<String>, value: &Value) -> Self {
        let canonical = serde_json::to_string(value).unwrap_or_default();
        let hash = format!("{:x}", Sha256::digest(canonical.as_bytes()));
        Self {
            kind,
            label: label.into(),
            hash,
        }
    }

    fn describe(&self) -> String {
        match self.kind {
            "system" => "system prompt changed".to_string(),
            "section" => format!("system section {} changed", self.label),
            "tool" => format!("tool `{}` definition changed", self.label),
            _ => format!("message {} changed", self.label),
        }
    }
}

#[derive(Clone)]
struct TrackedRequest {
    model: Model,
    thinking_level: oino_types::ThinkingLevel,
    session_id: Option<String>,
    cache_retention: oino_agent_loop::CacheRetention,
    messages: Vec<Message>,
}

#[derive(Default)]
struct TrackerState {
    fingerprints: Vec<Fingerprint>,
    prompt_tokens: u64,
    last_request: Option<TrackedRequest>,
    last_response_at: Option<Instant>,
    last_hit_rate: Option<f64>,
    last_diagnosis: Option<String>,
    last_decision: Option<String>,
    idle_warm_count: u32,
    warming_in_flight: bool,
    tool_running: bool,
}

/// Fingerprints + warming state for one harness.
pub struct CacheTracker {
    state: Arc<Mutex<TrackerState>>,
    provider: Arc<dyn StreamProvider>,
    config: AsyncMutex<CacheWarmingConfig>,
    notifier: AsyncMutex<Option<CacheMissNotifier>>,
    veto: AsyncMutex<Option<CacheWarmingVeto>>,
    session: AsyncMutex<Option<Arc<AsyncMutex<oino_session::SessionManager>>>>,
}

impl CacheTracker {
    pub fn new(provider: Arc<dyn StreamProvider>) -> Self {
        Self {
            state: Arc::new(Mutex::new(TrackerState::default())),
            provider,
            config: AsyncMutex::new(CacheWarmingConfig::default()),
            notifier: AsyncMutex::new(None),
            veto: AsyncMutex::new(None),
            session: AsyncMutex::new(None),
        }
    }

    pub async fn configure(&self, config: CacheWarmingConfig) {
        *self.config.lock().await = config;
    }

    pub async fn set_notifier(&self, notifier: Option<CacheMissNotifier>) {
        *self.notifier.lock().await = notifier;
    }

    pub async fn set_veto(&self, veto: Option<CacheWarmingVeto>) {
        *self.veto.lock().await = veto;
    }

    pub async fn set_session(&self, session: Arc<AsyncMutex<oino_session::SessionManager>>) {
        *self.session.lock().await = Some(session);
    }

    pub fn status(&self) -> CacheStatus {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        CacheStatus {
            last_hit_rate: state.last_hit_rate,
            last_diagnosis: state.last_diagnosis.clone(),
            next_decision: state.last_decision.clone(),
        }
    }

    /// Fingerprints for one outgoing provider request.
    pub(crate) fn request_fingerprints(&self, request: &StreamRequest) -> Vec<Fingerprint> {
        Self::fingerprint_request(request)
    }

    fn fingerprint_request(request: &StreamRequest) -> Vec<Fingerprint> {
        let mut out = Vec::new();
        let messages = request.transcript.messages();
        if let Some(system) = current_system(messages) {
            let Message::System {
                content, sections, ..
            } = &system
            else {
                return out;
            };
            out.push(Fingerprint::new(
                "system",
                "prompt",
                &Value::String(content.clone()),
            ));
            for section in sections {
                out.push(Fingerprint::new(
                    "section",
                    section.name.clone(),
                    &Value::String(section.text.clone().unwrap_or_default()),
                ));
            }
        }
        for tool in current_tools(messages) {
            out.push(Fingerprint::new(
                "tool",
                tool.name.clone(),
                &serde_json::to_value(&tool).unwrap_or_default(),
            ));
        }
        for (index, message) in messages.iter().enumerate() {
            let role = match message {
                Message::System { .. } => "system",
                Message::User { .. } => "user",
                Message::Assistant { .. } => "assistant",
                Message::ToolResult { .. } => "tool_result",
                _ => "other",
            };
            out.push(Fingerprint::new(
                "message",
                format!("#{index} ({role})"),
                &serde_json::to_value(message).unwrap_or_default(),
            ));
        }
        out
    }

    /// Record an outgoing (non-warm) request.
    pub fn begin_request(&self, request: &StreamRequest) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.fingerprints = Self::fingerprint_request(request);
        state.last_request = Some(TrackedRequest {
            model: request.model.clone(),
            thinking_level: request.thinking_level,
            session_id: request.session_id.clone(),
            cache_retention: request.cache_retention,
            messages: request.transcript.messages().to_vec(),
        });
    }

    /// After a response: hit rate + miss diagnosis vs the previous request's
    #[allow(clippy::type_complexity)]
    /// prompt size; updates fingerprints to the new request's shape is done in
    /// `begin_request`, so this compares `fingerprints` (this request) against
    /// `prev_fingerprints` (the request before it).
    pub(crate) fn complete_request(
        self: &Arc<Self>,
        request_fingerprints_before: Vec<Fingerprint>,
        usage: &Usage,
    ) {
        let prompt_tokens = usage.input_tokens + usage.cache_read_tokens + usage.cache_write_tokens;
        let (notice, idle_secs) = {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            let idle_secs = state
                .last_response_at
                .map(|t| t.elapsed().as_secs())
                .unwrap_or(0);
            let mut notice = None;
            let previous_prompt = state.prompt_tokens;
            if previous_prompt > 0 {
                let hit_rate = usage.cache_read_tokens as f64 / prompt_tokens.max(1) as f64;
                state.last_hit_rate = Some(hit_rate);
                if (usage.cache_read_tokens as f64) < 0.5 * previous_prompt as f64 {
                    let diagnosis = if let Some(first_diff) = request_fingerprints_before
                        .iter()
                        .zip(state.fingerprints.iter())
                        .find(|(a, b)| a != b)
                        .map(|(_, b)| b.describe())
                        .or_else(|| {
                            // diverged by length?
                            if request_fingerprints_before.len() != state.fingerprints.len() {
                                state
                                    .fingerprints
                                    .get(request_fingerprints_before.len())
                                    .map(|f| format!("new block: {}", f.describe()))
                            } else {
                                None
                            }
                        }) {
                        first_diff
                    } else {
                        format!(
                            "no prefix change — provider cache likely expired after {idle_secs}s"
                        )
                    };
                    state.last_diagnosis = Some(diagnosis.clone());
                    notice = Some(CacheMissNotice {
                        diagnosis,
                        hit_rate,
                        idle_secs,
                    });
                }
            }
            state.prompt_tokens = prompt_tokens;
            state.last_response_at = Some(Instant::now());
            state.idle_warm_count = 0; // a real request resets the idle-warm streak
            (notice, idle_secs)
        };
        self.schedule_warm_deadline();
        let _ = idle_secs;
        if let Some(notice) = notice {
            let notifier = self.notifier.try_lock().ok().and_then(|n| n.clone());
            let show = self
                .config
                .try_lock()
                .map(|c| c.show_miss_notices)
                .unwrap_or(false);
            if show {
                if let Some(notify) = notifier {
                    notify(notice);
                }
            }
        }
    }

    /// Arm a timer at 80% of the prompt-cache lifetime; when it fires while a
    /// tool is still running, `maybe_warm` performs the warm request. The
    /// eligibility re-check inside `maybe_warm` keeps stale timers
    /// (superseded by newer responses) from firing early.
    fn schedule_warm_deadline(self: &Arc<Self>) {
        let deadline = {
            let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            let config = self.config.try_lock();
            let (state_response, tracked_model, retention) =
                match (state.last_response_at, state.last_request.as_ref()) {
                    (Some(at), Some(tracked)) => {
                        (at, tracked.model.clone(), tracked.cache_retention)
                    }
                    _ => return,
                };
            let Ok(config) = config else { return };
            let Some(lookup) = &config.model_info else {
                return;
            };
            let Some(info) = lookup(&tracked_model) else {
                return;
            };
            let lifetime = match retention {
                oino_agent_loop::CacheRetention::Long => info.long_secs.or(info.short_secs),
                _ => info.short_secs.or(info.long_secs),
            };
            let Some(lifetime) = lifetime else { return };
            let fire_at = state_response + Duration::from_secs(lifetime * 8 / 10);
            fire_at.saturating_duration_since(Instant::now())
        };
        let this = Arc::clone(self);
        tokio::spawn(async move {
            tokio::time::sleep(deadline).await;
            this.maybe_warm(false).await;
        });
    }

    /// Called on agent events that may trigger a warm: `ToolExecutionStart`
    /// (tool running near expiry) and `Settled` (idle warm).
    pub async fn on_agent_event(&self, event: &AgentEvent) {
        match event {
            AgentEvent::ToolExecutionStart { .. } => {
                {
                    let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
                    state.tool_running = true;
                }
                self.maybe_warm(false).await;
            }
            AgentEvent::ToolExecutionEnd { .. } => {
                let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
                state.tool_running = false;
            }
            AgentEvent::Settled => {
                self.maybe_warm(true).await;
            }
            _ => {}
        }
    }

    async fn maybe_warm(&self, idle: bool) {
        enum GateStep {
            Decide,
            Payload(Box<(Model, Value, StreamRequest)>),
        }
        let step = {
            let config = self.config.lock().await;
            let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            let decide = |reason: &str, state: &mut TrackerState| -> GateStep {
                state.last_decision = Some(reason.to_string());
                GateStep::Decide
            };
            if config.mode == CacheWarmingMode::Off {
                return;
            }
            let mut state = state;
            if state.warming_in_flight {
                return;
            }
            let Some(tracked) = state.last_request.clone() else {
                return;
            };
            let Some(lookup) = &config.model_info else {
                drop(decide("no model cache info", &mut state));
                return;
            };
            let Some(info) = lookup(&tracked.model) else {
                drop(decide(
                    "model has no prompt-cache catalog entry",
                    &mut state,
                ));
                return;
            };
            let lifetime = match tracked.cache_retention {
                oino_agent_loop::CacheRetention::Long => info.long_secs.or(info.short_secs),
                _ => info.short_secs.or(info.long_secs),
            };
            let Some(lifetime) = lifetime else {
                drop(decide("model not eligible for cache warming", &mut state));
                return;
            };
            let (Some(input_price), Some(read_price)) = (info.input_price, info.cache_read_price)
            else {
                drop(decide("model has no cache pricing", &mut state));
                return;
            };
            let avoided = state.prompt_tokens as f64 * (input_price - read_price) / 1_000_000.0;
            if avoided < 0.05 {
                drop(decide(
                    &format!("avoided cost ${avoided:.4} below $0.05"),
                    &mut state,
                ));
                return;
            }
            let elapsed = state
                .last_response_at
                .map(|t| t.elapsed().as_secs())
                .unwrap_or(u64::MAX);
            if elapsed * 10 < lifetime * 8 {
                drop(decide(
                    &format!("not due yet ({elapsed}s of {lifetime}s lifetime)"),
                    &mut state,
                ));
                return;
            }
            if idle {
                if config.mode != CacheWarmingMode::ToolsAndIdle {
                    drop(decide("idle warming disabled", &mut state));
                    return;
                }
                if state.idle_warm_count >= 3 {
                    drop(decide("idle warm limit reached (3)", &mut state));
                    return;
                }
            } else if !state.tool_running {
                drop(decide("no tool running", &mut state));
                return;
            }
            let payload = serde_json::json!({
                "model": tracked.model.identifier(),
                "trigger": if idle { "idle" } else { "tool" },
                "prompt_tokens": state.prompt_tokens,
                "lifetime_secs": lifetime,
                "estimated_avoided_usd": avoided,
            });
            GateStep::Payload(Box::new((
                tracked.model.clone(),
                payload,
                StreamRequest {
                    model: tracked.model.clone(),
                    thinking_level: tracked.thinking_level,
                    transcript: oino_types::transcript::Transcript::new(tracked.messages.clone()),
                    session_id: tracked.session_id.clone(),
                    cache_retention: tracked.cache_retention,
                    max_output_tokens: Some(1),
                },
            )))
        };
        let GateStep::Payload(payload_parts) = step else {
            return;
        };
        let (tracked_model, payload, request) = *payload_parts;
        if let Some(veto) = self.veto.lock().await.clone() {
            if veto(payload).await {
                let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
                state.last_decision = Some("vetoed by cache_warming_decision hook".into());
                return;
            }
        }
        {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            state.warming_in_flight = true;
            if idle {
                state.idle_warm_count += 1;
            }
            state.last_decision = Some(format!(
                "warming ({} trigger)",
                if idle { "idle" } else { "tool" }
            ));
        }
        let _ = tracked_model;
        let provider = self.provider.clone();
        let session = self.session.lock().await.clone();
        let state_ref = Arc::clone(&self.state);
        let model = request.model.clone();
        tokio::spawn(async move {
            let outcome = provider.stream(request, AbortSignal::new()).await;
            let usage = outcome.ok().and_then(|events| {
                events.iter().rev().find_map(|event| match event {
                    AssistantStreamEvent::Usage { usage } => Some(usage.clone()),
                    AssistantStreamEvent::Done { .. } => None,
                    _ => None,
                })
            });
            if let (Some(session), Some(usage)) = (session, usage) {
                let mut session = session.lock().await;
                session.append_usage("cache_warm", model, usage);
            }
            let mut state = state_ref.lock().unwrap_or_else(|e| e.into_inner());
            state.warming_in_flight = false;
            state.last_response_at = Some(Instant::now());
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oino_agent_loop::LoopResult;
    use oino_session::{SessionHeader, SessionManager};
    use oino_types::{AssistantStreamEvent, StopReason};
    use std::path::PathBuf;
    use std::sync::Mutex as StdMutex;

    struct RecordingProvider {
        requests: StdMutex<Vec<StreamRequest>>,
        usage: oino_types::Usage,
    }

    #[async_trait::async_trait]
    impl StreamProvider for RecordingProvider {
        async fn stream(
            &self,
            request: StreamRequest,
            _signal: AbortSignal,
        ) -> LoopResult<Vec<AssistantStreamEvent>> {
            self.requests
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(request);
            Ok(vec![
                AssistantStreamEvent::TextDelta { delta: "w".into() },
                AssistantStreamEvent::Usage {
                    usage: self.usage.clone(),
                },
                AssistantStreamEvent::Done {
                    stop_reason: StopReason::EndTurn,
                    provider: None,
                },
            ])
        }
    }

    fn tracker_with_usage(usage: oino_types::Usage) -> (Arc<CacheTracker>, Arc<RecordingProvider>) {
        let provider = Arc::new(RecordingProvider {
            requests: StdMutex::new(Vec::new()),
            usage,
        });
        let tracker = Arc::new(CacheTracker::new(provider.clone()));
        (tracker, provider)
    }

    fn usage(input: u64, output: u64, read: u64, write: u64) -> oino_types::Usage {
        oino_types::Usage {
            input_tokens: input,
            output_tokens: output,
            cache_read_tokens: read,
            cache_write_tokens: write,
            cache_write_1h_tokens: None,
            reasoning_tokens: 0,
            cost: None,
        }
    }

    fn request(system: &str, user: &str) -> StreamRequest {
        StreamRequest {
            model: Model::new("anthropic", "claude-sonnet-5"),
            thinking_level: oino_types::ThinkingLevel::Off,
            transcript: oino_types::transcript::Transcript::new(vec![
                Message::System {
                    id: oino_types::OinoId::new_v4(),
                    content: system.into(),
                    sections: Vec::new(),
                    tools_added: Vec::new(),
                    tools_removed: Vec::new(),
                },
                Message::user_text(user),
            ]),
            session_id: Some("s".into()),
            cache_retention: oino_agent_loop::CacheRetention::Short,
            max_output_tokens: None,
        }
    }

    fn eligible_config() -> CacheWarmingConfig {
        CacheWarmingConfig {
            mode: CacheWarmingMode::ToolsAndIdle,
            show_miss_notices: true,
            model_info: Some(Arc::new(|model: &Model| {
                (model.name == "claude-sonnet-5").then_some(ModelCacheInfo {
                    short_secs: Some(300),
                    long_secs: Some(3600),
                    input_price: Some(3.0),
                    cache_read_price: Some(0.3),
                })
            })),
        }
    }

    /// Seed the tracker as if a request had just completed (big prompt, high
    /// cache write) so warming eligibility can trigger.
    fn seed_request(tracker: &Arc<CacheTracker>) {
        let req = request("sys", "hello");
        tracker.begin_request(&req);
        let fp = tracker.request_fingerprints(&req);
        tracker.complete_request(fp, &usage(20, 5, 0, 90000));
    }

    #[tokio::test(start_paused = true)]
    async fn miss_diagnosis_names_first_diverging_block() {
        let (tracker, _provider) = tracker_with_usage(usage(10, 5, 0, 0));
        let before = tracker.request_fingerprints(&request("sys", "hello"));
        tracker.begin_request(&request("sys", "hello"));
        tracker.complete_request(before.clone(), &usage(20, 5, 0, 90000));
        // second request with a changed system prompt → miss + system diff
        tracker.begin_request(&request("changed sys", "hello"));
        tracker.complete_request(
            tracker.request_fingerprints(&request("sys", "hello")),
            &usage(85000, 5, 0, 0),
        );
        let status = tracker.status();
        assert_eq!(
            status.last_diagnosis.as_deref(),
            Some("system prompt changed"),
            "status: {:?}",
            status.last_diagnosis
        );
        assert!(status.last_hit_rate.unwrap_or(1.0) < 0.5);
    }

    #[tokio::test(start_paused = true)]
    async fn expired_cache_diagnosis_when_prefix_unchanged() {
        let (tracker, _provider) = tracker_with_usage(usage(10, 5, 0, 0));
        let req = request("sys", "hello");
        let fp = tracker.request_fingerprints(&req);
        tracker.begin_request(&req);
        tracker.complete_request(fp.clone(), &usage(20, 5, 0, 90000));
        tracker.begin_request(&req);
        tracker.complete_request(fp, &usage(89000, 5, 0, 0));
        let status = tracker.status();
        let diagnosis = status.last_diagnosis.clone().unwrap_or_default();
        assert!(
            diagnosis.contains("provider cache likely expired"),
            "diagnosis: {diagnosis}"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn tool_trigger_warms_at_eighty_percent_of_lifetime() {
        let (tracker, provider) = tracker_with_usage(usage(10, 1, 1000, 0));
        tracker.configure(eligible_config()).await;
        seed_request(&tracker);
        // not due yet at 100s of 300s
        tokio::time::advance(Duration::from_secs(100)).await;
        tracker
            .on_agent_event(&AgentEvent::ToolExecutionStart {
                call: oino_agent_loop::ToolCall {
                    id: oino_types::OinoId::new_v4(),
                    name: "read".into(),
                    arguments: serde_json::json!({}),
                },
            })
            .await;
        tokio::task::yield_now().await;
        assert!(provider
            .requests
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_empty());
        // due at 240s+
        tokio::time::advance(Duration::from_secs(150)).await;
        tracker
            .on_agent_event(&AgentEvent::ToolExecutionStart {
                call: oino_agent_loop::ToolCall {
                    id: oino_types::OinoId::new_v4(),
                    name: "read".into(),
                    arguments: serde_json::json!({}),
                },
            })
            .await;
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
        let requests = provider.requests.lock().unwrap_or_else(|e| e.into_inner());
        assert_eq!(requests.len(), 1, "expected one warm request");
        assert_eq!(requests[0].max_output_tokens, Some(1));
        // warm request is the last-request snapshot — not appended anywhere
        assert_eq!(requests[0].transcript.messages().len(), 2);
        drop(requests);
        let status = tracker.status();
        assert!(status.next_decision.unwrap_or_default().contains("warming"));
    }

    #[tokio::test(start_paused = true)]
    async fn veto_blocks_warm_and_usage_records_on_session() {
        let (tracker, provider) = tracker_with_usage(usage(10, 1, 500, 0));
        tracker.configure(eligible_config()).await;
        let session = Arc::new(tokio::sync::Mutex::new(SessionManager::new(
            SessionHeader::new("h", PathBuf::from("/tmp")),
        )));
        tracker.set_session(session.clone()).await;
        tracker
            .set_veto(Some(Arc::new(|_payload| {
                Box::pin(async { true }) as oino_agent_loop::BoxFuture<'static, bool>
            })))
            .await;
        seed_request(&tracker);
        tokio::time::advance(Duration::from_secs(250)).await;
        tracker.on_agent_event(&AgentEvent::Settled).await;
        tokio::task::yield_now().await;
        assert!(provider
            .requests
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_empty());
        assert_eq!(
            tracker.status().next_decision.as_deref(),
            Some("vetoed by cache_warming_decision hook")
        );

        // without veto, the warm request records a cache_warm usage entry
        tracker
            .set_veto(Some(Arc::new(|_payload| {
                Box::pin(async { false }) as oino_agent_loop::BoxFuture<'static, bool>
            })))
            .await;
        tracker.on_agent_event(&AgentEvent::Settled).await;
        for _ in 0..20 {
            tokio::task::yield_now().await;
        }
        assert_eq!(
            provider
                .requests
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .len(),
            1
        );
        let entries = session.lock().await.usage_entries_on_branch();
        assert!(
            entries.iter().any(|(kind, _, _)| kind == "cache_warm"),
            "entries: {entries:?}"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn idle_warm_requires_tools_plus_idle_and_caps_at_three() {
        let (tracker, provider) = tracker_with_usage(usage(10, 1, 500, 0));
        let mut config = eligible_config();
        config.mode = CacheWarmingMode::Tools; // no idle warming
        tracker.configure(config).await;
        seed_request(&tracker);
        tokio::time::advance(Duration::from_secs(250)).await;
        tracker.on_agent_event(&AgentEvent::Settled).await;
        tokio::task::yield_now().await;
        assert!(provider
            .requests
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_empty());

        let mut config = eligible_config();
        config.mode = CacheWarmingMode::ToolsAndIdle;
        tracker.configure(config).await;
        for expected in 1..=3usize {
            tokio::time::advance(Duration::from_secs(250)).await;
            tracker.on_agent_event(&AgentEvent::Settled).await;
            for _ in 0..20 {
                tokio::task::yield_now().await;
            }
            assert_eq!(
                provider
                    .requests
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .len(),
                expected
            );
        }
        tokio::time::advance(Duration::from_secs(250)).await;
        tracker.on_agent_event(&AgentEvent::Settled).await;
        tokio::task::yield_now().await;
        assert_eq!(
            provider
                .requests
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .len(),
            3,
            "idle warm cap"
        );
        assert_eq!(
            tracker.status().next_decision.as_deref(),
            Some("idle warm limit reached (3)")
        );
    }

    #[tokio::test(start_paused = true)]
    async fn avoided_cost_below_threshold_skips_warm() {
        let (tracker, provider) = tracker_with_usage(usage(10, 1, 500, 0));
        tracker.configure(eligible_config()).await;
        let req = request("sys", "hello");
        tracker.begin_request(&req);
        // tiny prompt → avoided < $0.05
        tracker.complete_request(tracker.request_fingerprints(&req), &usage(10, 5, 0, 10));
        tokio::time::advance(Duration::from_secs(250)).await;
        tracker.on_agent_event(&AgentEvent::Settled).await;
        tokio::task::yield_now().await;
        assert!(provider
            .requests
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_empty());
        assert!(tracker
            .status()
            .next_decision
            .unwrap_or_default()
            .contains("avoided cost"));
    }
}
