//! Runtime glue between the generated `Guest` export trait and [`Extension`] handlers.
//! Components are single-threaded: handler tables live in thread-locals.

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::marker::PhantomData;
use std::rc::Rc;

use futures::FutureExt;
use futures::future::LocalBoxFuture;
use serde_json::{Value, json};

use crate::bindings::exports::oino::extension::guest::{self as wg, CompletionItem};
use crate::bindings::oino::extension::registry as wr;
use crate::bindings::oino::extension::types::{CallContext, CancelToken};
use crate::command::{Command, Shortcut};
use crate::context::{Cancel, CommandCtx, Ctx, ToolCtx};
use crate::events::{Event, EventKind};
use crate::init::{Extension, Init};
use crate::tool::{JsonMap, JsonMaybe, Tool, ToolRun};
use crate::{Error, Result};

type EventFn = Rc<dyn Fn(Event, Ctx) -> LocalBoxFuture<'static, Result<Option<Value>>>>;
type CmdFn = Rc<dyn Fn(String, CommandCtx) -> LocalBoxFuture<'static, Result<Option<Value>>>>;
type CompleteFn = Rc<dyn Fn(String, Ctx) -> LocalBoxFuture<'static, Vec<CompletionItem>>>;
type ShortcutFn = Rc<dyn Fn(CommandCtx) -> LocalBoxFuture<'static, Result<()>>>;
type ServiceFn = Rc<dyn Fn(Value, String, Ctx) -> LocalBoxFuture<'static, Result<Value>>>;
/// Renderer callback.
pub(crate) type RenderFn = Rc<dyn Fn(Value, Ctx) -> Option<Value>>;
type CxFn<T> = Rc<dyn Fn(Ctx) -> LocalBoxFuture<'static, T>>;
type ReqFn<T> = Rc<dyn Fn(Value, Ctx) -> LocalBoxFuture<'static, T>>;

struct ToolEntry {
    run: Option<ToolRun>,
    prepare_arguments: Option<JsonMap>,
    prepare_loadout: Option<JsonMaybe>,
    reaches: Option<JsonMap>,
}

/// Type-erased handler tables.
#[derive(Default)]
#[doc(hidden)]
pub struct Handlers {
    pub(crate) events: Vec<(EventKind, EventFn)>,
    tools: HashMap<String, ToolEntry>,
    commands: HashMap<String, (Option<CmdFn>, Option<CompleteFn>)>,
    shortcuts: HashMap<String, ShortcutFn>,
    pub(crate) services: HashMap<String, ServiceFn>,
    pub(crate) renderers: Vec<(String, RenderFn)>,
    pub(crate) nudge: Option<CxFn<Option<Value>>>,
    pub(crate) nudge_delivered: Option<Rc<dyn Fn(String, Ctx)>>,
    pub(crate) wait_reason: Option<CxFn<Option<String>>>,
    pub(crate) route: Option<ReqFn<Result<Value>>>,
    pub(crate) complete_input: Option<ReqFn<Option<Value>>>,
    pub(crate) shutdown: Option<Rc<dyn Fn(String) -> LocalBoxFuture<'static, ()>>>,
}

impl Handlers {
    pub(crate) fn install_tool<E: 'static>(&mut self, ext: &'static E, tool: Tool<E>) {
        let run = tool
            .run
            .map(|f| -> ToolRun { Rc::new(move |args, cx| f(ext, args, cx)) });
        self.tools.insert(
            tool.spec.name.clone(),
            ToolEntry {
                run,
                prepare_arguments: tool.prepare_arguments,
                prepare_loadout: tool.prepare_loadout,
                reaches: tool.reaches,
            },
        );
    }

    pub(crate) fn install_command<E: 'static>(&mut self, ext: &'static E, cmd: Command<E>) {
        let run = cmd
            .run
            .map(|f| -> CmdFn { Rc::new(move |args, cx| f(ext, args, cx)) });
        let complete = cmd
            .complete
            .map(|f| -> CompleteFn { Rc::new(move |p, cx| f(ext, p, cx)) });
        self.commands.insert(cmd.spec.name.clone(), (run, complete));
    }

    pub(crate) fn install_shortcut<E: 'static>(&mut self, ext: &'static E, sc: Shortcut<E>) {
        if let Some(f) = sc.run {
            self.shortcuts
                .insert(sc.spec.id.clone(), Rc::new(move |cx| f(ext, cx)));
        }
    }
}

thread_local! {
    static HANDLERS: RefCell<Handlers> = RefCell::new(Handlers::default());
    static EXTENSION: Cell<Option<&'static dyn Any>> = const { Cell::new(None) };
}

fn with<R>(f: impl FnOnce(&Handlers) -> R) -> R {
    HANDLERS.with(|h| f(&h.borrow()))
}

/// Installs a tool registered after `init`.
pub(crate) fn install_tool<E: 'static>(ext: &'static E, tool: Tool<E>) {
    HANDLERS.with(|h| h.borrow_mut().install_tool(ext, tool));
}

