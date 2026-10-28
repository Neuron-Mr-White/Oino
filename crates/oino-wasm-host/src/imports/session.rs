//! `session` and `control`.

use std::future::Future;
use std::sync::Arc;

use oino_extension_api::ops::{
    ControlOp, ControlReply, SessionQuery, SessionWrite, SessionsIndexQuery, SessionsScope,
};
use oino_extension_api::{CallContext, HostError, RawJson};
use wasmtime::component::{Access, Accessor, Resource};

use super::{out, ready, ready_wit};
use crate::bindings::oino::extension::{control as wc, session as ws, types as wt};
use crate::convert::{self, json};
use crate::resources::CancelToken;
use crate::state::{ExtShared, HostState, current, current_async};

type HostResult<T> = wasmtime::Result<Result<T, wt::HostError>>;

fn read(ext: &ExtShared, cx: &CallContext, q: SessionQuery) -> Result<RawJson, HostError> {
    ext.host.session_read(cx, q)
}

fn read_json(ext: &ExtShared, cx: &CallContext, q: SessionQuery) -> String {
    read(ext, cx, q).map_or_else(|_| "null".to_owned(), |j| out(&j))
}

fn read_opt_string(ext: &ExtShared, cx: &CallContext, q: SessionQuery) -> Option<String> {
    read(ext, cx, q)
        .ok()
        .and_then(|j| j.to_value().as_str().map(ToOwned::to_owned))
}

fn read_opt_json(ext: &ExtShared, cx: &CallContext, q: SessionQuery) -> Option<String> {
    read(ext, cx, q)
        .ok()
        .filter(|j| j.as_str() != "null")
        .map(|j| out(&j))
}

async fn write(
    ext: Arc<ExtShared>,
    cx: CallContext,
    w: SessionWrite,
) -> Result<Option<String>, HostError> {
    ext.host.session_write(&ext.id, &cx, w).await
}

impl ws::Host for HostState {}

impl<T: Send> ws::HostWithStore<T> for HostState {
    fn header(
        mut host: Access<'_, T, Self>,
    ) -> impl Future<Output = wasmtime::Result<String>> + Send {
        let (ext, cx) = current(&mut host);
        ready(read_json(&ext, &cx, SessionQuery::Header))
    }
    fn session_id(
        mut host: Access<'_, T, Self>,
    ) -> impl Future<Output = wasmtime::Result<String>> + Send {
        let (ext, cx) = current(&mut host);
        ready(read_opt_string(&ext, &cx, SessionQuery::SessionId).unwrap_or(cx.session_id))
    }
    fn session_file(
        mut host: Access<'_, T, Self>,
    ) -> impl Future<Output = wasmtime::Result<Option<String>>> + Send {
        let (ext, cx) = current(&mut host);
        ready(read_opt_string(&ext, &cx, SessionQuery::SessionFile))
    }
    fn leaf_id(
        mut host: Access<'_, T, Self>,
    ) -> impl Future<Output = wasmtime::Result<Option<String>>> + Send {
        let (ext, cx) = current(&mut host);
        ready(read_opt_string(&ext, &cx, SessionQuery::LeafId))
    }
    fn entry(
        mut host: Access<'_, T, Self>,
        id: String,
    ) -> impl Future<Output = wasmtime::Result<Option<String>>> + Send {
        let (ext, cx) = current(&mut host);
        ready(read_opt_json(&ext, &cx, SessionQuery::Entry { id }))
    }
    fn entries(
        mut host: Access<'_, T, Self>,
    ) -> impl Future<Output = wasmtime::Result<String>> + Send {
        let (ext, cx) = current(&mut host);
        ready(read_json(&ext, &cx, SessionQuery::Entries))
    }
    fn branch(
        mut host: Access<'_, T, Self>,
        from: Option<String>,
    ) -> impl Future<Output = wasmtime::Result<String>> + Send {
        let (ext, cx) = current(&mut host);
        ready(read_json(&ext, &cx, SessionQuery::Branch { from }))
    }
    fn context_entries(
        mut host: Access<'_, T, Self>,
    ) -> impl Future<Output = wasmtime::Result<String>> + Send {
        let (ext, cx) = current(&mut host);
        ready(read_json(&ext, &cx, SessionQuery::ContextEntries))
    }
    fn projection(
        mut host: Access<'_, T, Self>,
    ) -> impl Future<Output = wasmtime::Result<String>> + Send {
        let (ext, cx) = current(&mut host);
        ready(read_json(&ext, &cx, SessionQuery::Projection))
    }
    fn tree(
        mut host: Access<'_, T, Self>,
    ) -> impl Future<Output = wasmtime::Result<String>> + Send {
        let (ext, cx) = current(&mut host);
        ready(read_json(&ext, &cx, SessionQuery::Tree))
    }
    fn label(
        mut host: Access<'_, T, Self>,
        id: String,
    ) -> impl Future<Output = wasmtime::Result<Option<String>>> + Send {
        let (ext, cx) = current(&mut host);
        ready(read_opt_string(&ext, &cx, SessionQuery::Label { id }))
    }
    fn session_name(
        mut host: Access<'_, T, Self>,
    ) -> impl Future<Output = wasmtime::Result<Option<String>>> + Send {
        let (ext, cx) = current(&mut host);
        ready(read_opt_string(&ext, &cx, SessionQuery::SessionName))
    }

