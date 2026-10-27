//! Small commands (unipi `src/commands.ts`, `src/summarize/index.ts`).
//!
//! - `/continue` / `/retry` → a turn with no text (`unipi-continue`, hidden).
//! - `/cleanup [--dry-run] [--yes]` → allowlisted stale files, preview + confirm.
//! - `/doctor` → diagnostics report.
//! - `/summarize [focus]` → `/skill:summarize [focus]` as a user message.

use std::time::SystemTime;

use oino_extension::prelude::*;
use oino_extension::session::{CustomMessage, Delivery, SendOptions};
use oino_extension::wit::session as ws;

use crate::maintenance::{cleanup, doctor};

/// Hidden custom message that starts a turn.
pub const CONTINUE_TYPE: &str = "unipi-continue";
/// Display-only command response.
pub const RESPONSE_TYPE: &str = "unipi-response";
/// The bundled summary skill.
pub const SUMMARIZE_SKILL: &str = "summarize";

/// `true` (and a warning) while the agent runs.
#[must_use]
pub fn busy(cx: &CommandCtx) -> bool {
    if control::is_idle() {
        return false;
    }
    if cx.has_ui() {
        let _ = ui::notify(
            "Agent is busy. Press ESC to interrupt, then try again.",
            "warning",
        );
    }
    true
}

/// A markdown response (unipi `sendResponse`: follow-up, no turn).
pub async fn respond(markdown: &str) -> Result<()> {
    let msg = CustomMessage {
        custom_type: RESPONSE_TYPE.into(),
        content: json!(markdown),
        display: true,
        details: Some(
            json!({"unipiHarness": {"version": 1, "id": format!("hm-{}", unipi_kit::fmt::random_hex(8)), "source": "Utility", "title": "Response", "synopsis": "Command response", "delivery": "followUp"}}),
        ),
    };
    let options = SendOptions {
        trigger_turn: false,
        deliver_as: Some(Delivery::FollowUp),
        provenance: Some(session::Provenance {
            source: "Utility".into(),
            title: Some("Response".into()),
            synopsis: Some("Command response".into()),
            severity: None,
        }),
    };
    session::send_message(&msg, options).await
}

/// `/continue`, `/retry`.
pub async fn continue_turn(cx: &CommandCtx) -> Result<()> {
    if busy(cx) {
        return Ok(());
    }
    let msg = CustomMessage {
        custom_type: CONTINUE_TYPE.into(),
        content: json!(""),
        display: false,
        details: None,
    };
    session::send_message(
        &msg,
        SendOptions {
            trigger_turn: true,
            ..SendOptions::default()
        },
    )
    .await
}

fn show(p: &std::path::Path) -> String {
    unipi_kit::paths::host_path(p).map_or_else(
        || p.to_string_lossy().into_owned(),
        |h| h.to_string_lossy().into_owned(),
    )
}

/// Response text of `/cleanup` for a parsed argument set (pure).
#[must_use]
pub fn cleanup_text(preview: &str, dry: bool, cancelled: bool, has_ui: bool) -> String {
    if cancelled {
        return format!(
            "## Cleanup cancelled\n\n```\n{preview}\n```{}",
            if has_ui {
                ""
            } else {
                "\n\nRun `/cleanup --yes` to remove them."
            }
        );
    }
    format!(
        "## Cleanup{}\n\n```\n{preview}\n```",
        if dry { " (dry run)" } else { "" }
    )
}

/// `/cleanup`.
pub async fn cleanup(args: &str, cx: &CommandCtx) -> Result<()> {
    if busy(cx) {
        return Ok(());
    }
    let targets = cleanup::targets();
    let now = SystemTime::now();
    let items = cleanup::find(&targets, now);
    let preview = cleanup::preview(&items, &targets, &show);
    let dry = args.contains("--dry-run");
    if items.is_empty() || dry {
        return respond(&cleanup_text(&preview, dry, false, cx.has_ui())).await;
    }
    let confirmed = args.contains("--yes")
        || (cx.has_ui()
            && ui::confirm("Remove these files?", &preview)
                .await
                .unwrap_or(false));
    if !confirmed {
        return respond(&cleanup_text(&preview, false, true, cx.has_ui())).await;
    }
    let r = cleanup::remove(&items, &targets, now);
    let failed = if r.failed.is_empty() {
        String::new()
    } else {
        format!(
            "\n\nCould not remove:\n{}",
            r.failed
                .iter()
                .map(|p| format!("- `{}`", show(p)))
                .collect::<Vec<_>>()
                .join("\n")
        )
    };
    respond(&format!(
        "## Cleanup\n\nRemoved {} item(s), {}.{failed}",
        r.removed,
        cleanup::format_bytes(r.bytes)
    ))
    .await
}

