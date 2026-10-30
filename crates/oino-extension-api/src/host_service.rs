//! Host-provided services (ext-abi §3): named capabilities an embedding host (unipi-host)
//! registers with the runtime — `"browser"`, `"cron"`, … . Oino itself registers none:
//! under a plain TUI every name is unavailable and extensions degrade (typically: tools
//! report unavailable).
//!
//! One process-wide instance per name is installed into the WASM host hub
//! (`oino_wasm_host::LocalServices::register_host_service`); the hub serves
//! `host-service.call` from the registry before falling back to the inner
//! [`crate::HostServices`] implementation.

use crate::HostError;
use crate::RawJson;
use crate::runtime::BoxFut;
use tokio_util::sync::CancellationToken;

/// One host-provided service, registered process-wide by an embedding host.
///
/// `payload` and the reply are the service's own JSON contract (documented by the service,
/// not by the ABI); user-facing failures travel inside the reply JSON — `Err` is only for
/// permission, missing services and transport/timeout problems.
pub trait HostService: Send + Sync {
    /// Runs one op of this service.
    ///
    /// # Errors
    /// [`HostError::Timeout`], [`HostError::Cancelled`] and [`HostError::Io`] for
    /// transport problems; user-facing failures travel inside the reply JSON instead.
    fn call(
        &self,
        op: &str,
        payload: RawJson,
        cancel: CancellationToken,
    ) -> BoxFut<'_, Result<RawJson, HostError>>;
}