    fn set_session_name(
        mut host: Access<'_, T, Self>,
        name: Option<String>,
    ) -> impl Future<Output = HostResult<()>> + Send {
        let (ext, cx) = current(&mut host);
        async move {
            let r = match ExtShared::require(ext.perms().session_control, "sessionControl") {
                Ok(()) => write(ext, cx, SessionWrite::SetSessionName(name))
                    .await
                    .map(|_| ()),
                Err(e) => Err(e),
            };
            Ok(convert::wit(r))
        }
    }

    fn set_label(
        mut host: Access<'_, T, Self>,
        id: String,
        label: Option<String>,
    ) -> impl Future<Output = HostResult<()>> + Send {
        let (ext, cx) = current(&mut host);
        async move {
            let r = match ExtShared::require(ext.perms().session_control, "sessionControl") {
                Ok(()) => write(ext, cx, SessionWrite::SetLabel { id, label })
                    .await
                    .map(|_| ()),
                Err(e) => Err(e),
            };
            Ok(convert::wit(r))
        }
    }

    fn append_entry(
        mut host: Access<'_, T, Self>,
        custom_type: String,
        data: Option<String>,
    ) -> impl Future<Output = HostResult<String>> + Send {
        let (ext, cx) = current(&mut host);
        async move {
            let r = match data.map(json).transpose() {
                Ok(data) => write(ext, cx, SessionWrite::AppendEntry { custom_type, data })
                    .await
                    .and_then(|id| {
                        id.ok_or_else(|| HostError::Failed("append-entry returned no id".into()))
                    }),
                Err(e) => Err(e),
            };
            Ok(convert::wit(r))
        }
    }

    async fn send_message(
        accessor: &Accessor<T, Self>,
        message: String,
        options: ws::SendOptions,
    ) -> HostResult<()> {
        let (ext, cx) = current_async(accessor);
        let r = match json(message) {
            Ok(message) => write(
                ext,
                cx,
                SessionWrite::SendMessage {
                    message,
                    options: convert::send_options(options),
                },
            )
            .await
            .map(|_| ()),
            Err(e) => Err(e),
        };
        Ok(convert::wit(r))
    }

    async fn send_user_message(
        accessor: &Accessor<T, Self>,
        content: String,
        options: ws::UserMessageOptions,
    ) -> HostResult<()> {
        let (ext, cx) = current_async(accessor);
        let r = match (
            ExtShared::require(ext.perms().session_control, "sessionControl"),
            json(content),
        ) {
            (Ok(()), Ok(content)) => write(
                ext,
                cx,
                SessionWrite::SendUserMessage {
                    content,
                    options: convert::user_options(options),
                },
            )
            .await
            .map(|_| ()),
            (Err(e), _) | (_, Err(e)) => Err(e),
        };
        Ok(convert::wit(r))
    }

    async fn sessions_list(
        accessor: &Accessor<T, Self>,
        scope: ws::SessionsScope,
    ) -> HostResult<String> {
        let (ext, _) = current_async(accessor);
        let scope = match scope {
            ws::SessionsScope::Cwd => SessionsScope::Cwd,
            ws::SessionsScope::All => SessionsScope::All,
        };
        let r = match ExtShared::require(ext.perms().sessions.read_all, "sessions.readAll") {
            Ok(()) => ext
                .host
                .sessions_index(&ext.id, SessionsIndexQuery::List(scope))
                .await
                .map(|j| out(&j)),
            Err(e) => Err(e),
        };
        Ok(convert::wit(r))
    }

    async fn session_read(accessor: &Accessor<T, Self>, path: String) -> HostResult<String> {
        let (ext, _) = current_async(accessor);
        let r = match ExtShared::require(ext.perms().sessions.read_all, "sessions.readAll") {
            Ok(()) => ext
                .host
                .sessions_index(&ext.id, SessionsIndexQuery::Read { path })
                .await
                .map(|j| out(&j)),
            Err(e) => Err(e),
        };
        Ok(convert::wit(r))
    }
}

