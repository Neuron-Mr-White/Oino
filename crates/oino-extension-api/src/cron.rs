//! Cron scheduler control (unipi-port §3.24): the process-wide [`CronHost`] handed to the
//! WASM host, implemented by the daemon client or an embedded scheduler.

use crate::HostError;
use crate::RawJson;
use crate::runtime::BoxFut;

/// Serves `cron.call` ops from extensions with the `cron` permission.
///
/// One process-wide instance is installed into the WASM host hub
/// (`oino_wasm_host::Hub::set_cron`); without one the host falls back to the daemon client
/// (`oino_cron::client::DaemonCronHost`).
pub trait CronHost: Send + Sync {
    /// Runs one op. `op` is one of the names in `wit/oino-extension/cron.wit`; `payload`
    /// holds the request fields.
    ///
    /// # Errors
    /// [`HostError::Failed`] carries the scheduler's user-facing message.
    fn call(&self, op: &str, payload: RawJson) -> BoxFut<'_, Result<RawJson, HostError>>;
}
