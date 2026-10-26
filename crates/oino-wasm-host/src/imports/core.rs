//! `types` (cancel-token), `log`, `host-info`, `registry`, `provider-host`.

use std::future::Future;

use oino_extension_api::ops::{ProviderHook, SettingsLayer, SettingsScope};
use oino_extension_api::{HostError, LogLevel};
use wasmtime::component::{Access, Accessor, Resource};

use super::{out, ready, ready_wit};
use crate::bindings::oino::extension::{
    host_info as wh, log as wl, provider_host as wp, registry as wr, types as wt,
};
use crate::convert::{self, json};
use crate::resources::CancelToken;
use crate::state::{ExtShared, HostState, current, current_async};

type HostResult<T> = wasmtime::Result<Result<T, wt::HostError>>;

impl wt::Host for HostState {}
impl wt::HostCancelToken for HostState {}

impl<T: Send> wt::HostCancelTokenWithStore<T> for HostState {
    fn drop(
        accessor: &Accessor<T, Self>,
        rep: Resource<CancelToken>,
    ) -> impl Future<Output = wasmtime::Result<()>> + Send {
        let r = accessor.with(|mut a| a.get().table.delete(rep).map(|_| ()));
        std::future::ready(r.map_err(Into::into))
    }

    fn is_cancelled(
        mut host: Access<'_, T, Self>,
        self_: Resource<CancelToken>,
    ) -> impl Future<Output = wasmtime::Result<bool>> + Send {
        let r = host.get().table.get(&self_).map(|t| t.0.is_cancelled());
        std::future::ready(r.map_err(Into::into))
    }

    async fn wait(
        accessor: &Accessor<T, Self>,
        self_: Resource<CancelToken>,
    ) -> wasmtime::Result<()> {
        let token = accessor.with(|mut a| a.get().table.get(&self_).map(|t| t.0.clone()))?;
        token.cancelled().await;
        Ok(())
    }
}

impl wl::Host for HostState {}

impl<T: Send> wl::HostWithStore<T> for HostState {
    fn log(
        mut host: Access<'_, T, Self>,
        level: wl::LogLevel,
        message: String,
    ) -> impl Future<Output = wasmtime::Result<()>> + Send {
        let (ext, _) = current(&mut host);
        let level = match level {
            wl::LogLevel::Trace => LogLevel::Trace,
            wl::LogLevel::Debug => LogLevel::Debug,
            wl::LogLevel::Info => LogLevel::Info,
            wl::LogLevel::Warn => LogLevel::Warn,
            wl::LogLevel::Error => LogLevel::Error,
        };
        crate::logging::write(&ext, level, &message);
        ready(())
    }
}

fn platform() -> &'static str {
    match std::env::consts::OS {
        "macos" => "macos",
        "windows" => "windows",
        "android" => "android",
        "ios" => "ios",
        _ => "linux",
    }
}

impl wh::Host for HostState {}

impl<T: Send> wh::HostWithStore<T> for HostState {
    fn get(
        mut host: Access<'_, T, Self>,
    ) -> impl Future<Output = wasmtime::Result<wh::Info>> + Send {
        let (ext, cx) = current(&mut host);
        let ctx = ext.host.host_context(&cx);
        ready(wh::Info {
            oino_version: ext.config.oino_version.clone(),
            api_version: "1.0.0".into(),
            platform: platform().into(),
            arch: std::env::consts::ARCH.into(),
            mode: convert::mode(cx.mode),
            has_ui: cx.has_ui,
            frontends: ctx.frontends,
            extension_version: ext.manifest.version.clone(),
        })
    }

    fn workspace(
        mut host: Access<'_, T, Self>,
    ) -> impl Future<Output = wasmtime::Result<wh::WorkspaceInfo>> + Send {
        let (ext, cx) = current(&mut host);
        let info = ext
            .host
            .host_context(&cx)
            .workspace
            .unwrap_or_else(|| crate::paths::workspace_info(&cx.cwd));
        ready(wh::WorkspaceInfo {
            id: info.id,
            root: info.root,
            git_root: info.git_root,
            git_branch: info.git_branch,
            cwd: info.cwd,
        })
    }

    fn env_get(
        mut host: Access<'_, T, Self>,
        name: String,
    ) -> impl Future<Output = HostResult<Option<String>>> + Send {
        let (ext, _) = current(&mut host);
        let r = ExtShared::require(ext.perms().allows_env(&name), &format!("env:{name}"))
            .map(|()| std::env::var(&name).ok());
        ready_wit(r)
    }

    fn frontend_focused(
        mut host: Access<'_, T, Self>,
    ) -> impl Future<Output = wasmtime::Result<bool>> + Send {
        let (ext, cx) = current(&mut host);
        ready(ext.host.host_context(&cx).frontend_focused)
    }
}

