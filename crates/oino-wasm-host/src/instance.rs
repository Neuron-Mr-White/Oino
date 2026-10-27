//! [`WasmInstance`]: one component instance per extension, driven by an actor task that owns
//! the wasmtime `Store` inside `run_concurrent`, so many guest calls are in flight at once
//! (D6). A trap poisons the actor; the next call restarts it lazily (max 3 per 10 minutes)
//! and replays `init`.

use std::collections::VecDeque;
use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures::future::BoxFuture;
use futures::stream::{FuturesUnordered, StreamExt};
use oino_ai::{Model, ProviderEvent};
use oino_extension_api::{
    BoxFut, CallContext, CallKind, CancellationToken, CompletionItem, EventKind, ExtensionError,
    ExtensionId, ExtensionInstance, InitInput, Manifest, ProviderEventStream, ProviderInstance,
    RawJson, Registration, ToolInvocation,
};
use tokio::sync::{mpsc, oneshot};
use wasmtime::Store;
use wasmtime::component::{
    Accessor, ComponentNamedList, InstancePre, Lift, Lower, ResourceTable, TypedFunc,
};

use crate::bindings::exports::oino::extension::{guest as wg, provider as wpv};
use crate::convert;
use crate::engine::EngineShared;
use crate::resources::CancelToken;
use crate::state::{CallTable, ExtShared, HostState, lock};
use crate::streams::{ChannelConsumer, receiver_stream};

/// Loaded exports of one live instance.
pub(crate) struct Exports {
    guest: wg::Guest,
    provider: Option<wpv::Guest>,
}

type Job =
    Box<dyn for<'a> FnOnce(&'a Accessor<HostState>, &'a Exports) -> BoxFuture<'a, ()> + Send>;

const MAX_RESTARTS: usize = 3;
const RESTART_WINDOW: Duration = Duration::from_secs(600);

/// A loaded extension component.
pub struct WasmInstance {
    pub(crate) shared: Arc<ExtShared>,
    engine: Arc<EngineShared>,
    pre: InstancePre<HostState>,
    actor: Mutex<Option<mpsc::UnboundedSender<Job>>>,
    restarts: Mutex<VecDeque<Instant>>,
    started_once: AtomicBool,
    init_replay: Mutex<Option<(InitInput, CallContext)>>,
    disabled: AtomicBool,
    has_provider: bool,
    pub(crate) hub: std::sync::OnceLock<std::sync::Weak<crate::hub::LocalServices>>,
}

/// Calls `f` with the call context registered for the guest task it creates.
pub(crate) async fn invoke<P, R>(
    acc: &Accessor<HostState>,
    f: TypedFunc<P, R>,
    params: P,
    cx: Option<&CallContext>,
) -> wasmtime::Result<R>
where
    P: ComponentNamedList + Lower + Send + Sync + 'static,
    R: ComponentNamedList + Lift + Send + Sync + 'static,
{
    let call = acc.with(|mut store| -> wasmtime::Result<_> {
        let call = f.start_call_concurrent(&mut store, params)?;
        if let Some(cx) = cx {
            let state = store.data_mut();
            state.calls.insert(call.task(), cx.clone());
            *lock(&state.ext.last_cx) = cx.clone();
        }
        Ok(call)
    })?;
    f.finish_call_concurrent(acc, call).await
}

impl WasmInstance {
    pub(crate) fn new(
        shared: Arc<ExtShared>,
        engine: Arc<EngineShared>,
        pre: InstancePre<HostState>,
        has_provider: bool,
    ) -> Self {
        Self {
            shared,
            engine,
            pre,
            actor: Mutex::new(None),
            restarts: Mutex::new(VecDeque::new()),
            started_once: AtomicBool::new(false),
            init_replay: Mutex::new(None),
            disabled: AtomicBool::new(false),
            has_provider,
            hub: std::sync::OnceLock::new(),
        }
    }

    fn err(&self, call: &str, message: impl Into<String>) -> ExtensionError {
        ExtensionError::new(self.shared.id.as_str(), call, message)
    }

    fn trap(&self, call: &str, message: impl Into<String>) -> ExtensionError {
        ExtensionError::trap(self.shared.id.as_str(), call, message)
    }

