//! `models`, `ui`, `agents`, `tasks`, `bus`, `services`, `secrets` (delegated to
//! `HostServices` after permission checks).

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use oino_extension_api::ops::{
    DeviceOp, EditorOp, ModelsOp, ModelsReply, PanelOp, SecretOp, TaskOp,
};
use oino_extension_api::{CallContext, ExtensionId, HostError, RawJson};
use wasmtime::component::{Access, Accessor, Resource, StreamReader};

use super::{out, ready, ready_wit};
use crate::bindings::oino::extension::{
    agents as wa, bus as wb, models as wm, secrets as wsec, services as wsv, tasks as wta,
    types as wt, ui as wu,
};
use crate::convert::{self, json};
use crate::resources::{AgentRes, CancelToken};
use crate::state::{ExtShared, HostState, current, current_async};
use crate::streams::ItemProducer;

type HostResult<T> = wasmtime::Result<Result<T, wt::HostError>>;

fn token<T>(
    accessor: &Accessor<T, HostState>,
    res: Option<Resource<CancelToken>>,
) -> wasmtime::Result<Option<oino_extension_api::CancellationToken>> {
    match res {
        Some(res) => {
            Ok(Some(accessor.with(|mut a| {
                a.get().table.get(&res).map(|t| t.0.clone())
            })?))
        }
        None => Ok(None),
    }
}

// ── models ───────────────────────────────────────────────────────────────

async fn models(
    ext: &Arc<ExtShared>,
    cx: &CallContext,
    op: ModelsOp,
) -> Result<ModelsReply, HostError> {
    match &op {
        ModelsOp::CredentialFor(p) => ExtShared::require(
            ext.perms().allows_credential(p),
            &format!("credentials:{p}"),
        )?,
        _ => ExtShared::require(ext.perms().models.complete, "models.complete")?,
    }
    ext.host.models(&ext.id, cx, op).await
}

fn reply_json(r: Result<ModelsReply, HostError>) -> Result<String, HostError> {
    match r? {
        ModelsReply::Json(j) => Ok(out(&j)),
        other => Err(HostError::Failed(format!(
            "unexpected models reply {other:?}"
        ))),
    }
}

impl wm::Host for HostState {}

impl<T: Send> wm::HostWithStore<T> for HostState {
    async fn complete(
        accessor: &Accessor<T, Self>,
        request: String,
        cancel: Option<Resource<CancelToken>>,
    ) -> HostResult<String> {
        let (ext, cx) = current_async(accessor);
        let cancel = token(accessor, cancel)?;
        let r = match json(request) {
            Ok(request) => {
                reply_json(models(&ext, &cx, ModelsOp::Complete { request, cancel }).await)
            }
            Err(e) => Err(e),
        };
        Ok(convert::wit(r))
    }

    async fn stream(
        accessor: &Accessor<T, Self>,
        request: String,
    ) -> HostResult<StreamReader<String>> {
        let (ext, cx) = current_async(accessor);
        let r = match json(request) {
            Ok(request) => models(&ext, &cx, ModelsOp::Stream { request }).await,
            Err(e) => Err(e),
        };
        match r {
            Ok(ModelsReply::Stream(s)) => {
                Ok(Ok(accessor.with(|mut a| {
                    StreamReader::new(&mut a, ItemProducer::new(s))
                })?))
            }
            Ok(other) => Ok(Err(convert::host_error(HostError::Failed(format!(
                "unexpected models reply {other:?}"
            ))))),
            Err(e) => Ok(Err(convert::host_error(e))),
        }
    }

    async fn summarize_default(
        accessor: &Accessor<T, Self>,
        messages: String,
        custom_instructions: Option<String>,
    ) -> HostResult<String> {
        let (ext, cx) = current_async(accessor);
        let r = match json(messages) {
            Ok(messages) => reply_json(
                models(
                    &ext,
                    &cx,
                    ModelsOp::SummarizeDefault {
                        messages,
                        custom_instructions,
                    },
                )
                .await,
            ),
            Err(e) => Err(e),
        };
        Ok(convert::wit(r))
    }

