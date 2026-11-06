//! `unipi.dream` as a real component in the wasm host (v3 J2/J3): the host-cron schedule
//! job, the gated `/dream run --scheduled` the host fires (it waits for the dream agent),
//! and the services the app's Dream panel uses (`dream.status`, `dream.runs`,
//! `dream.detail`, `dream.reject`, `dream.approve`, `dream.dismiss`).
//!
//! The dream agent is the recording host's fake agent (it settles at once and writes
//! nothing), and `unipi.memory` isn't loaded, so the memory inventory is unavailable: the
//! run still digests every recent session across projects and records itself.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests assert by unwrapping"
)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, OnceLock};

use oino_extension_api::manifest::Manifest;
use oino_extension_api::testing::RecordingHostServices;
use oino_extension_api::{
    CallContext, CallKind, CancellationToken, ComponentSource, ExtensionId, ExtensionInstance,
    ExtensionRuntime, HostServices, InitInput, LoadedPackage, Mode, RawJson, Registration,
};
use oino_wasm_host::{HostConfig, WasmRuntime, WasmRuntimeOptions};
use serde_json::{Value, json};

const DIR: &str = "dream";
const CRATE: &str = "unipi_dream";

fn extensions_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .unwrap()
}

/// Builds the component once (release, wasm32-wasip2).
fn built() -> &'static PathBuf {
    static OUT: OnceLock<PathBuf> = OnceLock::new();
    OUT.get_or_init(|| {
        let ws =
            std::env::var("UNIPI_DREAM_WORKSPACE").map_or_else(|_| extensions_dir(), PathBuf::from);
        let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
        let mut cmd = Command::new(cargo);
        cmd.current_dir(&ws)
            .args(["build", "-j", "4", "--release", "--target", "wasm32-wasip2"])
            .args(["-p", &format!("unipi-{DIR}")]);
        for var in [
            "CARGO_TARGET_DIR",
            "RUSTFLAGS",
            "CARGO_ENCODED_RUSTFLAGS",
            "CARGO_BUILD_TARGET",
            "CARGO_MANIFEST_DIR",
        ] {
            cmd.env_remove(var);
        }
        cmd.env("CARGO_TARGET_DIR", extensions_dir().join("target"));
        let out = cmd.output().unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        extensions_dir().join("target/wasm32-wasip2/release")
    })
}

struct Env {
    runtime: WasmRuntime,
    host: Arc<RecordingHostServices>,
    agent: tempfile::TempDir,
    project: tempfile::TempDir,
}

impl Env {
    /// An agent dir with `cron/jobs` (the host scheduler's folder) and `sessions`.
    fn new() -> Self {
        let agent = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(agent.path().join("cron/jobs")).unwrap();
        std::fs::create_dir_all(agent.path().join("sessions")).unwrap();
        std::fs::create_dir_all(project.path().join("oino")).unwrap();
        let mut options = WasmRuntimeOptions::new(HostConfig::new(agent.path(), project.path()));
        options.cache_dir = Some(extensions_dir().join("target/oino-wasm-test-cache"));
        Self {
            runtime: WasmRuntime::new(options).unwrap(),
            host: RecordingHostServices::new(),
            agent,
            project,
        }
    }

    fn cwd(&self) -> PathBuf {
        self.project.path().join("oino")
    }

    fn cx(&self, kind: CallKind) -> CallContext {
        CallContext {
            session_id: "s1".into(),
            mode: Mode::Rpc,
            has_ui: true,
            cwd: self.cwd(),
            kind,
            ..CallContext::default()
        }
    }

    async fn load(&self, settings: Value) -> Arc<dyn ExtensionInstance> {
        let wasm = built().join(format!("{CRATE}.wasm"));
        let manifest = Manifest::parse(
            &std::fs::read_to_string(extensions_dir().join(format!("unipi/{DIR}/extension.json")))
                .unwrap(),
        )
        .unwrap();
        let package = LoadedPackage {
            manifest,
            component: ComponentSource::Path(wasm),
            root: None,
            bundled: true,
        };
        let host: Arc<dyn HostServices> = Arc::clone(&self.host) as Arc<dyn HostServices>;
        let ext = self.runtime.load(package, host).await.unwrap();
        let input = InitInput {
            settings: RawJson::from(json!({ "dream": settings })),
            ..InitInput::default()
        };
        let _reg: Registration = ext.init(input, self.cx(CallKind::Init)).await.unwrap();
        ext
    }

    async fn service(&self, ext: &Arc<dyn ExtensionInstance>, name: &str, payload: Value) -> Value {
        ext.handle_service(
            name,
            RawJson::from(payload),
            &ExtensionId::new("unipi.core").unwrap(),
            self.cx(CallKind::Service),
        )
        .await
        .unwrap()
        .to_value()
    }