    /// Builds a store, instantiates, and spawns the actor.
    async fn start_actor(&self) -> Result<mpsc::UnboundedSender<Job>, ExtensionError> {
        let state = HostState {
            wasi: crate::wasi::build(&self.shared)
                .map_err(|e| self.err("instantiate", e.to_string()))?,
            table: ResourceTable::new(),
            limits: EngineShared::limits(&self.shared.manifest),
            ext: Arc::clone(&self.shared),
            calls: CallTable::default(),
        };
        let mut store = Store::new(&self.engine.engine, state);
        store.limiter(|s| &mut s.limits);
        store.epoch_deadline_async_yield_and_update(1);
        let instance = self
            .pre
            .instantiate_async(&mut store)
            .await
            .map_err(|e| self.err("instantiate", format!("{e:#}")))?;
        let guest = wg::GuestIndices::new(&self.pre)
            .and_then(|i| i.load(&mut store, &instance))
            .map_err(|e| self.err("instantiate", format!("{e:#}")))?;
        let provider = if self.has_provider {
            wpv::GuestIndices::new(&self.pre)
                .and_then(|i| i.load(&mut store, &instance))
                .ok()
        } else {
            None
        };
        let exports = Exports { guest, provider };
        let (tx, mut rx) = mpsc::unbounded_channel::<Job>();
        let id = self.shared.id.clone();
        tokio::spawn(async move {
            let result = store
                .run_concurrent(async move |acc| {
                    let exports = exports;
                    let mut running = FuturesUnordered::new();
                    loop {
                        tokio::select! {
                            job = rx.recv() => match job {
                                Some(job) => running.push(job(acc, &exports)),
                                None => break,
                            },
                            Some(()) = running.next(), if !running.is_empty() => {}
                        }
                    }
                    while running.next().await.is_some() {}
                })
                .await;
            if let Err(e) = result {
                tracing::warn!(extension = %id, error = %format!("{e:#}"), "component instance stopped");
            }
        });
        Ok(tx)
    }

    /// The live actor, (re)starting it when needed.
    async fn actor(&self) -> Result<mpsc::UnboundedSender<Job>, ExtensionError> {
        if self.disabled.load(Ordering::SeqCst) {
            return Err(self.err("call", "extension disabled after repeated traps"));
        }
        if let Some(tx) = lock(&self.actor).as_ref().filter(|tx| !tx.is_closed()) {
            return Ok(tx.clone());
        }
        let restart = self.started_once.swap(true, Ordering::SeqCst);
        if restart {
            let mut restarts = lock(&self.restarts);
            let now = Instant::now();
            restarts.retain(|t| now.duration_since(*t) < RESTART_WINDOW);
            if restarts.len() >= MAX_RESTARTS {
                self.disabled.store(true, Ordering::SeqCst);
                return Err(self.err("call", "extension disabled after repeated traps"));
            }
            restarts.push_back(now);
        }
        let tx = self.start_actor().await?;
        *lock(&self.actor) = Some(tx.clone());
        let replay_input = if restart {
            lock(&self.init_replay).clone()
        } else {
            None
        };
        if let Some((input, cx)) = replay_input {
            tracing::info!(extension = %self.shared.id, "restarted after trap; replaying init");
            let replay = self.send(&tx, "init", move |acc, ex| {
                Box::pin(async move {
                    let f = ex.guest.func_init();
                    invoke(
                        acc,
                        f,
                        (init_input(&input), convert::call_context(&cx)),
                        Some(&cx),
                    )
                    .await
                    .map(|_| ())
                })
            });
            replay.await.ok();
        }
        Ok(tx)
    }

    fn poison(&self) {
        lock(&self.actor).take();
    }

