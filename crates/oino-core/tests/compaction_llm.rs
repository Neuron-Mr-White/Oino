//! Golden tests for the summary requests of `compact` / `generate_branch_summary` against pi
//! 1.4.0: exact request context and options (through a recording `StreamFn`), results,
//! failures and retry callbacks. Fixtures: `scripts/pi-oracle/core-runtime/compaction/`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests assert by unwrapping"
)]

use std::sync::{Arc, Mutex};

use futures::stream::{self, BoxStream, StreamExt as _};
use oino_agent::{StreamFn, StreamRequest};
use oino_ai::json::json_semantic_eq;
use oino_ai::{
    AgentMessage, AssistantMessage, AssistantMessageEvent, DoneReason, ErrorReason, Message, Model,
    StopReason,
};
use oino_core::compaction::{
    BranchSummaryOptions, CompactionPreparation, RetryPolicy, SummarizationRetryObserver,
    Summarizer, compact, generate_branch_summary, summarize_default,
};
use oino_session::{EntryId, EntryIndex, SessionEntry};
use serde_json::{Value, json};

fn fixture(name: &str) -> Value {
    let path = format!(
        "{}/tests/fixtures/compaction/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn assert_json(label: &str, actual: &Value, expected: &Value) {
    assert!(
        json_semantic_eq(actual, expected),
        "{label}\n  actual:   {actual}\n  expected: {expected}"
    );
}

/// Replays scripted replies and records each request in pi's oracle shape.
struct Recorder {
    replies: Vec<AssistantMessage>,
    requests: Mutex<Vec<Value>>,
}

impl Recorder {
    fn new(replies: Vec<AssistantMessage>) -> Arc<Self> {
        Arc::new(Self {
            replies,
            requests: Mutex::new(Vec::new()),
        })
    }

    fn requests(&self) -> Value {
        Value::Array(self.requests.lock().unwrap().clone())
    }
}

fn scrub_request(request: &StreamRequest) -> Value {
    let messages: Vec<Value> = request
        .context
        .messages
        .iter()
        .map(|message| {
            let mut value = serde_json::to_value(message).unwrap();
            if matches!(message, Message::User(_)) {
                value["timestamp"] = json!(0);
            }
            value
        })
        .collect();
    let mut options = serde_json::to_value(&request.options).unwrap();
    if let Some(key) = &request.api_key {
        options["apiKey"] = json!(key);
    }
    let session_id = options["sessionId"].as_str().unwrap().to_owned();
    if session_id != "route-1" {
        assert_eq!(session_id.len(), 36, "uuidv7 routing id {session_id}");
        assert_eq!(&session_id[14..15], "7");
        options["sessionId"] = json!("<uuidv7>");
    }
    json!({"model": request.model.id, "context": {"messages": messages}, "options": options})
}

impl StreamFn for Recorder {
    fn stream(&self, request: StreamRequest) -> BoxStream<'static, AssistantMessageEvent> {
        let mut requests = self.requests.lock().unwrap();
        let reply = self.replies[requests.len().min(self.replies.len() - 1)].clone();
        requests.push(scrub_request(&request));
        let event = match reply.stop_reason {
            StopReason::Error => AssistantMessageEvent::Error {
                reason: ErrorReason::Error,
                error: reply,
            },
            StopReason::Aborted => AssistantMessageEvent::Error {
                reason: ErrorReason::Aborted,
                error: reply,
            },
            StopReason::Length => AssistantMessageEvent::Done {
                reason: DoneReason::Length,
                message: reply,
            },
            _ => AssistantMessageEvent::Done {
                reason: DoneReason::Stop,
                message: reply,
            },
        };
        stream::iter([event]).boxed()
    }
}

#[derive(Default)]
struct Events(Mutex<Vec<Value>>);

impl SummarizationRetryObserver for Events {
    fn retry_scheduled(&self, attempt: u32, max_attempts: u32, delay_ms: u64, error_message: &str) {
        self.0
            .lock()
            .unwrap()
            .push(json!({"type": "scheduled", "attempt": attempt,
            "maxAttempts": max_attempts, "delayMs": delay_ms, "errorMessage": error_message}));
    }
    fn retry_attempt_start(&self) {
        self.0
            .lock()
            .unwrap()
            .push(json!({"type": "attempt_start"}));
    }
    fn retry_finished(&self, success: bool, attempt: u32, final_error: Option<&str>) {
        let mut event = json!({"type": "finished", "success": success, "attempt": attempt});
        if let Some(error) = final_error {
            event["finalError"] = json!(error);
        }
        self.0.lock().unwrap().push(event);
    }
}

