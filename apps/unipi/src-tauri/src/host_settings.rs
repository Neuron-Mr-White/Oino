//! Host settings + hub-level `_oino/*` commands for the UI (WP-A24, app spec
//! §3.14 / host.md §9.9):
//!
//! ```text
//! host_settings_get(hostId)                       → the masked config.json
//! host_settings_set(hostId, key, value)           → the masked config.json
//! host_cmd(hostId, cmd)                           → the command's data
//! ```
//!
//! All three are one request/reply op on a fresh stream (`chat::one_shot`);
//! The host permits paired devices to write reviewed non-security settings.

use tauri::State;
use unipi_proto::{
    settings::HostCmd, AppErrorDto, HostSettingsKey, HostSettingsSetRequest, Request,
};

use crate::chat::{one_shot, REPLY_TIMEOUT};
use crate::net::NetState;

/// `host_settings_get`: the host's `config.json`, secrets masked.
#[tauri::command]
pub async fn host_settings_get(
    app: tauri::AppHandle,
    state: State<'_, NetState>,
    host_id: String,
) -> Result<serde_json::Value, AppErrorDto> {
    one_shot(
        &app,
        &state,
        &host_id,
        Request::HostSettingsGet,
        REPLY_TIMEOUT,
    )
    .await
}

/// `host_settings_set{key, value}`: patch one dotted key (validated host-side;
/// an unknown key is `E_SCHEMA`).
#[tauri::command]
pub async fn host_settings_set(
    app: tauri::AppHandle,
    state: State<'_, NetState>,
    host_id: String,
    key: HostSettingsKey,
    value: serde_json::Value,
) -> Result<serde_json::Value, AppErrorDto> {
    one_shot(
        &app,
        &state,
        &host_id,
        Request::HostSettingsSet(HostSettingsSetRequest { key, value }),
        REPLY_TIMEOUT,
    )
    .await
}

/// `host_cmd{cmd}` (app spec §3.14): one hub-level `_oino/*` command (the
/// settings / auth / extension subset) against core services, no session.
#[tauri::command]
pub async fn host_cmd(
    app: tauri::AppHandle,
    state: State<'_, NetState>,
    host_id: String,
    cmd: HostCmd,
) -> Result<serde_json::Value, AppErrorDto> {
    one_shot(
        &app,
        &state,
        &host_id,
        Request::HostCmd { cmd },
        REPLY_TIMEOUT,
    )
    .await
}