    /// Sends one job to `tx` and awaits its result.
    async fn send<R, F>(
        &self,
        tx: &mpsc::UnboundedSender<Job>,
        call: &str,
        f: F,
    ) -> Result<R, ExtensionError>
    where
        R: Send + 'static,
        F: for<'a> FnOnce(
                &'a Accessor<HostState>,
                &'a Exports,
            ) -> BoxFuture<'a, wasmtime::Result<R>>
            + Send
            + 'static,
    {
        let (reply_tx, reply_rx) = oneshot::channel();
        let job: Job = Box::new(move |acc, ex| {
            Box::pin(async move {
                let result = f(acc, ex).await;
                let _ = reply_tx.send(result);
            })
        });
        if tx.send(job).is_err() {
            self.poison();
            return Err(self.trap(call, "component instance stopped"));
        }
        match reply_rx.await {
            Ok(Ok(r)) => Ok(r),
            Ok(Err(e)) => {
                self.poison();
                Err(self.trap(call, format!("{e:#}")))
            }
            Err(_) => {
                self.poison();
                Err(self.trap(call, "component instance stopped"))
            }
        }
    }

    async fn call<R, F>(&self, call: &str, f: F) -> Result<R, ExtensionError>
    where
        R: Send + 'static,
        F: for<'a> FnOnce(
                &'a Accessor<HostState>,
                &'a Exports,
            ) -> BoxFuture<'a, wasmtime::Result<R>>
            + Send
            + 'static,
    {
        let tx = self.actor().await?;
        self.send(&tx, call, f).await
    }

    /// Blocking variant for the synchronous exports (`prepare-*`, `render`, …). Requires a
    /// multi-thread tokio runtime when called from async code.
    fn call_blocking<R, F>(&self, call: &str, f: F) -> Result<R, ExtensionError>
    where
        R: Send + 'static,
        F: for<'a> FnOnce(
                &'a Accessor<HostState>,
                &'a Exports,
            ) -> BoxFuture<'a, wasmtime::Result<R>>
            + Send
            + 'static,
    {
        let fut = self.call(call, f);
        match tokio::runtime::Handle::try_current() {
            Ok(handle) if handle.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread => {
                tokio::task::block_in_place(|| handle.block_on(fut))
            }
            Ok(_) => Err(self.err(
                call,
                "synchronous export needs a multi-thread tokio runtime",
            )),
            Err(_) => futures::executor::block_on(fut),
        }
    }

    fn guest_err(&self, call: &str, r: Result<String, ExtensionError>) -> ExtensionError {
        match r {
            Ok(message) => self.err(call, message),
            Err(e) => e,
        }
    }
}

/// `fut` with `deadline`, paused while `waiting()` (a dialog is open); after the dialog
/// closes the handler gets a fresh deadline (GA-42). `None` when it expired.
async fn pausable_timeout<T>(
    fut: impl Future<Output = T>,
    deadline: Duration,
    waiting: impl Fn() -> bool,
) -> Option<T> {
    const RECHECK: Duration = Duration::from_millis(200);
    let mut fut = std::pin::pin!(fut);
    loop {
        if let Ok(value) = tokio::time::timeout(deadline, &mut fut).await {
            return Some(value);
        }
        if !waiting() {
            return None;
        }
        loop {
            if let Ok(value) = tokio::time::timeout(RECHECK, &mut fut).await {
                return Some(value);
            }
            if !waiting() {
                break;
            }
        }
    }
}

fn init_input(input: &InitInput) -> wg::InitInput {
    wg::InitInput {
        manifest: input.manifest.as_str().to_owned(),
        settings: input.settings.as_str().to_owned(),
        flags: input.flags.as_str().to_owned(),
    }
}

/// Runs `f` with a fresh cancel-token resource that is deleted afterwards.
async fn with_token<R, Fut>(
    acc: &Accessor<HostState>,
    token: CancellationToken,
    f: impl FnOnce(wasmtime::component::Resource<CancelToken>) -> Fut,
) -> wasmtime::Result<R>
where
    Fut: Future<Output = wasmtime::Result<R>>,
{
    let res = acc.with(|mut s| s.data_mut().table.push(CancelToken(token)))?;
    let rep = res.rep();
    let out = f(wasmtime::component::Resource::new_borrow(rep)).await;
    acc.with(|mut s| s.data_mut().table.delete(res).map(|_| ()))?;
    out
}

impl ExtensionInstance for WasmInstance {
    fn id(&self) -> &ExtensionId {
        &self.shared.id
    }

    fn manifest(&self) -> &Manifest {
        &self.shared.manifest
    }