impl wr::Host for HostState {}

impl<T: Send> wr::HostWithStore<T> for HostState {
    fn subscribe(
        mut host: Access<'_, T, Self>,
        events: Vec<wr::EventKind>,
    ) -> impl Future<Output = wasmtime::Result<()>> + Send {
        let (ext, _) = current(&mut host);
        ext.host
            .subscribe(&ext.id, &convert::events_from_wit(events));
        ready(())
    }

    fn unsubscribe(
        mut host: Access<'_, T, Self>,
        events: Vec<wr::EventKind>,
    ) -> impl Future<Output = wasmtime::Result<()>> + Send {
        let (ext, _) = current(&mut host);
        ext.host
            .unsubscribe(&ext.id, &convert::events_from_wit(events));
        ready(())
    }

    fn register_tool(
        mut host: Access<'_, T, Self>,
        spec: wr::ToolSpec,
    ) -> impl Future<Output = HostResult<()>> + Send {
        let (ext, _) = current(&mut host);
        ready_wit(convert::tool_spec(spec).and_then(|s| ext.host.register_tool(&ext.id, s)))
    }

    fn unregister_tool(
        mut host: Access<'_, T, Self>,
        name: String,
    ) -> impl Future<Output = wasmtime::Result<()>> + Send {
        let (ext, _) = current(&mut host);
        ext.host.unregister_tool(&ext.id, &name);
        ready(())
    }

    fn register_command(
        mut host: Access<'_, T, Self>,
        spec: wr::CommandSpec,
    ) -> impl Future<Output = HostResult<String>> + Send {
        let (ext, _) = current(&mut host);
        ready_wit(
            ext.host
                .register_command(&ext.id, convert::command_spec(spec)),
        )
    }

    fn unregister_command(
        mut host: Access<'_, T, Self>,
        name: String,
    ) -> impl Future<Output = wasmtime::Result<()>> + Send {
        let (ext, _) = current(&mut host);
        ext.host.unregister_command(&ext.id, &name);
        ready(())
    }

    fn register_shortcut(
        mut host: Access<'_, T, Self>,
        spec: wr::ShortcutSpec,
    ) -> impl Future<Output = HostResult<()>> + Send {
        let (ext, _) = current(&mut host);
        ready_wit(
            ext.host
                .register_shortcut(&ext.id, convert::shortcut_spec(spec)),
        )
    }

    fn unregister_shortcut(
        mut host: Access<'_, T, Self>,
        id: String,
    ) -> impl Future<Output = wasmtime::Result<()>> + Send {
        let (ext, _) = current(&mut host);
        ext.host.unregister_shortcut(&ext.id, &id);
        ready(())
    }

    fn register_flag(
        mut host: Access<'_, T, Self>,
        spec: wr::FlagSpec,
    ) -> impl Future<Output = HostResult<()>> + Send {
        let (ext, _) = current(&mut host);
        ready_wit(convert::flag_spec(spec).and_then(|s| ext.host.register_flag(&ext.id, s)))
    }

    fn get_flag(
        mut host: Access<'_, T, Self>,
        name: String,
    ) -> impl Future<Output = wasmtime::Result<Option<String>>> + Send {
        let (ext, _) = current(&mut host);
        ready(ext.host.get_flag(&ext.id, &name).as_ref().map(out))
    }

    fn register_renderer(
        mut host: Access<'_, T, Self>,
        spec: wr::RendererSpec,
    ) -> impl Future<Output = wasmtime::Result<()>> + Send {
        let (ext, _) = current(&mut host);
        ext.host
            .register_renderer(&ext.id, convert::renderer_spec(spec));
        ready(())
    }

    fn register_service(
        mut host: Access<'_, T, Self>,
        name: String,
    ) -> impl Future<Output = HostResult<()>> + Send {
        let (ext, _) = current(&mut host);
        ready_wit(ext.host.register_service(&ext.id, &name))
    }

    fn set_arbiter_roles(
        mut host: Access<'_, T, Self>,
        roles: wr::ArbiterRoles,
    ) -> impl Future<Output = wasmtime::Result<()>> + Send {
        let (ext, _) = current(&mut host);
        ext.host.set_arbiter_roles(&ext.id, convert::arbiter(roles));
        ready(())
    }

    fn register_settings(
        mut host: Access<'_, T, Self>,
        namespace: String,
    ) -> impl Future<Output = HostResult<()>> + Send {
        let (ext, _) = current(&mut host);
        ready_wit(json(namespace).and_then(|ns| ext.host.register_settings(&ext.id, ns)))
    }