/// `/doctor`.
pub async fn doctor(cx: &CommandCtx) -> Result<()> {
    if busy(cx) {
        return Ok(());
    }
    let checks = doctor::run().await;
    respond(&doctor::format_report(
        &checks,
        &unipi_kit::fmt::iso_from_ms(timers::now_ms()),
    ))
    .await
}

/// The text `/summarize` sends.
#[must_use]
pub fn summarize_prompt(focus: &str) -> String {
    let f = focus.trim();
    if f.is_empty() {
        format!("/skill:{SUMMARIZE_SKILL}")
    } else {
        format!("/skill:{SUMMARIZE_SKILL} {f}")
    }
}

/// The skill command is loaded.
#[must_use]
pub fn has_skill(commands: &[Value]) -> bool {
    let want = format!("skill:{SUMMARIZE_SKILL}");
    commands
        .iter()
        .any(|c| c.get("name").and_then(Value::as_str) == Some(want.as_str()))
}

/// `/summarize [focus]` (queued as a follow-up while busy).
pub async fn summarize(args: &str, cx: &CommandCtx) -> Result<()> {
    let busy = !control::is_idle();
    if !has_skill(&control::commands().unwrap_or_default()) {
        if cx.has_ui() {
            let _ = ui::notify(
                "The summarize skill is off or missing. Turn it on in /skills.",
                "warning",
            );
        }
        return Ok(());
    }
    let focus = args.trim();
    let options = ws::UserMessageOptions {
        deliver_as: busy.then_some(ws::Delivery::FollowUp),
        expand_prompt_templates: true,
        provenance: Some(ws::Provenance {
            source: "Utility".into(),
            title: Some("Summarize".into()),
            synopsis: Some(if focus.is_empty() {
                "last reply".into()
            } else {
                format!("ask: {focus}")
            }),
            severity: None,
        }),
    };
    ws::send_user_message(json!(summarize_prompt(focus)).to_string(), options).await?;
    if busy && cx.has_ui() {
        let _ = ui::notify(
            "Summary queued — it runs when the current turn ends.",
            "info",
        );
    }
    if !cx.has_ui() {
        // Print mode exits when the handler returns: wait for the summary turn there.
        let _ = control::wait_for_idle().await;
    }
    Ok(())
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::assert_is_empty,
    clippy::format_collect,
    clippy::cloned_ref_to_slice_refs,
    clippy::duration_suboptimal_units,
    clippy::too_many_lines,
    clippy::needless_pass_by_value,
    reason = "tests assert by unwrapping"
)]
mod tests {
    use super::*;

    #[test]
    fn summarize_prompts() {
        assert_eq!(summarize_prompt(""), "/skill:summarize");
        assert_eq!(
            summarize_prompt("  what broke? "),
            "/skill:summarize what broke?"
        );
        assert!(has_skill(&[json!({"name": "skill:summarize"})]));
        assert!(!has_skill(&[json!({"name": "skill:other"})]));
    }

    #[test]
    fn cleanup_texts() {
        assert_eq!(
            cleanup_text("Nothing to clean.", false, false, true),
            "## Cleanup\n\n```\nNothing to clean.\n```"
        );
        assert!(cleanup_text("p", true, false, true).starts_with("## Cleanup (dry run)"));
        assert!(
            cleanup_text("p", false, true, false).ends_with("Run `/cleanup --yes` to remove them.")
        );
        assert!(!cleanup_text("p", false, true, true).contains("--yes"));
    }
}