/// Installs a command registered after `init`.
pub fn register_command<E: 'static>(ext: &'static E, cmd: Command<E>) -> Result<String> {
    let spec = cmd.spec.clone();
    HANDLERS.with(|h| h.borrow_mut().install_command(ext, cmd));
    Ok(wr::register_command(&spec)?)
}

fn instance<E: Extension>() -> &'static E {
    if let Some(e) = EXTENSION
        .with(Cell::get)
        .and_then(|any| any.downcast_ref::<E>())
    {
        return e;
    }
    let leaked: &'static E = Box::leak(Box::new(E::default()));
    EXTENSION.with(|c| c.set(Some(leaked)));
    leaked
}

fn parse(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or(Value::Null)
}

/// Runs `fut` while watching `cancel`; sets the flag when the host cancels.
async fn watch_cancel<T>(
    cancel: &CancelToken,
    flag: &Cancel,
    fut: LocalBoxFuture<'static, T>,
) -> T {
    let mut fut = fut.fuse();
    let mut wait = std::pin::pin!(cancel.wait().fuse());
    futures::select! {
        out = fut => out,
        () = wait => {
            flag.0.set(true);
            fut.await
        }
    }
}

/// The generated `Guest` implementation for extension type `E`.
#[doc(hidden)]
pub struct Shim<E>(PhantomData<E>);

impl<E: Extension> wg::Guest for Shim<E> {
    async fn init(input: wg::InitInput, ctx: CallContext) -> Result<wr::Registration, String> {
        let ext = instance::<E>();
        let mut init = Init {
            ext,
            ctx: Ctx::new(ctx),
            manifest: parse(&input.manifest),
            settings: parse(&input.settings),
            flags: parse(&input.flags),
            registration: wr::Registration {
                events: Vec::new(),
                tools: Vec::new(),
                commands: Vec::new(),
                shortcuts: Vec::new(),
                flags: Vec::new(),
                renderers: Vec::new(),
                services: Vec::new(),
                arbiter: wr::ArbiterRoles {
                    nudge_provider: false,
                    wait_source: false,
                },
                settings: Vec::new(),
            },
            handlers: Handlers::default(),
            timers: Vec::new(),
        };
        ext.init(&mut init).await.map_err(|e| e.to_string())?;
        let Init {
            registration,
            handlers,
            timers,
            ..
        } = init;
        HANDLERS.with(|h| *h.borrow_mut() = handlers);
        for (id, delay, repeat) in timers {
            crate::timers::set(&id, delay, repeat);
        }
        Ok(registration)
    }

    async fn on_event(
        kind: EventKind,
        payload: String,
        ctx: CallContext,
    ) -> Result<Option<String>, String> {
        let handlers: Vec<EventFn> = with(|h| {
            h.events
                .iter()
                .filter(|(k, _)| *k == kind)
                .map(|(_, f)| Rc::clone(f))
                .collect()
        });
        let payload = parse(&payload);
        let mut result = None;
        for f in handlers {
            let event = Event {
                kind,
                payload: payload.clone(),
            };
            if let Some(r) = f(event, Ctx::new(ctx.clone()))
                .await
                .map_err(|e| e.to_string())?
            {
                result = Some(r);
            }
        }
        Ok(result.map(|r| r.to_string()))
    }

    async fn run_tool(
        invocation: wg::ToolInvocation,
        cancel: &CancelToken,
        ctx: CallContext,
    ) -> Result<String, String> {
        let run = with(|h| h.tools.get(&invocation.name).and_then(|t| t.run.clone()))
            .ok_or_else(|| format!("unknown tool {}", invocation.name))?;
        let flag = Cancel::default();
        let cx = ToolCtx {
            cx: Ctx::new(ctx),
            tool_call_id: invocation.tool_call_id,
            cancel: flag.clone(),
        };
        let out = watch_cancel(cancel, &flag, run(parse(&invocation.args), cx))
            .await
            .map_err(|e| e.to_string())?;
        serde_json::to_string(&out).map_err(|e| e.to_string())
    }

    fn prepare_arguments(tool: String, raw: String) -> String {
        match with(|h| h.tools.get(&tool).and_then(|t| t.prepare_arguments.clone())) {
            Some(f) => f(parse(&raw)).to_string(),
            None => raw,
        }
    }

    fn prepare_loadout(tool: String, loadout: String) -> Option<String> {
        with(|h| h.tools.get(&tool).and_then(|t| t.prepare_loadout.clone()))
            .and_then(|f| f(parse(&loadout)))
            .map(|v| v.to_string())
    }

    fn reaches_tools(tool: String, input: String) -> String {
        match with(|h| h.tools.get(&tool).and_then(|t| t.reaches.clone())) {
            Some(f) => f(parse(&input)).to_string(),
            None => json!([]).to_string(),
        }
    }

