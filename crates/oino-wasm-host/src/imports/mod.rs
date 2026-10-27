//! WIT import implementations. Every import resolves the caller's [`CallContext`] from the
//! guest task (see [`crate::state::current`]), checks the manifest grant (§5.3) and either
//! acts locally (process, http, fs, timers, kv, log, host-info) or delegates to
//! [`oino_extension_api::HostServices`].
//!
//! [`CallContext`]: oino_extension_api::CallContext

mod browser;
mod core;
mod io;
mod runtime;
mod session;

use std::future::Future;

use oino_extension_api::HostError;

use crate::bindings::oino::extension::types as wt;
use crate::convert;

/// A ready import result.
pub(crate) fn ready<T: Send>(value: T) -> impl Future<Output = wasmtime::Result<T>> + Send {
    std::future::ready(Ok(value))
}

/// A ready fallible import result.
pub(crate) fn ready_wit<T: Send>(
    value: Result<T, HostError>,
) -> impl Future<Output = wasmtime::Result<Result<T, wt::HostError>>> + Send {
    std::future::ready(Ok(convert::wit(value)))
}

/// Encodes JSON for the guest.
pub(crate) fn out(json: &oino_extension_api::RawJson) -> String {
    json.as_str().to_owned()
}