    async fn command(&self, ext: &Arc<dyn ExtensionInstance>, args: &str) {
        ext.run_command(
            "dream",
            args,
            CancellationToken::new(),
            self.cx(CallKind::Command),
        )
        .await
        .unwrap();
    }

    /// Every notification text so far, oldest first.
    fn notes(&self) -> Vec<String> {
        self.host
            .calls_named("ui_fire")
            .into_iter()
            .map(|c| c.detail.to_string())
            .collect()
    }

    fn job_file(&self) -> PathBuf {
        self.agent.path().join("cron/jobs/unipi-dream.json")
    }

    fn global(&self) -> PathBuf {
        self.agent.path().join("state/unipi.dream/global")
    }

    /// One session file of a project at `cwd` (`--<cwd>--/<name>.jsonl`).
    fn session(&self, cwd: &str, name: &str, user: &[&str]) {
        let dir = self
            .agent
            .path()
            .join("sessions")
            .join(format!("--{}--", cwd.trim_matches('/').replace('/', "-")));
        std::fs::create_dir_all(&dir).unwrap();
        let mut lines =
            vec![json!({"type": "session", "version": 3, "id": name, "cwd": cwd}).to_string()];
        for (i, u) in user.iter().enumerate() {
            lines.push(
                json!({"type": "message", "id": format!("m{i}"), "message": {"role": "user", "content": [{"type": "text", "text": u}]}})
                    .to_string(),
            );
        }
        std::fs::write(dir.join(format!("{name}.jsonl")), lines.join("\n") + "\n").unwrap();
    }
}

fn read_json(p: &Path) -> Value {
    serde_json::from_slice(&std::fs::read(p).unwrap_or_else(|e| panic!("{}: {e}", p.display())))
        .unwrap()
}

/// J2/J3: the settings become a host cron job (local time), status reports it, and
/// turning the schedule to manual removes it.
#[tokio::test(flavor = "multi_thread")]
async fn schedule_becomes_a_host_cron_job() {
    let env = Env::new();
    let ext = env
        .load(json!({"enabled": true, "schedule": {"mode": "daily", "time": "04:30"}, "maxRuntimeMin": 20}))
        .await;
    let st = env.service(&ext, "dream.status", json!({})).await;
    assert_eq!(st["v"], 2, "{st}");
    assert_eq!(
        st["schedule"],
        json!({"mode": "daily", "time": "04:30", "intervalHours": 24.0})
    );
    assert_eq!(st["scheduler"]["registered"], true, "{st}");
    assert_eq!(st["scheduler"]["cron"], "0 30 4 * * *");
    assert_eq!(
        st["scheduler"]["hostRunning"], false,
        "no host scheduler in the harness"
    );
    assert!(st["scheduler"]["nextRun"].is_null());
    assert_eq!(st["runs"], json!([]));
    assert_eq!(st["staleDays"], 30.0);

    let job = read_json(&env.job_file());
    assert_eq!(job["v"], 2);
    assert_eq!(job["id"], "unipi-dream");
    assert_eq!(job["prompt"], "/dream run --scheduled");
    assert_eq!(job["schedule"], "0 30 4 * * *");
    assert_eq!(job["tz"], "local");
    assert_eq!(job["timeLimitS"], 20 * 60 + 300);
    assert_eq!(job["overlap"], "skip");
    assert_eq!(
        Path::new(job["cwd"].as_str().unwrap())
            .canonicalize()
            .unwrap(),
        env.agent.path().canonicalize().unwrap(),
        "the job runs in the agent dir: {job}"
    );
    let created = job["created"].clone();

    // Every 6 hours: the job is rewritten in place (same `created`).
    let ext = env
        .load(json!({"enabled": true, "schedule": {"mode": "interval", "time": "04:30", "intervalHours": 6}}))
        .await;
    let st = env.service(&ext, "dream.status", json!({})).await;
    assert_eq!(st["scheduler"]["cron"], "0 30 */6 * * *");
    let job = read_json(&env.job_file());
    assert_eq!(job["schedule"], "0 30 */6 * * *");
    assert_eq!(job["created"], created);

    // Manual only: no job.
    let ext = env
        .load(json!({"enabled": true, "schedule": {"mode": "manual"}}))
        .await;
    let st = env.service(&ext, "dream.status", json!({})).await;
    assert_eq!(st["scheduler"]["registered"], false);
    assert!(st["scheduler"]["cron"].is_null());
    assert!(!env.job_file().exists(), "manual removes the job");

    // `/dream schedule` explains the state.
    env.command(&ext, "schedule").await;
    let last = env.notes().pop().unwrap_or_default();
    assert!(last.contains("dream schedule: manual only"), "{last}");
}

