//! Scheduled pi runs from the app (UNI-317) and their settings (UNI-318):
//! the UI's `src/lib/cron.ts` calls these, which forward `cron_*` requests
//! to the paired PC's `unipi-host` over the shared iroh connection (same
//! seam as `fs.rs`/`board.rs`). Every op needs the `cron` sharing feature
//! once the viewed PC isn't the one this app runs on (UNI-248 pattern).

use unipi_proto::{
    AppErrorDto, CronAddRequest, CronEditRequest, CronJob, CronJobsData, CronLogData,
    CronParseData, CronRunStartedData, CronRunsData, CronSettingsData, CronSettingsSetRequest,
    CronShowData, CronStatusData, CronStoppedData, Request,
};

use crate::fs::simple_op;

#[tauri::command]
pub async fn cron_list(
    app: tauri::AppHandle,
    state: tauri::State<'_, crate::net::NetState>,
    host_id: String,
) -> Result<CronJobsData, AppErrorDto> {
    let data = simple_op(&app, &state, &host_id, Request::CronList).await?;
    serde_json::from_value(data).map_err(|e| unipi_proto::ProtoError::Internal(e.to_string()).to_app_error())
}

#[tauri::command]
pub async fn cron_show(
    app: tauri::AppHandle,
    state: tauri::State<'_, crate::net::NetState>,
    host_id: String,
    id: String,
) -> Result<CronShowData, AppErrorDto> {
    let id = id.try_into().map_err(|e: String| crate::fs::schema_err("id", e))?;
    let data = simple_op(&app, &state, &host_id, Request::CronShow { id }).await?;
    serde_json::from_value(data).map_err(|e| unipi_proto::ProtoError::Internal(e.to_string()).to_app_error())
}

#[tauri::command]
pub async fn cron_runs(
    app: tauri::AppHandle,
    state: tauri::State<'_, crate::net::NetState>,
    host_id: String,
    id: Option<String>,
    limit: Option<u32>,
) -> Result<CronRunsData, AppErrorDto> {
    let id = id.map(|id| id.try_into().map_err(|e: String| crate::fs::schema_err("id", e))).transpose()?;
    let limit = limit
        .map(|v| unipi_proto::CronRunsLimit::try_from(v as usize).map_err(|e: String| crate::fs::schema_err("limit", e)))
        .transpose()?;
    let data = simple_op(&app, &state, &host_id, Request::CronRuns { id, limit }).await?;
    serde_json::from_value(data).map_err(|e| unipi_proto::ProtoError::Internal(e.to_string()).to_app_error())
}

#[tauri::command]
pub async fn cron_log(
    app: tauri::AppHandle,
    state: tauri::State<'_, crate::net::NetState>,
    host_id: String,
    run_id: String,
) -> Result<CronLogData, AppErrorDto> {
    let run_id = run_id.try_into().map_err(|e: String| crate::fs::schema_err("run_id", e))?;
    let data = simple_op(&app, &state, &host_id, Request::CronLog { run_id }).await?;
    serde_json::from_value(data).map_err(|e| unipi_proto::ProtoError::Internal(e.to_string()).to_app_error())
}

#[tauri::command]
pub async fn cron_add(
    app: tauri::AppHandle,
    state: tauri::State<'_, crate::net::NetState>,
    host_id: String,
    job: CronAddRequest,
) -> Result<CronJob, AppErrorDto> {
    let data = simple_op(&app, &state, &host_id, Request::CronAdd(job)).await?;
    serde_json::from_value(data).map_err(|e| unipi_proto::ProtoError::Internal(e.to_string()).to_app_error())
}

#[tauri::command]
pub async fn cron_edit(
    app: tauri::AppHandle,
    state: tauri::State<'_, crate::net::NetState>,
    host_id: String,
    id: String,
    patch: CronEditRequest,
) -> Result<CronJob, AppErrorDto> {
    let id = id.try_into().map_err(|e: String| crate::fs::schema_err("id", e))?;
    let data = simple_op(&app, &state, &host_id, Request::CronEdit { id, patch }).await?;
    serde_json::from_value(data).map_err(|e| unipi_proto::ProtoError::Internal(e.to_string()).to_app_error())
}

#[tauri::command]
pub async fn cron_rm(
    app: tauri::AppHandle,
    state: tauri::State<'_, crate::net::NetState>,
    host_id: String,
    id: String,
) -> Result<(), AppErrorDto> {
    let id = id.try_into().map_err(|e: String| crate::fs::schema_err("id", e))?;
    simple_op(&app, &state, &host_id, Request::CronRm { id }).await?;
    Ok(())
}

#[tauri::command]
pub async fn cron_enable(
    app: tauri::AppHandle,
    state: tauri::State<'_, crate::net::NetState>,
    host_id: String,
    id: String,
) -> Result<(), AppErrorDto> {
    let id = id.try_into().map_err(|e: String| crate::fs::schema_err("id", e))?;
    simple_op(&app, &state, &host_id, Request::CronEnable { id }).await?;
    Ok(())
}

