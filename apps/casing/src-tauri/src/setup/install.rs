//! UNI-240 (plan S2, docs/m8/SELF-INSTALL-AND-EMBEDDED-HOST.md §4.1, §4.3):
//! the install engine. It turns a [`SetupReport`](super::SetupReport) into a
//! plan and runs it, streaming progress and writing a log.
//!
//! Rules baked in from the spec and the user's decisions:
//!
//! - **Node** — reuse the user's when it is ≥ 22.19 (decision 4: newer is left
//!   alone). Otherwise download a Node 22 LTS tarball from nodejs.org, verify
//!   it against `SHASUMS256`, and unpack it into `~/.local/share/pi-node/<ver>`
//!   with a `current` symlink — the exact place pi's own installer uses, so the
//!   two agree.
//! - **pi** — N1: pi.dev's installer with our Node first on PATH (it needs
//!   Node already, and skips the offer without a TTY). On failure, N2:
//!   `npm install --ignore-scripts --prefix ~/.pi/agent/npm
//!   @earendil-works/pi-coding-agent`. **Never install pi when a working pi
//!   exists anywhere** — a user's own pi is never replaced.
//! - **UniPi** — `pi install npm:@pi-unipi/unipi@<channel>`, the channel from
//!   `setup::channel_for`, so it follows the updater's rules rather than a
//!   hardcoded `alpha`.
//! - **kanboard** — nothing to install: it comes with the extensions. Its step
//!   is a check, and a repair means reinstalling UniPi.
//! - **herdr** — herdr.dev's installer (decision 8: several methods, fallback
//!   to showing the command).
//! - **tmux** — never automatic. Its step is skipped with the package-manager
//!   command for this PC.
//! - **"Install for me"** (decision 8, §9) — the separate, explicit call the
//!   button makes: [`install_component_here`] detects again, builds a plan with
//!   only that component's step (`Action::InstallForMe`) and runs a **method
//!   chain**. tmux goes through `pkexec` on Linux and `osascript … with
//!   administrator privileges` on macOS; herdr tries its own installer and then
//!   a static release binary in `~/.local/bin`. Every method that fails (or is
//!   unavailable — no `pkexec` on this PC, say) moves the chain on, and a chain
//!   that runs out of methods ends in the fallback command for Copy. The
//!   default plan never contains it, so tmux stays never automatic.
//!
//! Everything is idempotent: a step that is already satisfied is
//! `AlreadyPresent`, and skipping it is a no-op. Cancel stops between steps
//! and leaves the earlier steps installed. Progress streams over a Tauri
//! Channel; the full output goes to `~/.unipi/logs/setup-<ts>.log`.
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use sha2::{Digest, Sha256};
use tokio::sync::watch;

use super::{channel_for, install_spec, Component, ComponentState, DistTags, SetupReport};
use unipi_proto::AppErrorDto;

/// Node 22 LTS is what we install (pi needs >= 22.19).
pub const NODE_INSTALL_VERSION: &str = "22.19.0";
/// Where a standalone Node goes — the same place pi's installer uses.
pub const NODE_ROOT_DIR: &str = ".local/share/pi-node";
/// The npm package that is pi.
pub const PI_PACKAGE: &str = "@earendil-works/pi-coding-agent";
/// pi's official installer.
pub const PI_INSTALLER_URL: &str = "https://pi.dev/install.sh";
/// herdr's official installer.
pub const HERDR_INSTALLER_URL: &str = "https://herdr.dev/install.sh";
/// The npm registry, for the offline pre-check.
pub const NPM_REGISTRY: &str = "https://registry.npmjs.org";
/// Node's release index.
pub const NODE_DIST: &str = "https://nodejs.org/dist";
/// The log's directory.
const LOG_DIR: &str = ".unipi/logs";
/// herdr's release assets (`herdr-<os>-<arch>`, herdr.dev/docs/install).
pub const HERDR_RELEASES: &str = "https://github.com/herdrdev/herdr/releases/latest/download";
/// Where `pkexec` lives when the GUI's PATH doesn't have it — the standard
/// locations `host_supervisor::systemctl_in` falls back to for `systemctl`.
const PKEXEC_STANDARD: &[&str] = &["/usr/bin", "/bin", "/usr/local/bin"];

// ------------------------------------------------------------------- shapes

/// A step of the plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanStep {
    pub id: String,
    pub label: String,
    /// What the step will do.
    pub action: Action,
    /// Rough size, for the consent line.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size_hint: Option<String>,
    /// Steps that must succeed first.
    #[serde(default)]
    pub needs: Vec<String>,
}

/// What a step would do.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Action {
    /// Nothing to do — already satisfied.
    AlreadyPresent,
    /// Install from scratch.
    Install,
    /// Decision 4: older than the floor, offered after a prompt.
    Update,
    /// Never automatic: show the user the command.
    Manual,
    /// Decision 8: the user pressed "Install for me", so the method chain runs
    /// for this component. Only [`plan_for_component`] ever produces it — the
    /// default plan keeps tmux `Manual`.
    InstallForMe,
}

/// A plan: what to do, in order, and what it needs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    pub steps: Vec<PlanStep>,
    /// Step ids that need the internet.
    pub needs_internet: Vec<String>,
    /// True when the pre-check could not reach the network.
    pub offline: bool,
    /// The one consent line.
    pub consent: String,
}

impl Plan {
    /// Nothing to do at all? A manual step (tmux) is never work we do.
    pub fn is_noop(&self) -> bool {
        self.steps
            .iter()
            .filter(|s| s.action != Action::Manual)
            .all(|s| s.action == Action::AlreadyPresent)
    }
    pub fn step(&self, id: &str) -> Option<&PlanStep> {
        self.steps.iter().find(|s| s.id == id)
    }
}

/// Where a step is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum StepState {
    Waiting,
    Running,
    Done,
    Failed { message: String },
    Skipped { reason: String },
}

/// One progress event, streamed to the UI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub step: String,
    pub state: StepState,
    /// A one-line detail (bytes, a version, a reason, or the method a chain
    /// is on).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// The result of a run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Outcome {
    pub steps: Vec<StepResult>,
    pub cancelled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StepResult {
    pub step: String,
    pub state: StepState,
    /// The last log lines, for the error sheet.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub log_tail: Vec<String>,
    /// The methods the chain ran, in order (decision 8's "Install for me").
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attempted: Vec<String>,
    /// The "do it yourself" command, so the UI can offer Copy.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fallback_command: Option<String>,
}

// ------------------------------------------------------------------ planner

fn ok(state: ComponentState) -> bool {
    state == ComponentState::Ok
}

/// The parts of the report the planner needs, so tests build one by hand.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Detected {
    pub node: ComponentState,
    pub pi: ComponentState,
    pub unipi: ComponentState,
    pub kanboard: ComponentState,
    pub agent_browser: ComponentState,
    pub herdr: ComponentState,
    pub tmux: ComponentState,
    /// UNI-299: Xvfb or Xwayland (the browser's hidden display).
    pub xvfb: ComponentState,
    pub unipi_version: Option<String>,
    /// The pi settings spec (`npm:@pi-unipi/unipi@alpha`).
    pub unipi_spec: Option<String>,
    /// npm's dist-tags, for the channel choice on a PC with nothing installed.
    pub tags: DistTags,
}

impl Default for Detected {
    fn default() -> Self {
        Detected {
            node: ComponentState::Missing,
            pi: ComponentState::Missing,
            unipi: ComponentState::Missing,
            kanboard: ComponentState::Unsupported,
            agent_browser: ComponentState::Missing,
            herdr: ComponentState::Missing,
            tmux: ComponentState::Missing,
            xvfb: ComponentState::Missing,
            unipi_version: None,
            unipi_spec: None,
            tags: DistTags::default(),
        }
    }
}

impl From<&SetupReport> for Detected {
    fn from(report: &SetupReport) -> Self {
        let get = |name: &str| -> Component {
            report.get(name).cloned().unwrap_or(Component {
                name: name.to_string(),
                state: ComponentState::Missing,
                version: None,
                path: None,
                source: super::Source::Found,
                fix: None,
                note: None,
            })
        };
        Detected {
            node: get("node").state,
            pi: get("pi").state,
            unipi: get("unipi").state,
            kanboard: get("kanboard").state,
            agent_browser: get("agent-browser").state,
            herdr: get("herdr").state,
            tmux: get("tmux").state,
            xvfb: get("xvfb").state,
            unipi_version: get("unipi").version,
            unipi_spec: None,
            tags: DistTags::default(),
        }
    }
}

/// Builds the plan from what was detected.
pub fn plan_for(d: &Detected, offline: bool) -> Plan {
    let mut steps = Vec::new();
    let mut needs_internet = Vec::new();

    // 1. Node — reuse when usable; install when missing, update when too old.
    let node = if ok(d.node) {
        Action::AlreadyPresent
    } else {
        needs_internet.push("node".into());
        if d.node == ComponentState::Outdated { Action::Update } else { Action::Install }
    };
    steps.push(PlanStep {
        id: "node".into(),
        label: "Node (JavaScript runtime)".into(),
        action: node,
        size_hint: Some("~30 MB".into()),
        needs: vec![],
    });

    // 2. pi — never when a working one exists anywhere.
    let pi = if ok(d.pi) {
        Action::AlreadyPresent
    } else {
        needs_internet.push("pi".into());
        if d.pi == ComponentState::Outdated { Action::Update } else { Action::Install }
    };
    steps.push(PlanStep {
        id: "pi".into(),
        label: "pi (coding agent)".into(),
        action: pi,
        size_hint: Some("~440 MB".into()),
        needs: vec!["node".into()],
    });

    // 2.5. agent-browser — optional (the `skill:browser` helper). It `needs`
    // pi (its npm install goes into pi's own prefix), so it is pre-ticked
    // exactly when pi is being installed or already present — `run_step`'s
    // `needs` skip already makes an unticked/absent pi leave it undone.
    let agent_browser = if ok(d.agent_browser) {
        Action::AlreadyPresent
    } else {
        needs_internet.push("agent-browser".into());
        Action::Install
    };
    steps.push(PlanStep {
        id: "agent-browser".into(),
        label: "agent-browser (browser skill)".into(),
        action: agent_browser,
        size_hint: Some("~10 MB".into()),
        needs: vec!["pi".into()],
    });

    // 3. UniPi extensions.
    let unipi = if ok(d.unipi) {
        Action::AlreadyPresent
    } else {
        needs_internet.push("unipi".into());
        Action::Install
    };
    // Task 0 (UNI-241): a PC with nothing installed follows `latest` only once
    // `latest` is at least as new as this app build supports.
    let channel = channel_for(d.unipi_spec.as_deref(), d.unipi_version.as_deref(), &d.tags);
    steps.push(PlanStep {
        id: "unipi".into(),
        label: format!("UniPi extensions (channel: {channel})"),
        action: unipi,
        size_hint: None,
        needs: vec!["pi".into()],
    });

    // 4. kanboard — a check; it comes with the extensions.
    steps.push(PlanStep {
        id: "kanboard".into(),
        label: "Board binary".into(),
        action: if ok(d.kanboard) { Action::AlreadyPresent } else { Action::Install },
        size_hint: None,
        needs: vec!["unipi".into()],
    });

    // 5. herdr — optional but preselected on Linux/macOS.
    let herdr = if ok(d.herdr) {
        Action::AlreadyPresent
    } else {
        needs_internet.push("herdr".into());
        Action::Install
    };
    steps.push(PlanStep {
        id: "herdr".into(),
        label: "herdr (terminals)".into(),
        action: herdr,
        size_hint: Some("~5 MB".into()),
        needs: vec![],
    });

    // 6. tmux — never automatic.
    steps.push(PlanStep {
        id: "tmux".into(),
        label: "tmux".into(),
        action: Action::Manual,
        size_hint: None,
        needs: vec![],
    });

    let consent = steps
        .iter()
        .filter(|s| s.action != Action::AlreadyPresent && s.action != Action::Manual)
        .map(|s| match s.size_hint.as_deref() {
            Some(size) => format!("{} ({})", s.label, size),
            None => s.label.clone(),
        })
        .collect::<Vec<_>>()
        .join(", ");
    Plan { steps, needs_internet, offline, consent }
}

