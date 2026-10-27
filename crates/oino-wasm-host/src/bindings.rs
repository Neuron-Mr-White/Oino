//! Generated bindings for world `provider-extension` (a superset of `extension`).
#![allow(
    missing_docs,
    clippy::all,
    clippy::pedantic,
    unreachable_pub,
    unused_qualifications
)]
wasmtime::component::bindgen!({
    path: "../../wit/oino-extension",
    world: "provider-extension",
    imports: { default: async | store | trappable },
    exports: { default: async | store },
    with: {
        "wasi": wasmtime_wasi::p2::bindings,
        "oino:extension/types.cancel-token": crate::resources::CancelToken,
        "oino:extension/agents.agent": crate::resources::AgentRes,
        "oino:extension/process.child": crate::resources::ChildRes,
        "oino:extension/http.listener": crate::resources::ListenerRes,
        "oino:extension/fs.lock": crate::resources::LockRes,
        "oino:extension/fs.watcher": crate::resources::WatcherRes,
    },
});

/// Adds every `oino:extension` import to `linker` (WASI is added separately).
pub(crate) fn add_oino_to_linker(
    linker: &mut wasmtime::component::Linker<crate::state::HostState>,
) -> wasmtime::Result<()> {
    use crate::state::HostState;
    use oino::extension as e;
    fn get(s: &mut HostState) -> &mut HostState {
        s
    }
    e::types::add_to_linker::<HostState, HostState>(linker, get)?;
    e::log::add_to_linker::<HostState, HostState>(linker, get)?;
    e::host_info::add_to_linker::<HostState, HostState>(linker, get)?;
    e::registry::add_to_linker::<HostState, HostState>(linker, get)?;
    e::session::add_to_linker::<HostState, HostState>(linker, get)?;
    e::control::add_to_linker::<HostState, HostState>(linker, get)?;
    e::models::add_to_linker::<HostState, HostState>(linker, get)?;
    e::ui::add_to_linker::<HostState, HostState>(linker, get)?;
    e::agents::add_to_linker::<HostState, HostState>(linker, get)?;
    e::process::add_to_linker::<HostState, HostState>(linker, get)?;
    e::tasks::add_to_linker::<HostState, HostState>(linker, get)?;
    e::http::add_to_linker::<HostState, HostState>(linker, get)?;
    e::fs::add_to_linker::<HostState, HostState>(linker, get)?;
    e::timers::add_to_linker::<HostState, HostState>(linker, get)?;
    e::kv::add_to_linker::<HostState, HostState>(linker, get)?;
    e::secrets::add_to_linker::<HostState, HostState>(linker, get)?;
    e::bus::add_to_linker::<HostState, HostState>(linker, get)?;
    e::services::add_to_linker::<HostState, HostState>(linker, get)?;
    e::browser::add_to_linker::<HostState, HostState>(linker, get)?;
    e::provider_host::add_to_linker::<HostState, HostState>(linker, get)?;
    Ok(())
}