    fn init(
        &self,
        input: InitInput,
        cx: CallContext,
    ) -> BoxFut<'_, Result<Registration, ExtensionError>> {
        Box::pin(async move {
            let cx = cx.with_kind(CallKind::Init);
            *lock(&self.init_replay) = Some((input.clone(), cx.clone()));
            let r = self
                .call("init", move |acc, ex| {
                    Box::pin(async move {
                        let f = ex.guest.func_init();
                        invoke(
                            acc,
                            f,
                            (init_input(&input), convert::call_context(&cx)),
                            Some(&cx),
                        )
                        .await
                        .map(|(r,)| r)
                    })
                })
                .await?;
            match r {
                Ok(reg) => {
                    let reg =
                        convert::registration(reg).map_err(|e| self.err("init", e.to_string()))?;
                    if let Some(hub) = self.hub.get().and_then(std::sync::Weak::upgrade) {
                        hub.note_registration(&self.shared.id, &reg);
                    }
                    Ok(reg)
                }
                Err(message) => Err(self.err("init", message)),
            }
        })
    }

    fn on_event(
        &self,
        kind: EventKind,
        payload: RawJson,
        cx: CallContext,
    ) -> BoxFut<'_, Result<Option<RawJson>, ExtensionError>> {
        Box::pin(async move {
            let limits = self.shared.manifest.limits;
            let deadline = Duration::from_millis(if kind == EventKind::ToolCall {
                limits.tool_call_hook_timeout_ms
            } else {
                limits.hook_timeout_ms
            });
            let call_name = format!("on-event:{kind}");
            let fut = self.call(&call_name, move |acc, ex| {
                Box::pin(async move {
                    let f = ex.guest.func_on_event();
                    let params = (
                        convert::event_to_wit(kind),
                        payload.into_string(),
                        convert::call_context(&cx),
                    );
                    invoke(acc, f, params, Some(&cx)).await.map(|(r,)| r)
                })
            });
            let open = &self.shared.dialogs_open;
            let r = pausable_timeout(fut, deadline, || open.load(Ordering::SeqCst) > 0)
                .await
                .ok_or_else(|| {
                    self.err(
                        &call_name,
                        format!("handler timed out after {} ms", deadline.as_millis()),
                    )
                })??;
            match r {
                Ok(result) => Ok(result.map(convert::json_lossy)),
                Err(message) => Err(self.err(&call_name, message)),
            }
        })
    }

    fn run_tool(
        &self,
        inv: ToolInvocation,
        cancel: CancellationToken,
        cx: CallContext,
    ) -> BoxFut<'_, Result<RawJson, ExtensionError>> {
        Box::pin(async move {
            let call_name = format!("run-tool:{}", inv.name);
            let mut cx = cx.with_kind(CallKind::Tool);
            cx.tool_call_id = Some(inv.tool_call_id.clone());
            let r = self
                .call(&call_name, move |acc, ex| {
                    Box::pin(async move {
                        let f = ex.guest.func_run_tool();
                        let wit_inv = wg::ToolInvocation {
                            tool_call_id: inv.tool_call_id,
                            name: inv.name,
                            args: inv.args.into_string(),
                            parent_tool_call_id: inv.parent_tool_call_id,
                        };
                        with_token(acc, cancel, |tok| async move {
                            invoke(
                                acc,
                                f,
                                (wit_inv, tok, convert::call_context(&cx)),
                                Some(&cx),
                            )
                            .await
                            .map(|(r,)| r)
                        })
                        .await
                    })
                })
                .await?;
            r.map(convert::json_lossy)
                .map_err(|m| self.err(&call_name, m))
        })
    }

    fn prepare_arguments(&self, tool: &str, raw: RawJson) -> Result<RawJson, ExtensionError> {
        let tool = tool.to_owned();
        let out = self.call_blocking("prepare-arguments", move |acc, ex| {
            Box::pin(async move {
                invoke(
                    acc,
                    ex.guest.func_prepare_arguments(),
                    (tool, raw.into_string()),
                    None,
                )
                .await
                .map(|(r,)| r)
            })
        })?;
        Ok(convert::json_lossy(out))
    }

    fn prepare_loadout(
        &self,
        tool: &str,
        loadout: RawJson,
    ) -> Result<Option<RawJson>, ExtensionError> {
        let tool = tool.to_owned();
        let out = self.call_blocking("prepare-loadout", move |acc, ex| {
            Box::pin(async move {
                invoke(
                    acc,
                    ex.guest.func_prepare_loadout(),
                    (tool, loadout.into_string()),
                    None,
                )
                .await
                .map(|(r,)| r)
            })
        })?;
        Ok(out.map(convert::json_lossy))
    }

    fn reaches_tools(&self, tool: &str, input: RawJson) -> Result<RawJson, ExtensionError> {
        let tool = tool.to_owned();
        let out = self.call_blocking("reaches-tools", move |acc, ex| {
            Box::pin(async move {
                invoke(
                    acc,
                    ex.guest.func_reaches_tools(),
                    (tool, input.into_string()),
                    None,
                )
                .await
                .map(|(r,)| r)
            })
        })?;
        Ok(convert::json_lossy(out))
    }

    fn run_command(
        &self,
        name: &str,
        args: &str,
        cancel: CancellationToken,
        cx: CallContext,
    ) -> BoxFut<'_, Result<Option<RawJson>, ExtensionError>> {
        let (name, args) = (name.to_owned(), args.to_owned());
        Box::pin(async move {
            let call_name = format!("run-command:{name}");
            let cx = cx.with_kind(CallKind::Command);
            let r = self
                .call(&call_name, move |acc, ex| {
                    Box::pin(async move {
                        let f = ex.guest.func_run_command();
                        with_token(acc, cancel, |tok| async move {
                            invoke(
                                acc,
                                f,
                                (name, args, tok, convert::call_context(&cx)),
                                Some(&cx),
                            )
                            .await
                            .map(|(r,)| r)
                        })
                        .await
                    })
                })
                .await?;
            r.map(|o| o.map(convert::json_lossy))
                .map_err(|m| self.err(&call_name, m))
        })
    }

    fn complete_command(
        &self,
        name: &str,
        prefix: &str,
        cx: CallContext,
    ) -> BoxFut<'_, Vec<CompletionItem>> {
        let (name, prefix) = (name.to_owned(), prefix.to_owned());
        Box::pin(async move {
            let cx = cx.with_kind(CallKind::Command);
            self.call("complete-command", move |acc, ex| {
                Box::pin(async move {
                    invoke(
                        acc,
                        ex.guest.func_complete_command(),
                        (name, prefix, convert::call_context(&cx)),
                        Some(&cx),
                    )
                    .await
                    .map(|(r,)| r)
                })
            })
            .await
            .map(|items| items.into_iter().map(convert::completion_item).collect())
            .unwrap_or_default()
        })
    }

    fn run_shortcut(&self, id: &str, cx: CallContext) -> BoxFut<'_, Result<(), ExtensionError>> {
        let id = id.to_owned();
        Box::pin(async move {
            let cx = cx.with_kind(CallKind::Shortcut);
            let r = self
                .call("run-shortcut", move |acc, ex| {
                    Box::pin(async move {
                        invoke(
                            acc,
                            ex.guest.func_run_shortcut(),
                            (id, convert::call_context(&cx)),
                            Some(&cx),
                        )
                        .await
                        .map(|(r,)| r)
                    })
                })
                .await;
            r.and_then(|inner| inner.map_err(|m| self.guest_err("run-shortcut", Ok(m))))
        })
    }

    fn handle_service(
        &self,
        name: &str,
        payload: RawJson,
        caller: &ExtensionId,
        cx: CallContext,
    ) -> BoxFut<'_, Result<RawJson, ExtensionError>> {
        let (name, caller) = (name.to_owned(), caller.to_string());
        Box::pin(async move {
            let cx = cx.with_kind(CallKind::Service);
            let r = self
                .call("handle-service", move |acc, ex| {
                    Box::pin(async move {
                        invoke(
                            acc,
                            ex.guest.func_handle_service(),
                            (
                                name,
                                payload.into_string(),
                                caller,
                                convert::call_context(&cx),
                            ),
                            Some(&cx),
                        )
                        .await
                        .map(|(r,)| r)
                    })
                })
                .await?;
            r.map(convert::json_lossy)
                .map_err(|m| self.err("handle-service", m))
        })
    }

    fn propose_nudge(&self, cx: CallContext) -> BoxFut<'_, Option<RawJson>> {
        Box::pin(async move {
            let cx = cx.with_kind(CallKind::Nudge);
            self.call("propose-nudge", move |acc, ex| {
                Box::pin(async move {
                    invoke(
                        acc,
                        ex.guest.func_propose_nudge(),
                        (convert::call_context(&cx),),
                        Some(&cx),
                    )
                    .await
                    .map(|(r,)| r)
                })
            })
            .await
            .ok()
            .flatten()
            .map(convert::json_lossy)
        })
    }

    fn nudge_delivered(&self, id: &str, cx: CallContext) {
        let id = id.to_owned();
        let cx = cx.with_kind(CallKind::Nudge);
        let _ = self.call_blocking("nudge-delivered", move |acc, ex| {
            Box::pin(async move {
                invoke(
                    acc,
                    ex.guest.func_nudge_delivered(),
                    (id, convert::call_context(&cx)),
                    Some(&cx),
                )
                .await
            })
        });
    }

    fn wait_reason(&self, cx: CallContext) -> BoxFut<'_, Option<String>> {
        Box::pin(async move {
            let cx = cx.with_kind(CallKind::Nudge);
            self.call("wait-reason", move |acc, ex| {
                Box::pin(async move {
                    invoke(
                        acc,
                        ex.guest.func_wait_reason(),
                        (convert::call_context(&cx),),
                        Some(&cx),
                    )
                    .await
                    .map(|(r,)| r)
                })
            })
            .await
            .ok()
            .flatten()
        })
    }

    fn render(&self, request: RawJson, cx: CallContext) -> Option<RawJson> {
        let cx = cx.with_kind(CallKind::Render);
        self.call_blocking("render", move |acc, ex| {
            Box::pin(async move {
                invoke(
                    acc,
                    ex.guest.func_render(),
                    (request.into_string(), convert::call_context(&cx)),
                    Some(&cx),
                )
                .await
                .map(|(r,)| r)
            })
        })
        .ok()
        .flatten()
        .map(convert::json_lossy)
    }

    fn route_model(
        &self,
        request: RawJson,
        cx: CallContext,
    ) -> BoxFut<'_, Result<RawJson, ExtensionError>> {
        Box::pin(async move {
            let cx = cx.with_kind(CallKind::Route);
            let r = self
                .call("route-model", move |acc, ex| {
                    Box::pin(async move {
                        invoke(
                            acc,
                            ex.guest.func_route_model(),
                            (request.into_string(), convert::call_context(&cx)),
                            Some(&cx),
                        )
                        .await
                        .map(|(r,)| r)
                    })
                })
                .await?;
            r.map(convert::json_lossy)
                .map_err(|m| self.err("route-model", m))
        })
    }

    fn complete_input(&self, request: RawJson, cx: CallContext) -> BoxFut<'_, Option<RawJson>> {
        Box::pin(async move {
            self.call("complete-input", move |acc, ex| {
                Box::pin(async move {
                    invoke(
                        acc,
                        ex.guest.func_complete_input(),
                        (request.into_string(), convert::call_context(&cx)),
                        Some(&cx),
                    )
                    .await
                    .map(|(r,)| r)
                })
            })
            .await
            .ok()
            .flatten()
            .map(convert::json_lossy)
        })
    }

    fn provider(&self) -> Option<&dyn ProviderInstance> {
        if self.has_provider { Some(self) } else { None }
    }

    fn shutdown(&self, reason: &str) -> BoxFut<'_, ()> {
        let reason = reason.to_owned();
        Box::pin(async move {
            if lock(&self.actor).is_some() {
                let call = self.call("shutdown", move |acc, ex| {
                    Box::pin(
                        async move { invoke(acc, ex.guest.func_shutdown(), (reason,), None).await },
                    )
                });
                let _ = tokio::time::timeout(Duration::from_secs(5), call).await;
            }
            self.shared.timers.cancel_all();
            self.shared.processes.kill_all();
            lock(&self.actor).take();
        })
    }
}

