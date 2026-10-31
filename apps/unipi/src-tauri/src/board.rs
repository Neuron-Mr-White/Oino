//! Board commands, v2 (WP-A22, G-18; app spec §3.14, ext-abi §5.5): forward
//! `board_cli` / `board_watch` to the paired PC's `unipi-host` over the
//! shared op stream (`fs.rs`'s `simple_op` — the same seam as every other
//! one-shot command).
//!
//! The v1 daemon relay (`board_availability` / `board_request` / `board_events`
//! and its SSE) is gone with the daemon: the host runs the `unipi-kanboard` CLI, one
//! process per call, and answers with the §5.5 outcome.
//!
//! - `board_cli { hostId, args, project? }` → `BoardCliData`
//!   (`{ok:true, payload}` or the CLI's `{ok:false, error, kind}` — a domain
//!   failure the UI surfaces, not an `invoke` rejection)
//! - `board_watch { hostId, on? }` → `BoardWatchData` (`{watching}`); while it
//!   is on, `board_changed` hub events arrive on the one `events_open` stream
//!   (`chat.rs`), which is what the UI subscribes to.

use std::time::Duration;

use unipi_proto::{AppErrorDto, BoardCliData, BoardSlug, BoardWatchData, ProtoError, Request};

use crate::fs::{schema_err, simple_op_within};
use crate::net::NetState;

/// One CLI call (the host's own budget is 30 s: local file ops).
const CLI_TIMEOUT: Duration = Duration::from_secs(35);

/// Turns `(args, project)` into a `BoardCli` request, validating both the way
/// the host would so a bad call fails fast with `E_SCHEMA` (not a round trip).
pub(crate) fn cli_request(
    args: Vec<String>,
    project: Option<String>,
) -> Result<Request, AppErrorDto> {
    if let Some(reason) = unipi_proto::board::board_args_error(&args) {
        return Err(schema_err("args", reason));
    }
    let project = match project {
        Some(slug) => Some(BoardSlug::try_from(slug).map_err(|error| match error {
            ProtoError::Schema { message, .. } => schema_err("project", message),
            other => other.to_app_error(),
        })?),
        None => None,
    };
    Ok(Request::BoardCli { args, project })
}

/// `board_cli{args, project?}`: run one `unipi-kanboard` command on the PC.
#[tauri::command]
pub async fn board_cli(
    app: tauri::AppHandle,
    state: tauri::State<'_, NetState>,
    host_id: String,
    args: Vec<String>,
    project: Option<String>,
) -> Result<BoardCliData, AppErrorDto> {
    let request = cli_request(args, project)?;
    let data = simple_op_within(&app, &state, &host_id, request, CLI_TIMEOUT).await?;
    serde_json::from_value(data)
        .map_err(|error| ProtoError::Internal(error.to_string()).to_app_error())
}

/// `board_watch{on}`: refcount the host's board file watcher.
#[tauri::command]
pub async fn board_watch(
    app: tauri::AppHandle,
    state: tauri::State<'_, NetState>,
    host_id: String,
    on: Option<bool>,
) -> Result<BoardWatchData, AppErrorDto> {
    let data = simple_op_within(
        &app,
        &state,
        &host_id,
        Request::BoardWatch {
            on: on.unwrap_or(true),
        },
        Duration::from_secs(30),
    )
    .await?;
    serde_json::from_value(data)
        .map_err(|error| ProtoError::Internal(error.to_string()).to_app_error())
}

#[cfg(test)]
mod tests {
    use super::*;
    use unipi_proto::{BoardErrorKind, ProtoError};

    fn schema_reason(error: AppErrorDto) -> String {
        assert_eq!(error.code, unipi_proto::AppErrorCode::E_SCHEMA);
        error.message.as_str().to_string()
    }

    #[test]
    fn cli_request_pins_the_subcommand_and_project() {
        let request = cli_request(
            vec!["list".into(), "--all".into()],
            Some("demo-a1b2".to_string()),
        )
        .unwrap();
        match request {
            Request::BoardCli { args, project } => {
                assert_eq!(args, vec!["list".to_string(), "--all".to_string()]);
                assert_eq!(project.unwrap().as_str(), "demo-a1b2");
            }
            other => panic!("unexpected request: {other:?}"),
        }
    }

    #[test]
    fn cli_request_validates_args_and_slug() {
        assert!(schema_reason(cli_request(Vec::new(), None).unwrap_err())
            .contains("needs a subcommand"));
        assert!(
            schema_reason(cli_request(vec!["add".into(), String::new()], None).unwrap_err())
                .contains("must not be empty")
        );
        assert!(schema_reason(
            cli_request(vec!["add".into(), "x".into(), "a\nb".into()], None).unwrap_err()
        )
        .contains("printable"));
        assert!(
            schema_reason(cli_request(vec!["list".into()], Some(String::new())).unwrap_err())
                .contains("1..=")
        );
        assert!(schema_reason(
            cli_request(
                vec!["move".into(), "S-1".into(), "done".into()],
                Some("p\n".into())
            )
            .unwrap_err()
        )
        .contains("printable"));
    }

    #[test]
    fn cli_outcomes_serialize_to_the_section_5_5_shape() {
        let ok = BoardCliData::success(serde_json::json!({ "tasks": [] }));
        assert_eq!(
            serde_json::to_value(&ok).unwrap(),
            serde_json::json!({ "ok": true, "payload": { "tasks": [] } })
        );
        let refused = BoardCliData::failure("S-1 → done requires --comment", BoardErrorKind::Rule);
        assert_eq!(
            serde_json::to_value(&refused).unwrap(),
            serde_json::json!({ "ok": false, "error": "S-1 → done requires --comment", "kind": "rule" })
        );
        // A schema failure is an AppError, never a board outcome.
        assert!(matches!(
            ProtoError::Schema {
                message: String::new(),
                path: Vec::new(),
                detail: None
            },
            ProtoError::Schema { .. }
        ));
    }
}