/// `HEAD <url>` succeeds? The offline pre-check (5 s).
pub async fn reachable(url: &str) -> bool {
    let Ok(client) = reqwest::Client::builder().timeout(Duration::from_secs(5)).build() else { return false };
    client.head(url).send().await.map(|r| r.status().is_success()).unwrap_or(false)
}

// ------------------------------------------------------------------ layout

/// Where a standalone Node of `version` lives.
pub fn node_dir(home: &Path, version: &str) -> PathBuf {
    home.join(NODE_ROOT_DIR).join(version)
}

/// The `current` symlink, or the newest installed version when it is missing.
pub fn node_current(home: &Path) -> Option<PathBuf> {
    let root = home.join(NODE_ROOT_DIR);
    let current = root.join("current");
    if current.exists() {
        return Some(current);
    }
    let mut versions: Vec<PathBuf> = std::fs::read_dir(&root)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    versions.sort_by_key(|p| {
        let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        super::version_tuple(&name)
    });
    versions.pop()
}

/// `~/.pi/agent/npm` — the npm prefix pi's own installer uses.
pub fn pi_npm_prefix(home: &Path) -> PathBuf {
    home.join(".pi").join("agent").join("npm")
}

/// The log file for a run that started at `now`.
pub fn log_path(home: &Path, now: u64) -> PathBuf {
    home.join(LOG_DIR).join(format!("setup-{now}.log"))
}

// -------------------------------------------------------------------- env

/// Downloads a URL, returning its bytes and their sha256 hex.
type Download = Arc<dyn Fn(&str) -> Option<(Vec<u8>, String)> + Send + Sync>;
/// Runs a command with extra environment, returning its exit status and
/// combined output. The env matters: installers are `curl … | sh` and npm is a
/// `#!/usr/bin/env node` script, so our Node has to be on their PATH.
type Command = Arc<dyn Fn(&Path, &[String], &[(&str, String)]) -> (Option<i32>, String) + Send + Sync>;
/// Fetches a URL as text.
type Fetch = Arc<dyn Fn(&str) -> Option<String> + Send + Sync>;
/// Extracts a downloaded tarball into a directory.
type Extract = Arc<dyn Fn(&Path, &Path) -> std::io::Result<()> + Send + Sync>;

/// How the engine sees the outside world; tests inject fakes.
#[derive(Clone)]
pub struct Env {
    pub home: PathBuf,
    /// Downloads a URL, returning its bytes and their sha256 hex.
    pub download: Download,
    /// Runs a command, returning its exit status and combined output.
    pub run: Command,
    /// Fetches `SHASUMS256` for a Node version.
    pub shasums: Fetch,
    /// Extracts a downloaded tarball into a directory.
    pub extract: Extract,
    /// Now, unix seconds (the log file's name).
    pub now: u64,
    /// How a root-requiring method runs on this PC — `pkexec` on Linux,
    /// `osascript` on macOS. `None` when this PC has neither (no polkit,
    /// Windows), which makes those methods unavailable, not failures.
    /// Resolved once, like the merged PATH.
    pub root: Option<RootHelper>,
}

impl Env {
    /// The real thing.
    pub fn real(home: PathBuf) -> Self {
        Env {
            home,
            download: Arc::new(|url| {
                let client = reqwest::blocking::Client::builder().timeout(Duration::from_secs(900)).build().ok()?;
                let response = client.get(url).send().ok()?;
                if !response.status().is_success() {
                    return None;
                }
                let bytes = response.bytes().ok()?.to_vec();
                let digest = hex_lower(&Sha256::digest(&bytes));
                Some((bytes, digest))
            }),
            run: Arc::new(|bin, args, env| {
                let mut command = std::process::Command::new(bin);
                command.args(args);
                for (key, value) in env {
                    command.env(key, value);
                }
                match command.output() {
                    Ok(out) => {
                        let mut text = String::from_utf8_lossy(&out.stdout).to_string();
                        text.push_str(&String::from_utf8_lossy(&out.stderr));
                        (out.status.code(), text)
                    }
                    Err(e) => (None, format!("could not run {}: {e}", bin.display())),
                }
            }),
            shasums: Arc::new(|url| {
                let client = reqwest::blocking::Client::builder().timeout(Duration::from_secs(30)).build().ok()?;
                client.get(url).send().ok()?.text().ok()
            }),
            extract: Arc::new(|tarball, dest| {
                let status = std::process::Command::new("tar")
                    .arg("-xzf")
                    .arg(tarball)
                    .arg("-C")
                    .arg(dest)
                    .arg("--strip-components=1")
                    .status()?;
                if status.success() {
                    Ok(())
                } else {
                    Err(std::io::Error::other("tar failed"))
                }
            }),
            now: super::now_unix(),
            root: root_helper_here(),
        }
    }
}

/// Lowercase hex, so we do not need a hex crate.
fn hex_lower(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

// ----------------------------------------------------------------- installer

/// The engine, as the commands and tests use it: an environment plus the log a
/// run leaves behind. The steps themselves are free functions (see
/// [`run_steps`]) so they can move onto a blocking thread.
pub struct Installer {
    env: Env,
}

impl Installer {
    pub fn new(env: Env) -> Self {
        Installer { env }
    }

    /// Runs `plan`; `cancel` stops between steps.
    ///
    /// The steps are blocking work (a download, a child process), so they run on
    /// a blocking thread and their progress is handed back over a channel —
    /// running `reqwest::blocking` inside the async runtime panics at drop.
    pub async fn run<F>(&self, plan: &Plan, mut on_progress: F, cancel: &mut watch::Receiver<bool>) -> Outcome
    where
        F: FnMut(Progress),
    {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Progress>();
        let env = self.env.clone();
        let plan = plan.clone();
        let stop = cancel.clone();
        let joined = tokio::task::spawn_blocking(move || {
            run_steps(&env, Vec::new(), &plan, &stop, &tx)
        });
        while let Some(progress) = rx.recv().await {
            on_progress(progress);
        }
        joined.await.unwrap_or_else(|_| Outcome { steps: Vec::new(), cancelled: true })
    }
}

/// The steps. Each takes the environment and the shared log, so the loop can
/// run on a blocking thread (see [`run_steps`]).
// ------------------------------------------------------------------ node
fn step_node(env: &Env, log: &mut Vec<String>) -> StepState {
    let version = NODE_INSTALL_VERSION;
    log.push(format!("installing node {version}"));
    // nodejs.org serves the checksums as SHASUMS256.txt (the bare
    // SHASUMS256 redirects to a 404).
    let shasums_url = format!("{NODE_DIST}/v{version}/SHASUMS256.txt");
    let Some(shasums) = (env.shasums)(&shasums_url) else {
        log.push("could not fetch SHASUMS256.txt".into());
        return StepState::Failed { message: "Could not fetch Node's checksums".into() };
    };
    let tarball = node_tarball_name(version);
    let expected = shasums.lines().find_map(|line| {
        let (hash, name) = line.split_once("  ")?;
        (name.trim() == tarball).then(|| hash.trim().to_string())
    });
    let Some(expected) = expected else {
        log.push(format!("{tarball} not in SHASUMS256"));
        return StepState::Failed { message: "Node's checksum file has no entry for this platform".into() };
    };
    let Some((bytes, digest)) = (env.download)(&format!("{NODE_DIST}/v{version}/{tarball}")) else {
        log.push(format!("download failed: {tarball}"));
        return StepState::Failed { message: format!("Could not download {tarball}") };
    };
    if digest != expected {
        log.push(format!("checksum mismatch: got {digest}, want {expected}"));
        // Nothing is written before this point, so a bad download leaves the PC untouched.
        return StepState::Failed { message: "The download's checksum does not match Node's, so nothing was installed".into() };
    }
    let dest = node_dir(&env.home, version);
    let tmp = env.home.join(format!(".tmp-node-{version}"));
    let _ = std::fs::remove_dir_all(&tmp);
    if std::fs::create_dir_all(&tmp).is_err() {
        return StepState::Failed { message: "Could not create the Node directory".into() };
    }
    let tarball_path = tmp.join(&tarball);
    if std::fs::write(&tarball_path, &bytes).is_err() {
        let _ = std::fs::remove_dir_all(&tmp);
        return StepState::Failed { message: "Could not save the Node download".into() };
    }
    if (env.extract)(&tarball_path, &tmp).is_err() {
        let _ = std::fs::remove_dir_all(&tmp);
        return StepState::Failed { message: "Could not unpack the Node tarball".into() };
    }
    let _ = std::fs::remove_file(&tarball_path);
    // The version's parent may not exist yet on a bare PC.
    if let Some(parent) = dest.parent() {
        if std::fs::create_dir_all(parent).is_err() {
            let _ = std::fs::remove_dir_all(&tmp);
            return StepState::Failed { message: "Could not create the Node directory".into() };
        }
    }
    if std::fs::rename(&tmp, &dest).is_err() {
        let _ = std::fs::remove_dir_all(&tmp);
        return StepState::Failed { message: "Could not move Node into place".into() };
    }
    // `current` points at the version we just installed.
    let current = env.home.join(NODE_ROOT_DIR).join("current");
    let _ = std::fs::remove_file(&current);
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        let _ = symlink(&dest, &current);
    }
    log.push(format!("node {version} installed at {}", dest.display()));
    StepState::Done
}

// -------------------------------------------------------------------- pi

fn step_pi(env: &Env, log: &mut Vec<String>) -> StepState {
    // N1: pi.dev's own installer, run with the Node we have. It needs Node
    // already (without a TTY it skips its own Node offer), which is why we
    // install Node first.
    if node_binary(env).is_none() {
        log.push("N1 skipped: no Node to run the installer with".into());
        return install_pi_n2(env, log);
    }
    if matches!(install_pi_n1(env, log), StepState::Done) {
        return StepState::Done;
    }
    log.push("N1 failed, falling back to N2".into());
    install_pi_n2(env, log)
}