fn no_provider(id: &ExtensionId, call: &str) -> ExtensionError {
    ExtensionError::new(
        id.as_str(),
        call,
        "component does not export oino:extension/provider",
    )
}

impl ProviderInstance for WasmInstance {
    fn stream(&self, request: RawJson) -> BoxFut<'_, Result<ProviderEventStream, ExtensionError>> {
        Box::pin(async move {
            let id = self.shared.id.clone();
            let (tx, rx) = mpsc::unbounded_channel::<String>();
            let cx = lock(&self.shared.last_cx)
                .clone()
                .with_kind(CallKind::Provider);
            let started = self
                .call("provider.stream", move |acc, ex| {
                    Box::pin(async move {
                        let Some(provider) = &ex.provider else {
                            return Ok(Err("no provider export".to_owned()));
                        };
                        let (r,) = invoke(
                            acc,
                            provider.func_stream(),
                            (request.into_string(),),
                            Some(&cx),
                        )
                        .await?;
                        match r {
                            Ok(reader) => {
                                acc.with(|mut s| reader.pipe(&mut s, ChannelConsumer::new(tx)))?;
                                Ok(Ok(()))
                            }
                            Err(e) => Ok(Err(e)),
                        }
                    })
                })
                .await?;
            started.map_err(|m| self.err("provider.stream", m))?;
            let call_id = id.clone();
            let events = receiver_stream(rx).map(move |item| {
                serde_json::from_str::<ProviderEvent>(&item).map_err(|e| {
                    ExtensionError::new(
                        call_id.as_str(),
                        "provider.stream",
                        format!("invalid provider event: {e}: {item}"),
                    )
                })
            });
            Ok(Box::pin(events) as ProviderEventStream)
        })
    }

    fn list_models(
        &self,
        provider_id: &str,
        credential: Option<RawJson>,
    ) -> BoxFut<'_, Result<Vec<Model>, ExtensionError>> {
        let provider_id = provider_id.to_owned();
        Box::pin(async move {
            let r = self
                .call("provider.list-models", move |acc, ex| {
                    Box::pin(async move {
                        let Some(provider) = &ex.provider else {
                            return Ok(Err("no provider export".to_owned()));
                        };
                        invoke(
                            acc,
                            provider.func_list_models(),
                            (provider_id, credential.map(RawJson::into_string)),
                            None,
                        )
                        .await
                        .map(|(r,)| r)
                    })
                })
                .await?;
            let text = r.map_err(|m| self.err("provider.list-models", m))?;
            serde_json::from_str(&text)
                .map_err(|e| self.err("provider.list-models", format!("invalid models: {e}")))
        })
    }

    fn oauth_login(
        &self,
        provider_id: &str,
        cx: CallContext,
    ) -> BoxFut<'_, Result<RawJson, ExtensionError>> {
        let provider_id = provider_id.to_owned();
        let cx = cx.with_kind(CallKind::Provider);
        Box::pin(async move {
            let r = self
                .call("provider.oauth-login", move |acc, ex| {
                    Box::pin(async move {
                        let Some(provider) = &ex.provider else {
                            return Ok(Err("no provider export".to_owned()));
                        };
                        invoke(acc, provider.func_oauth_login(), (provider_id,), Some(&cx))
                            .await
                            .map(|(r,)| r)
                    })
                })
                .await?;
            r.map(convert::json_lossy)
                .map_err(|m| self.err("provider.oauth-login", m))
        })
    }

    fn oauth_refresh(
        &self,
        provider_id: &str,
        credential: RawJson,
    ) -> BoxFut<'_, Result<RawJson, ExtensionError>> {
        let provider_id = provider_id.to_owned();
        Box::pin(async move {
            if !self.has_provider {
                return Err(no_provider(&self.shared.id, "provider.oauth-refresh"));
            }
            let r = self
                .call("provider.oauth-refresh", move |acc, ex| {
                    Box::pin(async move {
                        let Some(provider) = &ex.provider else {
                            return Ok(Err("no provider export".to_owned()));
                        };
                        invoke(
                            acc,
                            provider.func_oauth_refresh(),
                            (provider_id, credential.into_string()),
                            None,
                        )
                        .await
                        .map(|(r,)| r)
                    })
                })
                .await?;
            r.map(convert::json_lossy)
                .map_err(|m| self.err("provider.oauth-refresh", m))
        })
    }
}
