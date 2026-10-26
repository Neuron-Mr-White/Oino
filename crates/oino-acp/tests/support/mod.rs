//! Test harness: an `oino-acp` agent over in-memory pipes, a scripted faux provider, a policy
//! extension raising approval dialogs, and a scripted JSON-RPC ACP client that records every
//! message and answers the agent's `fs/*`, `terminal/*`, permission and elicitation requests.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    dead_code,
    unreachable_pub,
    missing_docs,
    reason = "test support"
)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use agent_client_protocol::ByteStreams;
use futures::future::BoxFuture;
use oino_acp::{AcpAgent, AgentInfo, CoreSessionFactory, SessionDefaults, SessionFactory};
use oino_agent::{AgentToolResult, ToolInvocation, UpdateSink};
use oino_ai::{JsonObject, JsonValue};
use oino_core::extensions::api::{
    CallContext, CommandSpec, ExtensionCallError, ExtensionInstance, ExtensionMode, Registration,
};
use oino_core::extensions::host::{HostApi, HostOp};
use oino_core::services::{ExtensionLoader, ResourceFlags, ServicesOptions};
use oino_core::testing::{FauxComponent, FauxProvider, FauxResponse};
use serde_json::json;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{Notify, oneshot};
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

const TIMEOUT: Duration = Duration::from_secs(20);

/// Policy extension: approval dialog on every `bash` / `write` call; `/ask` raises a select.
struct Policy {
    host: std::sync::Weak<dyn HostApi>,
}

impl Policy {
    async fn host_call(
        &self,
        cx: &CallContext,
        op: HostOp,
    ) -> Result<JsonValue, ExtensionCallError> {
        let host = self
            .host
            .upgrade()
            .ok_or_else(|| ExtensionCallError::new("host gone"))?;
        host.call("policy", cx, op)
            .await
            .map_err(|e| ExtensionCallError::new(e.to_string()))
    }
}

fn object(value: JsonValue) -> JsonObject {
    serde_json::from_value(value).unwrap()
}

impl ExtensionInstance for Policy {
    fn id(&self) -> &'static str {
        "policy"
    }

    fn init(&self, _cx: CallContext) -> BoxFuture<'_, Result<Registration, ExtensionCallError>> {
        Box::pin(async {
            Ok(Registration {
                events: vec!["tool_call".into()],
                commands: vec![CommandSpec {
                    name: "ask".into(),
                    description: Some("Ask a question".into()),
                    hidden: false,
                    completions: false,
                }],
                ..Registration::default()
            })
        })
    }

    fn on_event(
        &self,
        kind: &str,
        payload: JsonValue,
        cx: CallContext,
    ) -> BoxFuture<'_, Result<Option<JsonValue>, ExtensionCallError>> {
        let kind = kind.to_owned();
        Box::pin(async move {
            if kind != "tool_call" || payload["toolName"] != json!("ls") {
                return Ok(None);
            }
            let answer = self
                .host_call(
                    &cx,
                    HostOp::UiDialog {
                        method: "approval".into(),
                        params: object(json!({
                            "title": "Allow ls?",
                            "toolCallId": payload["toolCallId"],
                            "toolName": "ls",
                            "summary": "List a directory",
                            "options": [
                                {"optionId": "allow", "label": "Allow", "kind": "allow_once"},
                                {"optionId": "deny", "label": "Deny", "kind": "reject_once"},
                            ],
                        })),
                        cancel: None,
                    },
                )
                .await?;
            if answer["value"]["optionId"] == json!("allow") {
                Ok(None)
            } else {
                Ok(Some(json!({"block": true, "reason": "Denied by policy"})))
            }
        })
    }

    fn run_tool(
        &self,
        tool: &str,
        _invocation: ToolInvocation,
        _updates: UpdateSink,
        _cancel: tokio_util::sync::CancellationToken,
        _cx: CallContext,
    ) -> BoxFuture<'_, Result<AgentToolResult, ExtensionCallError>> {
        let message = format!("unknown tool {tool}");
        Box::pin(async move { Err(ExtensionCallError::new(message)) })
    }

    fn run_command(
        &self,
        _name: &str,
        _args: &str,
        _cancel: tokio_util::sync::CancellationToken,
        cx: CallContext,
    ) -> BoxFuture<'_, Result<Option<JsonValue>, ExtensionCallError>> {
        Box::pin(async move {
            let answer = self
                .host_call(
                    &cx,
                    HostOp::UiDialog {
                        method: "select".into(),
                        params: object(json!({"title": "Pick one", "options": ["a", "b"]})),
                        cancel: None,
                    },
                )
                .await?;
            self.host_call(
                &cx,
                HostOp::UiFire {
                    method: "notify".into(),
                    params: object(json!({"message": format!("picked {answer}")})),
                },
            )
            .await?;
            Ok(None)
        })
    }
}