/// N1 alone, so the method chain can address it on its own: "Try another way"
/// then means "npm instead of pi.dev's installer", which is only true when N1
/// has not quietly already fallen through to N2 itself.
fn install_pi_n1(env: &Env, log: &mut Vec<String>) -> StepState {
    log.push("N1: pi.dev's installer".into());
    // It is a shell script: run it with `sh` (not as JavaScript) and with our
    // Node first on its PATH.
    let sh = std::env::var_os("SHELL").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/bin/sh"));
    let script = format!("curl -fsSL {PI_INSTALLER_URL} | sh");
    let (status, out) = (env.run)(&sh, &["-c".to_string(), script], &with_node_path(env));
    log.push(format!("N1 exit {status:?}: {out}"));
    if status == Some(0) {
        return StepState::Done;
    }
    StepState::Failed { message: describe_failure(&sh, status, &out) }
}

/// N2: npm install into pi's own prefix.
fn install_pi_n2(env: &Env, log: &mut Vec<String>) -> StepState {
    let Some(npm) = npm_binary(env) else {
        return StepState::Failed { message: "npm was not found".into() };
    };
    let prefix = pi_npm_prefix(&env.home);
    let _ = std::fs::create_dir_all(&prefix);
    let args = [
        "install".to_string(),
        "--global".to_string(),
        "--ignore-scripts".to_string(),
        format!("--prefix={}", prefix.display()),
        PI_PACKAGE.to_string(),
    ];
    let node_path = with_node_path(env);
    let (status, out) = (env.run)(&npm, &args, &node_path);
    log.push(format!("N2 exit {status:?}: {out}"));
    match status {
        Some(0) => StepState::Done,
        _ => StepState::Failed { message: "npm could not install pi".into() },
    }
}

// ---------------------------------------------------------------- unipi

fn step_unipi(env: &Env, log: &mut Vec<String>, channel: String) -> StepState {
    let Some(pi) = find_pi(&env.home) else {
        return StepState::Failed { message: "pi is not installed".into() };
    };
    // The channel follows the updater's rules, never a hardcoded alpha.
    let spec = install_spec(&channel);
    // pi is on the PATH we already searched, but a bare `pi` still needs its
    // Node — hand it the same PATH prepend.
    let (status, out) = (env.run)(&pi, &["install".to_string(), spec.clone()], &with_node_path(env));
    log.push(format!("pi install {spec} exit {status:?}: {out}"));
    match status {
        Some(0) => StepState::Done,
        _ => StepState::Failed { message: format!("pi could not install {spec}") },
    }
}

// ---------------------------------------------------------- agent-browser

/// The npm package name for the `skill:browser` helper.
pub const AGENT_BROWSER_PACKAGE: &str = "agent-browser";

/// `npm install -g agent-browser` into the npm prefix pi's own N2 fallback
/// uses (`~/.pi/agent/npm`), so it is found the same way pi is. Never runs
/// `agent-browser install` — that downloads its own Chrome, and the skill
/// drives the host's CloakBrowser instead.
fn step_agent_browser(env: &Env, log: &mut Vec<String>) -> StepState {
    let Some(npm) = npm_binary(env) else {
        return StepState::Failed { message: "npm was not found".into() };
    };
    let prefix = pi_npm_prefix(&env.home);
    let _ = std::fs::create_dir_all(&prefix);
    let args = [
        "install".to_string(),
        "--global".to_string(),
        format!("--prefix={}", prefix.display()),
        AGENT_BROWSER_PACKAGE.to_string(),
    ];
    let node_path = with_node_path(env);
    let (status, out) = (env.run)(&npm, &args, &node_path);
    log.push(format!("agent-browser install exit {status:?}: {out}"));
    if status != Some(0) {
        return StepState::Failed { message: "npm could not install agent-browser".into() };
    }
    // Verify it actually runs, the way the release-asset installs do.
    let bin = prefix.join("bin").join("agent-browser");
    let (status, out) = (env.run)(&bin, &["--version".to_string()], &node_path);
    log.push(format!("agent-browser --version exit {status:?}: {out}"));
    match status {
        Some(0) => StepState::Done,
        _ if !out.trim().is_empty() => StepState::Done,
        _ => StepState::Failed { message: "agent-browser was installed but does not run".into() },
    }
}

// -------------------------------------------------------------- kanboard

fn step_kanboard(env: &Env, log: &mut Vec<String>) -> StepState {
    // Normally nothing to install: the extensions pull the platform package
    // as an optional dependency. When npm skipped it (as it does under
    // `--ignore-scripts`), install it directly — that is exactly what a repair
    // means, and it is the file the host's own discovery looks for.
    if let Some(bin) = super::kanboard_binary(&super::known_dirs(&env.home), &env.home) {
        log.push(format!("board binary at {}", bin.display()));
        return StepState::Done;
    }
    let Some(pi) = find_pi(&env.home) else {
        return StepState::Failed { message: "pi is not installed".into() };
    };
    let platform = super::platform_key();
    let spec = format!("npm:@pi-unipi/kanboard-{platform}");
    let (status, out) = (env.run)(&pi, &["install".to_string(), spec.clone()], &with_node_path(env));
    log.push(format!("pi install {spec} exit {status:?}: {out}"));
    match (&status, super::kanboard_binary(&super::known_dirs(&env.home), &env.home)) {
        (Some(0), Some(bin)) => {
            log.push(format!("board binary at {}", bin.display()));
            StepState::Done
        }
        _ => StepState::Failed { message: "The board binary is still missing after installing UniPi".into() },
    }
}

// ---------------------------------------------------------------- herdr

fn step_herdr(env: &Env, log: &mut Vec<String>) -> StepState {
    // The plan's automatic herdr step runs herdr's installer and nothing else: a
    // preselected wizard step is not decision 8's explicit "Install for me", so
    // there is no chain to fall through. `Action::InstallForMe` takes the chain
    // below instead.
    let script = format!("curl -fsSL {HERDR_INSTALLER_URL} | sh");
    log.push(format!("running {HERDR_INSTALLER_URL}"));
    match run_installer(env, log, &script) {
        Ok(()) => StepState::Done,
        Err(_) => StepState::Failed { message: "herdr's installer failed".into() },
    }
}

// ------------------------------------------------ "Install for me" (decision 8)

/// How a root-requiring method runs on this PC.
///
/// Never a bare `sudo`/`apt` from a GUI process with no TTY (it just fails, or
/// blocks on a password prompt nobody can see): Linux raises a polkit prompt
/// through `pkexec`, macOS through AppleScript's
/// `with administrator privileges`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RootHelper {
    /// The binary that runs [`RootHelper::argv`]: `pkexec`, `/usr/bin/osascript`.
    pub bin: PathBuf,
    /// macOS hands the command to `osascript -e` as a shell script; Linux hands
    /// it to `pkexec` as argv, unchanged.
    pub shell_script: bool,
}

impl RootHelper {
    /// The argv that runs `command` under this helper. Pure — nothing is spawned,
    /// so the tests need no `pkexec` on the box.
    pub fn argv(&self, command: &[String]) -> Vec<String> {
        if self.shell_script {
            vec![
                "-e".to_string(),
                format!("do shell script \"{}\" with administrator privileges", command.join(" ")),
            ]
        } else {
            command.to_vec()
        }
    }
}

/// `pkexec` on this PC: the PATH first (an app or a test may put a fake one
/// there), then the standard locations — the same resolution
/// `host_supervisor::systemctl_in` uses for `systemctl`. `None` when this PC has
/// none, which makes a root method *unavailable*, never a failure.
pub fn pkexec_in(path_var: Option<&std::ffi::OsStr>, standard: &[&Path]) -> Option<PathBuf> {
    let from_path: Vec<PathBuf> = path_var.map(std::env::split_paths).into_iter().flatten().collect();
    super::find_in(&from_path, "pkexec").or_else(|| {
        let dirs: Vec<PathBuf> = standard.iter().map(|d| d.to_path_buf()).collect();
        super::find_in(&dirs, "pkexec")
    })
}

/// [`pkexec_in`] over this process's PATH and the standard locations.
fn pkexec_here() -> Option<PathBuf> {
    let standard: Vec<PathBuf> = PKEXEC_STANDARD.iter().map(PathBuf::from).collect();
    pkexec_in(std::env::var_os("PATH").as_deref(), &standard.iter().map(Path::new).collect::<Vec<_>>())
}

/// This PC's root helper, or `None` when there is nothing to run it with (no
/// polkit on Linux, Windows).
pub fn root_helper_here() -> Option<RootHelper> {
    #[cfg(target_os = "macos")]
    {
        Some(RootHelper { bin: PathBuf::from("/usr/bin/osascript"), shell_script: true })
    }
    #[cfg(target_os = "linux")]
    {
        pkexec_here().map(|bin| RootHelper { bin, shell_script: false })
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        None
    }
}

/// A package manager the app can drive non-interactively.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PkgManager {
    Apt,
    Dnf,
    Pacman,
    Brew,
}

impl PkgManager {
    /// The binary that installs the package.
    pub fn program(&self) -> &'static str {
        match self {
            PkgManager::Apt => "apt-get",
            PkgManager::Dnf => "dnf",
            PkgManager::Pacman => "pacman",
            PkgManager::Brew => "brew",
        }
    }

    /// The install argv for a package, without any root wrapper.
    pub fn install_args(&self, package: &str) -> Vec<String> {
        match self {
            // `apt-get`, not `apt`: the scriptable half of the pair, and the one
            // that answers with no TTY behind it.
            PkgManager::Apt => vec!["install".into(), "-y".into(), package.into()],
            PkgManager::Dnf => vec!["install".into(), "-y".into(), package.into()],
            PkgManager::Pacman => vec!["-S".into(), "--noconfirm".into(), package.into()],
            PkgManager::Brew => vec!["install".into(), package.into()],
        }
    }

    /// Does it need root? A system package manager does; Homebrew's prefix is
    /// owned by the user on macOS, so it does not.
    pub fn needs_root(&self) -> bool {
        !matches!(self, PkgManager::Brew)
    }

    /// This PC's package manager, from its release file — the same detection
    /// `setup::package_manager_tmux_command` shows the command for. `None` on
    /// Windows and on a Linux we cannot recognise.
    pub fn detect() -> Option<PkgManager> {
        #[cfg(target_os = "macos")]
        {
            Some(PkgManager::Brew)
        }
        #[cfg(all(not(target_os = "macos"), target_os = "linux"))]
        {
            if Path::new("/etc/debian_version").exists() {
                Some(PkgManager::Apt)
            } else if Path::new("/etc/fedora-release").exists() || Path::new("/etc/redhat-release").exists() {
                Some(PkgManager::Dnf)
            } else if Path::new("/etc/arch-release").exists() {
                Some(PkgManager::Pacman)
            } else {
                None
            }
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        {
            None
        }
    }
}

/// One way to install a component: decision 8's chain, in the order we try it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Method {
    /// What it is, as the progress names it: `apt-get (pkexec)`.
    pub name: String,
    /// Does it need root? Root methods run under [`RootHelper`], and a PC with
    /// none skips them instead of failing them.
    pub root: bool,
    /// The "do it yourself" command, offered with Copy.
    pub diy: String,
    /// What it runs.
    #[serde(skip)]
    kind: MethodKind,
}