fn summarizer(case: &Value, recorder: Arc<Recorder>, events: Arc<Events>) -> Summarizer {
    let model: Model = serde_json::from_value(case["model"].clone()).unwrap();
    let mut summarizer = Summarizer::new(recorder, model);
    summarizer.api_key = case["apiKey"].as_str().map(str::to_owned);
    summarizer.headers = serde_json::from_value(case["headers"].clone()).unwrap();
    summarizer.thinking_level = serde_json::from_value(case["thinkingLevel"].clone()).unwrap();
    summarizer.session_id = case["sessionId"].as_str().map(str::to_owned);
    summarizer.retry =
        serde_json::from_value::<Option<RetryPolicy>>(case["retryPolicy"].clone()).unwrap();
    summarizer.observer = Some(events);
    summarizer
}

fn replies(case: &Value) -> Vec<AssistantMessage> {
    serde_json::from_value(case["replies"].clone()).unwrap()
}

#[tokio::test]
async fn compact_requests_match_pi() {
    let llm = fixture("llm.json");
    for case in llm["compact"].as_array().unwrap() {
        let label = case["label"].as_str().unwrap();
        let preparation: CompactionPreparation =
            serde_json::from_value(case["preparation"].clone()).unwrap();
        let recorder = Recorder::new(replies(case));
        let events = Arc::new(Events::default());
        let summarizer = summarizer(case, recorder.clone(), events.clone());
        let result = compact(
            &preparation,
            &summarizer,
            case["customInstructions"].as_str(),
        )
        .await;
        let requests = recorder.requests();
        let expected = &case["requests"];
        assert_eq!(
            requests.as_array().unwrap().len(),
            expected.as_array().unwrap().len(),
            "{label} request count"
        );
        for (index, (actual, wanted)) in requests
            .as_array()
            .unwrap()
            .iter()
            .zip(expected.as_array().unwrap())
            .enumerate()
        {
            assert_json(&format!("{label} request {index}"), actual, wanted);
        }
        assert_json(
            &format!("{label} retry events"),
            &Value::Array(events.0.lock().unwrap().clone()),
            &case["events"],
        );
        match result {
            Ok(result) => {
                assert!(
                    case["error"].is_null(),
                    "{label} expected {}",
                    case["error"]
                );
                assert_json(
                    &format!("{label} result"),
                    &serde_json::to_value(&result).unwrap(),
                    &case["result"],
                );
            }
            Err(error) => assert_eq!(
                error.to_string(),
                case["error"].as_str().unwrap_or("<success>"),
                "{label} error"
            ),
        }
    }
}

#[tokio::test]
async fn branch_summary_requests_match_pi() {
    let llm = fixture("llm.json");
    let sessions = fixture("sessions.json");
    for case in llm["branch"].as_array().unwrap() {
        let label = case["label"].as_str().unwrap();
        let entries: Vec<SessionEntry> =
            serde_json::from_value(sessions[case["session"].as_str().unwrap()]["entries"].clone())
                .unwrap();
        let index = EntryIndex::new(entries);
        let entries: Vec<SessionEntry> = case["entryIds"]
            .as_array()
            .unwrap()
            .iter()
            .map(|id| {
                index
                    .get(&EntryId::new(id.as_str().unwrap()))
                    .unwrap()
                    .clone()
            })
            .collect();
        let recorder = Recorder::new(replies(case));
        let events = Arc::new(Events::default());
        let mut summarizer = summarizer(case, recorder.clone(), events.clone());
        summarizer.api_key = Some("key-b".to_owned());
        let options = BranchSummaryOptions {
            custom_instructions: case["customInstructions"].as_str().map(str::to_owned),
            replace_instructions: case["replaceInstructions"].as_bool().unwrap_or(false),
            reserve_tokens: case["reserveTokens"].as_u64().unwrap_or(16384),
        };
        let result = generate_branch_summary(&entries, &summarizer, &options).await;
        assert_json(
            &format!("{label} requests"),
            &recorder.requests(),
            &case["requests"],
        );
        assert_json(
            &format!("{label} retry events"),
            &Value::Array(events.0.lock().unwrap().clone()),
            &case["events"],
        );
        assert_json(
            &format!("{label} result"),
            &serde_json::to_value(&result).unwrap(),
            &case["result"],
        );
    }
}

