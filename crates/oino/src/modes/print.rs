//! pi `print-mode.ts`: `-p` text output and `--mode json` event streams.

use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use oino_ai::{AgentMessage, AssistantContent, StopReason};
use oino_core::events::{AgentSessionEvent, ClientEvent};
use oino_core::runtime::AgentSessionRuntime;
use oino_core::session::{PromptDisposition, PromptOptions};
use tokio::io::AsyncWriteExt;
use tokio::sync::watch;

use crate::cli::files::InitialMessage;
use crate::modes::signals;

/// Output flavour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrintOutput {
    /// Final assistant text.
    Text,
    /// Header + every session event as JSON lines.
    Json,
}

/// What to send.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PrintInput {
    /// First prompt (stdin + `@file`s + first message).
    pub initial: InitialMessage,
    /// Remaining positional messages, one prompt each.
    pub messages: Vec<String>,
}

fn stop_reason_name(reason: StopReason) -> String {
    serde_json::to_value(reason)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}

/// Forwards session events (JSON mode) and extension errors (both modes) until the hub
/// closes.
async fn forward(
    mut events: oino_core::hub::EventStream,
    output: PrintOutput,
    settled: watch::Sender<u64>,
) {
    let mut stdout = tokio::io::stdout();
    while let Some(event) = events.next().await {
        if matches!(
            event,
            ClientEvent::Session(AgentSessionEvent::AgentSettled { .. })
        ) {
            settled.send_modify(|n| *n += 1);
        }
        match event {
            ClientEvent::Session(event) if output == PrintOutput::Json => {
                let line =
                    oino_rpc::jsonl::serialize_line(&oino_rpc::wire::session_event_json(&event));
                if stdout.write_all(line.as_bytes()).await.is_err() {
                    return;
                }
                let _ = stdout.flush().await;
            }
            ClientEvent::ExtensionError(error) => {
                crate::eprintln_line(&format!(
                    "Extension error ({}): {}",
                    error.error.extension_path, error.error.error
                ));
            }
            _ => {}
        }
    }
}

/// How long to wait for a started run's `agent_settled` to reach the output after the
/// session went idle (the runtime forwards session events on its own task).
const SETTLE_GRACE: Duration = Duration::from_secs(5);

async fn prompts(
    runtime: &AgentSessionRuntime,
    input: PrintInput,
    mut settled: watch::Receiver<u64>,
) -> Result<(), String> {
    let mut queue: Vec<(String, Vec<oino_ai::ImageContent>)> = Vec::new();
    if let Some(text) = input.initial.text {
        queue.push((text, input.initial.images));
    }
    queue.extend(input.messages.into_iter().map(|m| (m, Vec::new())));
    for (text, images) in queue {
        let session = runtime.session();
        let before = *settled.borrow_and_update();
        let disposition = session
            .prompt(
                text,
                PromptOptions {
                    images,
                    ..PromptOptions::default()
                },
            )
            .await
            .map_err(|e| e.to_string())?;
        session.wait_for_idle().await;
        runtime.session().wait_for_idle().await;
        if disposition == PromptDisposition::Started {
            let _ = tokio::time::timeout(SETTLE_GRACE, settled.wait_for(|n| *n > before)).await;
        }
    }
    Ok(())
}

/// The text-mode result: stdout text and exit code (pi rules).
fn text_result(messages: &[AgentMessage]) -> (String, Option<String>, i32) {
    let Some(AgentMessage::Assistant(last)) = messages.last() else {
        return (String::new(), None, 0);
    };
    if matches!(last.stop_reason, StopReason::Error | StopReason::Aborted) {
        let message = last
            .error_message
            .clone()
            .filter(|m| !m.is_empty())
            .unwrap_or_else(|| format!("Request {}", stop_reason_name(last.stop_reason)));
        return (String::new(), Some(message), 1);
    }
    let mut out = String::new();
    for block in &last.content {
        if let AssistantContent::Text(text) = block {
            out.push_str(&text.text);
            out.push('\n');
        }
    }
    if last.stop_reason == StopReason::Length {
        return (
            out,
            Some("Response was truncated: the model reached its output token limit.".to_owned()),
            1,
        );
    }
    (out, None, 0)
}

/// Runs print/JSON mode; returns the exit code.
pub async fn run(runtime: Arc<AgentSessionRuntime>, output: PrintOutput, input: PrintInput) -> i32 {
    let mut stdout = tokio::io::stdout();
    if output == PrintOutput::Json {
        let header = runtime
            .session()
            .with_manager(|m| serde_json::to_value(m.header()).ok());
        if let Some(header) = header {
            let line = oino_rpc::jsonl::serialize_line(&header);
            let _ = stdout.write_all(line.as_bytes()).await;
            let _ = stdout.flush().await;
        }
    }
    let (settled, settled_rx) = watch::channel(0);
    let forwarder = tokio::spawn(forward(runtime.subscribe(), output, settled));
    let outcome = tokio::select! {
        result = prompts(&runtime, input, settled_rx) => Ok(result),
        signal = signals::next() => Err(signal),
    };
    let code = match outcome {
        Err(signal) => signal.exit_code(),
        Ok(Err(message)) => {
            crate::eprintln_line(&message);
            1
        }
        Ok(Ok(())) if output == PrintOutput::Text => {
            let (text, error, code) = text_result(&runtime.session().messages());
            let _ = stdout.write_all(text.as_bytes()).await;
            let _ = stdout.flush().await;
            if let Some(error) = error {
                crate::eprintln_line(&error);
            }
            code
        }
        Ok(Ok(())) => 0,
    };
    runtime.dispose().await;
    let _ = forwarder.await;
    let _ = stdout.flush().await;
    code
}

#[cfg(test)]
mod tests {
    use super::*;
    use oino_ai::AssistantMessage;

    fn assistant(stop: StopReason, text: &str, error: Option<&str>) -> Vec<AgentMessage> {
        let mut message = AssistantMessage::empty("faux", "faux", "m", 1);
        message.stop_reason = stop;
        message.content = vec![AssistantContent::text(text)];
        message.error_message = error.map(str::to_owned);
        vec![AgentMessage::Assistant(message)]
    }

    #[test]
    fn exit_rules() {
        assert_eq!(
            text_result(&assistant(StopReason::Stop, "hi", None)),
            ("hi\n".into(), None, 0)
        );
        assert_eq!(
            text_result(&assistant(StopReason::Error, "x", Some("boom"))),
            (String::new(), Some("boom".into()), 1)
        );
        assert_eq!(
            text_result(&assistant(StopReason::Aborted, "x", None)).1,
            Some("Request aborted".into())
        );
        let (out, error, code) = text_result(&assistant(StopReason::Length, "part", None));
        assert_eq!((out.as_str(), code), ("part\n", 1));
        assert_eq!(
            error.as_deref(),
            Some("Response was truncated: the model reached its output token limit.")
        );
        assert_eq!(text_result(&[]), (String::new(), None, 0));
    }
}