/// What a method actually does.
#[derive(Debug, Clone, PartialEq, Eq)]
enum MethodKind {
    /// A package manager's install argv, under the root helper when it needs root.
    Package(PkgManager),
    /// A shell one-liner (`curl … | sh`).
    Script(String),
    /// A static release asset written into `~/.local/bin`.
    StaticBinary,
    /// pi's own installer (`sh`), with the Node we have first on its PATH.
    PiInstaller,
    /// `npm install` into pi's own prefix.
    PiNpm,
}

/// The chain for a component, in the order we try it (decision 8).
///
/// * **pi** — N1: pi.dev's own installer, run with the Node we have. N2: npm
///   into pi's own prefix, for when the installer cannot be reached. The
///   wizard's "Try another way" runs this list, which is the same order the
///   automatic step already follows.
/// * **tmux** — a system package: root first (`pkexec apt-get install -y tmux`,
///   `dnf`, `pacman`). On macOS Homebrew's prefix belongs to the user, so
///   `brew install tmux` needs no root at all.
/// * **herdr** — its own installer (no root), then decision 8's static release
///   binary in `~/.local/bin` for when the installer cannot be reached.
///
/// Empty on Windows, and on a Linux we cannot name a package manager for: there
/// is nothing to run, so the step shows the command instead of failing.
pub fn methods_for(component: &str) -> Vec<Method> {
    match component {
        "pi" if cfg!(unix) => vec![
            Method { name: "pi.dev installer".into(), root: false, diy: format!("curl -fsSL {PI_INSTALLER_URL} | sh"), kind: MethodKind::PiInstaller },
            Method { name: "npm".into(), root: false, diy: pi_npm_diy(), kind: MethodKind::PiNpm },
        ],
        "tmux" => match PkgManager::detect() {
            Some(manager) => vec![Method {
                name: format!(
                    "{} ({})",
                    manager.program(),
                    if manager.needs_root() { "pkexec" } else { "no root" }
                ),
                root: manager.needs_root(),
                diy: super::package_manager_tmux_command().to_string(),
                kind: MethodKind::Package(manager),
            }],
            None => Vec::new(),
        },
        // UNI-299: the browser's hidden display — Linux only, package manager only.
        "xvfb" if cfg!(target_os = "linux") => match PkgManager::detect() {
            Some(manager) if manager != PkgManager::Brew => vec![Method {
                name: format!("{} (pkexec)", manager.program()),
                root: manager.needs_root(),
                diy: super::package_manager_xvfb_command().to_string(),
                kind: MethodKind::Package(manager),
            }],
            _ => Vec::new(),
        },
        "herdr" => {
            let mut methods = Vec::new();
            if cfg!(unix) {
                let installer = format!("curl -fsSL {HERDR_INSTALLER_URL} | sh");
                methods.push(Method {
                    name: "herdr installer".into(),
                    root: false,
                    diy: installer.clone(),
                    kind: MethodKind::Script(installer),
                });
                methods.push(Method {
                    name: "static binary in ~/.local/bin".into(),
                    root: false,
                    diy: herdr_static_diy(),
                    kind: MethodKind::StaticBinary,
                });
            }
            methods
        }
        _ => Vec::new(),
    }
}

/// The "do it yourself" line for herdr's static binary.
fn herdr_static_diy() -> String {
    format!(
        "curl -fsSL -o ~/.local/bin/herdr {HERDR_RELEASES}/{} && chmod +x ~/.local/bin/herdr",
        herdr_asset()
    )
}

/// herdr's release asset for this platform. herdr names its assets after
/// the platform (`herdr-linux-x86_64`, `herdr-macos-aarch64`), so Rust's
/// `consts` are the asset names as they are.
pub fn herdr_asset() -> String {
    format!("herdr-{}-{}", std::env::consts::OS, std::env::consts::ARCH)
}

/// The "do it yourself" command for a component — always available, whatever
/// the chain does.
pub fn fallback_command(component: &str) -> String {
    match component {
        "tmux" => super::package_manager_tmux_command().to_string(),
        "xvfb" => super::package_manager_xvfb_command().to_string(),
        // Windows's installer is a one-liner of its own (herdr.dev/docs/install).
        "herdr" if cfg!(windows) => "curl.exe -fsSLo install.cmd https://herdr.dev/install.cmd && install.cmd".into(),
        "herdr" => format!("curl -fsSL {HERDR_INSTALLER_URL} | sh"),
        // pi's npm way is the one that works without a network path to pi.dev.
        "pi" => pi_npm_diy(),
        other => format!("install {other}"),
    }
}

/// What running one step leaves behind.
struct StepOutcome {
    state: StepState,
    /// The methods the chain ran, in order (decision 8).
    attempted: Vec<String>,
    /// The "do it yourself" command, for Copy in the error sheet.
    fallback_command: Option<String>,
    /// The one-line detail the streamed `Progress` carries.
    detail: Option<String>,
}

impl StepOutcome {
    /// A step that is not a chain: nothing was tried, nothing to copy.
    fn plain(state: StepState) -> Self {
        StepOutcome { state, attempted: Vec::new(), fallback_command: None, detail: None }
    }
}

/// The method chain: the first method that works wins, and nothing is left
/// half-installed. A method that fails (or is unavailable, because this PC has
/// no `pkexec`) moves the chain on; when the chain runs out, the outcome names
/// the last error and the fallback command, so the UI can offer Copy.
fn chain_step(env: &Env, log: &mut Vec<String>, component: &str) -> StepOutcome {
    let fallback = fallback_command(component);
    let mut attempted: Vec<String> = Vec::new();
    let mut last_error = String::new();
    let mut done = false;
    for method in methods_for(component) {
        // No `pkexec` on this PC: the method is unavailable, not a failure.
        if method.root && env.root.is_none() {
            log.push(format!("{} unavailable: this PC has no pkexec", method.name));
            continue;
        }
        attempted.push(method.name.clone());
        log.push(format!("method {}: {}", method.name, method.diy));
        match run_method(env, log, component, &method) {
            Ok(()) => {
                log.push(format!("{} worked", method.name));
                done = true;
                break;
            }
            Err(error) => {
                log.push(format!("{} failed: {error}", method.name));
                last_error = error;
            }
        }
    }
    let (state, detail) = if done {
        (StepState::Done, format!("installed with {}", attempted.join(", ")))
    } else if attempted.is_empty() {
        (
            StepState::Skipped {
                reason: format!("no method available here — install it yourself: {fallback}"),
            },
            format!("nothing to run here; install it yourself: {fallback}"),
        )
    } else {
        (
            StepState::Failed {
                message: format!("{last_error}. Install it yourself: {fallback}"),
            },
            format!("tried {}, all failed: {last_error}", attempted.join(" → ")),
        )
    };
    StepOutcome { state, attempted, fallback_command: Some(fallback), detail: Some(detail) }
}

/// Runs one method. `Ok(())` when it worked, `Err` with what went wrong.
fn run_method(env: &Env, log: &mut Vec<String>, component: &str, method: &Method) -> Result<(), String> {
    match &method.kind {
        MethodKind::Package(manager) => {
            // UNI-299: Xvfb's package name differs per distro.
            let package = if component == "xvfb" { super::xvfb_package(manager.program()) } else { component };
            let args = manager.install_args(package);
            // The command the root helper runs: the manager's own argv.
            let mut command = vec![manager.program().to_string()];
            command.extend(args);
            let (bin, argv) = if method.root {
                let helper = env.root.as_ref().ok_or("this PC has no way to run as root")?;
                (helper.bin.clone(), helper.argv(&command))
            } else {
                (PathBuf::from(manager.program()), command)
            };
            let (status, out) = (env.run)(&bin, &argv, &[]);
            log.push(format!("{} exit {status:?}: {out}", argv.join(" ")));
            match status {
                Some(0) => Ok(()),
                _ => Err(describe_failure(&bin, status, &out)),
            }
        }
        MethodKind::Script(script) => {
            run_installer(env, log, script)
        }
        MethodKind::StaticBinary => install_static(env, log, component),
        // pi's two ways are the automatic step's own, so the chain and the
        // wizard's "Try another way" run exactly what a first install runs.
        MethodKind::PiInstaller => match install_pi_n1(env, log) {
            StepState::Done => Ok(()),
            other => Err(step_error(other)),
        },
        MethodKind::PiNpm => match install_pi_n2(env, log) {
            StepState::Done => Ok(()),
            other => Err(step_error(other)),
        },
    }
}

/// A step's failure as a method's error.
fn step_error(state: StepState) -> String {
    match state {
        StepState::Failed { message } => message,
        StepState::Skipped { reason } => reason,
        _ => "the step reported nothing".into(),
    }
}

/// The "do it yourself" line for pi's npm way.
fn pi_npm_diy() -> String {
    "npm install --global --ignore-scripts @earendil-works/pi-coding-agent".to_string()
}

/// `curl … | sh`: herdr's and pi's installers are shell one-liners.
fn run_installer(env: &Env, log: &mut Vec<String>, script: &str) -> Result<(), String> {
    let sh = std::env::var_os("SHELL").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/bin/sh"));
    let (status, out) = (env.run)(&sh, &["-c".to_string(), script.to_string()], &with_node_path(env));
    log.push(format!("{script} exit {status:?}: {out}"));
    match status {
        Some(0) => Ok(()),
        _ => Err(describe_failure(&sh, status, &out)),
    }
}

/// Decision 8's static-binary method: herdr's release asset lands in
/// `~/.local/bin`, mode 0755, written through a temp name and moved into place,
/// so a failed download can never leave a half-installed binary on the PATH.
fn install_static(env: &Env, log: &mut Vec<String>, component: &str) -> Result<(), String> {
    let url = match component {
        "herdr" => format!("{HERDR_RELEASES}/{}", herdr_asset()),
        other => return Err(format!("no static {other} binary is published")),
    };
    log.push(format!("downloading {url}"));
    let Some((bytes, digest)) = (env.download)(&url) else {
        return Err(format!("could not download {url}"));
    };
    let bin = crate::host_supervisor::bin_dir(&env.home).join(component);
    let dir = bin.parent().ok_or("no bin directory")?;
    std::fs::create_dir_all(dir).map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    let tmp = dir.join(format!(".{component}.new"));
    std::fs::write(&tmp, &bytes).map_err(|e| format!("could not write {}: {e}", tmp.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| format!("could not chmod {}: {e}", tmp.display()))?;
    }
    std::fs::rename(&tmp, &bin).map_err(|e| format!("could not move {} into place: {e}", bin.display()))?;
    log.push(format!("sha256 {digest}  {}", bin.display()));
    // Does it answer? A binary that cannot run is a silent failure, not an install.
    let (status, out) = (env.run)(&bin, &["--version".to_string()], &[]);
    log.push(format!("{} --version exit {status:?}: {out}", bin.display()));
    match status {
        Some(0) => Ok(()),
        _ if !out.trim().is_empty() => Ok(()),
        _ => Err(format!("{} was installed but does not run", bin.display())),
    }
}