    async fn run_command(
        name: String,
        args: String,
        cancel: &CancelToken,
        ctx: CallContext,
    ) -> Result<Option<String>, String> {
        let run = with(|h| h.commands.get(&name).and_then(|c| c.0.clone()))
            .ok_or_else(|| format!("unknown command {name}"))?;
        let flag = Cancel::default();
        let cx = CommandCtx {
            cx: Ctx::new(ctx),
            cancel: flag.clone(),
        };
        let out = watch_cancel(cancel, &flag, run(args, cx))
            .await
            .map_err(|e| e.to_string())?;
        Ok(out.map(|v| v.to_string()))
    }

    async fn complete_command(
        name: String,
        prefix: String,
        ctx: CallContext,
    ) -> Vec<CompletionItem> {
        match with(|h| h.commands.get(&name).and_then(|c| c.1.clone())) {
            Some(f) => f(prefix, Ctx::new(ctx)).await,
            None => Vec::new(),
        }
    }

    async fn run_shortcut(id: String, ctx: CallContext) -> Result<(), String> {
        let f = with(|h| h.shortcuts.get(&id).cloned())
            .ok_or_else(|| format!("unknown shortcut {id}"))?;
        f(CommandCtx {
            cx: Ctx::new(ctx),
            cancel: Cancel::default(),
        })
        .await
        .map_err(|e| e.to_string())
    }

    async fn handle_service(
        name: String,
        payload: String,
        caller: String,
        ctx: CallContext,
    ) -> Result<String, String> {
        let f = with(|h| h.services.get(&name).cloned())
            .ok_or_else(|| format!("unknown service {name}"))?;
        f(parse(&payload), caller, Ctx::new(ctx))
            .await
            .map(|v| v.to_string())
            .map_err(|e| e.to_string())
    }

    async fn propose_nudge(ctx: CallContext) -> Option<String> {
        let f = with(|h| h.nudge.clone())?;
        f(Ctx::new(ctx)).await.map(|v| v.to_string())
    }

    fn nudge_delivered(id: String, ctx: CallContext) {
        if let Some(f) = with(|h| h.nudge_delivered.clone()) {
            f(id, Ctx::new(ctx));
        }
    }

    async fn wait_reason(ctx: CallContext) -> Option<String> {
        let f = with(|h| h.wait_reason.clone())?;
        f(Ctx::new(ctx)).await
    }

    fn render(request: String, ctx: CallContext) -> Option<String> {
        let req = parse(&request);
        let key = req
            .get("key")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);
        let f = with(|h| {
            h.renderers
                .iter()
                .find(|(k, _)| key.as_deref().is_none_or(|key| key == k))
                .map(|(_, f)| Rc::clone(f))
        })?;
        f(req, Ctx::new(ctx)).map(|v| v.to_string())
    }

    async fn route_model(request: String, ctx: CallContext) -> Result<String, String> {
        let f = with(|h| h.route.clone())
            .ok_or_else(|| "this extension does not route models".to_owned())?;
        f(parse(&request), Ctx::new(ctx))
            .await
            .map(|v| v.to_string())
            .map_err(|e| e.to_string())
    }

    async fn complete_input(request: String, ctx: CallContext) -> Option<String> {
        let f = with(|h| h.complete_input.clone())?;
        f(parse(&request), Ctx::new(ctx))
            .await
            .map(|v| v.to_string())
    }

    async fn shutdown(reason: String) {
        if let Some(f) = with(|h| h.shutdown.clone()) {
            f(reason).await;
        }
    }
}

#[cfg(feature = "provider")]
impl<E: Extension> crate::bindings::exports::oino::extension::provider::Guest for Shim<E> {
    async fn stream(request: String) -> Result<wit_bindgen::StreamReader<String>, String> {
        let p = instance::<E>().provider().ok_or("not a provider")?;
        p.stream(parse(&request))
            .await
            .map(|s| s.reader)
            .map_err(|e| e.to_string())
    }

    async fn list_models(
        provider_id: String,
        credential: Option<String>,
    ) -> Result<String, String> {
        let p = instance::<E>().provider().ok_or("not a provider")?;
        p.list_models(provider_id, credential.map(|c| parse(&c)))
            .await
            .map(|v| v.to_string())
            .map_err(|e| e.to_string())
    }

    async fn oauth_login(provider_id: String) -> Result<String, String> {
        let p = instance::<E>().provider().ok_or("not a provider")?;
        p.oauth_login(provider_id)
            .await
            .map(|v| v.to_string())
            .map_err(|e| e.to_string())
    }

    async fn oauth_refresh(provider_id: String, credential: String) -> Result<String, String> {
        let p = instance::<E>().provider().ok_or("not a provider")?;
        p.oauth_refresh(provider_id, parse(&credential))
            .await
            .map(|v| v.to_string())
            .map_err(|e| e.to_string())
    }
}

/// Unused error anchor (keeps `Error` in scope for doc links).
#[allow(dead_code, reason = "doc anchor")]
fn _anchor(_: Error) {}
