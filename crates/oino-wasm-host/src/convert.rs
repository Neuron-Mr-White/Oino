//! Conversions between generated WIT types and `oino-extension-api` types.

use oino_extension_api::registry as ar;
use oino_extension_api::{CallContext, CallKind, EventKind, HostError, Mode, RawJson, ops as ao};

use crate::bindings::exports::oino::extension::guest as wg;
use crate::bindings::oino::extension::{registry as wr, session as ws, types as wt};

/// Parses guest JSON, mapping failures to `invalid-argument`.
pub(crate) fn json(text: String) -> Result<RawJson, HostError> {
    RawJson::parse(text).map_err(|e| HostError::InvalidArgument(format!("invalid JSON: {e}")))
}

/// Parses guest JSON returned from an export (falls back to a JSON string of the text).
pub(crate) fn json_lossy(text: String) -> RawJson {
    RawJson::parse(text.clone()).unwrap_or_else(|_| RawJson::from(serde_json::Value::String(text)))
}

pub(crate) fn host_error(e: HostError) -> wt::HostError {
    match e {
        HostError::Denied(s) => wt::HostError::Denied(s),
        HostError::StaleContext => wt::HostError::StaleContext,
        HostError::CommandContextRequired => wt::HostError::CommandContextRequired,
        HostError::InvalidArgument(s) => wt::HostError::InvalidArgument(s),
        HostError::NotFound(s) => wt::HostError::NotFound(s),
        HostError::Timeout => wt::HostError::Timeout,
        HostError::Cancelled => wt::HostError::Cancelled,
        HostError::Io(s) => wt::HostError::Io(s),
        HostError::Failed(s) => wt::HostError::Failed(s),
        other => wt::HostError::Failed(other.to_string()),
    }
}

/// `Result<T, HostError>` → WIT result.
pub(crate) fn wit<T>(r: Result<T, HostError>) -> Result<T, wt::HostError> {
    r.map_err(host_error)
}

pub(crate) fn mode(m: Mode) -> wt::Mode {
    match m {
        Mode::Tui => wt::Mode::Tui,
        Mode::Rpc => wt::Mode::Rpc,
        Mode::Json => wt::Mode::Json,
        Mode::Print => wt::Mode::Print,
        Mode::Acp => wt::Mode::Acp,
        Mode::Desktop => wt::Mode::Desktop,
        Mode::Mobile => wt::Mode::Mobile,
    }
}

fn kind(k: CallKind) -> wt::CallKind {
    match k {
        CallKind::Init => wt::CallKind::Init,
        CallKind::Event => wt::CallKind::Event,
        CallKind::Tool => wt::CallKind::Tool,
        CallKind::Command => wt::CallKind::Command,
        CallKind::Shortcut => wt::CallKind::Shortcut,
        CallKind::Service => wt::CallKind::Service,
        CallKind::Nudge => wt::CallKind::Nudge,
        CallKind::Render => wt::CallKind::Render,
        CallKind::Route => wt::CallKind::Route,
        CallKind::Provider => wt::CallKind::Provider,
    }
}

pub(crate) fn call_context(cx: &CallContext) -> wt::CallContext {
    wt::CallContext {
        session_id: cx.session_id.clone(),
        mode: mode(cx.mode),
        has_ui: cx.has_ui,
        cwd: cx.cwd.to_string_lossy().into_owned(),
        is_child: cx.is_child,
        child_role: cx.child_role.clone(),
        kind: kind(cx.kind),
        tool_call_id: cx.tool_call_id.clone(),
    }
}

macro_rules! event_map {
    ($($v:ident),* $(,)?) => {
        pub(crate) fn event_to_wit(k: EventKind) -> wr::EventKind {
            match k { $(EventKind::$v => wr::EventKind::$v,)* _ => wr::EventKind::Input }
        }
        pub(crate) fn event_from_wit(k: wr::EventKind) -> EventKind {
            match k { $(wr::EventKind::$v => EventKind::$v,)* }
        }
    };
}

