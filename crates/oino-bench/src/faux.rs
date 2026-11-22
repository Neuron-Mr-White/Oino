//! Faux OpenAI-compatible streaming provider (spec §6) the host's `oino.openai-compat`
//! extension talks to over HTTP, plus "dead local server" doubles.
//!
//! The script is picked from the newest user message containing `BENCH` (extensions may
//! append their own user-role context after the prompt):
//! - `BENCH:stream secs=<T> tps=<N>`: streams ~N tokens/s for T s, then stops;
//! - `BENCH:tools n=<K>`: a tool-call loop — one `bash` call (`true`) per request until K
//!   tool results follow the prompt, then a short answer; `delay_ms=<D>` waits D ms
//!   before each tool call (model think time, so loops are subagent-paced);
//! - anything else: a short answer.
//!
//! Every request's arrival is recorded against the `[b:<tag>]` in that message, so a
//! scenario measures "send → provider request leaves the host" (T5) exactly.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use anyhow::Result;
use serde_json::{Value, json};

/// The faux model id (models.json `bench/faux-1`).
pub(crate) const MODEL: &str = "faux-1";

#[derive(Default)]
struct Log {
    /// tag → arrival instants of every request carrying it.
    arrivals: HashMap<String, Vec<Instant>>,
}

/// A running faux provider.
pub(crate) struct Faux {
    pub(crate) port: u16,
    log: Arc<(Mutex<Log>, Condvar)>,
    requests: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
}

impl Faux {
    /// Binds 127.0.0.1:0 and serves on background threads.
    ///
    /// # Errors
    ///
    /// Bind failure.
    pub(crate) fn start() -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let port = listener.local_addr()?.port();
        let log = Arc::new((Mutex::new(Log::default()), Condvar::new()));
        let requests = Arc::new(AtomicU64::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let (l, r, s) = (Arc::clone(&log), Arc::clone(&requests), Arc::clone(&stop));
        std::thread::Builder::new()
            .name("faux-accept".into())
            .spawn(move || {
                for conn in listener.incoming() {
                    if s.load(Ordering::Relaxed) {
                        break;
                    }
                    let Ok(conn) = conn else { continue };
                    let (l, r) = (Arc::clone(&l), Arc::clone(&r));
                    let _ = std::thread::Builder::new()
                        .name("faux-conn".into())
                        .spawn(move || {
                            let _ = serve(conn, &l, &r);
                        });
                }
            })?;
        Ok(Self {
            port,
            log,
            requests,
            stop,
        })
    }

    /// Total chat requests served.
    pub(crate) fn requests(&self) -> u64 {
        self.requests.load(Ordering::Relaxed)
    }

    /// Requests for one loop during a measured interval (setup rounds excluded).
    pub(crate) fn rounds(&self, tag: &str, since: Instant) -> usize {
        self.log.0.lock().map_or(0, |log| {
            log.arrivals
                .get(tag)
                .map_or(0, |v| v.iter().filter(|at| **at >= since).count())
        })
    }

    /// Raw per-loop request times relative to the measurement start.
    pub(crate) fn round_times(&self, tag: &str, start: Instant, end: Instant) -> Vec<f64> {
        self.log.0.lock().map_or_else(
            |_| Vec::new(),
            |log| {
                log.arrivals
                    .get(tag)
                    .into_iter()
                    .flatten()
                    .filter(|at| **at >= start && **at <= end)
                    .map(|at| (*at - start).as_secs_f64())
                    .collect()
            },
        )
    }

    /// The first request tagged `tag` that arrived at/after `since`.
    pub(crate) fn wait_request(
        &self,
        tag: &str,
        since: Instant,
        timeout: Duration,
    ) -> Option<Instant> {
        let (lock, cvar) = &*self.log;
        let deadline = Instant::now() + timeout;
        let mut log = lock.lock().ok()?;
        loop {
            if let Some(at) = log
                .arrivals
                .get(tag)
                .and_then(|v| v.iter().find(|at| **at >= since).copied())
            {
                return Some(at);
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return None;
            }
            log = cvar.wait_timeout(log, left).ok()?.0;
        }
    }
}

impl Drop for Faux {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = TcpStream::connect(("127.0.0.1", self.port));
    }
}

/// A "dead local server": a bound port that accepts and never answers (a wedged
/// ollama/lmstudio — every probe holds until the client timeout).
pub(crate) struct Blackhole {
    pub(crate) port: u16,
}