/// What went wrong, in one line for the error sheet.
fn describe_failure(bin: &Path, status: Option<i32>, out: &str) -> String {
    let first = out.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    let ran = match status {
        Some(code) => format!("{} exited {code}", bin.display()),
        None => format!("could not run {}", bin.display()),
    };
    if first.is_empty() {
        ran
    } else {
        format!("{ran}: {first}")
    }
}

// ------------------------------------------------------------- binaries

/// Our Node's `bin` prepended to `PATH`, so installers and npm find it. Empty
/// when there is no Node yet (nothing to prepend).
fn with_node_path(env: &Env) -> Vec<(&str, String)> {
    let Some(dir) = node_current(&env.home).map(|d| d.join("bin")) else {
        return Vec::new();
    };
    let inherited = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
        .unwrap_or_default();
    let mut dirs = vec![dir];
    dirs.extend(inherited);
    let value = dirs.iter().map(|d| d.display().to_string()).collect::<Vec<_>>().join(":");
    vec![("PATH", value)]
}

fn node_binary(env: &Env) -> Option<PathBuf> {
    let bin = node_current(&env.home)?.join("bin").join("node");
    bin.exists().then_some(bin)
}

fn npm_binary(env: &Env) -> Option<PathBuf> {
    let bin = node_current(&env.home)?.join("bin").join("npm");
    bin.exists().then_some(bin)
}


/// The blocking half of [`Installer::run`]: the step loop, kept a free function
/// so it can move onto a blocking thread with everything it owns.
fn run_steps(
    env: &Env,
    mut log: Vec<String>,
    plan: &Plan,
    stop: &watch::Receiver<bool>,
    tx: &tokio::sync::mpsc::UnboundedSender<Progress>,
) -> Outcome {
    let mut results: Vec<StepResult> = Vec::new();
    let mut cancelled = false;
    for step in &plan.steps {
        if *stop.borrow() {
            cancelled = true;
            let state = StepState::Skipped { reason: "Cancelled".into() };
            results.push(StepResult {
                step: step.id.clone(),
                state: state.clone(),
                log_tail: tail_of(&log),
                attempted: Vec::new(),
                fallback_command: None,
            });
            let _ = tx.send(Progress { step: step.id.clone(), state, detail: None });
            continue;
        }
        // The running detail is the newest log line: bytes, a version, or the
        // method a chain is on.
        let _ = tx.send(Progress {
            step: step.id.clone(),
            state: StepState::Running,
            detail: log.last().cloned(),
        });
        let outcome = run_step(env, &mut log, &results, step);
        results.push(StepResult {
            step: step.id.clone(),
            state: outcome.state.clone(),
            log_tail: tail_of(&log),
            attempted: outcome.attempted,
            fallback_command: outcome.fallback_command,
        });
        let _ = tx.send(Progress {
            step: step.id.clone(),
            state: outcome.state,
            detail: outcome.detail,
        });
    }
    // The full log, for "Show log" in the UI.
    let path = log_path(&env.home, env.now);
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(&path, log.join("\n"));
    Outcome { steps: results, cancelled }
}

/// The last log lines, for the error sheet.
const LOG_TAIL_LINES: usize = 20;

fn tail_of(log: &[String]) -> Vec<String> {
    log.iter().rev().take(LOG_TAIL_LINES).rev().cloned().collect()
}

/// One step: skipped when a dependency did not succeed, a no-op when it is
/// already satisfied.
fn run_step(env: &Env, log: &mut Vec<String>, results: &[StepResult], step: &PlanStep) -> StepOutcome {
    for need in &step.needs {
        let unhappy = results.iter().any(|r| {
            r.step == *need && matches!(r.state, StepState::Failed { .. } | StepState::Skipped { .. })
        });
        if unhappy {
            return StepOutcome::plain(StepState::Skipped { reason: format!("needs {need}") });
        }
    }
    if step.action == Action::AlreadyPresent {
        return StepOutcome::plain(StepState::Done);
    }
    let state = match step.id.as_str() {
        "node" => step_node(env, log),
        // pi's chain is its N1 → N2 pair (see `methods_for`), so the failure
        // sheet's "Try another way" reaches the same code a first install runs.
        "pi" if step.action == Action::InstallForMe => return chain_step(env, log, "pi"),
        "pi" => step_pi(env, log),
        "unipi" => step_unipi(env, log, step_channel(step)),
        "agent-browser" => step_agent_browser(env, log),
        "kanboard" => step_kanboard(env, log),
        // Decision 8: the explicit button runs the chain, and the plan's
        // automatic herdr step runs herdr's installer on its own.
        "herdr" if step.action == Action::InstallForMe => return chain_step(env, log, "herdr"),
        "herdr" => step_herdr(env, log),
        // tmux is never automatic: the command is the answer — unless the user
        // asked for it by name.
        "tmux" if step.action == Action::InstallForMe => return chain_step(env, log, "tmux"),
        "xvfb" if step.action == Action::InstallForMe => return chain_step(env, log, "xvfb"),
        "tmux" => StepState::Skipped { reason: super::package_manager_tmux_command().to_string() },
        _ => StepState::Skipped { reason: "nothing to do".into() },
    };
    StepOutcome::plain(state)
}

/// The npm name of the Node tarball for this platform. nodejs.org uses npm's
/// platform names (`linux-x64`), not Rust's (`linux-x86_64`) — same mapping as
/// the host's kanboard discovery (board/discovery.rs::platform_key).
pub fn node_tarball_name(version: &str) -> String {
    let os = match std::env::consts::OS {
        "macos" => "darwin",
        "windows" => "win",
        other => other,
    };
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        "x86" => "ia32",
        other => other,
    };
    if os == "win" {
        format!("node-v{version}-win-{arch}.zip")
    } else {
        format!("node-v{version}-{os}-{arch}.tar.gz")
    }
}

/// The channel a `unipi` step's label carries (`channel: alpha`).
fn step_channel(step: &PlanStep) -> String {
    step.label
        .split_once("channel: ")
        .map(|(_, rest)| rest.trim_end_matches(')').to_string())
        .unwrap_or_else(|| "latest".to_string())
}

/// `pi` on the merged PATH or in the places pi's installers use.
pub fn find_pi(home: &Path) -> Option<PathBuf> {
    super::find_in(&super::known_dirs(home), "pi")
        .or_else(|| {
            let bin = home.join(".pi/agent/install/bin/pi");
            bin.exists().then_some(bin)
        })
        .or_else(|| {
            let bin = pi_npm_prefix(home).join("bin/pi");
            bin.exists().then_some(bin)
        })
}

// ---------------------------------------------------------------- commands

/// The cancel flag, shared between `setup_run` and `setup_cancel`.
#[derive(Default, Clone)]
pub struct CancelFlag(pub Arc<watch::Sender<bool>>);

pub async fn plan_here() -> Plan {
    plan_here_only(None).await
}

/// [`plan_here`], narrowed to `only` — the step ids the user left ticked in the
/// wizard's consent list. A step another one `needs` is never dropped: that
/// would turn "skipped, needs pi" into a broken install.
pub async fn plan_here_only(only: Option<Vec<String>>) -> Plan {
    let home = crate::host_supervisor::home_dir();
    let report = super::detect_here(&home).await;
    let offline = !reachable(NPM_REGISTRY).await;
    let mut detected = Detected::from(&report);
    detected.unipi_spec = super::read_unipi_spec(&home, Path::new("."));
    detected.tags = super::unipi_dist_tags().await;
    let plan = plan_for(&detected, offline);
    narrow(&plan, only)
}

/// A plan narrowed to `only` (the wizard's ticked step ids), plus whatever
/// those steps `needs`. A step another one depends on is never dropped — that
/// would turn "skipped, needs pi" into a broken install.
fn narrow(plan: &Plan, only: Option<Vec<String>>) -> Plan {
    let Some(ticked) = only else { return plan.clone() };
    // Close over dependencies: if a ticked step needs `pi`, and `pi` needs
    // `node`, all three run — in the planner's order.
    let mut keep: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut frontier: Vec<String> = ticked.into_iter().collect();
    while let Some(id) = frontier.pop() {
        if !keep.insert(id.clone()) {
            continue;
        }
        if let Some(step) = plan.step(&id) {
            frontier.extend(step.needs.iter().cloned());
        }
    }
    let steps: Vec<PlanStep> = plan.steps.iter().filter(|s| keep.contains(&s.id)).cloned().collect();
    Plan { steps, ..plan.clone() }
}

/// `setup_plan`: the plan for this PC (decision 4's items come back as
/// `update`, so the UI can confirm them).
#[tauri::command]
pub async fn setup_plan() -> Result<Plan, AppErrorDto> {
    Ok(plan_here().await)
}

/// The plan for an explicit "Install for me" (decision 8): one step, for this
/// component only, with the method chain enabled.
///
/// A pure function over what was detected, like [`plan_for`], so the UI's call
/// is the only part that needs a PC. The default plan never calls it: tmux
/// stays `Action::Manual` there and is never installed on its own.
pub fn plan_for_component(name: &str, d: &Detected) -> Plan {
    let (present, label, size_hint) = match name {
        "tmux" => (d.tmux == ComponentState::Ok, "tmux".to_string(), None),
        "xvfb" => (d.xvfb == ComponentState::Ok, "Hidden browser display (Xvfb)".to_string(), Some("~3 MB".into())),
        "herdr" => (d.herdr == ComponentState::Ok, "herdr (terminals)".to_string(), Some("~5 MB".into())),
        // pi gets the same treatment so the failure sheet's "Try another way"
        // has somewhere to go: N1 and N2 are its chain (see `methods_for`).
        "pi" => (d.pi == ComponentState::Ok, "pi (coding agent)".to_string(), Some("~440 MB".into())),
        // Anything we have no chain for is left alone rather than fake-planned.
        other => (true, other.to_string(), None),
    };
    let action = if present { Action::AlreadyPresent } else { Action::InstallForMe };
    Plan {
        steps: vec![PlanStep { id: name.to_string(), label: label.clone(), action, size_hint, needs: vec![] }],
        needs_internet: vec![name.to_string()],
        offline: false,
        consent: format!("UniPi will try several ways to install {label}"),
    }
}

/// `install_component_here`: decision 8's "Install for me" for one component
/// (herdr, tmux).
///
/// It detects again, builds a plan holding only that component's step with the
/// method chain enabled, and runs it through the same [`Installer`], so progress
/// streams and a log is written exactly as for `setup_run`. Nothing here runs
/// unless the user pressed the button: the default plan's tmux step is still
/// `Action::Manual`, so a plain "Install" never touches it.
#[tauri::command]
pub async fn install_component_here(
    name: String,
    channel: tauri::ipc::Channel<Progress>,
    cancel: tauri::State<'_, CancelFlag>,
) -> Result<Outcome, AppErrorDto> {
    let home = crate::host_supervisor::home_dir();
    let report = super::detect_here(&home).await;
    let plan = plan_for_component(&name, &Detected::from(&report));
    let installer = Installer::new(Env::real(home));
    let mut receiver = cancel.0.subscribe();
    let _ = receiver.borrow_and_update();
    let send = channel.clone();
    Ok(installer
        .run(&plan, move |p| { let _ = send.send(p); }, &mut receiver)
        .await)
}