/// J2: the host fires `/dream run --scheduled`: the gates apply (skips are recorded for
/// the UI), a passing fire digests every project's recent sessions, spawns the dream agent
/// and returns only when the agent is done.
#[tokio::test(flavor = "multi_thread")]
async fn scheduled_run_gates_digests_and_waits() {
    let env = Env::new();
    env.session(
        "/w/oino",
        "a1",
        &["no, use cargo xtask wasm", "I prefer short commit messages"],
    );
    let ext = env
        .load(json!({"enabled": true, "minSessions": 2, "minGapHours": 6, "model": "anthropic/claude-x", "thinking": "high"}))
        .await;

    env.command(&ext, "run --scheduled").await;
    let st = env.service(&ext, "dream.status", json!({})).await;
    assert_eq!(st["runs"], json!([]), "one session: skipped");
    assert_eq!(st["lastCheck"]["trigger"], "scheduled");
    assert_eq!(st["lastCheck"]["ran"], false);
    assert_eq!(
        st["lastCheck"]["reason"], "needs 1 more new sessions",
        "{st}"
    );
    assert_eq!(st["checks"].as_array().unwrap().len(), 1);
    assert!(env.host.calls_named("agent_spawn").is_empty());

    env.session("/w/My Web", "b1", &["add a dark mode toggle"]);
    env.command(&ext, "run --scheduled").await;
    let spawns = env.host.calls_named("agent_spawn");
    assert_eq!(spawns.len(), 1, "the gates pass");
    let spec = &spawns[0].detail["spec"];
    assert_eq!(spec["childRole"], "dream");
    assert_eq!(spec["model"], "anthropic/claude-x");
    assert_eq!(spec["thinking"], "high");
    assert_eq!(spec["extensions"]["only"], json!(["unipi.memory"]));

    // The command returned after the (fake) agent settled: the run has ended.
    let runs = env.service(&ext, "dream.runs", json!({})).await;
    let runs = runs["runs"].as_array().unwrap().clone();
    assert_eq!(runs.len(), 1, "{runs:?}");
    let run = &runs[0];
    assert_ne!(run["status"], "running", "{run}");
    assert_eq!(run["trigger"], "scheduled");
    assert_eq!(run["sessions"], 2);
    let mut projects: Vec<String> = serde_json::from_value(run["projects"].clone()).unwrap();
    projects.sort();
    assert_eq!(projects, ["my_web", "oino"]);
    assert!(run.get("endedAt").is_some(), "{run}");

    let id = run["id"].as_str().unwrap();
    let staging = env.global().join("runs/all").join(id);
    let index = read_json(&staging.join("digests/INDEX.json"));
    assert_eq!(index["sessions"].as_array().unwrap().len(), 2, "{index}");
    let prompt = std::fs::read_to_string(staging.join("PROMPT.md")).unwrap();
    assert!(prompt.contains("Consolidate"), "{prompt}");
    assert!(
        prompt.contains("inventory is unavailable"),
        "no unipi.memory loaded"
    );
    let st = env.service(&ext, "dream.status", json!({})).await;
    assert_eq!(st["lastCheck"]["ran"], true);
    assert!(st["lastRunAt"].as_i64().unwrap() > 0);
    assert_eq!(st["running"], false);

    // A second fire right away: the min gap holds.
    env.command(&ext, "run --scheduled").await;
    assert_eq!(env.host.calls_named("agent_spawn").len(), 1);
    let st = env.service(&ext, "dream.status", json!({})).await;
    assert!(
        st["lastCheck"]["reason"]
            .as_str()
            .unwrap()
            .starts_with("min gap not reached"),
        "{st}"
    );
    assert_eq!(st["checks"].as_array().unwrap().len(), 3);

    // The detail of the run: report, changes, actions, trajectory.
    let d = env.service(&ext, "dream.detail", json!({"id": id})).await;
    assert_eq!(d["id"], id);
    assert_eq!(d["run"]["trigger"], "scheduled");
    assert!(d["changes"].is_null(), "no inventory, no change list");
    assert_eq!(d["actions"], json!([]));
}