/// Permission + context checks, then `HostServices::control`.
pub(crate) async fn control(
    ext: Arc<ExtShared>,
    cx: CallContext,
    op: ControlOp,
) -> Result<ControlReply, HostError> {
    if op.needs_session_control() {
        ExtShared::require(ext.perms().session_control, "sessionControl")?;
    }
    if op.needs_command_context() && !cx.kind.is_command_context() {
        return Err(HostError::CommandContextRequired);
    }
    match &op {
        ControlOp::ExecuteTool { .. } if cx.tool_call_id.is_none() => {
            return Err(HostError::InvalidArgument(
                "execute-tool is only available while serving a tool call".into(),
            ));
        }
        ControlOp::BashDetach { .. } => {
            ExtShared::require(ext.perms().process.spawn, "process.spawn")?;
        }
        _ => {}
    }
    ext.host.control(&ext.id, &cx, op).await
}

fn as_json(r: Result<ControlReply, HostError>) -> Result<String, HostError> {
    r.map(|reply| match reply {
        ControlReply::Json(j) | ControlReply::MaybeJson(Some(j)) => out(&j),
        ControlReply::MaybeJson(None) | ControlReply::Unit => "null".into(),
        ControlReply::Bool(b) => b.to_string(),
        ControlReply::String(s) => serde_json::Value::String(s).to_string(),
        ControlReply::Strings(s) => serde_json::to_string(&s).unwrap_or_else(|_| "[]".into()),
    })
}

fn as_bool(r: Result<ControlReply, HostError>) -> Result<bool, HostError> {
    r.map(|reply| match reply {
        ControlReply::Bool(b) => b,
        ControlReply::Json(j) | ControlReply::MaybeJson(Some(j)) => {
            j.to_value().as_bool().unwrap_or(false)
        }
        _ => false,
    })
}

fn as_string(r: Result<ControlReply, HostError>) -> Result<String, HostError> {
    r.map(|reply| match reply {
        ControlReply::String(s) => s,
        ControlReply::Json(j) | ControlReply::MaybeJson(Some(j)) => j
            .to_value()
            .as_str()
            .map_or_else(|| out(&j), ToOwned::to_owned),
        _ => String::new(),
    })
}

fn as_strings(r: Result<ControlReply, HostError>) -> Result<Vec<String>, HostError> {
    r.map(|reply| match reply {
        ControlReply::Strings(s) => s,
        ControlReply::Json(j) => j.decode().unwrap_or_default(),
        _ => Vec::new(),
    })
}

fn as_opt_json(r: Result<ControlReply, HostError>) -> Result<Option<String>, HostError> {
    r.map(|reply| match reply {
        ControlReply::Json(j) | ControlReply::MaybeJson(Some(j)) if j.as_str() != "null" => {
            Some(out(&j))
        }
        _ => None,
    })
}

macro_rules! ctl {
    ($host:ident, $op:expr, $conv:ident, $fallback:expr) => {{
        let (ext, cx) = current(&mut $host);
        async move { Ok($conv(control(ext, cx, $op).await).unwrap_or_else(|_| $fallback)) }
    }};
}

macro_rules! ctl_wit {
    ($host:ident, $op:expr, $conv:ident) => {{
        let (ext, cx) = current(&mut $host);
        async move { Ok(convert::wit($conv(control(ext, cx, $op).await))) }
    }};
}

macro_rules! ctl_async {
    ($accessor:ident, $op:expr, $conv:ident) => {{
        let (ext, cx) = current_async($accessor);
        Ok(convert::wit($conv(control(ext, cx, $op).await)))
    }};
}

impl wc::Host for HostState {}