event_map!(
    ProjectTrust,
    ResourcesDiscover,
    McpServersChange,
    SessionStart,
    SessionInfoChanged,
    SessionBeforeSwitch,
    SessionBeforeFork,
    SessionBeforeCompact,
    SessionCompact,
    SessionCompactFailed,
    SessionShutdown,
    SessionBeforeTree,
    SessionTree,
    Context,
    ContextWithSystem,
    CacheWarmingDecision,
    BeforeProviderHeaders,
    BeforeProviderRequest,
    AfterProviderResponse,
    ProviderStreamEvent,
    BeforeAgentStart,
    AgentStart,
    AgentEnd,
    AgentBeforeSettle,
    AgentSettled,
    UiPromptStart,
    UiPromptEnd,
    TurnStart,
    TurnEnd,
    MessageStart,
    MessageUpdate,
    MessageEnd,
    ToolExecutionStart,
    ToolExecutionUpdate,
    ToolExecutionEnd,
    ModelSelect,
    ThinkingLevelSelect,
    ToolCall,
    ToolResult,
    UserBash,
    Input,
    OinoTimer,
    OinoBus,
    OinoUiAction,
    OinoEditor,
    OinoSettingsChanged,
    OinoProcess,
    OinoAgent,
    OinoTask,
);

pub(crate) fn events_from_wit(list: Vec<wr::EventKind>) -> Vec<EventKind> {
    list.into_iter().map(event_from_wit).collect()
}

pub(crate) fn tool_spec(s: wr::ToolSpec) -> Result<ar::ToolSpec, HostError> {
    Ok(ar::ToolSpec {
        name: s.name,
        label: s.label,
        description: s.description,
        parameters: json(s.parameters)?,
        output_schema: s.output_schema.map(json).transpose()?,
        prompt_snippet: s.prompt_snippet,
        prompt_guidelines: s.prompt_guidelines,
        exposure: match s.exposure {
            wr::Exposure::Direct => ar::Exposure::Direct,
            wr::Exposure::ModelOnly => ar::Exposure::ModelOnly,
            wr::Exposure::Deferred => ar::Exposure::Deferred,
            wr::Exposure::Hidden => ar::Exposure::Hidden,
        },
        default_active: s.default_active,
        execution_mode: match s.execution_mode {
            wr::ExecutionMode::Parallel => ar::ExecutionMode::Parallel,
            wr::ExecutionMode::Sequential => ar::ExecutionMode::Sequential,
        },
        replay: s.replay.map(|r| match r {
            wr::Replay::Never => ar::Replay::Never,
            wr::Replay::Safe => ar::Replay::Safe,
        }),
        kind: match s.kind {
            wr::ToolKind::Read => ar::ToolKind::Read,
            wr::ToolKind::Edit => ar::ToolKind::Edit,
            wr::ToolKind::Delete => ar::ToolKind::Delete,
            wr::ToolKind::Move => ar::ToolKind::Move,
            wr::ToolKind::Search => ar::ToolKind::Search,
            wr::ToolKind::Execute => ar::ToolKind::Execute,
            wr::ToolKind::Think => ar::ToolKind::Think,
            wr::ToolKind::Fetch => ar::ToolKind::Fetch,
            wr::ToolKind::SwitchMode => ar::ToolKind::SwitchMode,
            wr::ToolKind::Other => ar::ToolKind::Other,
        },
        namespace: s.namespace.map(|n| ar::ToolNamespace {
            name: n.name,
            aliases: n.aliases,
            description: n.description,
            instructions: n.instructions,
        }),
        annotations: s.annotations.map(|a| ar::ToolAnnotations {
            read_only: a.read_only,
            destructive: a.destructive,
            idempotent: a.idempotent,
            open_world: a.open_world,
        }),
        reaches_tools: match s.reaches_tools {
            wr::ReachesTools::None => ar::ReachesTools::None,
            wr::ReachesTools::All => ar::ReachesTools::All,
            wr::ReachesTools::Names(n) => ar::ReachesTools::Names(n),
            wr::ReachesTools::Dynamic => ar::ReachesTools::Dynamic,
        },
        prepare_arguments: s.prepare_arguments,
        prepare_loadout: s.prepare_loadout,
        render: s.render.map(json).transpose()?,
    })
}

