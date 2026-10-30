//! `kanboard` (round-5 #7): in-process `oino-kanboard` commands — the host
//! holds the library, so extension calls never spawn the binary and never
//! touch HTTP. `cwd` defaults to the call's session cwd; when the request
//! leaves both board homes unset the host's agent dir supplies
//! `OINO_AGENT_DIR`, so the board lives next to the session's other state.

use oino_extension_api::{CallContext, HostError};
use wasmtime::component::Access;

use super::ready_wit;
use crate::bindings::oino::extension::{kanboard as wkb, types as wt};
use crate::state::{ExtShared, HostState, current};

type HostResult<T> = wasmtime::Result<Result<T, wt::HostError>>;

impl wkb::Host for HostState {}

impl<T: Send> wkb::HostWithStore<T> for HostState {
    fn exec(
        mut host: Access<'_, T, Self>,
        request: String,
    ) -> impl Future<Output = HostResult<String>> + Send {
        let (ext, cx) = current(&mut host);
        ready_wit(exec(&ext, &cx, &request))
    }
}

fn exec(ext: &ExtShared, cx: &CallContext, request: &str) -> Result<String, HostError> {
    let invalid =
        |e: &dyn std::fmt::Display| HostError::InvalidArgument(format!("kanboard.exec: {e}"));
    let value: serde_json::Value = serde_json::from_str(request).map_err(|e| invalid(&e))?;
    let mut req = unipi_kanboard::service::parse_request(&value).map_err(|e| invalid(&e))?;
    if req.cwd.is_none() {
        req.cwd = Some(cx.cwd.clone());
    }
    // The board home follows the host's agent dir unless the caller pinned a
    // home explicitly (tests do).
    if !req.env.contains_key(unipi_kanboard::store::HOME_ENV)
        && !req.env.contains_key("OINO_AGENT_DIR")
    {
        req.env.insert(
            "OINO_AGENT_DIR".to_owned(),
            ext.config.agent_dir.display().to_string(),
        );
    }
    let payload = unipi_kanboard::service::exec(&req);
    Ok(payload.to_string())
}
