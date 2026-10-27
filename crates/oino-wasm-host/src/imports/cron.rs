//! `cron` (unipi-port §3.24): permission check, then `HostServices::cron_call` (the hub's
//! installed [`oino_extension_api::cron::CronHost`], defaulting to the daemon client).

use oino_extension_api::HostError;

use crate::bindings::oino::extension::{cron as wc, types as wt};
use crate::convert::{self, json};
use crate::imports::out;
use crate::state::{ExtShared, HostState, current_async};

impl wc::Host for HostState {}

impl<T: Send> wc::HostWithStore<T> for HostState {
    async fn call(
        accessor: &wasmtime::component::Accessor<T, Self>,
        op: String,
        payload: String,
    ) -> wasmtime::Result<Result<String, wt::HostError>> {
        let (ext, cx) = current_async(accessor);
        let r: Result<String, HostError> =
            match ExtShared::require(ext.perms().cron, "cron").and_then(|()| json(payload)) {
                Ok(payload) => match ext.host.cron_call(&ext.id, &cx, &op, payload).await {
                    Ok(reply) => Ok(out(&reply)),
                    Err(e) => Err(e),
                },
                Err(e) => Err(e),
            };
        Ok(convert::wit(r))
    }
}