impl Blackhole {
    /// Binds and accepts forever, holding every connection open.
    ///
    /// # Errors
    ///
    /// Bind failure.
    pub(crate) fn start() -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let port = listener.local_addr()?.port();
        std::thread::Builder::new()
            .name("blackhole".into())
            .spawn(move || {
                let mut held = Vec::new();
                for conn in listener.incoming().flatten() {
                    held.push(conn);
                    if held.len() > 256 {
                        held.remove(0);
                    }
                }
            })?;
        Ok(Self { port })
    }
}

/// A port nothing listens on (connection refused, the common "ollama not running").
pub(crate) fn refused_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .and_then(|l| l.local_addr())
        .map_or(9, |a| a.port())
}

fn text_of(message: &Value) -> String {
    match &message["content"] {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter_map(|p| p["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

fn arg<T: std::str::FromStr>(text: &str, key: &str) -> Option<T> {
    let at = text.find(&format!("{key}="))? + key.len() + 1;
    text[at..]
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '.')
        .next()?
        .parse()
        .ok()
}

fn tag_of(text: &str) -> Option<String> {
    let at = text.find("[b:")? + 3;
    let end = text[at..].find(']')?;
    Some(text[at..at + end].to_owned())
}

enum Script {
    Stream { secs: f64, tps: f64 },
    Tool { delay_ms: u64 },
    Answer,
}

fn script(messages: &[Value]) -> (Option<String>, Script) {
    let idx = messages
        .iter()
        .rposition(|m| m["role"] == "user" && text_of(m).contains("BENCH"))
        .or_else(|| messages.iter().rposition(|m| m["role"] == "user"));
    let Some(idx) = idx else {
        return (None, Script::Answer);
    };
    let text = text_of(&messages[idx]);
    let tag = tag_of(&text);
    let tool_results = messages[idx + 1..]
        .iter()
        .filter(|m| m["role"] == "tool")
        .count();
    let after_assistant = messages[idx + 1..].iter().any(|m| m["role"] == "assistant");
    if text.contains("BENCH:tools") {
        let n: usize = arg(&text, "n").unwrap_or(1);
        let delay_ms = arg(&text, "delay_ms").unwrap_or(0);
        return (
            tag,
            if tool_results < n {
                Script::Tool { delay_ms }
            } else {
                Script::Answer
            },
        );
    }
    if text.contains("BENCH:stream") && !after_assistant {
        return (
            tag,
            Script::Stream {
                secs: arg(&text, "secs").unwrap_or(5.0),
                tps: arg(&text, "tps").unwrap_or(50.0),
            },
        );
    }
    (tag, Script::Answer)
}

fn tool_arguments(call: u64) -> String {
    json!({"command": format!("true # bench-call-{call}")}).to_string()
}

#[allow(
    clippy::too_many_lines,
    reason = "Linear measurement protocol keeps setup, observation windows, and cleanup ordered; splitting changes benchmark review context."
)]
fn serve(
    mut conn: TcpStream,
    log: &Arc<(Mutex<Log>, Condvar)>,
    requests: &AtomicU64,
) -> Result<()> {
    let mut reader = BufReader::new(conn.try_clone()?);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let post = line.starts_with("POST");
    let mut length = 0usize;
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header)? == 0 || header == "\r\n" {
            break;
        }
        if let Some((k, v)) = header.split_once(':')
            && k.eq_ignore_ascii_case("content-length")
        {
            length = v.trim().parse().unwrap_or(0);
        }
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body)?;
    if !post {
        let body =
            json!({"object": "list", "data": [{"id": MODEL, "object": "model"}]}).to_string();
        write!(
            conn,
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        )?;
        return Ok(());
    }
    let arrived = Instant::now();
    let call = requests.fetch_add(1, Ordering::Relaxed) + 1;
    let request: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    let messages = request["messages"].as_array().cloned().unwrap_or_default();
    // Side agents (titles, recaps, judges) send no tools: they never count as the turn's
    // request and always get a short answer.
    let has_tools = request["tools"].as_array().is_some_and(|t| !t.is_empty());
    let (tag, script) = if has_tools {
        script(&messages)
    } else {
        (None, Script::Answer)
    };
    if let Some(tag) = tag {
        let (lock, cvar) = &**log;
        if let Ok(mut log) = lock.lock() {
            let v = log.arrivals.entry(tag).or_default();
            v.push(arrived);
        }
        cvar.notify_all();
    }
    conn.write_all(
        b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncache-control: no-cache\r\nconnection: close\r\n\r\n",
    )?;
    let send = |conn: &mut TcpStream, chunk: Value| -> std::io::Result<()> {
        conn.write_all(format!("data: {chunk}\n\n").as_bytes())?;
        conn.flush()
    };
    let base = |delta: Value, finish: Value| {
        json!({"id": "faux", "object": "chat.completion.chunk", "model": MODEL,
               "choices": [{"index": 0, "delta": delta, "finish_reason": finish}]})
    };
    let finish = match script {
        Script::Stream { secs, tps } => {
            // Batches of tokens every 50 ms at the configured rate.
            #[allow(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "Positive rounded token rate intentionally becomes a saturating per-tick count."
            )]
            let per_tick = (tps / 20.0).max(1.0).round() as usize;
            let end = Instant::now() + Duration::from_secs_f64(secs);
            while Instant::now() < end {
                send(
                    &mut conn,
                    base(json!({"content": "tok ".repeat(per_tick)}), Value::Null),
                )?;
                std::thread::sleep(Duration::from_millis(50));
            }
            "stop"
        }
        Script::Tool { delay_ms } => {
            std::thread::sleep(Duration::from_millis(delay_ms));
            let n = call;
            send(
                &mut conn,
                base(json!({"content": "Running a step."}), Value::Null),
            )?;
            send(
                &mut conn,
                base(
                    json!({"tool_calls": [{"index": 0, "id": format!("call_{n}"), "type": "function",
                        "function": {"name": "bash", "arguments": tool_arguments(n)}}]}),
                    Value::Null,
                ),
            )?;
            "tool_calls"
        }
        Script::Answer => {
            send(&mut conn, base(json!({"content": "ok"}), Value::Null))?;
            "stop"
        }
    };
    send(&mut conn, base(json!({}), json!(finish)))?;
    send(
        &mut conn,
        json!({"id": "faux", "object": "chat.completion.chunk", "model": MODEL, "choices": [],
               "usage": {"prompt_tokens": 100, "completion_tokens": 10, "total_tokens": 110}}),
    )?;
    conn.write_all(b"data: [DONE]\n\n")?;
    conn.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Context;

    #[test]
    fn http_tool_loop_does_not_repeat_arguments() -> Result<()> {
        let faux = Faux::start()?;
        let since = Instant::now();
        let mut seen = std::collections::HashSet::new();
        for _ in 0..8 {
            let body = json!({"tools": [{"type": "function"}], "messages": [
                {"role": "user", "content": "BENCH:tools n=100 [b:http-loop]"}
            ]})
            .to_string();
            let mut conn = TcpStream::connect(("127.0.0.1", faux.port))?;
            conn.set_read_timeout(Some(Duration::from_secs(5)))?;
            write!(
                conn,
                "POST /v1/chat/completions HTTP/1.1\r\ncontent-length: {}\r\n\r\n{body}",
                body.len()
            )?;
            let mut reply = String::new();
            conn.read_to_string(&mut reply)?;
            let args = reply
                .lines()
                .filter_map(|line| line.strip_prefix("data: "))
                .filter_map(|line| serde_json::from_str::<Value>(line).ok())
                .find_map(|chunk| {
                    chunk["choices"][0]["delta"]["tool_calls"][0]["function"]["arguments"]
                        .as_str()
                        .map(str::to_owned)
                })
                .context("tool call arguments missing")?;
            assert!(
                seen.insert(args),
                "repeat-call guard would terminate this loop"
            );
        }
        assert_eq!(faux.rounds("http-loop", since), 8);
        assert_eq!(faux.requests(), 8);
        Ok(())
    }

    #[test]
    fn tool_arguments_vary_on_every_call() -> Result<()> {
        let mut seen = std::collections::HashSet::new();
        for call in 1..=100 {
            let args = tool_arguments(call);
            assert!(seen.insert(args.clone()));
            let value: Value = serde_json::from_str(&args)?;
            assert_eq!(value["command"], format!("true # bench-call-{call}"));
        }
        Ok(())
    }

    #[test]
    fn script_picks_the_newest_bench_message() {
        let msgs = vec![
            json!({"role": "user", "content": "BENCH:tools n=2 [b:x]"}),
            json!({"role": "user", "content": [{"type": "text", "text": "memory recall"}]}),
            json!({"role": "assistant", "content": "s"}),
            json!({"role": "tool", "content": "ok"}),
        ];
        let (tag, s) = script(&msgs);
        assert_eq!(tag.as_deref(), Some("x"));
        assert!(matches!(s, Script::Tool { delay_ms: 0 }));
        let mut done = msgs.clone();
        done.push(json!({"role": "assistant", "content": "s"}));
        done.push(json!({"role": "tool", "content": "ok"}));
        assert!(matches!(script(&done).1, Script::Answer));
        let (_, s) =
            script(&[json!({"role": "user", "content": "BENCH:stream secs=2 tps=40 [b:y]"})]);
        assert!(
            matches!(s, Script::Stream { secs, tps } if (secs - 2.0).abs() < 1e-9 && (tps - 40.0).abs() < 1e-9)
        );
    }
}
