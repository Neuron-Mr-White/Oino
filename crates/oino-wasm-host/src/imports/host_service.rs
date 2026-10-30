//! `host-service` (v2 ext-abi §3): discovery ungated; `call` is permission-gated per
//! service name, served from the hub's registered [`HostService`]s first, then the inner
//! [`HostServices`] host. A name neither provides is `not-found`.

use std::future::Future;

use oino_extension_api::{CancellationToken, HostError};
use wasmtime::component::{Access, Accessor, Resource};

use super::ready;
use crate::bindings::oino::extension::{host_service as whs, types as wt};
use crate::convert::{self, json};
use crate::imports::out;
use crate::resources::CancelToken;
use crate::state::{ExtShared, HostState, current, current_async};

impl whs::Host for HostState {}

impl<T: Send> whs::HostWithStore<T> for HostState {
    async fn call(
        accessor: &Accessor<T, Self>,
        name: String,
        op: String,
        payload: String,
        cancel: Option<Resource<CancelToken>>,
    ) -> wasmtime::Result<Result<String, wt::HostError>> {
        let (ext, cx) = current_async(accessor);
        let cancel = match cancel {
            Some(res) => accessor.with(|mut a| a.get().table.get(&res).map(|t| t.0.clone()))?,
            None => CancellationToken::new(),
        };
        // Permission first (`denied("hostServices:<name>")`), then existence (`not-found`),
        // then the call itself.
        let provided = ext.host.host_services().iter().any(|n| n == &name);
        let r: Result<String, HostError> = match ExtShared::require(
            ext.perms().allows_host_service(&name, ext.bundled),
            &format!("hostServices:{name}"),
        ) {
            Err(e) => Err(e),
            Ok(()) if !provided => Err(HostError::NotFound(format!("host service {name}"))),
            Ok(()) => match json(payload) {
                Ok(payload) => {
                    match ext
                        .host
                        .host_service(&ext.id, &cx, &name, &op, payload, cancel)
                        .await
                    {
                        Ok(reply) => Ok(out(&reply)),
                        Err(e) => Err(e),
                    }
                }
                Err(e) => Err(e),
            },
        };
        Ok(convert::wit(r))
    }

    fn list(
        mut host: Access<'_, T, Self>,
    ) -> impl Future<Output = wasmtime::Result<Vec<String>>> + Send {
        let (ext, _cx) = current(&mut host);
        let mut names = ext.host.host_services();
        names.sort();
        names.dedup();
        ready(names)
    }

    fn available(
        mut host: Access<'_, T, Self>,
        name: String,
    ) -> impl Future<Output = wasmtime::Result<bool>> + Send {
        let (ext, _cx) = current(&mut host);
        let available = ext.host.host_services().iter().any(|n| n == &name);
        ready(available)
    }
}