/// The Dream panel's run/report/proposal services over a finished run (as a real dream
/// leaves it): memory change counts, the change list, proposals with their project, reject,
/// approve (refused from another project), dismiss.
#[tokio::test(flavor = "multi_thread")]
async fn runs_detail_proposals_and_dismiss() {
    let env = Env::new();
    let staging = env.global().join("runs/all/staging-1000");
    std::fs::create_dir_all(staging.join("proposals/skill-deploy")).unwrap();
    std::fs::write(
        staging.join("proposals/skill-deploy/SKILL.md"),
        "---\nname: deploy\ndescription: Deploy the site.\n---\n# Deploy\n",
    )
    .unwrap();
    std::fs::write(
        staging.join("proposals/lint-check.md"),
        "# Lint check\nrun clippy\n",
    )
    .unwrap();
    std::fs::write(
        staging.join("proposals/projects.json"),
        json!({"skill-deploy": "/w/web", "lint-check.md": env.cwd().display().to_string()})
            .to_string(),
    )
    .unwrap();
    std::fs::write(
        staging.join("run.json"),
        json!({"startedAt": 1000, "pid": -1, "endedAt": 2000, "exitCode": 0, "sessions": 4, "events": 2,
               "trigger": "scheduled", "projects": ["oino", "web"]})
        .to_string(),
    )
    .unwrap();
    std::fs::write(
        staging.join("DREAM_REPORT.md"),
        "## Memory edits\n- merged: two graph notes\n- pruned: old port note\n## Proposals\n1. deploy\n## Skipped\n- none\n",
    )
    .unwrap();
    let changes = json!({
        "counts": {"added": 1, "updated": 0, "merged": 1, "pruned": 1},
        "added": [{"project": "oino", "id": "prefers_short_commits", "title": "Prefers short commits", "type": "preference"}],
        "updated": [],
        "merged": [{"project": "oino", "id": "graph_retries", "title": "Graph retries", "type": "pattern", "new": true,
                    "from": [{"project": "oino", "id": "a"}, {"project": "oino", "id": "b"}]}],
        "pruned": [{"project": "web", "id": "port_3000_today", "title": "Port 3000 today", "type": "discovery"}],
        "actions": [],
    });
    std::fs::write(staging.join("changes.json"), changes.to_string()).unwrap();
    std::fs::write(
        staging.join("memory-changes.json"),
        json!([{"action": "pruned", "project": "web", "title": "Port 3000 today", "reason": "transient", "sources": []}]).to_string(),
    )
    .unwrap();

    let ext = env.load(json!({})).await;
    let runs = env.service(&ext, "dream.runs", json!({"limit": 5})).await;
    let run = &runs["runs"][0];
    assert_eq!(run["id"], "staging-1000");
    assert_eq!(run["status"], "finished", "{run}");
    assert_eq!(
        run["memoryChanges"],
        json!({"added": 1, "updated": 0, "merged": 1, "pruned": 1})
    );
    assert_eq!(run["pending"], 2);
    let proposals = run["proposals"].as_array().unwrap();
    let deploy = proposals.iter().find(|p| p["name"] == "deploy").unwrap();
    assert_eq!(deploy["kind"], "skill");
    assert_eq!(deploy["cwd"], "/w/web");

    let d = env
        .service(&ext, "dream.detail", json!({"id": "staging-1000"}))
        .await;
    assert_eq!(d["changes"]["pruned"][0]["id"], "port_3000_today");
    assert_eq!(
        d["changes"]["merged"][0]["from"].as_array().unwrap().len(),
        2
    );
    assert_eq!(d["actions"][0]["reason"], "transient");
    assert!(d["report"].as_str().unwrap().contains("## Memory edits"));

    // The skill is for another project: approving it here is refused, not recorded.
    let r = env
        .service(
            &ext,
            "dream.approve",
            json!({"runId": "staging-1000", "proposalId": deploy["id"]}),
        )
        .await;
    assert_eq!(r["ok"], false, "{r}");
    assert!(r["message"].as_str().unwrap().contains("/w/web"), "{r}");
    // A check approves anywhere (it only records the decision and shows its head).
    let lint = proposals
        .iter()
        .find(|p| p["name"] == "lint-check")
        .unwrap();
    let r = env
        .service(
            &ext,
            "dream.approve",
            json!({"runId": "staging-1000", "proposalId": lint["id"]}),
        )
        .await;
    assert_eq!(r["ok"], true, "{r}");
    let r = env
        .service(
            &ext,
            "dream.reject",
            json!({"runId": "staging-1000", "proposalId": deploy["id"]}),
        )
        .await;
    assert_eq!(r["ok"], true, "{r}");
    let runs = env.service(&ext, "dream.runs", json!({})).await;
    let decisions: Vec<(String, String)> = runs["runs"][0]["proposals"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| {
            (
                p["name"].as_str().unwrap().to_owned(),
                p["decision"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    assert_eq!(
        decisions,
        [
            ("deploy".to_owned(), "rejected".to_owned()),
            ("lint-check".to_owned(), "approved".to_owned())
        ]
    );
    assert_eq!(runs["runs"][0]["pending"], 0);

    let r = env
        .service(&ext, "dream.dismiss", json!({"runId": "staging-1000"}))
        .await;
    assert_eq!(r["ok"], true, "{r}");
    let runs = env.service(&ext, "dream.runs", json!({})).await;
    assert_eq!(runs["runs"], json!([]));
}