/// `setup_cancel`: the running install stops after the current step.
#[tauri::command]
pub async fn setup_cancel(cancel: tauri::State<'_, CancelFlag>) -> Result<(), AppErrorDto> {
    let _ = cancel.0.send(true);
    Ok(())
}

/// `setup_run`: runs the plan, streaming `Progress` on `channel`.
///
/// `only` is the wizard's consent list: the step ids the user left ticked. It
/// narrows the plan (adding anything those steps `needs`), so an unticked
/// component is genuinely not installed.
#[tauri::command]
pub async fn setup_run(
    channel: tauri::ipc::Channel<Progress>,
    cancel: tauri::State<'_, CancelFlag>,
    only: Option<Vec<String>>,
) -> Result<Outcome, AppErrorDto> {
    let home = crate::host_supervisor::home_dir();
    let plan = plan_here_only(only).await;
    let installer = Installer::new(Env::real(home));
    let mut receiver = cancel.0.subscribe();
    let _ = receiver.borrow_and_update();
    let send = channel.clone();
    Ok(installer
        .run(&plan, move |p| { let _ = send.send(p); }, &mut receiver)
        .await)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nothing_installed() -> Detected {
        Detected {
            node: ComponentState::Missing,
            pi: ComponentState::Missing,
            unipi: ComponentState::Missing,
            kanboard: ComponentState::Unsupported,
            herdr: ComponentState::Missing,
            ..Default::default()
        }
    }

    fn ready() -> Detected {
        Detected {
            node: ComponentState::Ok,
            pi: ComponentState::Ok,
            unipi: ComponentState::Ok,
            kanboard: ComponentState::Ok,
            agent_browser: ComponentState::Ok,
            herdr: ComponentState::Ok,
            ..Default::default()
        }
    }

    fn env_with_root(home: &Path, root: Option<RootHelper>) -> Env {
        Env {
            home: home.to_path_buf(),
            download: Arc::new(|_| Some((b"tarball".to_vec(), "aaa".to_string()))),
            // A download whose hash matches SHASUMS256 (aaa), so the happy path passes verification.
            run: Arc::new(|_, _, _| (Some(0), String::new())),
            shasums: Arc::new(|_| Some("aaa  node-v22.19.0-linux-x64.tar.gz".to_string())),
            extract: Arc::new(|_tarball, dest| {
                // Stand in for `tar`: lay down the two binaries pi's installers run.
                std::fs::create_dir_all(dest.join("bin"))?;
                for bin in ["node", "npm"] {
                    std::fs::write(dest.join("bin").join(bin), "#!/bin/sh\n")?;
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        std::fs::set_permissions(dest.join("bin").join(bin), std::fs::Permissions::from_mode(0o755))?;
                    }
                }
                Ok(())
            }),
            now: 1,
            root,
        }
    }

    /// An `Env` with no root helper at all (a PC without `pkexec`).
    fn env(home: &Path) -> Env {
        env_with_root(home, None)
    }

    /// A `pkexec` stub inside the fake `$HOME`, for a chain that needs root.
    fn with_pkexec(home: &Path) -> Env {
        let bindir = home.join(".local/bin");
        std::fs::create_dir_all(&bindir).unwrap();
        let pkexec = bindir.join("pkexec");
        std::fs::write(&pkexec, "#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&pkexec, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        env_with_root(home, Some(RootHelper { bin: pkexec, shell_script: false }))
    }

    /// Runs `plan` with `env` and answers with the outcome.
    async fn run(env: Env, plan: &Plan) -> Outcome {
        let installer = Installer::new(env);
        let (_tx, mut rx) = watch::channel(false);
        installer.run(plan, |_| {}, &mut rx).await
    }

    #[test]
    fn a_ready_pc_is_a_noop_plan() {
        let plan = plan_for(&ready(), false);
        assert!(plan.is_noop(), "{plan:?}");
        assert!(plan.consent.is_empty());
        assert_eq!(plan.step("tmux").unwrap().action, Action::Manual);
    }

    #[test]
    fn a_bare_pc_installs_everything_in_order() {
        let plan = plan_for(&nothing_installed(), false);
        assert!(!plan.is_noop());
        assert_eq!(plan.needs_internet, vec!["node", "pi", "agent-browser", "unipi", "herdr"]);
        assert_eq!(plan.step("pi").unwrap().needs, vec!["node"]);
        assert_eq!(plan.step("agent-browser").unwrap().needs, vec!["pi"]);
        assert_eq!(plan.step("unipi").unwrap().needs, vec!["pi"]);
        assert_eq!(plan.step("kanboard").unwrap().needs, vec!["unipi"]);
        assert!(plan.consent.contains("Node"));
        assert!(plan.consent.contains("pi (coding agent)"));
        assert_eq!(plan.step("kanboard").unwrap().action, Action::Install);
        assert_eq!(plan.step("kanboard").unwrap().size_hint, None);
    }

    #[test]
    fn a_newer_node_is_left_alone_an_older_one_is_an_update() {
        assert_eq!(plan_for(&ready(), false).step("node").unwrap().action, Action::AlreadyPresent);
        let mut d = ready();
        d.node = ComponentState::Outdated;
        let plan = plan_for(&d, false);
        assert_eq!(plan.step("node").unwrap().action, Action::Update);
        assert!(plan.consent.contains("Node"));
    }

    #[test]
    fn a_working_pi_is_never_reinstalled() {
        let mut d = nothing_installed();
        d.pi = ComponentState::Ok;
        let plan = plan_for(&d, false);
        assert_eq!(plan.step("pi").unwrap().action, Action::AlreadyPresent);
        assert!(!plan.consent.contains("pi (coding agent)"));
    }

    #[test]
    fn xvfb_is_only_installed_when_asked_for() {
        // UNI-299: never in the default plan; "Install for me" plans one step.
        let d = Detected { xvfb: ComponentState::Missing, ..Default::default() };
        assert!(plan_for(&d, true).step("xvfb").is_none() && plan_for(&d, false).step("xvfb").is_none());
        let plan = plan_for_component("xvfb", &d);
        assert_eq!(plan.step("xvfb").unwrap().action, Action::InstallForMe);
        let have = Detected { xvfb: ComponentState::Ok, ..Default::default() };
        assert_ne!(plan_for_component("xvfb", &have).step("xvfb").map(|s| s.action.clone()), Some(Action::InstallForMe));
    }

    #[test]
    fn agent_browser_needs_pi_and_is_pre_ticked_on_a_bare_pc() {
        let plan = plan_for(&nothing_installed(), false);
        let step = plan.step("agent-browser").unwrap();
        assert_eq!(step.action, Action::Install);
        assert_eq!(step.needs, vec!["pi"]);
        assert!(plan.consent.contains("agent-browser"));

        // Already there: left alone, like everything else decision 4 covers.
        let mut d = ready();
        d.agent_browser = ComponentState::Missing;
        assert_eq!(plan_for(&d, false).step("agent-browser").unwrap().action, Action::Install);
        d.agent_browser = ComponentState::Ok;
        assert_eq!(plan_for(&d, false).step("agent-browser").unwrap().action, Action::AlreadyPresent);
    }

    #[tokio::test]
    async fn agent_browser_installs_with_npm_into_pis_prefix() {
        let home = tempfile::tempdir().unwrap();
        let node_dir = home.path().join(".local/share/pi-node/22.19.0/bin");
        std::fs::create_dir_all(&node_dir).unwrap();
        for bin in ["node", "npm"] {
            std::fs::write(node_dir.join(bin), "#!/bin/sh\n").unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(node_dir.join(bin), std::fs::Permissions::from_mode(0o755)).unwrap();
            }
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let _ = symlink(home.path().join(".local/share/pi-node/22.19.0"), home.path().join(".local/share/pi-node/current"));
        }
        let mut log = Vec::new();
        let mut e = env(home.path());
        let seen: Arc<std::sync::Mutex<Vec<Vec<String>>>> = Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen2 = seen.clone();
        e.run = Arc::new(move |_bin, args, _env| {
            seen2.lock().unwrap().push(args.to_vec());
            (Some(0), String::new())
        });
        let state = step_agent_browser(&e, &mut log);
        assert_eq!(state, StepState::Done);
        let calls = seen.lock().unwrap();
        let prefix = pi_npm_prefix(home.path());
        assert!(calls[0].iter().any(|a| a == &format!("--prefix={}", prefix.display())));
        assert!(calls[0].contains(&"agent-browser".to_string()));
        // Verified with `--version`, never `agent-browser install` (that would
        // download its own Chrome — the host's CloakBrowser is used instead).
        assert!(calls[1].contains(&"--version".to_string()));
        assert!(!calls.iter().any(|a| a.len() == 1 && a[0] == "install"));
    }

    #[test]
    fn the_unipi_step_follows_the_updaters_channel() {
        let mut d = nothing_installed();
        d.unipi_version = Some("3.0.0-alpha.41".into());
        d.unipi_spec = Some("npm:@pi-unipi/unipi@alpha".into());
        assert!(plan_for(&d, false).step("unipi").unwrap().label.contains("alpha"));
        let mut d = nothing_installed();
        d.unipi_version = Some("3.0.0".into());
        assert!(plan_for(&d, false).step("unipi").unwrap().label.contains("latest"));
    }

    #[test]
    fn pi_has_a_chain_so_the_sheet_can_offer_another_way() {
        // N1 then N2, the same order the automatic step follows: the failure
        // sheet's "Try another way" runs this list, so it must exist and be
        // ordered.
        let pi = methods_for("pi");
        assert_eq!(pi.iter().map(|m| m.name.as_str()).collect::<Vec<_>>(), vec!["pi.dev installer", "npm"]);
        assert!(pi.iter().all(|m| !m.root), "neither way needs root");
        // …and it is a plan, so `install_component_here("pi")` really runs it.
        let mut d = nothing_installed();
        d.pi = ComponentState::Missing;
        assert_eq!(plan_for_component("pi", &d).step("pi").unwrap().action, Action::InstallForMe);
        let mut d = nothing_installed();
        d.pi = ComponentState::Ok;
        assert_eq!(plan_for_component("pi", &d).step("pi").unwrap().action, Action::AlreadyPresent);
    }

    #[tokio::test]
    async fn the_pi_chain_falls_through_from_the_installer_to_npm() {
        let home = tempfile::tempdir().unwrap();
        let node = home.path().join(".local/share/pi-node/22.19.0");
        std::fs::create_dir_all(node.join("bin")).unwrap();
        for bin in ["node", "npm"] {
            std::fs::write(node.join("bin").join(bin), "#!/bin/sh\n").unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(node.join("bin").join(bin), std::fs::Permissions::from_mode(0o755)).unwrap();
            }
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            symlink(&node, home.path().join(".local/share/pi-node/current")).unwrap();
        }
        // N1 fails, N2 works: running the chain as a step reports both.
        let mut e = env(home.path());
        e.run = Arc::new(|_bin, args, _env| {
            if args.iter().any(|a| a.contains(PI_INSTALLER_URL)) {
                (Some(1), "N1 blew up".into())
            } else {
                (Some(0), String::new())
            }
        });
        let mut plan = plan_for_component("pi", &Detected { pi: ComponentState::Missing, ..Default::default() });
        plan.steps.retain(|s| s.id == "pi");
        let installer = Installer::new(e);
        let (_tx, mut rx) = watch::channel(false);
        let outcome = installer.run(&plan, |_| {}, &mut rx).await;
        let step = &outcome.steps[0];
        assert_eq!(step.state, StepState::Done, "{outcome:?}");
        assert_eq!(step.attempted, vec!["pi.dev installer", "npm"]);
        assert_eq!(step.fallback_command.as_deref(), Some("npm install --global --ignore-scripts @earendil-works/pi-coding-agent"));
    }

    #[test]
    fn the_wizards_consent_list_narrows_the_plan_and_keeps_dependencies() {
        let plan = plan_for(&nothing_installed(), false);
        // Nothing unticked: every step the core planned.
        assert!(narrow(&plan, None).steps.len() == plan.steps.len());
        // Only Node and pi ticked: the rest is really gone, so an unticked
        // component is not installed behind the user's back.
        let picked = narrow(&plan, Some(vec!["node".into(), "pi".into()]));
        assert_eq!(picked.steps.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), vec!["node", "pi"]);
        assert!(picked.step("herdr").is_none() && picked.step("tmux").is_none());
        // UniPi needs pi: ticking only UniPi pulls the step it needs in, rather
        // than failing with "skipped, needs pi".
        let unipi = narrow(&plan, Some(vec!["unipi".into()]));
        let ids: Vec<&str> = unipi.steps.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, vec!["node", "pi", "unipi"], "dependencies come first");
        assert!(!unipi.needs_internet.is_empty(), "the consent line still lists what needs the network");
    }

    #[test]
    fn offline_marks_the_steps_that_need_the_network() {
        let plan = plan_for(&nothing_installed(), true);
        assert!(plan.offline);
        assert_eq!(plan.needs_internet, vec!["node", "pi", "agent-browser", "unipi", "herdr"]);
    }

    #[tokio::test]
    async fn a_happy_path_runs_every_step_and_logs_it() {
        let home = tempfile::tempdir().unwrap();
        // pi's installer puts `pi` on the user's PATH — the fake N1 does that,
        // so the UniPi step afterwards finds it.
        let mut e = env(home.path());
        let home_dir = home.path().to_path_buf();
        let inner = e.run.clone();
        e.run = Arc::new(move |bin, args, extra| {
            let out = inner(bin, args, extra);
            let bin_dir = home_dir.join(".local/bin");
            let _ = std::fs::create_dir_all(&bin_dir);
            #[cfg(unix)]
            fn make_executable(path: &Path) {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755));
            }
            // pi's installer puts `pi` on the user's PATH…
            if args.iter().any(|a| a.contains(PI_INSTALLER_URL)) {
                let _ = std::fs::write(bin_dir.join("pi"), "#!/bin/sh\n");
                #[cfg(unix)]
                make_executable(&bin_dir.join("pi"));
            }
            // …and the UniPi extensions bring the board binary with them.
            if args.iter().any(|a| a.starts_with("npm:@pi-unipi/unipi")) {
                let _ = std::fs::write(bin_dir.join("unipi-kanboard"), "#!/bin/sh\n");
                #[cfg(unix)]
                make_executable(&bin_dir.join("unipi-kanboard"));
            }
            out
        });
        let plan = plan_for(&nothing_installed(), false);
        let installer = Installer::new(e);
        let (_tx, mut rx) = watch::channel(false);
        let mut seen = Vec::new();
        let outcome = installer.run(&plan, |p| seen.push(p), &mut rx).await;
        assert!(!outcome.cancelled);
        assert!(
            outcome.steps.iter().all(|s| matches!(s.state, StepState::Done | StepState::Skipped { .. })),
            "{outcome:?}"
        );
        assert!(outcome.steps.iter().any(|s| s.step == "node" && s.state == StepState::Done));
        assert!(seen.iter().any(|p| p.step == "pi" && p.state == StepState::Running));
        assert!(home.path().join(".unipi/logs/setup-1.log").exists());
    }

    #[tokio::test]
    async fn a_checksum_mismatch_fails_without_installing() {
        let home = tempfile::tempdir().unwrap();
        let mut e = env(home.path());
        e.download = Arc::new(|_| Some((b"tarball".to_vec(), "999".to_string())));
        let mut plan = plan_for(&nothing_installed(), false);
        plan.steps.retain(|s| s.id == "node");
        let installer = Installer::new(e);
        let (_tx, mut rx) = watch::channel(false);
        let outcome = installer.run(&plan, |_| {}, &mut rx).await;
        let node = outcome.steps.iter().find(|s| s.step == "node").unwrap();
        assert!(matches!(&node.state, StepState::Failed { message } if message.contains("checksum")), "{node:?}");
        assert!(!home.path().join(".local/share/pi-node/22.19.0").exists());
        assert!(!home.path().join(".tmp-node-22.19.0").exists());
    }

    #[tokio::test]
    async fn cancel_stops_between_steps_and_the_rest_is_skipped() {
        let home = tempfile::tempdir().unwrap();
        let plan = plan_for(&nothing_installed(), false);
        let installer = Installer::new(env(home.path()));
        let (tx, mut rx) = watch::channel(true);
        let outcome = installer.run(&plan, |_| {}, &mut rx).await;
        assert!(outcome.cancelled);
        assert!(outcome.steps.iter().all(|s| matches!(s.state, StepState::Skipped { .. })));
        drop(tx);
    }

    #[tokio::test]
    async fn a_failed_step_skips_what_depends_on_it() {
        let home = tempfile::tempdir().unwrap();
        let mut e = env(home.path());
        e.shasums = Arc::new(|_| None);
        let plan = plan_for(&nothing_installed(), false);
        let installer = Installer::new(e);
        let (_tx, mut rx) = watch::channel(false);
        let outcome = installer.run(&plan, |_| {}, &mut rx).await;
        let get = |id: &str| outcome.steps.iter().find(|s| s.step == id).unwrap().state.clone();
        assert!(matches!(get("node"), StepState::Failed { .. }), "{outcome:?}");
        assert!(matches!(get("pi"), StepState::Skipped { reason } if reason.contains("node")), "{outcome:?}");
        assert!(matches!(get("unipi"), StepState::Skipped { reason } if reason.contains("pi")), "{outcome:?}");
        assert!(matches!(get("kanboard"), StepState::Skipped { reason } if reason.contains("unipi")), "{outcome:?}");
    }

    #[tokio::test]
    async fn a_rerun_is_a_noop() {
        let home = tempfile::tempdir().unwrap();
        let plan = plan_for(&ready(), false);
        assert!(plan.is_noop());
        let installer = Installer::new(env(home.path()));
        let (_tx, mut rx) = watch::channel(false);
        let outcome = installer.run(&plan, |_| {}, &mut rx).await;
        // Only tmux is skipped — it is the one step we never do ourselves.
        assert!(
            outcome.steps.iter().all(|s| s.state == StepState::Done || s.step == "tmux"),
            "{outcome:?}"
        );
        assert!(!home.path().join(".local").exists());
    }

    #[tokio::test]
    async fn n1_failure_falls_back_to_n2() {
        let home = tempfile::tempdir().unwrap();
        let node_dir = home.path().join(".local/share/pi-node/22.19.0");
        std::fs::create_dir_all(node_dir.join("bin")).unwrap();
        std::fs::write(node_dir.join("bin/node"), "#!/bin/sh
").unwrap();
        std::fs::write(node_dir.join("bin/npm"), "#!/bin/sh
").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for bin in ["node", "npm"] {
                std::fs::set_permissions(node_dir.join("bin").join(bin), std::fs::Permissions::from_mode(0o755)).unwrap();
            }
            use std::os::unix::fs::symlink;
            symlink(&node_dir, home.path().join(".local/share/pi-node/current")).unwrap();
        }
        let calls = Arc::new(std::sync::Mutex::new(Vec::<Vec<String>>::new()));
        let seen = calls.clone();
        let mut e = env(home.path());
        e.run = Arc::new(move |_bin, args, _env| {
            seen.lock().unwrap().push(args.to_vec());
            if args.iter().any(|a| a.contains(PI_INSTALLER_URL)) {
                (Some(1), "N1 blew up".into())
            } else {
                (Some(0), String::new())
            }
        });
        let mut plan = plan_for(&nothing_installed(), false);
        plan.steps.retain(|s| s.id == "pi");
        let installer = Installer::new(e);
        let (_tx, mut rx) = watch::channel(false);
        let outcome = installer.run(&plan, |_| {}, &mut rx).await;
        assert_eq!(outcome.steps[0].state, StepState::Done, "{outcome:?}");
        let invoked = calls.lock().unwrap();
        assert!(invoked.iter().any(|a| a.iter().any(|x| x.contains(PI_INSTALLER_URL))), "N1 was tried");
        assert!(invoked.iter().any(|a| a.iter().any(|x| x == "install")), "N2 was tried");
    }

    // ------------------------------------------------ decision 8: the chain

    #[test]
    fn the_default_plan_never_installs_tmux_automatically() {
        // Regression guard for decision 8: the chain runs only from
        // `Action::InstallForMe`, which the default plan never produces.
        for d in [nothing_installed(), ready()] {
            let plan = plan_for(&d, false);
            assert_eq!(plan.step("tmux").unwrap().action, Action::Manual);
            assert!(!plan.steps.iter().any(|s| s.action == Action::InstallForMe));
        }
        let plan = plan_for(&nothing_installed(), false);
        assert!(!plan.consent.contains("tmux"));
        assert!(!plan.needs_internet.contains(&"tmux".to_string()));
        // herdr, preselected, is a real install; tmux is not, whatever the PC has.
        let mut d = nothing_installed();
        d.herdr = ComponentState::Missing;
        assert_eq!(plan_for(&d, false).step("herdr").unwrap().action, Action::Install);
    }

    #[test]
    fn the_chain_order_per_component() {
        let herdr = methods_for("herdr");
        assert_eq!(
            herdr.iter().map(|m| m.name.as_str()).collect::<Vec<_>>(),
            vec!["herdr installer", "static binary in ~/.local/bin"]
        );
        // herdr needs no root: its installer runs as the user, and so does
        // `~/.local/bin`.
        assert!(herdr.iter().all(|m| !m.root));
        assert!(herdr[1].diy.contains("~/.local/bin/herdr"));
        assert_eq!(herdr[0].kind, MethodKind::Script(format!("curl -fsSL {HERDR_INSTALLER_URL} | sh")));
        assert_eq!(herdr[1].kind, MethodKind::StaticBinary);

        let tmux = methods_for("tmux");
        match PkgManager::detect() {
            Some(manager) => {
                // Root first, and the only method: there is no tmux tarball.
                assert_eq!(tmux.len(), 1);
                assert_eq!(tmux[0].kind, MethodKind::Package(manager));
                assert_eq!(tmux[0].root, manager.needs_root());
                assert_eq!(tmux[0].diy, fallback_command("tmux"));
                if manager.needs_root() {
                    assert!(tmux[0].name.contains("pkexec"), "{}", tmux[0].name);
                } else {
                    // Homebrew's prefix belongs to the user: no root, no pkexec.
                    assert!(tmux[0].name.contains("no root"), "{}", tmux[0].name);
                }
            }
            None => assert!(tmux.is_empty(), "nothing to run: the command is the answer"),
        }
        assert!(methods_for("node").is_empty());
    }

    #[test]
    fn the_install_argv_per_package_manager_and_per_os() {
        assert_eq!(PkgManager::Apt.program(), "apt-get");
        assert_eq!(PkgManager::Apt.install_args("tmux"), vec!["install", "-y", "tmux"]);
        assert_eq!(PkgManager::Dnf.install_args("tmux"), vec!["install", "-y", "tmux"]);
        assert_eq!(PkgManager::Pacman.install_args("tmux"), vec!["-S", "--noconfirm", "tmux"]);
        assert_eq!(PkgManager::Brew.install_args("tmux"), vec!["install", "tmux"]);
        assert!(!PkgManager::Brew.needs_root());
        assert!(PkgManager::Apt.needs_root() && PkgManager::Dnf.needs_root() && PkgManager::Pacman.needs_root());

        // Linux: `pkexec` takes the command as argv, unchanged.
        let pkexec = RootHelper { bin: PathBuf::from("/usr/bin/pkexec"), shell_script: false };
        let apt = manager_command(&PkgManager::Apt, "tmux");
        assert_eq!(pkexec.argv(&apt), apt, "pkexec apt-get install -y tmux");
        // macOS: osascript wraps it in `with administrator privileges`.
        let osascript = RootHelper { bin: PathBuf::from("/usr/bin/osascript"), shell_script: true };
        assert_eq!(
            osascript.argv(&manager_command(&PkgManager::Brew, "tmux")),
            vec![
                "-e".to_string(),
                "do shell script \"brew install tmux\" with administrator privileges".to_string(),
            ]
        );
    }

    /// `apt-get install -y tmux`: a manager's full command, before any wrapper.
    fn manager_command(manager: &PkgManager, package: &str) -> Vec<String> {
        let mut argv = vec![manager.program().to_string()];
        argv.extend(manager.install_args(package));
        argv
    }

    #[test]
    fn pkexec_is_resolved_from_the_path_then_the_standard_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let stub = dir.path().join("pkexec");
        std::fs::write(&stub, "#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        assert_eq!(pkexec_in(Some(dir.path().as_os_str()), &[]), Some(stub));
        // No pkexec on the PATH and no standard dirs to fall back to: "not
        // there", which is what makes a root method unavailable.
        let empty = tempfile::tempdir().unwrap();
        assert_eq!(pkexec_in(Some(empty.path().as_os_str()), &[]), None);
        assert_eq!(pkexec_in(None, &[]), None);
        // …and the standard locations are the fallback, as for `systemctl`.
        let standard = [PathBuf::from("/usr/bin")];
        let expected = PathBuf::from("/usr/bin/pkexec");
        if expected.exists() {
            assert_eq!(pkexec_in(None, &standard.iter().map(Path::new).collect::<Vec<_>>()), Some(expected));
        }
    }

    #[tokio::test]
    async fn a_missing_pkexec_skips_the_method_instead_of_failing_it() {
        let home = tempfile::tempdir().unwrap();
        let calls = Arc::new(std::sync::Mutex::new(Vec::<(PathBuf, Vec<String>)>::new()));
        let seen = calls.clone();
        let mut e = env(home.path());
        e.run = Arc::new(move |bin, args, _| {
            seen.lock().unwrap().push((bin.to_path_buf(), args.to_vec()));
            (Some(0), String::new())
        });
        e.root = None;
        let plan = plan_for_component("tmux", &nothing_installed());
        let outcome = run(e, &plan).await;
        let tmux = outcome.steps.iter().find(|s| s.step == "tmux").unwrap();
        let reason = match &tmux.state {
            StepState::Skipped { reason } => reason.clone(),
            other => panic!("a missing pkexec is not a failure: {other:?}"),
        };
        assert!(reason.contains(&fallback_command("tmux")), "{reason}");
        assert_eq!(tmux.fallback_command.as_deref(), Some(fallback_command("tmux").as_str()));
        assert!(tmux.attempted.is_empty());
        assert!(calls.lock().unwrap().is_empty(), "nothing was run");
    }

    #[tokio::test]
    async fn install_for_me_tmux_asks_pkexec_then_falls_back_to_the_command() {
        let home = tempfile::tempdir().unwrap();
        let calls = Arc::new(std::sync::Mutex::new(Vec::<(PathBuf, Vec<String>)>::new()));
        let seen = calls.clone();
        let mut e = with_pkexec(home.path());
        e.run = Arc::new(move |bin, args, _| {
            seen.lock().unwrap().push((bin.to_path_buf(), args.to_vec()));
            (Some(100), "polkit-agent: not authorised".into())
        });
        let plan = plan_for_component("tmux", &nothing_installed());
        let outcome = run(e, &plan).await;
        let tmux = outcome.steps.iter().find(|s| s.step == "tmux").unwrap();
        let StepState::Failed { message } = &tmux.state else { panic!("{outcome:?}") };
        assert!(message.contains("not authorised"), "{message}");
        // The fallback command is carried, so the UI can offer Copy.
        assert!(message.contains(&fallback_command("tmux")), "{message}");
        assert_eq!(tmux.fallback_command.as_deref(), Some(fallback_command("tmux").as_str()));
        // `pkexec apt-get install -y tmux`, through the stub.
        let manager = PkgManager::detect().expect("this PC's package manager is known");
        let invoked = calls.lock().unwrap();
        assert_eq!(invoked.len(), 1, "{invoked:?}");
        assert_eq!(invoked[0].0, home.path().join(".local/bin/pkexec"));
        assert_eq!(invoked[0].1, manager_command(&manager, "tmux"));
    }

    #[tokio::test]
    async fn install_for_me_falls_through_to_the_second_method() {
        let home = tempfile::tempdir().unwrap();
        let calls = Arc::new(std::sync::Mutex::new(Vec::<(PathBuf, Vec<String>)>::new()));
        let downloads = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let seen = calls.clone();
        let urls = downloads.clone();
        let mut e = env(home.path());
        // herdr's installer cannot be reached…
        e.run = Arc::new(move |bin, args, _| {
            seen.lock().unwrap().push((bin.to_path_buf(), args.to_vec()));
            if args.iter().any(|a| a.contains(HERDR_INSTALLER_URL)) {
                (None, "curl: (6) could not resolve host".into())
            } else {
                (Some(0), String::new())
            }
        });
        // …so the static release binary is downloaded instead.
        e.download = Arc::new(move |url| {
            urls.lock().unwrap().push(url.to_string());
            Some((b"#!/bin/sh\necho herdr 0.9.0\n".to_vec(), "decaf".to_string()))
        });
        let plan = plan_for_component("herdr", &nothing_installed());
        assert_eq!(plan.steps.len(), 1);
        assert_eq!(plan.step("herdr").unwrap().action, Action::InstallForMe);
        let outcome = run(e, &plan).await;
        let herdr = outcome.steps.iter().find(|s| s.step == "herdr").unwrap();
        assert_eq!(herdr.state, StepState::Done, "{outcome:?}");
        assert_eq!(herdr.attempted, vec!["herdr installer", "static binary in ~/.local/bin"]);
        // The asset landed in `~/.local/bin`, 0755, and answers --version.
        let bin = crate::host_supervisor::bin_dir(home.path()).join("herdr");
        assert_eq!(std::fs::read_to_string(&bin).unwrap(), "#!/bin/sh\necho herdr 0.9.0\n");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&bin).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o755, "{mode:o}");
            // Written through a temp name, so nothing half-installed is left behind.
            assert!(!bin.with_file_name(".herdr.new").exists());
        }
        let invoked = calls.lock().unwrap();
        assert!(invoked.iter().any(|(bin, args)| bin.file_name() == Some("herdr".as_ref())
            && args.iter().any(|a| a == "--version")),
            "{invoked:?}");
        assert_eq!(*downloads.lock().unwrap(), vec![format!("{HERDR_RELEASES}/{}", herdr_asset())]);
    }

    #[tokio::test]
    async fn install_for_me_failure_carries_the_last_error_and_the_fallback() {
        let home = tempfile::tempdir().unwrap();
        let mut e = env(home.path());
        e.run = Arc::new(|_, _, _| (Some(1), "E: Could not get lock".into()));
        e.download = Arc::new(|_| None);
        let plan = plan_for_component("herdr", &nothing_installed());
        let outcome = run(e, &plan).await;
        let herdr = outcome.steps.iter().find(|s| s.step == "herdr").unwrap();
        let StepState::Failed { message } = &herdr.state else { panic!("{outcome:?}") };
        // The last error, then the command to run by hand.
        assert!(message.contains("could not download"), "{message}");
        assert!(message.contains("herdr.dev/install.sh"), "{message}");
        assert_eq!(
            herdr.fallback_command.as_deref(),
            Some(format!("curl -fsSL {HERDR_INSTALLER_URL} | sh").as_str())
        );
        assert_eq!(herdr.attempted.len(), 2);
        // Nothing half-installed: a failed download writes nothing.
        assert!(!crate::host_supervisor::bin_dir(home.path()).join("herdr").exists());
        assert!(herdr.log_tail.iter().any(|l| l.contains("downloading")), "{:?}", herdr.log_tail);
    }

    #[test]
    fn install_for_me_plans_one_step_with_the_chain_enabled() {
        let plan = plan_for_component("herdr", &nothing_installed());
        assert_eq!(plan.steps.len(), 1);
        assert_eq!(plan.step("herdr").unwrap().action, Action::InstallForMe);
        assert!(plan.needs_internet.contains(&"herdr".to_string()));
        assert!(!plan.is_noop());
        // Already there: nothing to do.
        assert_eq!(plan_for_component("herdr", &ready()).step("herdr").unwrap().action, Action::AlreadyPresent);
        // tmux, asked for by name, becomes a chain step.
        let mut d = nothing_installed();
        assert_eq!(plan_for_component("tmux", &d).step("tmux").unwrap().action, Action::InstallForMe);
        d.tmux = ComponentState::Ok;
        assert_eq!(plan_for_component("tmux", &d).step("tmux").unwrap().action, Action::AlreadyPresent);
        // A component with no chain is left alone rather than fake-planned.
        assert_eq!(plan_for_component("node", &nothing_installed()).step("node").unwrap().action, Action::AlreadyPresent);
    }

    #[test]
    fn node_layout_matches_pis_own_installer() {
        let home = Path::new("/home/t");
        assert_eq!(node_dir(home, "22.19.0"), home.join(".local/share/pi-node/22.19.0"));
        assert_eq!(pi_npm_prefix(home), home.join(".pi/agent/npm"));
        assert_eq!(log_path(home, 7), home.join(".unipi/logs/setup-7.log"));
        let tarball = node_tarball_name("22.19.0");
        assert!(tarball.starts_with("node-v22.19.0-"), "{tarball}");
        assert!(tarball.ends_with(".tar.gz"), "{tarball}");
    }
}