    async fn credential_for(
        accessor: &Accessor<T, Self>,
        provider_or_base_url: String,
    ) -> HostResult<Option<wm::Credential>> {
        let (ext, cx) = current_async(accessor);
        let r = models(&ext, &cx, ModelsOp::CredentialFor(provider_or_base_url))
            .await
            .and_then(|reply| match reply {
                ModelsReply::Credential(c) => Ok(c.map(|c| wm::Credential {
                    headers: c.headers,
                    base_url: c.base_url,
                    provider: c.provider,
                })),
                other => Err(HostError::Failed(format!(
                    "unexpected models reply {other:?}"
                ))),
            });
        Ok(convert::wit(r))
    }

    async fn classify(accessor: &Accessor<T, Self>, request: String) -> HostResult<String> {
        let (ext, cx) = current_async(accessor);
        let r = match json(request) {
            Ok(request) => reply_json(models(&ext, &cx, ModelsOp::Classify(request)).await),
            Err(e) => Err(e),
        };
        Ok(convert::wit(r))
    }

    async fn generate_image(accessor: &Accessor<T, Self>, request: String) -> HostResult<String> {
        let (ext, cx) = current_async(accessor);
        let r = match json(request) {
            Ok(request) => reply_json(models(&ext, &cx, ModelsOp::GenerateImage(request)).await),
            Err(e) => Err(e),
        };
        Ok(convert::wit(r))
    }
}

// ── ui ───────────────────────────────────────────────────────────────────

fn ui_allowed(ext: &ExtShared) -> Result<(), HostError> {
    ExtShared::require(ext.perms().ui, "ui")
}

async fn device(
    ext: Arc<ExtShared>,
    cx: CallContext,
    op: DeviceOp,
) -> Result<Option<RawJson>, HostError> {
    let perms = ext.perms();
    let (granted, name) = match &op {
        DeviceOp::OsNotify { .. } => (perms.os_notify, "osNotify"),
        DeviceOp::ClipboardWriteText(_) => (perms.clipboard.write, "clipboard.write"),
        DeviceOp::ClipboardReadImage => (perms.clipboard.read, "clipboard.read"),
        DeviceOp::OpenUrl(_) => (perms.open_url, "openUrl"),
        DeviceOp::EditExternally { .. } => (perms.edit_externally, "editExternally"),
    };
    ExtShared::require(granted, name)?;
    ext.host.ui_device(&ext.id, &cx, op).await
}

/// Counts an open `ui.dialog` for the handler deadline (decrements on drop).
struct DialogOpen<'a>(&'a std::sync::atomic::AtomicUsize);

impl<'a> DialogOpen<'a> {
    fn new(counter: &'a std::sync::atomic::AtomicUsize) -> Self {
        counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Self(counter)
    }
}

impl Drop for DialogOpen<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
}

impl wu::Host for HostState {}

impl<T: Send> wu::HostWithStore<T> for HostState {
    async fn dialog(
        accessor: &Accessor<T, Self>,
        request: String,
        cancel: Option<Resource<CancelToken>>,
    ) -> HostResult<String> {
        let (ext, cx) = current_async(accessor);
        let cancel = token(accessor, cancel)?;
        let r = match ui_allowed(&ext).and_then(|()| json(request)) {
            Ok(request) => {
                let _open = DialogOpen::new(&ext.dialogs_open);
                ext.host
                    .ui_dialog(&ext.id, &cx, request, cancel)
                    .await
                    .map(|j| out(&j))
            }
            Err(e) => Err(e),
        };
        Ok(convert::wit(r))
    }

    fn fire(
        mut host: Access<'_, T, Self>,
        request: String,
    ) -> impl Future<Output = HostResult<()>> + Send {
        let (ext, cx) = current(&mut host);
        ready_wit(
            ui_allowed(&ext)
                .and_then(|()| json(request))
                .and_then(|r| ext.host.ui_fire(&ext.id, &cx, r)),
        )
    }

    fn panel_open(
        mut host: Access<'_, T, Self>,
        spec: String,
    ) -> impl Future<Output = HostResult<String>> + Send {
        let (ext, cx) = current(&mut host);
        let r = ui_allowed(&ext)
            .and_then(|()| json(spec))
            .and_then(|s| ext.host.ui_panel(&ext.id, &cx, PanelOp::Open(s)))
            .and_then(|id| id.ok_or_else(|| HostError::Failed("panel-open returned no id".into())));
        ready_wit(r)
    }

