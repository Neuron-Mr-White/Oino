//! `unipi.cron` — scheduled, unattended Oino runs (unipi-port §3.24): the `/cron` command,
//! the `cron_*` agent tools, scheduler settings in the hub and the bundled `cron` skill.
//! All scheduler access goes through the host `cron` capability, which routes to the
//! running scheduler (daemon or embedded).
//!
//! | unipi | here |
//! |---|---|
//! | `/unipi:cron` window onto `unipi-host cron` | [`args`]: `/cron` subcommands + `add`/`edit`/`remove`, job-id completions |
//! | `unipi-host cron` CLI | the `oino cron` CLI (crates/oino) — the same ops over the same store |
//! | app cron UI | the desktop Automations page (apps/oino-ui) |
//!
//! `export_extension!` is gated on `target_arch = "wasm32"` (native `cdylib` links fail on
//! the `:`-qualified export symbols, unipi-core deviation 11).
//! | skill `cron` | bundled skill, Oino wording |

#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        reason = "tests assert by unwrapping"
    )
)]
#![allow(
    clippy::missing_errors_doc,
    reason = "fallible fns return the host's error, documented on the error types"
)]
#![allow(
    clippy::unused_async_trait_impl,
    reason = "`Extension::init` is async in the SDK; registration here is synchronous"
)]

pub mod args;
pub mod settings;
pub mod tools;

use std::cell::RefCell;

use oino_extension::cron;
use oino_extension::prelude::*;
use unipi_kit::settings::Store;

use crate::args::{SUBCOMMANDS, split_cron_args};
use crate::settings::NAMESPACE;

/// The bundled skill's name.
pub const CRON_SKILL: &str = "cron";
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Job-id completion cache (refreshed at most every 5 s, like unipi's).
struct IdsCache {
    at: i64,
    ids: Vec<String>,
}

/// The extension.
#[derive(Default)]
pub struct Cron {
    settings: Store,
    ids: RefCell<Option<IdsCache>>,
}

impl Cron {
    /// One scheduler op.
    async fn op(
        &'static self,
        op: &str,
        payload: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        cron::call(op, &payload).await.map_err(|e| e.to_string())
    }

    /// The job ids for completions (cached briefly).
    async fn job_ids(&'static self) -> Vec<String> {
        let now = oino_extension::timers::now_ms();
        if let Some(cache) = self.ids.borrow().as_ref()
            && now.saturating_sub(cache.at) < 5000
        {
            return cache.ids.clone();
        }
        let ids = self
            .op("list", serde_json::json!({}))
            .await
            .ok()
            .and_then(|value| {
                value
                    .get("jobs")
                    .and_then(|jobs| jobs.as_array())
                    .map(|jobs| {
                        jobs.iter()
                            .filter_map(|j| j.get("id").and_then(|i| i.as_str()))
                            .map(str::to_owned)
                            .collect::<Vec<_>>()
                    })
            })
            .unwrap_or_default();
        *self.ids.borrow_mut() = Some(IdsCache {
            at: now,
            ids: ids.clone(),
        });
        ids
    }

    /// `/cron …`.
    async fn cron_command(&'static self, text: &str, cx: &Ctx) {
        let (sub, rest) = split_cron_args(text);
        args::run_subcommand(self, &sub, &rest, cx).await;
    }
}

impl Extension for Cron {
    async fn init(&'static self, cx: &mut Init<Self>) -> Result<()> {
        let ns = settings::namespace();
        self.settings.declare(NAMESPACE, ns.defaults.clone());
        self.settings.load(cx.settings());
        cx.settings_namespace(&ns.to_json());
        cx.on(EventKind::OinoSettingsChanged, |ext, ev, _cx| async move {
            if ext
                .settings
                .on_changed(&ev.payload)
                .is_some_and(|ns| ns == NAMESPACE)
            {
                settings::push(&ext.settings.get(NAMESPACE)).await;
            }
            Ok(())
        });
        register_commands(cx);
        tools::register(cx);
        let _ = VERSION;
        Ok(())
    }
}

fn register_commands(cx: &mut Init<Cron>) {
    cx.command(
        Command::<Cron>::new("cron")
            .description("Scheduled, unattended Oino runs — jobs, run history, pause/resume (see skill:cron)")
            .argument_hint("[list|show <id>|runs <id> [N]|run <id>|stop <id>|enable|disable <id>|pause|resume|status|add|edit|remove]")
            .run(|ext, args, cx| async move {
                ext.cron_command(&args, &cx.cx).await;
                Ok(())
            })
            .complete(|ext, prefix, _cx| async move { args::completions(ext, &prefix).await }),
    );
}

/// The `/cron` help text (unknown subcommand).
pub(crate) fn help_text() -> String {
    let mut lines = vec![
        "Cron commands (scheduled, unattended Oino runs — see skill:cron)".to_owned(),
        "  /cron                    every job: next fire + status".to_owned(),
        "  /cron list               every job".to_owned(),
        "  /cron show <id>          one job + its recent runs".to_owned(),
        "  /cron runs <id> [N]      a job's run history".to_owned(),
        "  /cron run <id>           fire it now".to_owned(),
        "  /cron stop <id>          kill a running run".to_owned(),
        "  /cron enable|disable <id>".to_owned(),
        "  /cron pause / resume     pause every job / resume".to_owned(),
        "  /cron status             next fire per job, what's running".to_owned(),
    ];
    lines.extend(
        [
            "  /cron add name=\"…\" schedule=\"…\" prompt=\"…\" [cwd=…] [key=value…]".to_owned(),
            "  /cron edit <id> [key=value…]   change fields (same keys as add)".to_owned(),
            "  /cron remove <id>       delete the job and its history".to_owned(),
        ]
        .iter()
        .cloned(),
    );
    let _ = SUBCOMMANDS.len();
    lines.join("\n")
}

#[cfg(target_arch = "wasm32")]
oino_extension::export_extension!(Cron);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn help_lists_the_subcommands() {
        let help = help_text();
        for sub in SUBCOMMANDS {
            assert!(help.contains(sub), "help misses {sub}");
        }
    }
}
