//! `browser` (browser-control.md §8.3): permission check, then `HostServices::browser_execute`.

use std::future::Future;

use oino_browser_protocol::BrowserResult;
use oino_extension_api::{CancellationToken, HostError};
use wasmtime::component::{Access, Accessor, Resource};

use super::ready;
use crate::bindings::oino::extension::{browser as wbr, types as wt};
use crate::convert::{self, json};
use crate::resources::CancelToken;
use crate::state::{ExtShared, HostState, current, current_async};

type HostResult<T> = wasmtime::Result<Result<T, wt::HostError>>;

fn outcome(mut r: BrowserResult) -> wbr::Outcome {
    let image = r.image.take().map(|i| wbr::Image {
        mime_type: i.mime_type,
        data: i.data,
        path: i.path,
    });
    wbr::Outcome {
        result: serde_json::to_string(&r).unwrap_or_else(|_| "{}".into()),
        image,
    }
}

impl wbr::Host for HostState {}

impl<T: Send> wbr::HostWithStore<T> for HostState {
    async fn execute(
        accessor: &Accessor<T, Self>,
        command: String,
        cancel: Option<Resource<CancelToken>>,
    ) -> HostResult<wbr::Outcome> {
        let (ext, cx) = current_async(accessor);
        let cancel = match cancel {
            Some(res) => accessor.with(|mut a| a.get().table.get(&res).map(|t| t.0.clone()))?,
            None => CancellationToken::new(),
        };
        let r =
            match ExtShared::require(ext.perms().browser, "browser").and_then(|()| json(command)) {
                Ok(request) => ext
                    .host
                    .browser_execute(&ext.id, &cx, request, cancel)
                    .await
                    .map(outcome),
                Err(e) => Err(e),
            };
        Ok(convert::wit(r))
    }

    fn status(
        mut host: Access<'_, T, Self>,
    ) -> impl Future<Output = wasmtime::Result<String>> + Send {
        let (ext, cx) = current(&mut host);
        let status = if ext.perms().browser {
            ext.host.browser_status(&ext.id, &cx).as_str().to_owned()
        } else {
            let denied: Result<(), HostError> = ExtShared::require(false, "browser");
            serde_json::json!({"enabled": false, "available": false, "reason": "disabled",
                "stealth": false, "display": "headless", "error": denied.err().map(|e| e.to_string())})
            .to_string()
        };
        ready(status)
    }
}