    fn panel_update(
        mut host: Access<'_, T, Self>,
        panel_id: String,
        spec: String,
    ) -> impl Future<Output = HostResult<()>> + Send {
        let (ext, cx) = current(&mut host);
        let r = ui_allowed(&ext)
            .and_then(|()| json(spec))
            .and_then(|spec| {
                ext.host
                    .ui_panel(&ext.id, &cx, PanelOp::Update { panel_id, spec })
            })
            .map(|_| ());
        ready_wit(r)
    }

    fn panel_close(
        mut host: Access<'_, T, Self>,
        panel_id: String,
    ) -> impl Future<Output = wasmtime::Result<()>> + Send {
        let (ext, cx) = current(&mut host);
        if ui_allowed(&ext).is_ok() {
            let _ = ext.host.ui_panel(&ext.id, &cx, PanelOp::Close(panel_id));
        }
        ready(())
    }

    fn editor_text(
        mut host: Access<'_, T, Self>,
    ) -> impl Future<Output = wasmtime::Result<String>> + Send {
        let (ext, cx) = current(&mut host);
        let text = ui_allowed(&ext)
            .and_then(|()| ext.host.ui_editor(&cx, EditorOp::Text))
            .ok()
            .flatten();
        ready(text.unwrap_or_default())
    }

    fn editor_set(
        mut host: Access<'_, T, Self>,
        text: String,
    ) -> impl Future<Output = wasmtime::Result<()>> + Send {
        let (ext, cx) = current(&mut host);
        if ui_allowed(&ext).is_ok() {
            let _ = ext.host.ui_editor(&cx, EditorOp::Set(text));
        }
        ready(())
    }

    fn editor_insert(
        mut host: Access<'_, T, Self>,
        text: String,
    ) -> impl Future<Output = wasmtime::Result<()>> + Send {
        let (ext, cx) = current(&mut host);
        if ui_allowed(&ext).is_ok() {
            let _ = ext.host.ui_editor(&cx, EditorOp::Insert(text));
        }
        ready(())
    }

    fn os_notify(
        mut host: Access<'_, T, Self>,
        title: String,
        body: String,
        urgency: String,
        action_url: Option<String>,
    ) -> impl Future<Output = HostResult<()>> + Send {
        let (ext, cx) = current(&mut host);
        async move {
            Ok(convert::wit(
                device(
                    ext,
                    cx,
                    DeviceOp::OsNotify {
                        title,
                        body,
                        urgency,
                        action_url,
                    },
                )
                .await
                .map(|_| ()),
            ))
        }
    }

    fn clipboard_write_text(
        mut host: Access<'_, T, Self>,
        text: String,
    ) -> impl Future<Output = HostResult<()>> + Send {
        let (ext, cx) = current(&mut host);
        async move {
            Ok(convert::wit(
                device(ext, cx, DeviceOp::ClipboardWriteText(text))
                    .await
                    .map(|_| ()),
            ))
        }
    }

    async fn clipboard_read_image(accessor: &Accessor<T, Self>) -> HostResult<Option<String>> {
        let (ext, cx) = current_async(accessor);
        Ok(convert::wit(
            device(ext, cx, DeviceOp::ClipboardReadImage)
                .await
                .map(|j| j.as_ref().map(out)),
        ))
    }

    fn open_url(
        mut host: Access<'_, T, Self>,
        url: String,
    ) -> impl Future<Output = HostResult<()>> + Send {
        let (ext, cx) = current(&mut host);
        async move {
            Ok(convert::wit(
                device(ext, cx, DeviceOp::OpenUrl(url)).await.map(|_| ()),
            ))
        }
    }

    async fn edit_externally(
        accessor: &Accessor<T, Self>,
        text: String,
        extension_hint: Option<String>,
    ) -> HostResult<Option<String>> {
        let (ext, cx) = current_async(accessor);
        let r = device(
            ext,
            cx,
            DeviceOp::EditExternally {
                text,
                extension_hint,
            },
        )
        .await
        .map(|j| j.and_then(|j| j.to_value().as_str().map(ToOwned::to_owned)));
        Ok(convert::wit(r))
    }
}

// ── agents ───────────────────────────────────────────────────────────────