struct Loader {
    provider: Arc<FauxProvider>,
}

impl ExtensionLoader for Loader {
    fn load(
        &self,
        host: Arc<dyn HostApi>,
        _project_trusted: bool,
    ) -> BoxFuture<'_, Vec<Arc<dyn ExtensionInstance>>> {
        let provider = Arc::clone(&self.provider);
        Box::pin(async move {
            vec![
                Arc::new(FauxComponent(provider)) as Arc<dyn ExtensionInstance>,
                Arc::new(Policy {
                    host: Arc::downgrade(&host),
                }),
            ]
        })
    }
}

/// Directories and the shared provider of one test.
pub struct Fixture {
    pub dir: tempfile::TempDir,
    pub provider: Arc<FauxProvider>,
    pub factory: Arc<CoreSessionFactory>,
}

impl Fixture {
    pub fn new(responses: Vec<FauxResponse>) -> Self {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("project")).unwrap();
        let provider = FauxProvider::new(responses);
        let agent_dir = dir.path().join("agent");
        let loader_provider = Arc::clone(&provider);
        let factory = CoreSessionFactory::new(
            move |cwd: &Path| {
                let models = oino_models::ModelRegistry::in_memory(Arc::new(
                    oino_models::env::MapEnv::new(),
                ));
                models.set_runtime_api_key("faux", "test-key");
                ServicesOptions {
                    cwd: cwd.to_path_buf(),
                    agent_dir: agent_dir.clone(),
                    docs_root: agent_dir.clone(),
                    mode: ExtensionMode::Acp,
                    has_ui: true,
                    flags: ResourceFlags {
                        no_skills: true,
                        no_prompt_templates: true,
                        no_themes: true,
                        no_context_files: true,
                        ..ResourceFlags::default()
                    },
                    extension_flag_values: indexmap::IndexMap::new(),
                    loader: Arc::new(Loader {
                        provider: Arc::clone(&loader_provider),
                    }),
                    approve: Some(true),
                    models: Some(models),
                    settings: Some(oino_settings::SettingsManager::in_memory(JsonObject::new())),
                }
            },
            SessionDefaults {
                session_dir: Some(dir.path().join("sessions")),
                model: Some(FauxProvider::model()),
                tools: oino_core::runtime::ToolSelection::List(
                    ["read", "bash", "edit", "write", "ls"]
                        .map(str::to_owned)
                        .to_vec(),
                ),
                ..SessionDefaults::default()
            },
        );
        Self {
            dir,
            provider,
            factory: Arc::new(factory),
        }
    }

    pub fn cwd(&self) -> PathBuf {
        self.dir.path().join("project")
    }

    /// Starts an agent and a client with `caps` (`clientCapabilities`).
    pub fn connect(&self, policy: ClientPolicy) -> TestClient {
        let (client_io, agent_io) = tokio::io::duplex(1 << 20);
        let (agent_read, agent_write) = tokio::io::split(agent_io);
        let agent = AcpAgent::new(
            Arc::clone(&self.factory) as Arc<dyn SessionFactory>,
            AgentInfo::default(),
            self.cwd(),
        );
        let task = tokio::spawn(async move {
            let transport = ByteStreams::new(agent_write.compat_write(), agent_read.compat());
            let _ = agent.serve(transport).await;
        });
        TestClient::start(client_io, policy, task)
    }
}

/// How the scripted client answers the agent.
#[derive(Clone, Debug, Default)]
pub struct ClientPolicy {
    /// `clientCapabilities` sent in `initialize`.
    pub caps: JsonValue,
    /// Permission option ids to select, in order (then `cancelled`).
    pub permissions: Vec<String>,
    /// Elicitation answers (`content.value`), in order (then `cancel`).
    pub elicitations: Vec<JsonValue>,
}

#[derive(Default)]
struct Terminal {
    output: String,
    code: i32,
}

#[derive(Default)]
struct Inbox {
    /// Every message the agent sent (requests, notifications), in order.
    messages: Vec<JsonValue>,
    pending: HashMap<u64, oneshot::Sender<JsonValue>>,
    terminals: HashMap<String, Terminal>,
    policy: ClientPolicy,
}