#[tauri::command]
pub async fn cron_disable(
    app: tauri::AppHandle,
    state: tauri::State<'_, crate::net::NetState>,
    host_id: String,
    id: String,
) -> Result<(), AppErrorDto> {
    let id = id.try_into().map_err(|e: String| crate::fs::schema_err("id", e))?;
    simple_op(&app, &state, &host_id, Request::CronDisable { id }).await?;
    Ok(())
}

#[tauri::command]
pub async fn cron_run(
    app: tauri::AppHandle,
    state: tauri::State<'_, crate::net::NetState>,
    host_id: String,
    id: String,
) -> Result<CronRunStartedData, AppErrorDto> {
    let id = id.try_into().map_err(|e: String| crate::fs::schema_err("id", e))?;
    let data = simple_op(&app, &state, &host_id, Request::CronRun { id }).await?;
    serde_json::from_value(data).map_err(|e| unipi_proto::ProtoError::Internal(e.to_string()).to_app_error())
}

#[tauri::command]
pub async fn cron_stop(
    app: tauri::AppHandle,
    state: tauri::State<'_, crate::net::NetState>,
    host_id: String,
    id: String,
) -> Result<CronStoppedData, AppErrorDto> {
    let id = id.try_into().map_err(|e: String| crate::fs::schema_err("id", e))?;
    let data = simple_op(&app, &state, &host_id, Request::CronStop { id }).await?;
    serde_json::from_value(data).map_err(|e| unipi_proto::ProtoError::Internal(e.to_string()).to_app_error())
}

#[tauri::command]
pub async fn cron_pause_all(
    app: tauri::AppHandle,
    state: tauri::State<'_, crate::net::NetState>,
    host_id: String,
) -> Result<(), AppErrorDto> {
    simple_op(&app, &state, &host_id, Request::CronPauseAll).await?;
    Ok(())
}

#[tauri::command]
pub async fn cron_resume_all(
    app: tauri::AppHandle,
    state: tauri::State<'_, crate::net::NetState>,
    host_id: String,
) -> Result<(), AppErrorDto> {
    simple_op(&app, &state, &host_id, Request::CronResumeAll).await?;
    Ok(())
}

#[tauri::command]
pub async fn cron_status(
    app: tauri::AppHandle,
    state: tauri::State<'_, crate::net::NetState>,
    host_id: String,
) -> Result<CronStatusData, AppErrorDto> {
    let data = simple_op(&app, &state, &host_id, Request::CronStatus).await?;
    serde_json::from_value(data).map_err(|e| unipi_proto::ProtoError::Internal(e.to_string()).to_app_error())
}

#[tauri::command]
pub async fn cron_parse(
    app: tauri::AppHandle,
    state: tauri::State<'_, crate::net::NetState>,
    host_id: String,
    schedule: Option<String>,
    at: Option<String>,
    tz: Option<String>,
) -> Result<CronParseData, AppErrorDto> {
    let data = simple_op(&app, &state, &host_id, Request::CronParse { schedule, at, tz }).await?;
    serde_json::from_value(data).map_err(|e| unipi_proto::ProtoError::Internal(e.to_string()).to_app_error())
}

#[tauri::command]
pub async fn cron_settings_get(
    app: tauri::AppHandle,
    state: tauri::State<'_, crate::net::NetState>,
    host_id: String,
) -> Result<CronSettingsData, AppErrorDto> {
    let data = simple_op(&app, &state, &host_id, Request::CronSettingsGet).await?;
    serde_json::from_value(data).map_err(|e| unipi_proto::ProtoError::Internal(e.to_string()).to_app_error())
}

#[tauri::command]
pub async fn cron_settings_set(
    app: tauri::AppHandle,
    state: tauri::State<'_, crate::net::NetState>,
    host_id: String,
    patch: CronSettingsSetRequest,
) -> Result<CronSettingsData, AppErrorDto> {
    let data = simple_op(&app, &state, &host_id, Request::CronSettingsSet(patch)).await?;
    serde_json::from_value(data).map_err(|e| unipi_proto::ProtoError::Internal(e.to_string()).to_app_error())
}

/// UNI-319: open a run's saved session as a live chat (a pi already on it,
/// or one started at `target`). Waits for the new pi's bridge (≤ 45 s on
/// the host), like `chat_new_session`.
#[tauri::command]
pub async fn cron_open_session(
    app: tauri::AppHandle,
    state: tauri::State<'_, crate::net::NetState>,
    host_id: String,
    run_id: String,
    target: serde_json::Value,
) -> Result<unipi_proto::CronOpenSessionData, AppErrorDto> {
    let run_id = run_id.try_into().map_err(|e: String| crate::fs::schema_err("run_id", e))?;
    let target: unipi_proto::ChatNewTarget = serde_json::from_value(target).map_err(|e| crate::fs::schema_err("target", e.to_string()))?;
    let data = crate::fs::simple_op_within(&app, &state, &host_id, Request::CronOpenSession { run_id, target }, std::time::Duration::from_secs(75)).await?;
    serde_json::from_value(data).map_err(|e| unipi_proto::ProtoError::Internal(e.to_string()).to_app_error())
}