pub(crate) fn command_spec(s: wr::CommandSpec) -> ar::CommandSpec {
    ar::CommandSpec {
        name: s.name,
        description: s.description,
        argument_hint: s.argument_hint,
        completions: s.completions,
        hidden: s.hidden,
    }
}

pub(crate) fn shortcut_spec(s: wr::ShortcutSpec) -> ar::ShortcutSpec {
    ar::ShortcutSpec {
        id: s.id,
        default_keys: s.default_keys,
        description: s.description,
    }
}

pub(crate) fn flag_spec(s: wr::FlagSpec) -> Result<ar::FlagSpec, HostError> {
    Ok(ar::FlagSpec {
        name: s.name,
        kind: match s.type_ {
            wr::FlagType::Boolean => ar::FlagType::Boolean,
            wr::FlagType::String => ar::FlagType::String,
        },
        default_value: s.default_value.map(json).transpose()?,
        description: s.description,
    })
}

pub(crate) fn renderer_spec(s: wr::RendererSpec) -> ar::RendererSpec {
    ar::RendererSpec {
        target: match s.target {
            wr::RendererTarget::CustomMessage => ar::RendererTarget::CustomMessage,
            wr::RendererTarget::CustomEntry => ar::RendererTarget::CustomEntry,
            wr::RendererTarget::Tool => ar::RendererTarget::Tool,
        },
        key: s.key,
    }
}

pub(crate) fn arbiter(r: wr::ArbiterRoles) -> ar::ArbiterRoles {
    ar::ArbiterRoles {
        nudge_provider: r.nudge_provider,
        wait_source: r.wait_source,
    }
}

pub(crate) fn registration(r: wr::Registration) -> Result<ar::Registration, HostError> {
    Ok(ar::Registration {
        events: events_from_wit(r.events),
        tools: r
            .tools
            .into_iter()
            .map(tool_spec)
            .collect::<Result<_, _>>()?,
        commands: r.commands.into_iter().map(command_spec).collect(),
        shortcuts: r.shortcuts.into_iter().map(shortcut_spec).collect(),
        flags: r
            .flags
            .into_iter()
            .map(flag_spec)
            .collect::<Result<_, _>>()?,
        renderers: r.renderers.into_iter().map(renderer_spec).collect(),
        services: r.services,
        arbiter: arbiter(r.arbiter),
        settings: r.settings.into_iter().map(json).collect::<Result<_, _>>()?,
    })
}

fn delivery(d: ws::Delivery) -> ao::Delivery {
    match d {
        ws::Delivery::Steer => ao::Delivery::Steer,
        ws::Delivery::FollowUp => ao::Delivery::FollowUp,
        ws::Delivery::NextTurn => ao::Delivery::NextTurn,
    }
}

fn provenance(p: ws::Provenance) -> ao::Provenance {
    ao::Provenance {
        source: p.source,
        title: p.title,
        synopsis: p.synopsis,
        severity: p.severity,
    }
}

pub(crate) fn send_options(o: ws::SendOptions) -> ao::SendOptions {
    ao::SendOptions {
        trigger_turn: o.trigger_turn,
        deliver_as: o.deliver_as.map(delivery),
        provenance: o.provenance.map(provenance),
    }
}

pub(crate) fn user_options(o: ws::UserMessageOptions) -> ao::UserMessageOptions {
    ao::UserMessageOptions {
        deliver_as: o.deliver_as.map(delivery),
        expand_prompt_templates: o.expand_prompt_templates,
        provenance: o.provenance.map(provenance),
    }
}

pub(crate) fn completion_item(c: wg::CompletionItem) -> oino_extension_api::CompletionItem {
    oino_extension_api::CompletionItem {
        value: c.value,
        label: c.label,
        description: c.description,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_event_maps_both_ways() {
        for k in EventKind::ALL {
            assert_eq!(event_from_wit(event_to_wit(*k)), *k);
        }
    }
}