pub struct TestClient {
    writer: Arc<tokio::sync::Mutex<tokio::io::WriteHalf<tokio::io::DuplexStream>>>,
    inbox: Arc<Mutex<Inbox>>,
    changed: Arc<Notify>,
    next_id: Mutex<u64>,
    agent: tokio::task::JoinHandle<()>,
    reader: tokio::task::JoinHandle<()>,
}

async fn write_line(
    writer: &tokio::sync::Mutex<tokio::io::WriteHalf<tokio::io::DuplexStream>>,
    value: &JsonValue,
) {
    let mut line = serde_json::to_string(value).unwrap();
    line.push('\n');
    let mut writer = writer.lock().await;
    writer.write_all(line.as_bytes()).await.unwrap();
    writer.flush().await.unwrap();
}

/// The client side of the agent's requests.
fn answer(inbox: &Mutex<Inbox>, method: &str, params: &JsonValue) -> JsonValue {
    let mut inbox = inbox.lock().unwrap();
    match method {
        "fs/read_text_file" => {
            let content = std::fs::read_to_string(params["path"].as_str().unwrap()).unwrap();
            json!({"result": {"content": content}})
        }
        "fs/write_text_file" => {
            std::fs::write(
                params["path"].as_str().unwrap(),
                params["content"].as_str().unwrap(),
            )
            .unwrap();
            json!({"result": {}})
        }
        "session/request_permission" => {
            let outcome = if inbox.policy.permissions.is_empty() {
                json!({"outcome": "cancelled"})
            } else {
                let id = inbox.policy.permissions.remove(0);
                json!({"outcome": "selected", "optionId": id})
            };
            json!({"result": {"outcome": outcome}})
        }
        "elicitation/create" => {
            if inbox.policy.elicitations.is_empty() {
                json!({"result": {"action": "cancel"}})
            } else {
                let value = inbox.policy.elicitations.remove(0);
                json!({"result": {"action": "accept", "content": {"value": value}}})
            }
        }
        "terminal/create" => {
            let command = params["command"].as_str().unwrap().to_owned();
            let args: Vec<String> = params["args"]
                .as_array()
                .unwrap()
                .iter()
                .map(|a| a.as_str().unwrap().to_owned())
                .collect();
            let output = std::process::Command::new(&command)
                .args(&args)
                .current_dir(params["cwd"].as_str().unwrap_or("/"))
                .output()
                .unwrap();
            let id = format!("term-{}", inbox.terminals.len() + 1);
            let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
            text.push_str(&String::from_utf8_lossy(&output.stderr));
            inbox.terminals.insert(
                id.clone(),
                Terminal {
                    output: text,
                    code: output.status.code().unwrap_or(1),
                },
            );
            json!({"result": {"terminalId": id}})
        }
        "terminal/output" => {
            let terminal = &inbox.terminals[params["terminalId"].as_str().unwrap()];
            json!({"result": {"output": terminal.output, "truncated": false,
                              "exitStatus": {"exitCode": terminal.code}}})
        }
        "terminal/wait_for_exit" => {
            let terminal = &inbox.terminals[params["terminalId"].as_str().unwrap()];
            json!({"result": {"exitCode": terminal.code}})
        }
        "terminal/kill" | "terminal/release" => json!({"result": {}}),
        other => json!({"error": {"code": -32601, "message": format!("no {other}")}}),
    }
}