fn agent<T>(
    accessor: &Accessor<T, HostState>,
    res: &Resource<AgentRes>,
) -> wasmtime::Result<Arc<dyn oino_extension_api::AgentHandle>> {
    Ok(accessor.with(|mut a| a.get().table.get(res).map(|r| Arc::clone(&r.0)))?)
}

fn agent_sync<T>(
    host: &mut Access<'_, T, HostState>,
    res: &Resource<AgentRes>,
) -> wasmtime::Result<Arc<dyn oino_extension_api::AgentHandle>> {
    Ok(host.get().table.get(res).map(|r| Arc::clone(&r.0))?)
}

impl wa::Host for HostState {}
impl wa::HostAgent for HostState {}

impl<T: Send> wa::HostAgentWithStore<T> for HostState {
    async fn drop(accessor: &Accessor<T, Self>, rep: Resource<AgentRes>) -> wasmtime::Result<()> {
        let handle = accessor.with(|mut a| a.get().table.delete(rep))?;
        handle.0.close(Duration::from_millis(0)).await;
        Ok(())
    }

    async fn spawn(accessor: &Accessor<T, Self>, spec: String) -> HostResult<Resource<AgentRes>> {
        let (ext, cx) = current_async(accessor);
        let r = match ExtShared::require(ext.perms().agents.spawn, "agents.spawn")
            .and_then(|()| json(spec))
        {
            Ok(spec) => ext.host.agent_spawn(&ext.id, &cx, spec).await,
            Err(e) => Err(e),
        };
        match r {
            Ok(handle) => Ok(Ok(
                accessor.with(|mut a| a.get().table.push(AgentRes(handle)))?
            )),
            Err(e) => Ok(Err(convert::host_error(e))),
        }
    }

    fn id(
        mut host: Access<'_, T, Self>,
        self_: Resource<AgentRes>,
    ) -> impl Future<Output = wasmtime::Result<String>> + Send {
        std::future::ready(agent_sync(&mut host, &self_).map(|a| a.id()))
    }

    fn session_file(
        mut host: Access<'_, T, Self>,
        self_: Resource<AgentRes>,
    ) -> impl Future<Output = wasmtime::Result<Option<String>>> + Send {
        std::future::ready(agent_sync(&mut host, &self_).map(|a| a.session_file()))
    }

    async fn prompt(
        accessor: &Accessor<T, Self>,
        self_: Resource<AgentRes>,
        text: String,
        delivery: Option<String>,
    ) -> HostResult<String> {
        Ok(convert::wit(
            agent(accessor, &self_)?.prompt(text, delivery).await,
        ))
    }

    async fn steer(
        accessor: &Accessor<T, Self>,
        self_: Resource<AgentRes>,
        text: String,
    ) -> HostResult<()> {
        Ok(convert::wit(agent(accessor, &self_)?.steer(text).await))
    }

    async fn follow_up(
        accessor: &Accessor<T, Self>,
        self_: Resource<AgentRes>,
        text: String,
    ) -> HostResult<()> {
        Ok(convert::wit(agent(accessor, &self_)?.follow_up(text).await))
    }

    fn abort(
        mut host: Access<'_, T, Self>,
        self_: Resource<AgentRes>,
    ) -> impl Future<Output = wasmtime::Result<()>> + Send {
        std::future::ready(agent_sync(&mut host, &self_).map(|a| a.abort()))
    }

    fn set_thinking(
        mut host: Access<'_, T, Self>,
        self_: Resource<AgentRes>,
        level: String,
    ) -> impl Future<Output = HostResult<String>> + Send {
        std::future::ready(
            agent_sync(&mut host, &self_).map(|a| convert::wit(a.set_thinking(&level))),
        )
    }

    fn last_assistant_text(
        mut host: Access<'_, T, Self>,
        self_: Resource<AgentRes>,
    ) -> impl Future<Output = wasmtime::Result<Option<String>>> + Send {
        std::future::ready(agent_sync(&mut host, &self_).map(|a| a.last_assistant_text()))
    }

    async fn wait_idle(accessor: &Accessor<T, Self>, self_: Resource<AgentRes>) -> HostResult<()> {
        Ok(convert::wit(agent(accessor, &self_)?.wait_idle().await))
    }