impl<T: Send> wc::HostWithStore<T> for HostState {
    fn model(
        mut host: Access<'_, T, Self>,
    ) -> impl Future<Output = wasmtime::Result<Option<String>>> + Send {
        ctl!(host, ControlOp::Model, as_opt_json, None)
    }
    fn routed_model(
        mut host: Access<'_, T, Self>,
    ) -> impl Future<Output = wasmtime::Result<Option<String>>> + Send {
        ctl!(host, ControlOp::RoutedModel, as_opt_json, None)
    }
    fn set_model(
        mut host: Access<'_, T, Self>,
        provider: String,
        id: String,
    ) -> impl Future<Output = HostResult<bool>> + Send {
        ctl_wit!(host, ControlOp::SetModel { provider, id }, as_bool)
    }
    fn set_default_model(
        mut host: Access<'_, T, Self>,
        provider: String,
        id: String,
        thinking: Option<String>,
    ) -> impl Future<Output = HostResult<bool>> + Send {
        ctl_wit!(
            host,
            ControlOp::SetDefaultModel {
                provider,
                id,
                thinking,
            },
            as_bool
        )
    }
    fn thinking_level(
        mut host: Access<'_, T, Self>,
    ) -> impl Future<Output = wasmtime::Result<String>> + Send {
        ctl!(host, ControlOp::ThinkingLevel, as_string, "off".to_owned())
    }
    fn set_thinking_level(
        mut host: Access<'_, T, Self>,
        level: String,
    ) -> impl Future<Output = HostResult<String>> + Send {
        ctl_wit!(host, ControlOp::SetThinkingLevel(level), as_string)
    }
    fn models_list(
        mut host: Access<'_, T, Self>,
    ) -> impl Future<Output = wasmtime::Result<String>> + Send {
        ctl!(host, ControlOp::ModelsList, as_json, "[]".to_owned())
    }
    fn scoped_models(
        mut host: Access<'_, T, Self>,
    ) -> impl Future<Output = wasmtime::Result<String>> + Send {
        ctl!(host, ControlOp::ScopedModels, as_json, "[]".to_owned())
    }
    fn run_state(
        mut host: Access<'_, T, Self>,
    ) -> impl Future<Output = wasmtime::Result<String>> + Send {
        ctl!(host, ControlOp::RunState, as_json, "null".to_owned())
    }
    fn is_idle(
        mut host: Access<'_, T, Self>,
    ) -> impl Future<Output = wasmtime::Result<bool>> + Send {
        ctl!(host, ControlOp::IsIdle, as_bool, true)
    }
    fn has_pending_messages(
        mut host: Access<'_, T, Self>,
    ) -> impl Future<Output = wasmtime::Result<bool>> + Send {
        ctl!(host, ControlOp::HasPendingMessages, as_bool, false)
    }
    fn context_usage(
        mut host: Access<'_, T, Self>,
    ) -> impl Future<Output = wasmtime::Result<String>> + Send {
        ctl!(host, ControlOp::ContextUsage, as_json, "null".to_owned())
    }
    fn session_stats(
        mut host: Access<'_, T, Self>,
    ) -> impl Future<Output = wasmtime::Result<String>> + Send {
        ctl!(host, ControlOp::SessionStats, as_json, "null".to_owned())
    }
    fn system_prompt(
        mut host: Access<'_, T, Self>,
    ) -> impl Future<Output = wasmtime::Result<String>> + Send {
        ctl!(host, ControlOp::SystemPrompt, as_string, String::new())
    }
    fn is_project_trusted(
        mut host: Access<'_, T, Self>,
    ) -> impl Future<Output = wasmtime::Result<bool>> + Send {
        ctl!(host, ControlOp::IsProjectTrusted, as_bool, false)
    }
    fn get_settings(
        mut host: Access<'_, T, Self>,
    ) -> impl Future<Output = wasmtime::Result<String>> + Send {
        ctl!(host, ControlOp::GetSettings, as_json, "{}".to_owned())
    }
    fn abort(mut host: Access<'_, T, Self>) -> impl Future<Output = wasmtime::Result<()>> + Send {
        let (ext, cx) = current(&mut host);
        async move {
            let _ = control(ext, cx, ControlOp::Abort).await;
            Ok(())
        }
    }
    async fn compact(
        accessor: &Accessor<T, Self>,
        custom_instructions: Option<String>,
    ) -> HostResult<String> {
        ctl_async!(
            accessor,
            ControlOp::Compact {
                custom_instructions
            },
            as_json
        )
    }
    fn shutdown(
        mut host: Access<'_, T, Self>,
    ) -> impl Future<Output = wasmtime::Result<()>> + Send {
        let (ext, cx) = current(&mut host);
        async move {
            let _ = control(ext, cx, ControlOp::Shutdown).await;
            Ok(())
        }
    }
    fn system_prompt_options(
        mut host: Access<'_, T, Self>,
    ) -> impl Future<Output = HostResult<String>> + Send {
        ctl_wit!(host, ControlOp::SystemPromptOptions, as_json)
    }
    async fn wait_for_idle(accessor: &Accessor<T, Self>) -> HostResult<()> {
        let (ext, cx) = current_async(accessor);
        Ok(convert::wit(
            control(ext, cx, ControlOp::WaitForIdle).await.map(|_| ()),
        ))
    }
    async fn new_session(
        accessor: &Accessor<T, Self>,
        parent_session: Option<String>,
    ) -> HostResult<bool> {
        ctl_async!(accessor, ControlOp::NewSession { parent_session }, as_bool)
    }
    async fn fork(accessor: &Accessor<T, Self>, entry_id: String, at: bool) -> HostResult<String> {
        ctl_async!(accessor, ControlOp::Fork { entry_id, at }, as_json)
    }
    async fn switch_session(accessor: &Accessor<T, Self>, path: String) -> HostResult<bool> {
        ctl_async!(accessor, ControlOp::SwitchSession { path }, as_bool)
    }
    async fn navigate_tree(
        accessor: &Accessor<T, Self>,
        target_id: String,
        options: String,
    ) -> HostResult<String> {
        let (ext, cx) = current_async(accessor);
        let r = match json(options) {
            Ok(options) => {
                as_json(control(ext, cx, ControlOp::NavigateTree { target_id, options }).await)
            }
            Err(e) => Err(e),
        };
        Ok(convert::wit(r))
    }
    async fn reload(accessor: &Accessor<T, Self>) -> HostResult<()> {
        let (ext, cx) = current_async(accessor);
        Ok(convert::wit(
            control(ext, cx, ControlOp::Reload).await.map(|_| ()),
        ))
    }
    fn active_tools(
        mut host: Access<'_, T, Self>,
    ) -> impl Future<Output = wasmtime::Result<Vec<String>>> + Send {
        ctl!(host, ControlOp::ActiveTools, as_strings, Vec::new())
    }
    fn set_active_tools(
        mut host: Access<'_, T, Self>,
        names: Vec<String>,
    ) -> impl Future<Output = HostResult<()>> + Send {
        let (ext, cx) = current(&mut host);
        async move {
            Ok(convert::wit(
                control(ext, cx, ControlOp::SetActiveTools(names))
                    .await
                    .map(|_| ()),
            ))
        }
    }
    fn all_tools(
        mut host: Access<'_, T, Self>,
    ) -> impl Future<Output = wasmtime::Result<String>> + Send {
        ctl!(host, ControlOp::AllTools, as_json, "[]".to_owned())
    }
    fn commands_list(
        mut host: Access<'_, T, Self>,
    ) -> impl Future<Output = wasmtime::Result<String>> + Send {
        ctl!(host, ControlOp::CommandsList, as_json, "[]".to_owned())
    }
    async fn command_invoke(
        accessor: &Accessor<T, Self>,
        name: String,
        args: String,
    ) -> HostResult<Option<String>> {
        ctl_async!(
            accessor,
            ControlOp::CommandInvoke { name, args },
            as_opt_json
        )
    }
    async fn execute_tool(
        accessor: &Accessor<T, Self>,
        name: String,
        args: String,
        cancel: Option<Resource<CancelToken>>,
    ) -> HostResult<String> {
        let (ext, cx) = current_async(accessor);
        let cancel = match cancel {
            Some(res) => Some(accessor.with(|mut a| a.get().table.get(&res).map(|t| t.0.clone()))?),
            None => None,
        };
        let r = match json(args) {
            Ok(args) => {
                as_json(control(ext, cx, ControlOp::ExecuteTool { name, args, cancel }).await)
            }
            Err(e) => Err(e),
        };
        Ok(convert::wit(r))
    }
    fn tool_progress(
        mut host: Access<'_, T, Self>,
        tool_call_id: String,
        partial: String,
    ) -> impl Future<Output = wasmtime::Result<()>> + Send {
        let (ext, cx) = current(&mut host);
        if let Ok(partial) = json(partial) {
            ext.host.tool_progress(&cx, &tool_call_id, partial);
        }
        ready(())
    }
    fn bash_detach(
        mut host: Access<'_, T, Self>,
        tool_call_id: String,
        reason: String,
    ) -> impl Future<Output = HostResult<String>> + Send {
        ctl_wit!(
            host,
            ControlOp::BashDetach {
                tool_call_id,
                reason
            },
            as_string
        )
    }
    fn running_bash(
        mut host: Access<'_, T, Self>,
    ) -> impl Future<Output = wasmtime::Result<String>> + Send {
        ctl!(host, ControlOp::RunningBash, as_json, "[]".to_owned())
    }
}

#[allow(dead_code, reason = "keeps ready_wit referenced for symmetric helpers")]
fn _unused() -> impl Future<Output = HostResult<()>> {
    ready_wit(Ok(()))
}