    fn extension_settings(
        mut host: Access<'_, T, Self>,
        namespace: String,
        layer: wr::SettingsLayer,
    ) -> impl Future<Output = HostResult<String>> + Send {
        let (ext, _) = current(&mut host);
        let layer = match layer {
            wr::SettingsLayer::Effective => SettingsLayer::Effective,
            wr::SettingsLayer::Global => SettingsLayer::Global,
            wr::SettingsLayer::Project => SettingsLayer::Project,
        };
        ready_wit(
            ext.host
                .extension_settings(&ext.id, &namespace, layer)
                .map(|v| v.as_str().to_owned()),
        )
    }

    fn set_extension_setting(
        mut host: Access<'_, T, Self>,
        namespace: String,
        key: String,
        value: Option<String>,
        scope: wr::SettingsScope,
    ) -> impl Future<Output = HostResult<()>> + Send {
        let (ext, _) = current(&mut host);
        let scope = match scope {
            wr::SettingsScope::Global => SettingsScope::Global,
            wr::SettingsScope::Project => SettingsScope::Project,
        };
        ready_wit(value.map(json).transpose().and_then(|value| {
            ext.host
                .set_extension_setting(&ext.id, &namespace, &key, value, scope)
        }))
    }

    fn register_provider(
        mut host: Access<'_, T, Self>,
        config: String,
    ) -> impl Future<Output = HostResult<()>> + Send {
        let (ext, _) = current(&mut host);
        let r = json(config).and_then(|cfg| {
            if let Some(base) = cfg.to_value().get("baseUrl").and_then(|v| v.as_str()) {
                ext.http_grants.add_url(base);
            }
            ext.host.register_provider(&ext.id, cfg)
        });
        ready_wit(r)
    }

    fn unregister_provider(
        mut host: Access<'_, T, Self>,
        provider_id: String,
    ) -> impl Future<Output = wasmtime::Result<()>> + Send {
        let (ext, _) = current(&mut host);
        ext.host.unregister_provider(&ext.id, &provider_id);
        ready(())
    }

    fn register_virtual_model(
        mut host: Access<'_, T, Self>,
        definition: String,
    ) -> impl Future<Output = HostResult<()>> + Send {
        let (ext, _) = current(&mut host);
        ready_wit(json(definition).and_then(|d| ext.host.register_virtual_model(&ext.id, d)))
    }

    fn unregister_virtual_model(
        mut host: Access<'_, T, Self>,
        provider: String,
        id: String,
    ) -> impl Future<Output = wasmtime::Result<()>> + Send {
        let (ext, _) = current(&mut host);
        ext.host.unregister_virtual_model(&ext.id, &provider, &id);
        ready(())
    }
}

impl wp::Host for HostState {}

type Headers = Vec<(String, String)>;

impl<T: Send> wp::HostWithStore<T> for HostState {
    async fn before_request(
        accessor: &Accessor<T, Self>,
        request_id: String,
        payload: String,
        headers: Headers,
    ) -> wasmtime::Result<(String, Headers)> {
        let (ext, _) = current_async(accessor);
        let Ok(raw) = json(payload.clone()) else {
            return Ok((payload, headers));
        };
        let hook = ProviderHook::BeforeRequest {
            request_id,
            payload: raw,
            headers: headers.clone(),
        };
        match ext.host.provider_hook(&ext.id, hook).await {
            oino_extension_api::ops::ProviderHookReply::BeforeRequest { payload, headers } => {
                Ok((out(&payload), headers))
            }
            oino_extension_api::ops::ProviderHookReply::Done => Ok((payload, headers)),
        }
    }

    fn after_response(
        mut host: Access<'_, T, Self>,
        request_id: String,
        status: u16,
        headers: Headers,
    ) -> impl Future<Output = wasmtime::Result<()>> + Send {
        let (ext, _) = current(&mut host);
        async move {
            let _ = ext
                .host
                .provider_hook(
                    &ext.id,
                    ProviderHook::AfterResponse {
                        request_id,
                        status,
                        headers,
                    },
                )
                .await;
            Ok(())
        }
    }

    fn stream_event(
        mut host: Access<'_, T, Self>,
        request_id: String,
        data: String,
    ) -> impl Future<Output = wasmtime::Result<()>> + Send {
        let (ext, _) = current(&mut host);
        async move {
            if let Ok(data) = json(data) {
                let _ = ext
                    .host
                    .provider_hook(&ext.id, ProviderHook::StreamEvent { request_id, data })
                    .await;
            }
            Ok(())
        }
    }
}

/// Unused-import guard for `HostError` in this module's doc examples.
#[allow(dead_code, reason = "type anchor")]
fn _anchor(_: HostError) {}