    fn usage(
        mut host: Access<'_, T, Self>,
        self_: Resource<AgentRes>,
    ) -> impl Future<Output = wasmtime::Result<String>> + Send {
        std::future::ready(agent_sync(&mut host, &self_).map(|a| out(&a.usage())))
    }

    fn events(
        mut host: Access<'_, T, Self>,
        self_: Resource<AgentRes>,
    ) -> impl Future<Output = wasmtime::Result<StreamReader<String>>> + Send {
        let r = agent_sync(&mut host, &self_)
            .and_then(|a| StreamReader::new(&mut host, ItemProducer::new(a.events())));
        std::future::ready(r)
    }

    async fn close(
        accessor: &Accessor<T, Self>,
        self_: Resource<AgentRes>,
        grace_ms: u32,
    ) -> wasmtime::Result<()> {
        agent(accessor, &self_)?
            .close(Duration::from_millis(u64::from(grace_ms)))
            .await;
        Ok(())
    }
}

impl<T: Send> wa::HostWithStore<T> for HostState {
    fn list(
        mut host: Access<'_, T, Self>,
    ) -> impl Future<Output = wasmtime::Result<String>> + Send {
        let (ext, cx) = current(&mut host);
        ready(out(&ext.host.agents_list(&cx)))
    }
}

// ── tasks ────────────────────────────────────────────────────────────────

fn tasks(ext: &ExtShared, cx: &CallContext, op: TaskOp) -> Result<RawJson, HostError> {
    ExtShared::require(ext.perms().process.spawn, "process.spawn")?;
    ext.host.tasks(&ext.id, cx, op)
}

fn task_string(r: Result<RawJson, HostError>) -> Result<String, HostError> {
    r.map(|j| {
        j.to_value()
            .as_str()
            .map_or_else(|| out(&j), ToOwned::to_owned)
    })
}

impl wta::Host for HostState {}

impl<T: Send> wta::HostWithStore<T> for HostState {
    fn start(
        mut host: Access<'_, T, Self>,
        spec: String,
    ) -> impl Future<Output = HostResult<String>> + Send {
        let (ext, cx) = current(&mut host);
        ready_wit(json(spec).and_then(|s| task_string(tasks(&ext, &cx, TaskOp::Start(s)))))
    }
    fn adopt_bash(
        mut host: Access<'_, T, Self>,
        tool_call_id: String,
        reason: String,
    ) -> impl Future<Output = HostResult<String>> + Send {
        let (ext, cx) = current(&mut host);
        ready_wit(task_string(tasks(
            &ext,
            &cx,
            TaskOp::AdoptBash {
                tool_call_id,
                reason,
            },
        )))
    }
    fn list(
        mut host: Access<'_, T, Self>,
    ) -> impl Future<Output = wasmtime::Result<String>> + Send {
        let (ext, cx) = current(&mut host);
        ready(tasks(&ext, &cx, TaskOp::List).map_or_else(|_| "[]".to_owned(), |j| out(&j)))
    }
    fn get(
        mut host: Access<'_, T, Self>,
        id: String,
    ) -> impl Future<Output = wasmtime::Result<Option<String>>> + Send {
        let (ext, cx) = current(&mut host);
        ready(
            tasks(&ext, &cx, TaskOp::Get(id))
                .ok()
                .filter(|j| j.as_str() != "null")
                .map(|j| out(&j)),
        )
    }
    fn logs(
        mut host: Access<'_, T, Self>,
        id: String,
        max_bytes: u32,
        tail: bool,
    ) -> impl Future<Output = HostResult<String>> + Send {
        let (ext, cx) = current(&mut host);
        ready_wit(task_string(tasks(
            &ext,
            &cx,
            TaskOp::Logs {
                id,
                max_bytes,
                tail,
            },
        )))
    }
    fn kill(
        mut host: Access<'_, T, Self>,
        id: String,
    ) -> impl Future<Output = HostResult<()>> + Send {
        let (ext, cx) = current(&mut host);
        ready_wit(tasks(&ext, &cx, TaskOp::Kill(id)).map(|_| ()))
    }
    fn clear_finished(
        mut host: Access<'_, T, Self>,
    ) -> impl Future<Output = wasmtime::Result<()>> + Send {
        let (ext, cx) = current(&mut host);
        let _ = tasks(&ext, &cx, TaskOp::ClearFinished);
        ready(())
    }
}