#[tokio::test]
async fn summarize_default_uses_the_first_compaction_prompt() {
    let llm = fixture("llm.json");
    let case = &llm["compact"][0];
    assert_eq!(case["label"], "simple");
    let preparation: CompactionPreparation =
        serde_json::from_value(case["preparation"].clone()).unwrap();
    let recorder = Recorder::new(replies(case));
    let summarizer = summarizer(case, recorder.clone(), Arc::new(Events::default()));
    let output = summarize_default(&preparation.messages_to_summarize, None, None, &summarizer)
        .await
        .unwrap();
    assert_eq!(output.summary, "SUMMARY");
    assert_json("request", &recorder.requests()[0], &case["requests"][0]);
}

/// pi `generateSummary(…, previousSummary)`: the update prompt plus a `<previous-summary>`
/// block instead of a fresh summary.
#[tokio::test]
async fn summarize_default_updates_a_previous_summary() {
    let llm = fixture("llm.json");
    let case = &llm["compact"][0];
    let preparation: CompactionPreparation =
        serde_json::from_value(case["preparation"].clone()).unwrap();
    let recorder = Recorder::new(replies(case));
    let summarizer = summarizer(case, recorder.clone(), Arc::new(Events::default()));
    summarize_default(
        &preparation.messages_to_summarize,
        None,
        Some("OLD SUMMARY"),
        &summarizer,
    )
    .await
    .unwrap();
    let request = &recorder.requests()[0];
    let text = request["context"]["messages"]
        .as_array()
        .and_then(|messages| {
            messages
                .iter()
                .find(|message| message["role"] == json!("user"))
                .and_then(|message| message["content"][0]["text"].as_str())
                .map(ToOwned::to_owned)
        })
        .unwrap_or_default();
    assert!(
        text.contains("<previous-summary>\nOLD SUMMARY\n</previous-summary>"),
        "{text}"
    );
    assert!(
        text.contains("Update") || text.contains("update"),
        "update prompt expected: {text}"
    );
    assert!(
        !text.contains("Additional focus"),
        "no instructions were passed: {text}"
    );
}

/// Never answers; used to check cancellation during a retry backoff.
struct FailingStream;

impl StreamFn for FailingStream {
    fn stream(&self, _request: StreamRequest) -> BoxStream<'static, AssistantMessageEvent> {
        let mut error = AssistantMessage::empty("faux", "faux", "sum-1", 0);
        error.stop_reason = StopReason::Error;
        error.error_message = Some("overloaded".to_owned());
        stream::iter([AssistantMessageEvent::Error {
            reason: ErrorReason::Error,
            error,
        }])
        .boxed()
    }
}

#[tokio::test(start_paused = true)]
async fn cancelling_a_backoff_aborts_without_error_text() {
    let model: Model = serde_json::from_value(json!({"id":"sum-1","name":"s","api":"faux",
        "provider":"faux","contextWindow":1000,"maxTokens":100}))
    .unwrap();
    let events = Arc::new(Events::default());
    let mut summarizer = Summarizer::new(Arc::new(FailingStream), model);
    summarizer.retry = Some(RetryPolicy {
        enabled: true,
        max_retries: 3,
        base_delay_ms: 60_000,
        max_agent_delay_ms: None,
    });
    summarizer.observer = Some(events.clone());
    let cancel = summarizer.cancel.clone();
    let task = tokio::spawn(async move {
        summarizer
            .complete(
                oino_core::compaction::summarization_context("x"),
                oino_ai::StreamOptions::default(),
            )
            .await
    });
    tokio::task::yield_now().await;
    cancel.cancel();
    let response = task.await.unwrap();
    assert_eq!(response.stop_reason, StopReason::Aborted);
    assert_eq!(response.error_message, None);
    assert_json(
        "events",
        &Value::Array(events.0.lock().unwrap().clone()),
        &json!([
            {"type":"scheduled","attempt":1,"maxAttempts":3,"delayMs":60000,"errorMessage":"overloaded"},
            {"type":"finished","success":false,"attempt":1,"finalError":"overloaded"},
        ]),
    );
}

#[test]
fn preparation_serializes_like_pi() {
    let llm = fixture("llm.json");
    let raw = &llm["compact"][0]["preparation"];
    let preparation: CompactionPreparation = serde_json::from_value(raw.clone()).unwrap();
    assert_json(
        "round trip",
        &serde_json::to_value(&preparation).unwrap(),
        raw,
    );
    assert!(
        preparation
            .messages_to_summarize
            .iter()
            .all(|message| !matches!(message, AgentMessage::System(_)))
    );
}