impl TestClient {
    fn start(
        io: tokio::io::DuplexStream,
        policy: ClientPolicy,
        agent: tokio::task::JoinHandle<()>,
    ) -> Self {
        let (read, write) = tokio::io::split(io);
        let writer = Arc::new(tokio::sync::Mutex::new(write));
        let inbox = Arc::new(Mutex::new(Inbox {
            policy,
            ..Inbox::default()
        }));
        let changed = Arc::new(Notify::new());
        let reader = {
            let (writer, inbox, changed) = (
                Arc::clone(&writer),
                Arc::clone(&inbox),
                Arc::clone(&changed),
            );
            tokio::spawn(async move {
                let mut lines = BufReader::new(read).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let message: JsonValue = serde_json::from_str(&line).unwrap();
                    let method = message.get("method").and_then(JsonValue::as_str);
                    match (method, message.get("id")) {
                        (None, Some(id)) => {
                            let sender =
                                inbox.lock().unwrap().pending.remove(&id.as_u64().unwrap());
                            if let Some(sender) = sender {
                                let _ = sender.send(message);
                            }
                        }
                        (Some(method), id) => {
                            inbox.lock().unwrap().messages.push(message.clone());
                            if let Some(id) = id {
                                let mut reply = answer(&inbox, method, &message["params"]);
                                reply["jsonrpc"] = json!("2.0");
                                reply["id"] = id.clone();
                                write_line(&writer, &reply).await;
                            }
                        }
                        (None, None) => {}
                    }
                    changed.notify_waiters();
                }
            })
        };
        Self {
            writer,
            inbox,
            changed,
            next_id: Mutex::new(0),
            agent,
            reader,
        }
    }

    /// Sends a request; returns the whole response (`result` or `error`).
    pub async fn call(&self, method: &str, params: JsonValue) -> JsonValue {
        let id = {
            let mut next = self.next_id.lock().unwrap();
            *next += 1;
            *next
        };
        let (tx, rx) = oneshot::channel();
        self.inbox.lock().unwrap().pending.insert(id, tx);
        write_line(
            &self.writer,
            &json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}),
        )
        .await;
        tokio::time::timeout(TIMEOUT, rx)
            .await
            .unwrap_or_else(|_| panic!("{method} timed out"))
            .unwrap()
    }

    /// [`TestClient::call`] expecting success; returns `result`.
    pub async fn ok(&self, method: &str, params: JsonValue) -> JsonValue {
        let response = self.call(method, params).await;
        assert!(
            response.get("error").is_none(),
            "{method} failed: {response}"
        );
        response["result"].clone()
    }

    pub async fn notify(&self, method: &str, params: JsonValue) {
        write_line(
            &self.writer,
            &json!({"jsonrpc": "2.0", "method": method, "params": params}),
        )
        .await;
    }

    /// `initialize` with the policy's capabilities.
    pub async fn initialize(&self) -> JsonValue {
        let caps = self.inbox.lock().unwrap().policy.caps.clone();
        self.ok(
            "initialize",
            json!({"protocolVersion": 1, "clientCapabilities": caps,
                   "clientInfo": {"name": "test", "version": "1"}}),
        )
        .await
    }

    /// `session/new` in `cwd`; returns the result.
    pub async fn new_session(&self, cwd: &Path) -> JsonValue {
        self.ok("session/new", json!({"cwd": cwd, "mcpServers": []}))
            .await
    }

    /// Every message from the agent so far.
    pub fn messages(&self) -> Vec<JsonValue> {
        self.inbox.lock().unwrap().messages.clone()
    }

    /// `session/update` payloads (`params.update`) so far.
    pub fn updates(&self) -> Vec<JsonValue> {
        self.messages()
            .into_iter()
            .filter(|m| m["method"] == json!("session/update"))
            .map(|m| m["params"]["update"].clone())
            .collect()
    }

    /// Updates of one `sessionUpdate` kind.
    pub fn updates_of(&self, kind: &str) -> Vec<JsonValue> {
        self.updates()
            .into_iter()
            .filter(|u| u["sessionUpdate"] == json!(kind))
            .collect()
    }

    /// Requests the agent sent with `method`.
    pub fn requests(&self, method: &str) -> Vec<JsonValue> {
        self.messages()
            .into_iter()
            .filter(|m| m["method"] == json!(method) && m.get("id").is_some())
            .collect()
    }

    /// Waits until `predicate` holds over the messages.
    pub async fn wait_for(&self, what: &str, predicate: impl Fn(&[JsonValue]) -> bool) {
        let deadline = tokio::time::Instant::now() + TIMEOUT;
        loop {
            let notified = self.changed.notified();
            if predicate(&self.messages()) {
                return;
            }
            let woke = tokio::time::timeout_at(deadline, notified).await.is_ok();
            assert!(woke, "timed out waiting for {what}: {:#?}", self.messages());
        }
    }

    /// Text of all `agent_message_chunk`s.
    pub fn agent_text(&self) -> String {
        self.updates_of("agent_message_chunk")
            .iter()
            .map(|u| u["content"]["text"].as_str().unwrap_or_default().to_owned())
            .collect()
    }

    /// Closes the connection and waits for the agent to dispose its sessions.
    pub async fn close(self) {
        self.writer.lock().await.shutdown().await.unwrap();
        drop(self.writer);
        let _ = tokio::time::timeout(TIMEOUT, self.agent).await;
        self.reader.abort();
    }
}

/// A tool call response for the faux provider.
pub fn tool_call(id: &str, name: &str, args: JsonValue) -> FauxResponse {
    FauxResponse::ToolCalls(vec![(id.into(), name.into(), object(args))])
}