// ── bus / services / secrets ─────────────────────────────────────────────

impl wb::Host for HostState {}

impl<T: Send> wb::HostWithStore<T> for HostState {
    fn publish(
        mut host: Access<'_, T, Self>,
        topic: String,
        data: String,
        sticky: bool,
    ) -> impl Future<Output = wasmtime::Result<()>> + Send {
        let (ext, cx) = current(&mut host);
        if let Ok(data) = json(data) {
            ext.host.bus_publish(&ext.id, &cx, &topic, data, sticky);
        }
        ready(())
    }
    fn subscribe(
        mut host: Access<'_, T, Self>,
        pattern: String,
    ) -> impl Future<Output = wasmtime::Result<()>> + Send {
        let (ext, cx) = current(&mut host);
        ext.host.bus_subscribe(&ext.id, &cx, &pattern, true);
        ready(())
    }
    fn unsubscribe(
        mut host: Access<'_, T, Self>,
        pattern: String,
    ) -> impl Future<Output = wasmtime::Result<()>> + Send {
        let (ext, cx) = current(&mut host);
        ext.host.bus_subscribe(&ext.id, &cx, &pattern, false);
        ready(())
    }
    fn get(
        mut host: Access<'_, T, Self>,
        topic: String,
    ) -> impl Future<Output = wasmtime::Result<Option<String>>> + Send {
        let (ext, cx) = current(&mut host);
        ready(ext.host.bus_get(&cx, &topic).as_ref().map(out))
    }
}

impl wsv::Host for HostState {}

impl<T: Send> wsv::HostWithStore<T> for HostState {
    async fn call(
        accessor: &Accessor<T, Self>,
        target: Option<String>,
        name: String,
        payload: String,
        timeout_ms: Option<u32>,
    ) -> HostResult<String> {
        let (ext, cx) = current_async(accessor);
        let check = ExtShared::require(
            ext.perms().allows_service(target.as_deref(), &name),
            &format!("services.call:{}/{name}", target.as_deref().unwrap_or("*")),
        )
        .and_then(|()| {
            target
                .map(|t| ExtensionId::new(t).map_err(|e| HostError::InvalidArgument(e.to_string())))
                .transpose()
        })
        .and_then(|t| json(payload).map(|p| (t, p)));
        let r = match check {
            Ok((target, payload)) => {
                let timeout = Duration::from_millis(u64::from(timeout_ms.unwrap_or(5000)));
                ext.host
                    .service_call(&ext.id, &cx, target.as_ref(), &name, payload, timeout)
                    .await
                    .map(|j| out(&j))
            }
            Err(e) => Err(e),
        };
        Ok(convert::wit(r))
    }

    fn list(
        mut host: Access<'_, T, Self>,
    ) -> impl Future<Output = wasmtime::Result<Vec<(String, String)>>> + Send {
        let (ext, _) = current(&mut host);
        ready(
            ext.host
                .services_list()
                .into_iter()
                .map(|(id, name)| (id.to_string(), name))
                .collect(),
        )
    }
}

fn secret(ext: &ExtShared, op: SecretOp) -> Result<Option<String>, HostError> {
    ExtShared::require(ext.perms().secrets, "secrets")?;
    ext.host.secret(&ext.id, op)
}

impl wsec::Host for HostState {}

impl<T: Send> wsec::HostWithStore<T> for HostState {
    fn get(
        mut host: Access<'_, T, Self>,
        name: String,
    ) -> impl Future<Output = HostResult<Option<String>>> + Send {
        let (ext, _) = current(&mut host);
        ready_wit(secret(&ext, SecretOp::Get(name)))
    }
    fn set(
        mut host: Access<'_, T, Self>,
        name: String,
        value: String,
    ) -> impl Future<Output = HostResult<()>> + Send {
        let (ext, _) = current(&mut host);
        ready_wit(secret(&ext, SecretOp::Set { name, value }).map(|_| ()))
    }
    fn delete(
        mut host: Access<'_, T, Self>,
        name: String,
    ) -> impl Future<Output = HostResult<()>> + Send {
        let (ext, _) = current(&mut host);
        ready_wit(secret(&ext, SecretOp::Delete(name)).map(|_| ()))
    }
}
