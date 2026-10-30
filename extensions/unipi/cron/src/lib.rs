//! `unipi.cron` — scheduled, unattended Oino runs (unipi-port §3.24; v2 ext-abi §6.4):
//! the `/cron` command, the `cron_*` agent tools, scheduler settings in the hub and the
//! bundled `cron` skill.
//!
//! **Job management goes straight to the files** ([`jobs`]): the extension holds fs
//! preopens on `~/.oino/cron` (read) and `~/.oino/cron/jobs` (write), so `list`, `show`,
//! `runs`, `log`, `add`, `edit`, `remove`, `enable` and `disable` work identically with
//! or without a host. **Live-scheduler ops** (`run`, `stop`, `pause`, `resume`,
//! `status`, `settings-get`, `settings-set`, `test-notify`) call the `"cron"` host
//! service; when the host provides none (a plain `oino` TUI) they report "no scheduler
//! running (unipi-host is not running)".
//!
//! | unipi | here |
//! |---|---|
//! | `/unipi:cron` window onto `unipi-host cron` | [`args`]: `/cron` subcommands + `add`/`edit`/`remove`, job-id completions |
//! | `unipi-host cron` CLI | the unipi-cron library behind the `unipi-host cron` CLI (wave 2, WP-H8) |
//! | app cron UI | the Automations screen in the `UniPi` app (docs/dev/v2/DIRECTION.md) |
//! | skill `cron` | bundled skill, Oino wording |
//!
//! `export_extension!` is gated on `target_arch = "wasm32"` (native `cdylib` links fail on
//! the `:`-qualified export symbols, unipi-core deviation 11).

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
pub mod jobs;
pub mod settings;
pub mod tools;

use std::cell::RefCell;

use oino_extension::context;
use oino_extension::host_service;
use oino_extension::prelude::*;
use serde_json::{Value, json};
use unipi_kit::settings::Store;

use crate::args::split_cron_args;
use crate::jobs::Job;
use crate::settings::NAMESPACE;

/// The bundled skill's name.
pub const CRON_SKILL: &str = "cron";

/// The message shown for live-scheduler ops without a host (ext-abi §6.3).
pub const NOT_RUNNING: &str = "the cron scheduler is not running (unipi-host is not running)";

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
    /// One live-scheduler op over the `cron` host service.
    async fn host_op(&'static self, op: &str, payload: Value) -> Result<Value, String> {
        if !host_service::available("cron") {
            return Err(NOT_RUNNING.to_owned());
        }
        host_service::call("cron", op, &payload)
            .await
            .map_err(|e| e.to_string())
    }

    /// The job ids for completions (cached briefly; read from the files).
    fn job_ids(&'static self) -> Vec<String> {
        let now = oino_extension::timers::now_ms();
        if let Some(cache) = self.ids.borrow().as_ref()
            && now.saturating_sub(cache.at) < 5000
        {
            return cache.ids.clone();
        }
        let ids: Vec<String> = jobs::list().into_iter().map(|j| j.id).collect();
        *self.ids.borrow_mut() = Some(IdsCache {
            at: now,
            ids: ids.clone(),
        });
        ids
    }

    /// The `list` reply shape (jobs with `next_fire`), shared by `/cron` and `cron_list`.
    pub(crate) fn list_value(&'static self) -> Value {
        let _ = self;
        json!({ "jobs": jobs::list().iter().map(jobs::job_value).collect::<Vec<_>>() })
    }

    /// `/cron …`.
    async fn cron_command(&'static self, text: &str, cx: &context::Ctx) {
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
            .complete(|ext, prefix, _cx| async move { args::completions(ext, &prefix) }),
    );
}

/// `/cron` help text (unknown subcommand).
pub(crate) fn help_text() -> String {
    let mut lines = vec![
        "Cron commands (scheduled, unattended Oino runs — see skill:cron)".to_owned(),
        "  /cron                    every job: next fire + status".to_owned(),
        "  /cron list               every job".to_owned(),
        "  /cron show <id>          one job + its recent runs".to_owned(),
        "  /cron runs <id> [N]      a job's run history".to_owned(),
        "  /cron run <id>           fire it now (needs the UniPi host)".to_owned(),
        "  /cron stop <id>          kill a running run (needs the UniPi host)".to_owned(),
        "  /cron enable|disable <id>".to_owned(),
        "  /cron pause / resume     pause every job / resume (needs the UniPi host)".to_owned(),
        "  /cron status             next fire per job, what's running".to_owned(),
    ];
    lines.extend(
        [
            "  /cron add name=\"…\" schedule=\"…\" prompt=\"…\" [cwd=…] [key=value…]".to_owned(),
            "  /cron edit <id> [key=value…]   change fields (same keys as add)".to_owned(),
            "  /cron remove <id>       delete the job (its run history ages out)".to_owned(),
        ]
        .iter()
        .cloned(),
    );
    lines.join("\n")
}

/// A fresh job with defaults, for `add` (the shared field mapping lives in `args`).
pub(crate) fn new_job(
    id: String,
    name: String,
    prompt: String,
    schedule: String,
    tz: String,
    cwd: String,
) -> Job {
    let now = jobs::now_ms();
    Job {
        id,
        name,
        prompt,
        schedule,
        tz,
        cwd,
        model: None,
        thinking: None,
        enabled: true,
        time_limit_s: u64::try_from(
            jobs::settings()["default_time_limit_s"]
                .as_i64()
                .unwrap_or(1800),
        )
        .unwrap_or(1800)
        .max(1),
        cost_cap_usd: None,
        overlap: "skip".to_owned(),
        missed: "run-latest".to_owned(),
        notify: None,
        created: now,
        updated: now,
    }
}

#[cfg(target_arch = "wasm32")]
oino_extension::export_extension!(Cron);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::args::SUBCOMMANDS;

    #[test]
    fn help_lists_the_subcommands() {
        let help = help_text();
        for sub in SUBCOMMANDS {
            assert!(help.contains(sub), "help misses {sub}");
        }
    }
}
